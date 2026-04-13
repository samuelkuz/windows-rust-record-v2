use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{AppResult, config::RecorderConfig};

const DEFAULT_HOTKEY: &str = "Ctrl+Alt+S";
const STARTUP_FILE_NAME: &str = "Windows Rust Record.cmd";

#[derive(Clone, Debug)]
pub(crate) struct AppSettings {
    pub(crate) hotkey: String,
    pub(crate) start_with_windows: bool,
    pub(crate) path: PathBuf,
}

impl AppSettings {
    pub(crate) fn load_or_create(config: &RecorderConfig) -> AppResult<Self> {
        let path = config.settings_path();
        if !path.exists() {
            write_default_settings(&path)?;
        }

        let content = fs::read_to_string(&path)?;
        let mut hotkey = DEFAULT_HOTKEY.to_string();
        let mut start_with_windows = startup_shortcut_path().is_ok_and(|path| path.exists());

        for raw_line in content.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                continue;
            };

            match key.trim() {
                "hotkey" => hotkey = unquote(value.trim()).to_string(),
                "start_with_windows" => start_with_windows = parse_bool(value.trim())?,
                _ => {}
            }
        }

        Ok(Self {
            hotkey,
            start_with_windows,
            path,
        })
    }

    pub(crate) fn save(&self) -> AppResult<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(
            &self.path,
            format!(
                "# Windows Rust Record settings\n# Edit hotkey, then use Reload settings from the tray menu.\nhotkey = \"{}\"\nstart_with_windows = {}\n",
                self.hotkey, self.start_with_windows
            ),
        )?;
        Ok(())
    }

    pub(crate) fn open_editor(&self) -> AppResult<()> {
        Command::new("notepad")
            .arg(&self.path)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("Could not open settings file: {error}").into())
    }

    pub(crate) fn set_start_with_windows(&mut self, enabled: bool) -> AppResult<()> {
        set_startup_enabled(enabled)?;
        self.start_with_windows = enabled;
        self.save()
    }
}

fn write_default_settings(path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let settings = AppSettings {
        hotkey: DEFAULT_HOTKEY.to_string(),
        start_with_windows: startup_shortcut_path().is_ok_and(|path| path.exists()),
        path: path.to_path_buf(),
    };
    settings.save()
}

fn set_startup_enabled(enabled: bool) -> AppResult<()> {
    let path = startup_shortcut_path()?;

    if enabled {
        let exe_path = env::current_exe()?;
        let exe_path = exe_path.display().to_string();
        fs::write(&path, format!("@echo off\r\nstart \"\" \"{exe_path}\"\r\n"))?;
    } else if path.exists() {
        fs::remove_file(path)?;
    }

    Ok(())
}

fn startup_shortcut_path() -> AppResult<PathBuf> {
    let app_data = env::var_os("APPDATA").ok_or("APPDATA is not set")?;
    Ok(PathBuf::from(app_data)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup")
        .join(STARTUP_FILE_NAME))
}

fn parse_bool(value: &str) -> AppResult<bool> {
    match unquote(value).to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Ok(true),
        "false" | "no" | "0" | "off" => Ok(false),
        _ => Err(format!("Invalid boolean setting: {value}").into()),
    }
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
}
