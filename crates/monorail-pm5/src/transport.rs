//! USB HID transport for the PM5 (ADR 0003).
//!
//! Discovery by Concept2 vendor ID, HID report exchange with timeouts, and
//! a reconnect-on-unplug loop. Report framing (HID report IDs and sizes)
//! lands here together with the poll loops.

use thiserror::Error;

use crate::CONCEPT2_VID;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("no PM5 found (vendor id {CONCEPT2_VID:#06x})")]
    NotFound,
    #[error("hid error: {0}")]
    Hid(#[from] hidapi::HidError),
    #[error("read timed out after {0} ms")]
    Timeout(u32),
}

/// Handle to a connected PM5.
pub struct Pm5Device {
    device: hidapi::HidDevice,
}

impl Pm5Device {
    /// Open the first PM5 found on the bus.
    pub fn open_first(api: &hidapi::HidApi) -> Result<Self, TransportError> {
        let info = api
            .device_list()
            .find(|d| d.vendor_id() == CONCEPT2_VID)
            .ok_or(TransportError::NotFound)?;
        let device = info.open_device(api)?;
        Ok(Self { device })
    }

    /// Send a CSAFE frame and read the response frame.
    ///
    /// TODO(ADR 0003): HID report id/size handling, response reassembly
    /// across reports, retry policy.
    pub fn exchange(&mut self, _frame: &[u8], _timeout_ms: u32) -> Result<Vec<u8>, TransportError> {
        let _ = &self.device;
        unimplemented!("CSAFE exchange over HID reports not yet implemented")
    }
}
