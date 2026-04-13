use std::{
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use crate::{
    AppResult,
    config::RecorderConfig,
    recorder::{self, CaptureBackend, ReplayRecorder},
    replay::ReplayBuffer,
};

pub(crate) struct ReplayApp {
    config: RecorderConfig,
    replay_buffer: Arc<ReplayBuffer>,
    recorder: Option<ReplayRecorder>,
    state: Arc<Mutex<AppState>>,
}

impl ReplayApp {
    pub(crate) fn start(config: RecorderConfig) -> AppResult<Self> {
        let replay_buffer = Arc::new(ReplayBuffer::new(config.clone())?);
        let recorder = recorder::start(replay_buffer.clone(), config.clone())?;
        let backend = recorder.backend();

        tracing::info!(backend = backend.label(), "recorder started");

        Ok(Self {
            config,
            replay_buffer,
            recorder: Some(recorder),
            state: Arc::new(Mutex::new(AppState::recording())),
        })
    }

    pub(crate) fn backend(&self) -> Option<CaptureBackend> {
        self.recorder.as_ref().map(ReplayRecorder::backend)
    }

    pub(crate) fn snapshot(&self) -> AppSnapshot {
        let state = self.lock_state().clone();
        AppSnapshot {
            status: state.status,
            last_saved_clip: state.last_saved_clip,
            clip_dir: self.config.clip_dir(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn pause(&mut self) {
        if self.recorder.take().is_some() {
            self.lock_state().status = AppStatus::Paused;
            tracing::info!("recorder paused");
        }
    }

    #[allow(dead_code)]
    pub(crate) fn resume(&mut self) -> AppResult<()> {
        if self.recorder.is_some() {
            return Ok(());
        }

        let recorder = recorder::start(self.replay_buffer.clone(), self.config.clone())?;
        let backend = recorder.backend();
        self.recorder = Some(recorder);
        self.lock_state().status = AppStatus::Recording;
        tracing::info!(backend = backend.label(), "recorder resumed");
        Ok(())
    }

    pub(crate) fn save_recent_clip_after_post_roll(
        &self,
        on_finished: impl FnOnce(Result<PathBuf, String>) + Send + 'static,
    ) {
        let replay_buffer = self.replay_buffer.clone();
        let replay_seconds = self.config.replay_clip_seconds();
        let post_roll_seconds = self.config.post_roll_seconds;
        let state = self.state.clone();

        {
            let mut state = lock_state(&state);
            state.status = AppStatus::Saving;
        }

        tracing::info!(replay_seconds, post_roll_seconds, "replay save requested");
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(post_roll_seconds));
            match replay_buffer.save_recent_clip(replay_seconds) {
                Ok(path) => {
                    tracing::info!(path = %path.display(), "saved replay clip");
                    let mut state = lock_state(&state);
                    state.status = AppStatus::Recording;
                    state.last_saved_clip = Some(path.clone());
                    on_finished(Ok(path));
                }
                Err(error) => {
                    tracing::error!(error = %error, "failed to save replay clip");
                    let error = error.to_string();
                    let mut state = lock_state(&state);
                    state.status = AppStatus::Error(error.clone());
                    on_finished(Err(error));
                }
            }
        });
    }

    #[allow(dead_code)]
    pub(crate) fn open_clips_folder(&self) -> AppResult<()> {
        Command::new("explorer")
            .arg(self.config.clip_dir())
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("Could not open clips folder: {error}").into())
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, AppState> {
        lock_state(&self.state)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct AppSnapshot {
    pub(crate) status: AppStatus,
    #[allow(dead_code)]
    pub(crate) last_saved_clip: Option<PathBuf>,
    pub(crate) clip_dir: PathBuf,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) enum AppStatus {
    Recording,
    Paused,
    Saving,
    Error(String),
}

impl AppStatus {
    pub(crate) fn label(&self) -> &str {
        match self {
            Self::Recording => "recording",
            Self::Paused => "paused",
            Self::Saving => "saving",
            Self::Error(_) => "error",
        }
    }
}

#[derive(Clone)]
struct AppState {
    status: AppStatus,
    last_saved_clip: Option<PathBuf>,
}

impl AppState {
    fn recording() -> Self {
        Self {
            status: AppStatus::Recording,
            last_saved_clip: None,
        }
    }
}

fn lock_state(state: &Mutex<AppState>) -> std::sync::MutexGuard<'_, AppState> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
