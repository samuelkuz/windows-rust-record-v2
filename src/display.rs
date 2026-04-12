use windows::Win32::{
    Foundation::POINT,
    Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint},
};

use crate::AppResult;

pub(crate) struct MonitorHandle(u64);

impl MonitorHandle {
    pub(crate) fn as_u64(&self) -> u64 {
        self.0
    }
}

pub(crate) fn primary_monitor_handle() -> AppResult<MonitorHandle> {
    let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    if monitor.is_invalid() {
        return Err("Windows did not return a primary monitor handle".into());
    }

    Ok(MonitorHandle(monitor.0 as usize as u64))
}
