//! ITM/DWT tracing with the Embedded Trace FIFO
//!
//! # Design
//! The tracing infrastructure allows trace data to be generated on the SWO
//! output, which is a UART output to the debug probe. Because of the nature of
//! this output, the throughput is inherently limited. Additionally, there is
//! very little buffering between ITM packet generation and SWO output, so even
//! a small amount of trace data generated in a short time interval can result
//! in the trace data overflowing in the SWO data path.
//!
//! Alternatively there is a wide, source synchronous and fast output through
//! the TPIU. But that requires the availability of certain pins and a logic
//! analyzer or a capable probe to capture the data.
//!
//! To work around issues with buffering and throughput of the SWO output and to
//! avoid the need for, special hardware, this program provides a mechanism to
//! instead capture DWT/ITM trace data within the Embedded Trace Buffer/FIFO
//! (ETB/ETF). The ETF is a 4 KiB (usually) FIFO in SRAM that can be used to
//! buffer data before draining the trace data to an external source. The ETF
//! supports draining data through the debug registers.
//!
//! This program uses the ETF in "software" mode with no external tracing
//! utilities required. Instead, the ETF is used to buffer up a trace which is
//! then read out from the device via the debug probe.
use anyhow::Context;
use clap::Parser;
use log::info;
use probe_rs::{architecture::arm::component::TraceSink, Error, Probe};
use std::io::{Seek, Write};

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short, long, default_value = "STM32H743ZITx")]
    target: String,
    #[clap(short, long)]
    output: String,
    #[clap(short, long, default_value_t = 400_000_000)]
    coreclk: u32,
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("etf_trace=info"))
        .init();

    let cli = Args::parse();

    let probe = Probe::list_all()
        .get(0)
        .ok_or(Error::UnableToOpenProbe("not found"))?
        .open()?;

    let mut session = probe.attach(cli.target, probe_rs::Permissions::default())?;
    session.setup_tracing(0, TraceSink::TraceMemory)?;

    let itm_trace = session.read_trace_data()?;

    let mut output = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(cli.output)?;

    output.write_all(&itm_trace)?;

    // Parse ITM trace and print.
    let mut itm_trace = std::io::BufReader::new(std::io::Cursor::new(itm_trace.as_slice()));
    itm_trace.rewind()?;
    let decoder = itm::Decoder::new(itm_trace, itm::DecoderOptions { ignore_eof: false });
    let timestamp_cfg = itm::TimestampsConfiguration {
        clock_frequency: cli.coreclk,
        lts_prescaler: itm::LocalTimestampOptions::Enabled,
        expect_malformed: false,
    };
    for packets in decoder.timestamps(timestamp_cfg) {
        match packets {
            Err(e) => return Err(e).context("Decoder error"),
            Ok(packets) => info!("{packets:?}"),
        }
    }

    Ok(())
}
