use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    thread::{self, JoinHandle},
};

use crate::{
    AppResult,
    config::VoiceTriggerConfig,
    tray::{TrayAction, TrayActionSender},
};

pub(crate) struct VoiceTrigger {
    child: Child,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
}

impl Drop for VoiceTrigger {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();

        if let Some(thread) = self.stdout_thread.take() {
            let _ = thread.join();
        }

        if let Some(thread) = self.stderr_thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn start(
    config: &VoiceTriggerConfig,
    action_sender: TrayActionSender,
) -> AppResult<Option<VoiceTrigger>> {
    if !config.enabled {
        return Ok(None);
    }

    let mut command = Command::new(&config.python_path);
    command
        .arg("-u")
        .arg(&config.script_path)
        .arg("--model")
        .arg(&config.model_path)
        .arg("--threshold")
        .arg(config.threshold.to_string())
        .arg("--duration-minutes")
        .arg("0")
        .arg("--cooldown-seconds")
        .arg(config.cooldown_seconds.to_string())
        .arg("--status-interval-seconds")
        .arg("60")
        .arg("--session-note")
        .arg("rust app voice trigger")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(device) = &config.device {
        command.arg("--device").arg(device);
    }

    let mut child = command.spawn().map_err(|error| {
        format!(
            "Could not start voice trigger using {}: {error}",
            config.python_path.display()
        )
    })?;

    let stdout = child.stdout.take().ok_or("Could not capture voice trigger stdout")?;
    let stderr = child.stderr.take().ok_or("Could not capture voice trigger stderr")?;

    let stdout_thread = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else {
                break;
            };

            if line.starts_with("HIT") {
                println!("Voice trigger detected; saving replay.");
                if let Err(error) = action_sender.send(TrayAction::SaveReplay) {
                    eprintln!("Voice trigger could not request replay save: {error}");
                }
            } else {
                tracing::debug!(line = %line, "voice trigger output");
            }
        }
    });

    let stderr_thread = thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            match line {
                Ok(line) => eprintln!("Voice trigger: {line}"),
                Err(_) => break,
            }
        }
    });

    Ok(Some(VoiceTrigger {
        child,
        stdout_thread: Some(stdout_thread),
        stderr_thread: Some(stderr_thread),
    }))
}
