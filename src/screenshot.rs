use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{AppResult, capture::PrimaryDisplayCapture, config::RecorderConfig, ffmpeg};

pub(crate) fn capture_desktop(
    capturer: &PrimaryDisplayCapture,
    config: &RecorderConfig,
) -> AppResult<PathBuf> {
    let image = capturer.capture()?;
    let path = screenshot_path(config)?;
    ffmpeg::write_png(&path, image.width, image.height, &image.pixels)?;
    Ok(path)
}

fn screenshot_path(config: &RecorderConfig) -> AppResult<PathBuf> {
    let mut directory = config.screenshot_dir();
    fs::create_dir_all(&directory)?;

    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    directory.push(format!("screenshot-{timestamp}.png"));
    Ok(directory)
}
