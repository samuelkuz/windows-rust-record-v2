#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod audio;
mod capture;
mod config;
mod diagnostics;
mod display;
mod ffmpeg;
mod hotkey;
mod paths;
mod recorder;
mod replay;
mod screenshot;
mod settings;
mod tray;
mod voice_trigger;

use std::{sync::Arc, thread, time::Duration};

use app::ReplayApp;
use capture::PrimaryDisplayCapture;
use config::{AppCommand, AppConfig, RecorderConfig};
use replay::ReplayBuffer;
use settings::AppSettings;
use tray::{TrayAction, TrayApp};

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
                clips_dir = %app_config.recorder.clip_dir().display(),
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
                clips_dir = %app_config.recorder.clip_dir().display(),
                replay_segments_dir = %app_config.recorder.segment_dir().display(),
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
    let recorder_config = app_config.recorder;
    let mut settings = AppSettings::load_or_create(&recorder_config)?;
    let mut registered_hotkey = Some(register_hotkey_from_settings(&settings)?);
    let mut app = ReplayApp::start(recorder_config.clone())?;
    let snapshot = app.snapshot();
    let tray = TrayApp::new()?;
    let _voice_trigger =
        voice_trigger::start(&recorder_config.voice_trigger, tray.action_sender())?;

    println!("Replay recorder is running.");
    if let Some(backend) = app.backend() {
        println!("Capture backend: {}", backend.label());
    }
    println!("Status: {}", snapshot.status.label());
    println!("Clips folder: {}", snapshot.clip_dir.display());
    println!(
        "Press {} to save the last {} seconds plus {} seconds after the hotkey.",
        registered_hotkey
            .as_ref()
            .map(hotkey::RegisteredHotkey::label)
            .unwrap_or("unregistered"),
        recorder_config.replay_seconds,
        recorder_config.post_roll_seconds
    );
    println!("Press Ctrl+C in this terminal to stop the app.");
    println!("Settings file: {}", settings.path.display());
    if recorder_config.voice_trigger.enabled {
        println!(
            "Voice trigger: listening for clip that with model {}",
            recorder_config.voice_trigger.model_path.display()
        );
    }
    println!(
        "A tray menu is also available with Save replay, Pause / resume, Open clips folder, Open settings, Reload settings, Toggle start with Windows, and Quit."
    );

    tray.run_event_loop(move |action| match action {
        TrayAction::SaveReplay => {
            println!("Replay save requested; saving clip after post-roll...");
            app.save_recent_clip_after_post_roll(|result| match result {
                Ok(path) => {
                    println!("Saved replay clip: {}", path.display());
                }
                Err(error) => {
                    eprintln!("Failed to save replay clip: {error}");
                }
            });
        }
        TrayAction::OpenClipsFolder => {
            if let Err(error) = app.open_clips_folder() {
                eprintln!("Failed to open clips folder: {error}");
            }
        }
        TrayAction::TogglePause => {
            if let Err(error) = app.toggle_pause() {
                eprintln!("Failed to toggle recording pause: {error}");
            } else {
                println!("Status: {}", app.snapshot().status.label());
            }
        }
        TrayAction::OpenSettings => {
            if let Err(error) = settings.open_editor() {
                eprintln!("Failed to open settings: {error}");
            }
        }
        TrayAction::ReloadSettings => match AppSettings::load_or_create(&recorder_config) {
            Ok(next_settings) => {
                drop(registered_hotkey.take());
                match register_hotkey_from_settings(&next_settings) {
                    Ok(next_hotkey) => {
                        println!("Reloaded settings. Hotkey: {}", next_hotkey.label());
                        registered_hotkey = Some(next_hotkey);
                        settings = next_settings;
                    }
                    Err(error) => {
                        eprintln!("Failed to reload hotkey from settings: {error}");
                        match register_hotkey_from_settings(&settings) {
                            Ok(previous_hotkey) => {
                                eprintln!("Restored previous hotkey: {}", previous_hotkey.label());
                                registered_hotkey = Some(previous_hotkey);
                            }
                            Err(previous_error) => {
                                eprintln!("Could not restore previous hotkey: {previous_error}");
                            }
                        }
                    }
                }
            }
            Err(error) => eprintln!("Failed to reload settings: {error}"),
        },
        TrayAction::ToggleStartup => {
            let enabled = !settings.start_with_windows;
            match settings.set_start_with_windows(enabled) {
                Ok(()) => {
                    println!(
                        "Start with Windows: {}",
                        if settings.start_with_windows {
                            "on"
                        } else {
                            "off"
                        }
                    );
                }
                Err(error) => eprintln!("Failed to update startup setting: {error}"),
            }
        }
        TrayAction::Quit => {
            println!("Quit requested; stopping replay recorder.");
        }
    });

    Ok(())
}

fn register_hotkey_from_settings(settings: &AppSettings) -> AppResult<hotkey::RegisteredHotkey> {
    let hotkey = hotkey::Hotkey::parse(&settings.hotkey)?;
    hotkey::register(hotkey)
}
