use std::{env, path::PathBuf};

use crate::AppResult;

const APP_FOLDER_NAME: &str = "Windows Rust Record";

#[derive(Clone, Debug)]
pub(crate) struct AppPaths {
    pub(crate) clips_dir: PathBuf,
    pub(crate) replay_segments_dir: PathBuf,
    pub(crate) logs_dir: PathBuf,
    pub(crate) screenshots_dir: PathBuf,
    pub(crate) settings_path: PathBuf,
}

impl AppPaths {
    pub(crate) fn installed_defaults() -> Self {
        let videos_dir = known_folder("USERPROFILE")
            .map(|path| path.join("Videos"))
            .unwrap_or_else(|| fallback_app_dir());
        let local_app_dir = known_folder("LOCALAPPDATA").unwrap_or_else(fallback_app_dir);
        let roaming_app_dir = known_folder("APPDATA").unwrap_or_else(fallback_app_dir);

        let media_dir = videos_dir.join(APP_FOLDER_NAME);
        let local_dir = local_app_dir.join(APP_FOLDER_NAME);
        let roaming_dir = roaming_app_dir.join(APP_FOLDER_NAME);

        Self {
            clips_dir: media_dir.join("clips"),
            replay_segments_dir: local_dir.join("replay-segments"),
            logs_dir: local_dir.join("logs"),
            screenshots_dir: media_dir.join("screenshots"),
            settings_path: roaming_dir.join("settings.toml"),
        }
    }

    pub(crate) fn from_output_dir(output_dir: PathBuf) -> Self {
        let clips_dir = if output_dir.file_name().is_some_and(|name| name == "clips") {
            output_dir.clone()
        } else {
            output_dir.join("clips")
        };

        Self {
            clips_dir,
            replay_segments_dir: output_dir.join("replay-segments"),
            logs_dir: output_dir.join("logs"),
            screenshots_dir: output_dir.join("screenshots"),
            settings_path: output_dir.join("settings.toml"),
        }
    }
}

pub(crate) fn absolute_path(path: PathBuf) -> AppResult<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(env::current_dir()?.join(path))
    }
}

fn known_folder(variable: &str) -> Option<PathBuf> {
    env::var_os(variable).map(PathBuf::from)
}

fn fallback_app_dir() -> PathBuf {
    env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
