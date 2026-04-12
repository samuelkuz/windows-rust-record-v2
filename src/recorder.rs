use std::{
    io::Write,
    process::Child,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use crate::{
    AppResult, capture::PrimaryDisplayCapture, config::FRAME_RATE, display, ffmpeg,
    replay::ReplayBuffer,
};

pub(crate) struct ReplayRecorder {
    backend: CaptureBackend,
    gpu_encoder: Option<Child>,
    _cpu_capture_thread: Option<thread::JoinHandle<()>>,
}

impl ReplayRecorder {
    pub(crate) fn backend(&self) -> CaptureBackend {
        self.backend
    }
}

impl Drop for ReplayRecorder {
    fn drop(&mut self) {
        if let Some(gpu_encoder) = &mut self.gpu_encoder {
            let _ = gpu_encoder.kill();
            let _ = gpu_encoder.wait();
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum CaptureBackend {
    GpuFfmpegGfxCapture,
    CpuReadback,
}

impl CaptureBackend {
    pub(crate) fn label(self) -> &'static str {
        match self {
            CaptureBackend::GpuFfmpegGfxCapture => "FFmpeg gfxcapture D3D11 -> NVENC",
            CaptureBackend::CpuReadback => "Rust DXGI readback -> ffmpeg stdin",
        }
    }
}

pub(crate) fn start(replay_buffer: Arc<ReplayBuffer>) -> AppResult<ReplayRecorder> {
    match start_gpu_gfxcapture(replay_buffer.clone()) {
        Ok(recorder) => Ok(recorder),
        Err(error) => {
            eprintln!(
                "Could not start GPU replay capture ({error}); falling back to CPU readback."
            );
            start_cpu_readback(replay_buffer)
        }
    }
}

fn start_gpu_gfxcapture(replay_buffer: Arc<ReplayBuffer>) -> AppResult<ReplayRecorder> {
    let primary_monitor = display::primary_monitor_handle()?;
    let mut gpu_encoder = ffmpeg::start_gpu_replay_encoder(
        primary_monitor.as_u64(),
        replay_buffer.segment_path_pattern(),
    )?;

    thread::sleep(Duration::from_millis(500));
    if let Some(status) = gpu_encoder.try_wait()? {
        return Err(
            format!("ffmpeg GPU capture exited during startup with status {status}").into(),
        );
    }

    Ok(ReplayRecorder {
        backend: CaptureBackend::GpuFfmpegGfxCapture,
        gpu_encoder: Some(gpu_encoder),
        _cpu_capture_thread: None,
    })
}

fn start_cpu_readback(replay_buffer: Arc<ReplayBuffer>) -> AppResult<ReplayRecorder> {
    let capturer = PrimaryDisplayCapture::new()?;
    let cpu_capture_thread = start_cpu_readback_capture_thread(capturer, replay_buffer)?;

    Ok(ReplayRecorder {
        backend: CaptureBackend::CpuReadback,
        gpu_encoder: None,
        _cpu_capture_thread: Some(cpu_capture_thread),
    })
}

fn start_cpu_readback_capture_thread(
    capturer: PrimaryDisplayCapture,
    replay_buffer: Arc<ReplayBuffer>,
) -> AppResult<thread::JoinHandle<()>> {
    let mut encoder = ffmpeg::start_cpu_replay_encoder(
        capturer.width(),
        capturer.height(),
        replay_buffer.segment_path_pattern(),
    )?;
    let mut encoder_stdin = encoder
        .stdin
        .take()
        .ok_or("Could not open ffmpeg stdin for replay recording")?;

    Ok(thread::spawn(move || {
        let frame_interval = Duration::from_millis(1_000 / FRAME_RATE as u64);
        let mut last_frame: Option<Vec<u8>> = None;

        loop {
            let frame_started = Instant::now();
            match capturer.capture() {
                Ok(frame) => {
                    last_frame = Some(frame.pixels);
                    if let Some(frame_pixels) = &last_frame {
                        if let Err(error) = encoder_stdin.write_all(frame_pixels) {
                            eprintln!("Replay encoder stopped accepting frames: {error}");
                            break;
                        }
                    }
                }
                Err(error) => {
                    if let Some(frame_pixels) = &last_frame {
                        if let Err(write_error) = encoder_stdin.write_all(frame_pixels) {
                            eprintln!(
                                "Replay encoder stopped accepting repeated frames: {write_error}"
                            );
                            break;
                        }
                    } else {
                        eprintln!("Could not capture initial replay frame: {error}");
                    }
                }
            }

            let elapsed = frame_started.elapsed();
            if elapsed < frame_interval {
                thread::sleep(frame_interval - elapsed);
            }
        }

        drop(encoder_stdin);
        let _ = encoder.kill();
        let _ = encoder.wait();
    }))
}
