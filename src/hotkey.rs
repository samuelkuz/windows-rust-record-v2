use std::mem::zeroed;

use windows::Win32::{
    Foundation::{ERROR_HOTKEY_ALREADY_REGISTERED, GetLastError},
    UI::{
        Input::KeyboardAndMouse::{
            HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey,
            UnregisterHotKey, VK_S,
        },
        WindowsAndMessaging::{DispatchMessageW, GetMessageW, MSG, TranslateMessage, WM_HOTKEY},
    },
};

use crate::AppResult;

const HOTKEY_ID: i32 = 1;
const HOTKEY: Hotkey = Hotkey {
    label: "Ctrl+Alt+S",
    modifiers: HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0 | MOD_NOREPEAT.0),
    key: VK_S.0 as u32,
};

pub(crate) struct RegisteredHotkey {
    hotkey: Hotkey,
}

impl RegisteredHotkey {
    pub(crate) fn label(&self) -> &'static str {
        self.hotkey.label
    }
}

impl Drop for RegisteredHotkey {
    fn drop(&mut self) {
        unsafe {
            let _ = UnregisterHotKey(None, HOTKEY_ID);
        }
    }
}

pub(crate) fn register() -> AppResult<RegisteredHotkey> {
    if unsafe { RegisterHotKey(None, HOTKEY_ID, HOTKEY.modifiers, HOTKEY.key) }.is_ok() {
        Ok(RegisteredHotkey { hotkey: HOTKEY })
    } else {
        let error = unsafe { GetLastError() };
        Err(format!(
            "Could not register {} as the replay hotkey: Windows error {} ({})",
            HOTKEY.label,
            error.0,
            registration_error_message(error)
        )
        .into())
    }
}

pub(crate) fn run_message_loop(mut on_hotkey: impl FnMut()) {
    let mut message = unsafe { zeroed::<MSG>() };
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
        if message.message == WM_HOTKEY && message.wParam.0 == HOTKEY_ID as usize {
            on_hotkey();
        } else {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

fn registration_error_message(error: windows::Win32::Foundation::WIN32_ERROR) -> &'static str {
    match error {
        ERROR_HOTKEY_ALREADY_REGISTERED => {
            "that hotkey is already registered by Windows or another app"
        }
        _ => "unknown registration failure",
    }
}

#[derive(Clone, Copy)]
struct Hotkey {
    label: &'static str,
    modifiers: HOT_KEY_MODIFIERS,
    key: u32,
}
