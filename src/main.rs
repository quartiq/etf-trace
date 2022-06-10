mod etf;

use probe_rs::{
    architecture::arm::{
        component::{Dwt, Itm, TraceFunnel},
        memory::PeripheralType,
    },
    Probe,
};

const CSTF_BASE_ADDRESS: u64 = 0xE00F_3000;

fn main() {
    env_logger::init();
    let probes = Probe::list_all();
    let probe = probes[0].open().unwrap();

    let mut session = probe
        .attach("STM32H743ZITx", probe_rs::Permissions::default())
        .unwrap();

    let components = session.get_arm_components().unwrap();

    // Disable tracing of the H7 core.
    {
        let mut core = session.core(0).unwrap();
        probe_rs::architecture::arm::component::disable_swv(&mut core).unwrap();
    }

    let interface = session.get_arm_interface().unwrap();

    // Configure the trace funnel to the ETF. Do not configure the SWO trace funnel.
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

    // Configure the ITM
    let mut itm = Itm::new(
        interface,
        components
            .iter()
            .find_map(|component| component.find_component(PeripheralType::Itm))
            .unwrap(),
    );

    itm.unlock().unwrap();
    itm.tx_enable().unwrap();

    // Configure the DWT
    let mut dwt = Dwt::new(
        interface,
        components
            .iter()
            .find_map(|component| component.find_component(PeripheralType::Dwt))
            .unwrap(),
    );

    dwt.enable().unwrap();
    dwt.enable_exception_trace().unwrap();

    // Enable tracing of the H7 core.
    {
        let mut core = session.core(0).unwrap();
        probe_rs::architecture::arm::component::enable_tracing(&mut core).unwrap();
    }

    let interface = session.get_arm_interface().unwrap();

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

    etf.disable_capture().unwrap();
    println!("Read ETF FIFO Size: {}", etf.fifo_size().unwrap());
    etf.set_mode(etf::Mode::Software).unwrap();
    println!("Enabling ETF capture");
    println!("ETF READY: {}", etf.is_ready().unwrap());
    etf.enable_capture().unwrap();

    // Wait until ETB buffer fills.
    println!("Waiting for capture FIFO to fill");
    println!("ETF READY: {}", etf.is_ready().unwrap());
    while !etf.is_full().unwrap() {
        let level = etf.fill_level().unwrap();
        if level > 0 {
            println!("ETF_CBUFLVL: {}", level);
        }
    }
    println!("ETF capture complete");
    etf.stop_on_flush(true).unwrap();
    etf.manual_flush().unwrap();

    // Extract ETB data.
    let mut etf_data = vec![];
    while let Some(data) = etf.read().unwrap() {
        etf_data.push(data);
        println!("{:8X}", data);
    }

    etf.disable_capture().unwrap();

    // TODO: Decode ETF data through the ITM decoder.
    // TODO: Export ETF data to a file.
}
