use std::{env, path::PathBuf};

use crate::AppResult;

pub(crate) const DEFAULT_FRAME_RATE: u32 = 60;
pub(crate) const DEFAULT_REPLAY_SECONDS: u64 = 15;
pub(crate) const DEFAULT_POST_ROLL_SECONDS: u64 = 5;
pub(crate) const DEFAULT_SEGMENT_SECONDS: u64 = 1;
pub(crate) const DEFAULT_SEGMENT_BUFFER_SECONDS: u64 = 90;
pub(crate) const DXGI_FRAME_TIMEOUT_MS: u32 = 50;

#[derive(Clone, Debug)]
pub(crate) struct AppConfig {
    pub(crate) command: AppCommand,
    pub(crate) recorder: RecorderConfig,
}

#[derive(Clone, Debug)]
pub(crate) enum AppCommand {
    Run,
    Help,
    Screenshot,
    RecordTest { seconds: u64 },
}

enum ParsedCommand {
    Run,
    Help,
    Screenshot,
    RecordTest { seconds: Option<u64> },
}

#[derive(Clone, Debug)]
pub(crate) struct RecorderConfig {
    pub(crate) frame_rate: u32,
    pub(crate) replay_seconds: u64,
    pub(crate) post_roll_seconds: u64,
    pub(crate) segment_seconds: u64,
    pub(crate) segment_buffer_seconds: u64,
    pub(crate) output_dir: PathBuf,
}

impl RecorderConfig {
    pub(crate) fn replay_clip_seconds(&self) -> u64 {
        self.replay_seconds + self.post_roll_seconds
    }

    pub(crate) fn segment_dir(&self) -> PathBuf {
        self.output_dir.join("replay-segments")
    }

    pub(crate) fn clip_dir(&self) -> PathBuf {
        self.output_dir.join("clips")
    }

    pub(crate) fn screenshot_dir(&self) -> PathBuf {
        self.output_dir.join("screenshots")
    }
}

impl Default for RecorderConfig {
    fn default() -> Self {
        Self {
            frame_rate: DEFAULT_FRAME_RATE,
            replay_seconds: DEFAULT_REPLAY_SECONDS,
            post_roll_seconds: DEFAULT_POST_ROLL_SECONDS,
            segment_seconds: DEFAULT_SEGMENT_SECONDS,
            segment_buffer_seconds: DEFAULT_SEGMENT_BUFFER_SECONDS,
            output_dir: env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

pub(crate) fn parse_args() -> AppResult<AppConfig> {
    let mut recorder = RecorderConfig::default();
    let mut command = ParsedCommand::Run;
    let args = env::args().skip(1).collect::<Vec<_>>();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--help" | "-h" => {
                command = ParsedCommand::Help;
                index += 1;
            }
            "--screenshot" => {
                command = ParsedCommand::Screenshot;
                index += 1;
            }
            "--record-test" => {
                let seconds = args
                    .get(index + 1)
                    .filter(|value| !value.starts_with("--"))
                    .map(|value| parse_u64(value, "--record-test"))
                    .transpose()?;
                command = ParsedCommand::RecordTest { seconds };
                index += if args
                    .get(index + 1)
                    .is_some_and(|value| !value.starts_with("--"))
                {
                    2
                } else {
                    1
                };
            }
            "--frame-rate" => {
                recorder.frame_rate = parse_next_u32(&args, &mut index, "--frame-rate")?;
            }
            "--replay-seconds" => {
                recorder.replay_seconds = parse_next_u64(&args, &mut index, "--replay-seconds")?;
            }
            "--post-roll-seconds" => {
                recorder.post_roll_seconds =
                    parse_next_u64(&args, &mut index, "--post-roll-seconds")?;
            }
            "--segment-seconds" => {
                recorder.segment_seconds = parse_next_u64(&args, &mut index, "--segment-seconds")?;
            }
            "--segment-buffer-seconds" => {
                recorder.segment_buffer_seconds =
                    parse_next_u64(&args, &mut index, "--segment-buffer-seconds")?;
            }
            "--output-dir" => {
                let value = next_value(&args, &mut index, "--output-dir")?;
                recorder.output_dir = absolute_path(PathBuf::from(value))?;
            }
            unknown => return Err(format!("Unknown argument: {unknown}\n\n{}", usage()).into()),
        }
    }

    validate_recorder_config(&recorder)?;
    let command = match command {
        ParsedCommand::Run => AppCommand::Run,
        ParsedCommand::Help => AppCommand::Help,
        ParsedCommand::Screenshot => AppCommand::Screenshot,
        ParsedCommand::RecordTest { seconds } => AppCommand::RecordTest {
            seconds: seconds.unwrap_or_else(|| recorder.replay_clip_seconds()),
        },
    };

    if let AppCommand::RecordTest { seconds } = command
        && seconds + recorder.segment_seconds >= recorder.segment_buffer_seconds
    {
        return Err(format!(
            "--record-test seconds plus --segment-seconds must fit within --segment-buffer-seconds ({})",
            recorder.segment_buffer_seconds
        )
        .into());
    }

    Ok(AppConfig { command, recorder })
}

fn validate_recorder_config(config: &RecorderConfig) -> AppResult<()> {
    if config.frame_rate == 0 {
        return Err("--frame-rate must be greater than 0".into());
    }

    if config.segment_seconds == 0 {
        return Err("--segment-seconds must be greater than 0".into());
    }

    if config.segment_buffer_seconds <= config.replay_clip_seconds() {
        return Err(format!(
            "--segment-buffer-seconds must be greater than replay + post-roll seconds ({})",
            config.replay_clip_seconds()
        )
        .into());
    }

    Ok(())
}

fn parse_next_u32(args: &[String], index: &mut usize, flag: &str) -> AppResult<u32> {
    let value = next_value(args, index, flag)?;
    parse_u32(&value, flag)
}

fn parse_next_u64(args: &[String], index: &mut usize, flag: &str) -> AppResult<u64> {
    let value = next_value(args, index, flag)?;
    parse_u64(&value, flag)
}

fn next_value(args: &[String], index: &mut usize, flag: &str) -> AppResult<String> {
    let value = args
        .get(*index + 1)
        .filter(|value| !value.starts_with("--"))
        .ok_or_else(|| format!("{flag} requires a value"))?
        .clone();
    *index += 2;
    Ok(value)
}

fn parse_u32(value: &str, flag: &str) -> AppResult<u32> {
    value
        .parse::<u32>()
        .map_err(|error| format!("{flag} must be a positive integer: {error}").into())
}

fn parse_u64(value: &str, flag: &str) -> AppResult<u64> {
    value
        .parse::<u64>()
        .map_err(|error| format!("{flag} must be a positive integer: {error}").into())
}

fn absolute_path(path: PathBuf) -> AppResult<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(env::current_dir()?.join(path))
    }
}

pub(crate) fn usage() -> &'static str {
    "Usage:
  windows-rust-record-v2 [options]
  windows-rust-record-v2 --screenshot [options]
  windows-rust-record-v2 --record-test [seconds] [options]

Options:
  --frame-rate <fps>
  --replay-seconds <seconds>
  --post-roll-seconds <seconds>
  --segment-seconds <seconds>
  --segment-buffer-seconds <seconds>
  --output-dir <path>"
}
