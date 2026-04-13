mod app;
mod audio;
mod capture;
mod config;
mod diagnostics;
mod display;
mod ffmpeg;
mod hotkey;
mod recorder;
mod replay;
mod screenshot;

use std::{sync::Arc, thread, time::Duration};

use app::ReplayApp;
use capture::PrimaryDisplayCapture;
use config::{AppCommand, AppConfig, RecorderConfig};
use replay::ReplayBuffer;

pub(crate) type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

fn main() -> AppResult<()> {
    let app_config = config::parse_args()?;

    match app_config.command {
        AppCommand::Help => {
            println!("{}", config::usage());
            Ok(())
        }
        AppCommand::Screenshot => {
            let log_path = diagnostics::init(&app_config.recorder)?;
            tracing::info!(log_path = %log_path.display(), "starting screenshot command");
            ffmpeg::ensure_available()?;
            let capturer = PrimaryDisplayCapture::new()?;
            let path = screenshot::capture_desktop(&capturer, &app_config.recorder)?;
            tracing::info!(path = %path.display(), "saved screenshot");
            println!("Saved screenshot: {}", path.display());
            Ok(())
        }
        AppCommand::RecordTest { seconds } => {
            let log_path = diagnostics::init(&app_config.recorder)?;
            tracing::info!(
                log_path = %log_path.display(),
                seconds,
                frame_rate = app_config.recorder.frame_rate,
                output_dir = %app_config.recorder.output_dir.display(),
                "starting record-test command"
            );
            ffmpeg::ensure_available()?;
            run_record_test(&app_config.recorder, seconds)
        }
        AppCommand::Run => {
            let log_path = diagnostics::init(&app_config.recorder)?;
            tracing::info!(
                log_path = %log_path.display(),
                frame_rate = app_config.recorder.frame_rate,
                replay_seconds = app_config.recorder.replay_seconds,
                post_roll_seconds = app_config.recorder.post_roll_seconds,
                output_dir = %app_config.recorder.output_dir.display(),
                "starting replay recorder command"
            );
            ffmpeg::ensure_available()?;
            run_replay_recorder(app_config)
        }
    }
}

fn run_record_test(config: &RecorderConfig, seconds: u64) -> AppResult<()> {
    let replay_buffer = Arc::new(ReplayBuffer::new(config.clone())?);
    let recorder = recorder::start(replay_buffer.clone(), config.clone())?;
    tracing::info!(backend = recorder.backend().label(), "recorder started");
    println!("Capture backend: {}", recorder.backend().label());
    println!("Recording test replay for {seconds} seconds...");
    thread::sleep(Duration::from_secs(seconds));
    let path = replay_buffer.save_recent_clip(seconds)?;
    tracing::info!(path = %path.display(), "saved record-test clip");
    println!("Saved replay clip: {}", path.display());
    Ok(())
}

fn run_replay_recorder(app_config: AppConfig) -> AppResult<()> {
    let registered_hotkey = hotkey::register()?;
    let recorder_config = app_config.recorder;
    let app = ReplayApp::start(recorder_config.clone())?;
    let snapshot = app.snapshot();

    println!("Replay recorder is running.");
    if let Some(backend) = app.backend() {
        println!("Capture backend: {}", backend.label());
    }
    println!("Status: {}", snapshot.status.label());
    println!("Clips folder: {}", snapshot.clip_dir.display());
    println!(
        "Press {} to save the last {} seconds plus {} seconds after the hotkey.",
        registered_hotkey.label(),
        recorder_config.replay_seconds,
        recorder_config.post_roll_seconds
    );
    println!("Press Ctrl+C in this terminal to stop the app.");

    hotkey::run_message_loop(move || {
        println!("Replay hotkey pressed; saving clip after post-roll...");
        app.save_recent_clip_after_post_roll(|result| match result {
            Ok(path) => {
                println!("Saved replay clip: {}", path.display());
            }
            Err(error) => {
                eprintln!("Failed to save replay clip: {error}");
            }
        });
    });

    Ok(())
}
