use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

use crate::{
    AppResult,
    config::{FRAME_RATE, SEGMENT_BUFFER_SECONDS, SEGMENT_SECONDS},
};

pub(crate) fn ensure_available() -> AppResult<()> {
    let status = Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("Could not start ffmpeg. Is it installed and on PATH? {error}"))?;

    if !status.success() {
        return Err(format!("ffmpeg -version failed with status {status}").into());
    }

    Ok(())
}

pub(crate) fn write_png(path: &Path, width: i32, height: i32, pixels: &[u8]) -> AppResult<()> {
    let mut child = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pixel_format",
            "bgr0",
            "-video_size",
            &format!("{width}x{height}"),
            "-i",
            "pipe:0",
            "-frames:v",
            "1",
            "-compression_level",
            "9",
            "-y",
        ])
        .arg(path)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start ffmpeg. Is it installed and on PATH? {error}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or("Could not open ffmpeg stdin for raw screenshot pixels")?;
    stdin.write_all(pixels)?;
    drop(stdin);

    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg failed to write {}: {stderr}", path.display()).into());
    }

    Ok(())
}

pub(crate) fn start_replay_encoder(
    width: i32,
    height: i32,
    segment_path_pattern: PathBuf,
) -> AppResult<Child> {
    let mut command = Command::new("ffmpeg");
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "rawvideo",
        "-pixel_format",
        "bgr0",
        "-video_size",
        &format!("{width}x{height}"),
        "-framerate",
        &FRAME_RATE.to_string(),
        "-i",
        "pipe:0",
        "-an",
    ]);

    if supports_encoder("h264_nvenc") {
        command.args([
            "-c:v",
            "h264_nvenc",
            "-preset",
            "p5",
            "-rc",
            "vbr",
            "-cq",
            "23",
        ]);
    } else {
        eprintln!("ffmpeg does not list h264_nvenc; falling back to libx264.");
        command.args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "23"]);
    }

    let gop_size = FRAME_RATE.to_string();
    let segment_seconds = SEGMENT_SECONDS.to_string();
    let segment_buffer_seconds = SEGMENT_BUFFER_SECONDS.to_string();
    let force_key_frames = format!("expr:gte(t,n_forced*{SEGMENT_SECONDS})");

    command
        .args([
            "-g",
            &gop_size,
            "-force_key_frames",
            &force_key_frames,
            "-pix_fmt",
            "yuv420p",
            "-f",
            "segment",
            "-segment_time",
            &segment_seconds,
            "-segment_wrap",
            &segment_buffer_seconds,
            "-reset_timestamps",
            "1",
            "-segment_format",
            "mpegts",
        ])
        .arg(segment_path_pattern)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("Could not start replay ffmpeg encoder: {error}").into())
}

fn supports_encoder(encoder: &str) -> bool {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).contains(encoder)
        }
        _ => false,
    }
}

pub(crate) fn concat_path(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "/")
        .replace('\'', "'\\''")
}
