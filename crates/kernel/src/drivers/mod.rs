use alloc::string::String;

pub mod serial;

#[derive(Debug, Eq, PartialEq)]
pub enum DeviceType {
    Net,
    Gpu,
    Input,
    Block,
    Rtc,
    Serial,
    Intc,
}

pub trait Driver: Send + Sync {
    // if interrupt belongs to this driver, handle it and return true
    // return false otherwise
    // irq number is provided when available
    // driver should skip handling when irq number is mismatched
    fn try_handle_interrupt(&self, irq: Option<usize>) -> bool;

    // return the correspondent device type, see DeviceType
    fn device_type(&self) -> DeviceType;

    // get unique identifier for this device
    // should be different for each instance
    fn get_id(&self) -> String;

    // trait casting
    // fn as_net(&self) -> Option<&dyn NetDriver> { None }
    // fn as_block(&self) -> Option<&dyn BlockDriver> { None }
    // fn as_rtc(&self) -> Option<&dyn RtcDriver> { None }
}
