//! STM32H7 tracing with the Embedded Trace FIFO
//!
//! # Design
//! The STM32H7 tracing infrastructure allows trace data to be generated on the SWO output, which
//! is a UART output to the debug probe. Because of the nature of this output, the throughput is
//! inherently limited. Additionally, there is very little buffering between ITM packet generation
//! and SWO output, so even a small amount of trace data generated in a short time interval can
//! result in the trace data overflowing in the SWO data path.
//!
//! To work around issues with buffering and throughput of the SWO output, this program provides a
//! mechanism to instead capture ITM trace data within the Embedded Trace FIFO (ETF). The ETC is a
//! 4KB FIFO stored in SRAM that can be used to buffer data before draining the trace data to an
//! external source. The ETF supports both draining data to the TPIU via a parallel trace hardware
//! interface as well as through the debug registers.
//!
//! This program uses the ETF in "software" mode with no external tracing utilities required.
//! Instead, the ETF is used to buffer up a trace which is then read out from the device via the
//! debug probe.
mod etf;
mod coresight_frame;

use coresight_frame::CoresightFrameBuffer;

use clap::Parser;
use std::io::Write;

use probe_rs::{
    architecture::arm::{
        component::{Dwt, Itm, TraceFunnel},
        memory::PeripheralType,
    },
    Probe,
};

// The base address of the ETF trace funnel.
const CSTF_BASE_ADDRESS: u64 = 0xE00F_3000;

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short, long)]
    output: String,
}

fn main() {
    env_logger::init();

    let cli = Args::parse();

    let probes = Probe::list_all();
    let probe = probes[0].open().unwrap();

    let mut session = probe
        .attach("STM32H743ZITx", probe_rs::Permissions::default())
        .unwrap();

    let components = session.get_arm_components().unwrap();

    // Enable tracing of the H7 core.
    {
        let mut core = session.core(0).unwrap();
        probe_rs::architecture::arm::component::enable_tracing(&mut core).unwrap();
    }

    let interface = session.get_arm_interface().unwrap();

    // Configure the trace funnel to the ETF. There are two trace funnels in the STM32H7 system
    // that are only distinguishable via the number of input ports and the base address. One is the
    // SWO funnel and the other is the ETF funnel.
    let cstf = components
        .iter()
        .find_map(|comp| {
            for component in comp.iter() {
                let id = component.component.id();
                if id.peripheral_id().is_of_type(PeripheralType::TraceFunnel)
                    && id.component_address() == CSTF_BASE_ADDRESS
                {
                    return Some(component);
                }
            }
            None
        })
        .unwrap();

    // Enable the ITM port of the trace funnel.
    let mut trace_funnel = TraceFunnel::new(interface, cstf);
    trace_funnel.unlock().unwrap();
    trace_funnel.enable_port(0b10).unwrap();

    // Configure the ITM to generate trace data from the DWT.
    let mut itm = Itm::new(
        interface,
        components
            .iter()
            .find_map(|component| component.find_component(PeripheralType::Itm))
            .unwrap(),
    );

    itm.unlock().unwrap();
    itm.tx_enable().unwrap();

    // Configure the DWT to trace exception entry and exit.
    let mut dwt = Dwt::new(
        interface,
        components
            .iter()
            .find_map(|component| component.find_component(PeripheralType::Dwt))
            .unwrap(),
    );

    dwt.enable().unwrap();
    dwt.enable_exception_trace().unwrap();

    // Configure the ETF.
    let etf = components
        .iter()
        .find_map(|comp| {
            for component in comp.iter() {
                let id = component.component.id().peripheral_id();
                let code = id.jep106().and_then(|jep106| jep106.get()).unwrap_or("");
                let part = id.part();
                if code == "ARM Ltd" && part == 0x961 {
                    return Some(component);
                }
            }
            None
        })
        .unwrap();

    let mut etf = etf::EmbeddedTraceFifo::new(interface, etf);
    let fifo_size = etf.fifo_size().unwrap();

    etf.disable_capture().unwrap();
    etf.set_mode(etf::Mode::Software).unwrap();
    etf.enable_capture().unwrap();

    // Wait until ETB buffer fills.
    println!("Waiting for capture to complete");
    while !etf.full().unwrap() {
        let level = etf.fill_level().unwrap();
        if level > 0 {
            println!("Received: {} of {} bytes", level, fifo_size);
        }
    }

    // This sequence is taken from "CoreSight Trace Memory Controller Technical Reference Manual"
    // Section 2.2.2 "Software FIFO Mode". Without following this procedure, the trace data does
    // not properly stop even after disabling capture.
    println!("Trace capture complete");
    etf.stop_on_flush(true).unwrap();
    etf.manual_flush().unwrap();

    let mut output = std::fs::File::create(&cli.output).unwrap();

    let mut frame_buffer = CoresightFrameBuffer::new();

    // Extract ETB data.
    while let Some(data) = etf.read().unwrap() {
        // The ETF is specified in "CoreSight Trace Memory Controller Technical Reference Manual"
        // Section 2.2.3. The trace funnel stores trace data in CoreSight frames, which contain more
        // metadata about the source of the trace than raw ITM data. Trace frames are described in
        // the ARM CoreSight Architecture Specification v3.0 D4.2. Each frame is composed of 4
        // bytes.

        // In this case, we are only tracing information from the ITM/DWT and we are not using the
        // ETM port (its disabled in the trace funnel). The rtic-scope and ITM decode utilities
        // process raw ITM data as opposed to CoreSight frames, so we need to extract the ITM trace
        // data and strip away the coresight framing.
        frame_buffer.add_word(data);

        // Write all of the available data from the frame buffer.
        output.write_all(&frame_buffer.drain()).unwrap();
    }

    etf.disable_capture().unwrap();

    drop(output);

    let reader = itm::Decoder::new(std::fs::File::open(cli.output).unwrap(), itm::DecoderOptions { ignore_eof: false }).singles();

    for packet in reader {
        println!("Packet: {:?}", packet);
    }
}
