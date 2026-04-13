use std::{
    io::Write,
    process::Child,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crate::{
    AppResult, audio, capture::PrimaryDisplayCapture, config::RecorderConfig, display, ffmpeg,
    replay::ReplayBuffer,
};

pub(crate) struct ReplayRecorder {
    backend: CaptureBackend,
    stop_requested: Arc<AtomicBool>,
    gpu_encoder: Option<Child>,
    cpu_capture_thread: Option<thread::JoinHandle<()>>,
    audio_capture_thread: Option<thread::JoinHandle<()>>,
}

impl ReplayRecorder {
    pub(crate) fn backend(&self) -> CaptureBackend {
        self.backend
    }
}

impl Drop for ReplayRecorder {
    fn drop(&mut self) {
        self.stop_requested.store(true, Ordering::Relaxed);

        if let Some(gpu_encoder) = &mut self.gpu_encoder {
            let _ = gpu_encoder.kill();
            let _ = gpu_encoder.wait();
        }

        if let Some(cpu_capture_thread) = self.cpu_capture_thread.take() {
            let _ = cpu_capture_thread.join();
        }

        if let Some(audio_capture_thread) = self.audio_capture_thread.take() {
            let _ = audio_capture_thread.join();
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

pub(crate) fn start(
    replay_buffer: Arc<ReplayBuffer>,
    config: RecorderConfig,
) -> AppResult<ReplayRecorder> {
    match start_gpu_gfxcapture(replay_buffer.clone(), &config) {
        Ok(recorder) => Ok(recorder),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "could not start GPU replay capture; falling back to CPU readback"
            );
            eprintln!(
                "Could not start GPU replay capture ({error}); falling back to CPU readback."
            );
            start_cpu_readback(replay_buffer, config)
        }
    }
}

fn start_gpu_gfxcapture(
    replay_buffer: Arc<ReplayBuffer>,
    config: &RecorderConfig,
) -> AppResult<ReplayRecorder> {
    let primary_monitor = display::primary_monitor_handle()?;
    let stop_requested = Arc::new(AtomicBool::new(false));
    let audio_input = prepare_audio_input();
    let mut gpu_encoder = ffmpeg::start_gpu_replay_encoder(
        primary_monitor.as_u64(),
        replay_buffer.segment_path_pattern(),
        config,
        audio_input
            .as_ref()
            .map(audio::PreparedLoopbackAudio::ffmpeg_input),
    )?;
    let audio_capture_thread =
        audio_input.map(|audio_input| audio_input.start(Arc::clone(&stop_requested)));

    thread::sleep(Duration::from_millis(500));
    if let Some(status) = gpu_encoder.try_wait()? {
        tracing::warn!(%status, "ffmpeg GPU capture exited during startup");
        return Err(
            format!("ffmpeg GPU capture exited during startup with status {status}").into(),
        );
    }

    Ok(ReplayRecorder {
        backend: CaptureBackend::GpuFfmpegGfxCapture,
        stop_requested,
        gpu_encoder: Some(gpu_encoder),
        cpu_capture_thread: None,
        audio_capture_thread,
    })
}

fn start_cpu_readback(
    replay_buffer: Arc<ReplayBuffer>,
    config: RecorderConfig,
) -> AppResult<ReplayRecorder> {
    let capturer = PrimaryDisplayCapture::new()?;
    let stop_requested = Arc::new(AtomicBool::new(false));
    let audio_input = prepare_audio_input();
    let audio_ffmpeg_input = audio_input
        .as_ref()
        .map(audio::PreparedLoopbackAudio::ffmpeg_input);
    let cpu_capture_thread = start_cpu_readback_capture_thread(
        capturer,
        replay_buffer,
        audio_ffmpeg_input,
        config,
        Arc::clone(&stop_requested),
    )?;
    let audio_capture_thread =
        audio_input.map(|audio_input| audio_input.start(Arc::clone(&stop_requested)));

    Ok(ReplayRecorder {
        backend: CaptureBackend::CpuReadback,
        stop_requested,
        gpu_encoder: None,
        cpu_capture_thread: Some(cpu_capture_thread),
        audio_capture_thread,
    })
}

fn start_cpu_readback_capture_thread(
    capturer: PrimaryDisplayCapture,
    replay_buffer: Arc<ReplayBuffer>,
    audio_input: Option<ffmpeg::AudioInput>,
    config: RecorderConfig,
    stop_requested: Arc<AtomicBool>,
) -> AppResult<thread::JoinHandle<()>> {
    let mut encoder = ffmpeg::start_cpu_replay_encoder(
        capturer.width(),
        capturer.height(),
        replay_buffer.segment_path_pattern(),
        &config,
        audio_input,
    )?;
    let mut encoder_stdin = encoder
        .stdin
        .take()
        .ok_or("Could not open ffmpeg stdin for replay recording")?;

    Ok(thread::spawn(move || {
        let frame_interval = Duration::from_secs_f64(1.0 / config.frame_rate as f64);
        let mut last_frame: Option<Vec<u8>> = None;

        while !stop_requested.load(Ordering::Relaxed) {
            let frame_started = Instant::now();
            match capturer.capture() {
                Ok(frame) => {
                    last_frame = Some(frame.pixels);
                    if let Some(frame_pixels) = &last_frame {
                        if let Err(error) = encoder_stdin.write_all(frame_pixels) {
                            tracing::warn!(error = %error, "replay encoder stopped accepting frames");
                            eprintln!("Replay encoder stopped accepting frames: {error}");
                            break;
                        }
                    }
                }
                Err(error) => {
                    if let Some(frame_pixels) = &last_frame {
                        if let Err(write_error) = encoder_stdin.write_all(frame_pixels) {
                            tracing::warn!(
                                error = %write_error,
                                "replay encoder stopped accepting repeated frames"
                            );
                            eprintln!(
                                "Replay encoder stopped accepting repeated frames: {write_error}"
                            );
                            break;
                        }
                    } else {
                        tracing::warn!(error = %error, "could not capture initial replay frame");
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

fn prepare_audio_input() -> Option<audio::PreparedLoopbackAudio> {
    match audio::prepare_loopback_audio() {
        Ok(audio_input) => Some(audio_input),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "could not prepare WASAPI loopback audio; recording video only"
            );
            eprintln!("Could not prepare WASAPI loopback audio; recording video only: {error}");
            None
        }
    }
}
