//! Windows hardware kind from the system battery.
use crate::device_info::DeviceKind;
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

pub(crate) fn device_kind() -> Option<DeviceKind> {
    let mut status = SYSTEM_POWER_STATUS::default();
    // The status is a plain struct filled during this synchronous call.
    if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
        return None;
    }
    // BatteryFlag 128 reports no system battery; 255 means it is unknown.
    match status.BatteryFlag {
        255 => None,
        flag if flag & 128 != 0 => Some(DeviceKind::Desktop),
        _ => Some(DeviceKind::Laptop),
    }
}
