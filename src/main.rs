mod capture;
mod config;
mod ffmpeg;
mod hotkey;
mod recorder;
mod replay;
mod screenshot;

use std::{sync::Arc, thread, time::Duration};

use capture::PrimaryDisplayCapture;
use config::{POST_ROLL_SECONDS, REPLAY_SECONDS};
use replay::ReplayBuffer;

pub(crate) type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

fn main() -> AppResult<()> {
    ffmpeg::ensure_available()?;
    let args = std::env::args().collect::<Vec<_>>();

    if args.iter().any(|arg| arg == "--screenshot") {
        let capturer = PrimaryDisplayCapture::new()?;
        let path = screenshot::capture_desktop(&capturer)?;
        println!("Saved screenshot: {}", path.display());
        return Ok(());
    }

    if let Some(index) = args.iter().position(|arg| arg == "--record-test") {
        let seconds = args
            .get(index + 1)
            .and_then(|seconds| seconds.parse::<u64>().ok())
            .unwrap_or(REPLAY_SECONDS + POST_ROLL_SECONDS);
        let capturer = PrimaryDisplayCapture::new()?;
        let replay_buffer = Arc::new(ReplayBuffer::new()?);
        let _capture_thread = recorder::start_capture_thread(capturer, replay_buffer.clone())?;
        println!("Recording test replay for {seconds} seconds...");
        thread::sleep(Duration::from_secs(seconds));
        let path = replay_buffer.save_recent_clip(seconds)?;
        println!("Saved replay clip: {}", path.display());
        return Ok(());
    }

    run_replay_recorder()
}

fn run_replay_recorder() -> AppResult<()> {
    let registered_hotkey = hotkey::register()?;
    let capturer = PrimaryDisplayCapture::new()?;
    let replay_buffer = Arc::new(ReplayBuffer::new()?);
    let _capture_thread = recorder::start_capture_thread(capturer, replay_buffer.clone())?;

    println!("Replay recorder is running.");
    println!(
        "Press {} to save the last {} seconds plus {} seconds after the hotkey.",
        registered_hotkey.label(),
        REPLAY_SECONDS,
        POST_ROLL_SECONDS
    );
    println!("Press Ctrl+C in this terminal to stop the app.");

    hotkey::run_message_loop(move || {
        let replay_buffer = replay_buffer.clone();
        println!("Replay hotkey pressed; saving clip after post-roll...");
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(POST_ROLL_SECONDS));
            match replay_buffer.save_recent_clip(REPLAY_SECONDS + POST_ROLL_SECONDS) {
                Ok(path) => println!("Saved replay clip: {}", path.display()),
                Err(error) => eprintln!("Failed to save replay clip: {error}"),
            }
        });
    });

    Ok(())
}
