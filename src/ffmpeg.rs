use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

use crate::{AppResult, config::RecorderConfig};

pub(crate) struct AudioInput {
    pub(crate) pipe_path: String,
    pub(crate) sample_format: &'static str,
    pub(crate) sample_rate: u32,
    pub(crate) channels: u16,
}

pub(crate) fn ensure_available() -> AppResult<()> {
    ensure_tool_available("ffmpeg")?;
    ensure_tool_available("ffprobe")?;
    Ok(())
}

fn ensure_tool_available(tool: &str) -> AppResult<()> {
    let status = Command::new(tool)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("Could not start {tool}. Is it installed and on PATH? {error}"))?;

    if !status.success() {
        return Err(format!("{tool} -version failed with status {status}").into());
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

pub(crate) fn start_gpu_replay_encoder(
    primary_monitor_handle: u64,
    segment_path_pattern: PathBuf,
    config: &RecorderConfig,
    audio_input: Option<AudioInput>,
) -> AppResult<Child> {
    if !supports_filter("gfxcapture") {
        tracing::warn!("ffmpeg does not list the gfxcapture D3D11 capture source");
        return Err("ffmpeg does not list the gfxcapture D3D11 capture source".into());
    }

    if !supports_encoder("h264_nvenc") {
        tracing::warn!("ffmpeg does not list h264_nvenc for D3D11 hardware-frame encoding");
        return Err("ffmpeg does not list h264_nvenc for D3D11 hardware-frame encoding".into());
    }

    tracing::info!(
        frame_rate = config.frame_rate,
        primary_monitor_handle,
        segment_path_pattern = %segment_path_pattern.display(),
        audio = audio_input.is_some(),
        "starting ffmpeg GPU replay encoder"
    );

    let mut command = Command::new("ffmpeg");
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        &format!(
            "gfxcapture=hmonitor={primary_monitor_handle}:max_framerate={}",
            config.frame_rate
        ),
    ]);
    apply_audio_input_options(&mut command, audio_input);
    apply_nvenc_options(&mut command);
    apply_segment_options(&mut command, segment_path_pattern, config)
}

pub(crate) fn start_cpu_replay_encoder(
    width: i32,
    height: i32,
    segment_path_pattern: PathBuf,
    config: &RecorderConfig,
    audio_input: Option<AudioInput>,
) -> AppResult<Child> {
    let has_audio = audio_input.is_some();
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
        &config.frame_rate.to_string(),
        "-i",
        "pipe:0",
    ]);
    apply_audio_input_options(&mut command, audio_input);

    if supports_encoder("h264_nvenc") {
        tracing::info!(
            width,
            height,
            frame_rate = config.frame_rate,
            segment_path_pattern = %segment_path_pattern.display(),
            audio = has_audio,
            "starting ffmpeg CPU replay encoder with NVENC"
        );
        apply_nvenc_options(&mut command);
    } else {
        tracing::warn!("ffmpeg does not list h264_nvenc; falling back to libx264");
        eprintln!("ffmpeg does not list h264_nvenc; falling back to libx264.");
        tracing::info!(
            width,
            height,
            frame_rate = config.frame_rate,
            segment_path_pattern = %segment_path_pattern.display(),
            audio = has_audio,
            "starting ffmpeg CPU replay encoder with libx264"
        );
        command.args([
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-tune",
            "zerolatency",
            "-crf",
            "23",
            "-bf",
            "0",
        ]);
    }

    command.args(["-pix_fmt", "yuv420p"]);
    apply_segment_options(&mut command, segment_path_pattern, config)
}

fn apply_audio_input_options(command: &mut Command, audio_input: Option<AudioInput>) {
    if let Some(audio_input) = audio_input {
        command
            .args(["-thread_queue_size", "1024"])
            .args(["-f", audio_input.sample_format])
            .arg("-ar")
            .arg(audio_input.sample_rate.to_string())
            .arg("-ac")
            .arg(audio_input.channels.to_string())
            .arg("-i")
            .arg(audio_input.pipe_path)
            .args([
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-af",
                "aresample=async=1:first_pts=0",
            ]);
    } else {
        command.arg("-an");
    }
}

fn apply_nvenc_options(command: &mut Command) {
    command.args([
        "-c:v",
        "h264_nvenc",
        "-preset",
        "p5",
        "-rc",
        "vbr",
        "-cq",
        "23",
        "-bf",
        "0",
    ]);
}

fn apply_segment_options(
    command: &mut Command,
    segment_path_pattern: PathBuf,
    config: &RecorderConfig,
) -> AppResult<Child> {
    let gop_size = config.frame_rate.to_string();
    let segment_seconds = config.segment_seconds.to_string();
    let segment_wrap_count = config
        .segment_buffer_seconds
        .div_ceil(config.segment_seconds)
        .to_string();
    let force_key_frames = format!("expr:gte(t,n_forced*{})", config.segment_seconds);

    command
        .args([
            "-g",
            &gop_size,
            "-force_key_frames",
            &force_key_frames,
            "-muxdelay",
            "0",
            "-muxpreload",
            "0",
            "-f",
            "segment",
            "-segment_time",
            &segment_seconds,
            "-segment_wrap",
            &segment_wrap_count,
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

pub(crate) fn supports_encoder(encoder: &str) -> bool {
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

fn supports_filter(filter: &str) -> bool {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-filters"])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).contains(filter)
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
