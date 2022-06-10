use probe_rs::{
    architecture::arm::{component::DebugRegister, memory::CoresightComponent, ArmProbeInterface},
    Error,
};

use bitfield::bitfield;

const REGISTER_OFFSET_RSZ: u32 = 0x04;
const REGISTER_OFFSET_RRD: u32 = 0x10;
const REGISTER_OFFSET_CTL: u32 = 0x20;
const REGISTER_OFFSET_LBUFLVL: u32 = 0x2C;
const REGISTER_OFFSET_CBUFLVL: u32 = 0x30;

#[repr(u8)]
pub enum Mode {
    /// Trace memory is used as a circular buffer. When the buffer fills, incoming trace data will
    /// overwrite older trace memory until the trace is stopped.
    Circular = 0b00,

    /// The trace memory is used as a FIFO that can be manually read through the RRD register. When
    /// the buffer fills, the incoming trace stream is stalled.
    Software = 0b01,

    /// Trace memory is used as a FIFO that is drained through hardware to the TPIU. Trace data
    /// is captured until the buffer fills, at which point the incoming trace stream is stalled.
    /// Whenever the buffer is non-empty, trace data is drained to the TPIU.
    Hardware = 0b10,
}

pub struct EmbeddedTraceFifo<'a> {
    component: &'a CoresightComponent,
    interface: &'a mut Box<dyn ArmProbeInterface>,
}

impl<'a> EmbeddedTraceFifo<'a> {
    /// Construct a new embedded trace fifo controller.
    pub fn new(
        interface: &'a mut Box<dyn ArmProbeInterface>,
        component: &'a CoresightComponent,
    ) -> Self {
        Self {
            component,
            interface,
        }
    }

    /// Configure the FIFO operational mode.
    ///
    /// # Args
    /// * `mode` - The desired operational mode of the FIFO.
    pub fn set_mode(&mut self, mode: Mode) -> Result<(), Error> {
        let mut mode_reg = EtfMode::load(self.component, self.interface)?;
        mode_reg.set_mode(mode as u8);
        mode_reg.store(self.component, self.interface)?;
        Ok(())
    }

    /// Enable trace captures using the FIFO.
    pub fn enable_capture(&mut self) -> Result<(), Error> {
        self.component
            .write_reg(self.interface, REGISTER_OFFSET_CTL, 1)?;
        Ok(())
    }

    /// Disable trace captures using the FIFO.
    pub fn disable_capture(&mut self) -> Result<(), Error> {
        self.component
            .write_reg(self.interface, REGISTER_OFFSET_CTL, 0)?;
        Ok(())
    }

    /// Attempt to read a value out of the FIFO
    pub fn read(&mut self) -> Result<Option<u32>, Error> {
        // Read the RRD register.
        match self
            .component
            .read_reg(self.interface, REGISTER_OFFSET_RRD)?
        {
            // The register has a sentinel value to indicate no more data is available in the FIFO.
            0xFFFF_FFFF => Ok(None),

            value => Ok(Some(value)),
        }
    }

    /// Check if the FIFO is full.
    pub fn is_full(&mut self) -> Result<bool, Error> {
        let status = Status::load(self.component, self.interface)?;
        Ok(status.full())
    }

    /// Check if the FIFO is empty.
    pub fn is_empty(&mut self) -> Result<bool, Error> {
        let status = Status::load(self.component, self.interface)?;
        Ok(status.empty())
    }

    /// Check if the ET capture has stopped and all internal pipelines and buffers have been
    /// drained.
    pub fn is_ready(&mut self) -> Result<bool, Error> {
        let status = Status::load(self.component, self.interface)?;
        Ok(status.ready())
    }

    pub fn latched_fill_level(&mut self) -> Result<u32, Error> {
        let level = self
            .component
            .read_reg(self.interface, REGISTER_OFFSET_LBUFLVL)?;
        Ok(level)
    }

    pub fn fill_level(&mut self) -> Result<u32, Error> {
        let level = self
            .component
            .read_reg(self.interface, REGISTER_OFFSET_CBUFLVL)?;
        Ok(level)
    }

    pub fn stop_on_flush(&mut self, stop: bool) -> Result<(), Error> {
        let mut ffcr = FormatFlushControl::load(self.component, self.interface)?;
        ffcr.set_stop_on_flush(stop);
        ffcr.store(self.component, self.interface)?;
        Ok(())
    }

    pub fn manual_flush(&mut self) -> Result<(), Error> {
        let mut ffcr = FormatFlushControl::load(self.component, self.interface)?;
        ffcr.set_manual_flush(true);
        ffcr.store(self.component, self.interface)?;
        Ok(())
    }

    /// Check if the ETF has triggered.
    ///
    /// # Note
    /// This will only be set when operating in circular buffer modes.
    pub fn is_triggered(&mut self) -> Result<bool, Error> {
        let status = Status::load(self.component, self.interface)?;
        Ok(status.trigd())
    }

    /// Get the size of the FIFO in bytes.
    pub fn fifo_size(&mut self) -> Result<u32, Error> {
        let size_words = self
            .component
            .read_reg(self.interface, REGISTER_OFFSET_RSZ)?;
        Ok(size_words * core::mem::size_of::<u32>() as u32)
    }
}

bitfield! {
    #[derive(Clone, Default)]
    pub struct FormatFlushControl(u32);
    impl Debug;

    pub manual_flush, set_manual_flush: 6;
    pub stop_on_flush, set_stop_on_flush: 12;
}

impl From<u32> for FormatFlushControl {
    fn from(raw: u32) -> Self {
        FormatFlushControl(raw)
    }
}

impl From<FormatFlushControl> for u32 {
    fn from(status: FormatFlushControl) -> u32 {
        status.0
    }
}

impl DebugRegister for FormatFlushControl {
    const ADDRESS: u32 = 0x304;
    const NAME: &'static str = "ETF_FFCR";
}

bitfield! {
    #[derive(Clone, Default)]
    pub struct Status(u32);
    impl Debug;

    pub empty, _: 4;
    pub ready, _: 2;
    pub trigd, _: 1;
    pub full, _: 0;
}

impl From<u32> for Status {
    fn from(raw: u32) -> Status {
        Status(raw)
    }
}

impl From<Status> for u32 {
    fn from(status: Status) -> u32 {
        status.0
    }
}

impl DebugRegister for Status {
    const ADDRESS: u32 = 0xC;
    const NAME: &'static str = "ETF_STS";
}

bitfield! {
    #[derive(Clone, Default)]
    pub struct EtfMode(u32);
    impl Debug;

    // The Mode register configures the operational mode of the FIFO.
    pub u8, mode, set_mode: 1, 0;
}

impl From<u32> for EtfMode {
    fn from(raw: u32) -> EtfMode {
        EtfMode(raw)
    }
}

impl From<EtfMode> for u32 {
    fn from(mode: EtfMode) -> u32 {
        mode.0
    }
}

impl DebugRegister for EtfMode {
    const ADDRESS: u32 = 0x28;
    const NAME: &'static str = "ETF_MODE";
}
