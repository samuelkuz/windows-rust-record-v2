use windows::Win32::{
    Foundation::{ERROR_HOTKEY_ALREADY_REGISTERED, GetLastError},
    UI::{
        Input::KeyboardAndMouse::{
            HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
            RegisterHotKey, UnregisterHotKey,
        },
        WindowsAndMessaging::MSG,
    },
};

use crate::AppResult;

const HOTKEY_ID: i32 = 1;

pub(crate) struct RegisteredHotkey {
    hotkey: Hotkey,
}

impl RegisteredHotkey {
    pub(crate) fn label(&self) -> &str {
        &self.hotkey.label
    }
}

impl Drop for RegisteredHotkey {
    fn drop(&mut self) {
        unsafe {
            let _ = UnregisterHotKey(None, HOTKEY_ID);
        }
    }
}

pub(crate) fn register(hotkey: Hotkey) -> AppResult<RegisteredHotkey> {
    if unsafe { RegisterHotKey(None, HOTKEY_ID, hotkey.modifiers, hotkey.key) }.is_ok() {
        Ok(RegisteredHotkey { hotkey })
    } else {
        let error = unsafe { GetLastError() };
        Err(format!(
            "Could not register {} as the replay hotkey: Windows error {} ({})",
            hotkey.label,
            error.0,
            registration_error_message(error)
        )
        .into())
    }
}

pub(crate) fn is_hotkey_message(message: &MSG) -> bool {
    message.wParam.0 == HOTKEY_ID as usize
}

fn registration_error_message(error: windows::Win32::Foundation::WIN32_ERROR) -> &'static str {
    match error {
        ERROR_HOTKEY_ALREADY_REGISTERED => {
            "that hotkey is already registered by Windows or another app"
        }
        _ => "unknown registration failure",
    }
}

#[derive(Clone)]
pub(crate) struct Hotkey {
    label: String,
    modifiers: HOT_KEY_MODIFIERS,
    key: u32,
}

impl Hotkey {
    pub(crate) fn parse(value: &str) -> AppResult<Self> {
        let mut modifiers = MOD_NOREPEAT.0;
        let mut key = None;
        let mut labels = Vec::new();

        for part in value.split('+') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => {
                    modifiers |= MOD_CONTROL.0;
                    labels.push("Ctrl".to_string());
                }
                "alt" => {
                    modifiers |= MOD_ALT.0;
                    labels.push("Alt".to_string());
                }
                "shift" => {
                    modifiers |= MOD_SHIFT.0;
                    labels.push("Shift".to_string());
                }
                "win" | "windows" | "super" => {
                    modifiers |= MOD_WIN.0;
                    labels.push("Win".to_string());
                }
                _ => {
                    if key.is_some() {
                        return Err(format!("Hotkey has more than one key: {value}").into());
                    }
                    let parsed_key = parse_key(part)?;
                    key = Some(parsed_key);
                    labels.push(part.to_ascii_uppercase());
                }
            }
        }

        let key = key.ok_or_else(|| format!("Hotkey must include a key: {value}"))?;
        if modifiers == MOD_NOREPEAT.0 {
            return Err(format!("Hotkey must include at least one modifier: {value}").into());
        }

        Ok(Self {
            label: labels.join("+"),
            modifiers: HOT_KEY_MODIFIERS(modifiers),
            key,
        })
    }
}

fn parse_key(value: &str) -> AppResult<u32> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err("Hotkey key must not be empty".into());
    };

    if chars.next().is_none() && first.is_ascii_alphanumeric() {
        return Ok(first.to_ascii_uppercase() as u32);
    }

    if let Some(number) = value
        .strip_prefix('F')
        .or_else(|| value.strip_prefix('f'))
        .and_then(|number| number.parse::<u32>().ok())
        && (1..=24).contains(&number)
    {
        return Ok(0x70 + number - 1);
    }

    Err(format!("Unsupported hotkey key: {value}").into())
}
