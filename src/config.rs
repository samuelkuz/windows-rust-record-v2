use std::{env, path::PathBuf};

use crate::{AppResult, paths::AppPaths};

pub(crate) const DEFAULT_FRAME_RATE: u32 = 60;
pub(crate) const DEFAULT_REPLAY_SECONDS: u64 = 15;
pub(crate) const DEFAULT_POST_ROLL_SECONDS: u64 = 5;
pub(crate) const DEFAULT_SEGMENT_SECONDS: u64 = 1;
pub(crate) const DEFAULT_SEGMENT_BUFFER_SECONDS: u64 = 90;
pub(crate) const DEFAULT_AUDIO_SAMPLE_RATE: u32 = 48_000;
pub(crate) const DEFAULT_AUDIO_CHANNELS: u16 = 2;
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
    pub(crate) paths: AppPaths,
    pub(crate) audio: AudioConfig,
    pub(crate) voice_trigger: VoiceTriggerConfig,
}

#[derive(Clone, Debug)]
pub(crate) struct AudioConfig {
    pub(crate) system_audio: bool,
    pub(crate) microphone: bool,
    pub(crate) render_device_id: Option<String>,
    pub(crate) capture_device_id: Option<String>,
    pub(crate) output_sample_rate: u32,
    pub(crate) output_channels: u16,
    pub(crate) bitrate: String,
}

#[derive(Clone, Debug)]
pub(crate) struct VoiceTriggerConfig {
    pub(crate) enabled: bool,
    pub(crate) python_path: PathBuf,
    pub(crate) script_path: PathBuf,
    pub(crate) model_path: PathBuf,
    pub(crate) threshold: f32,
    pub(crate) cooldown_seconds: f32,
    pub(crate) device: Option<String>,
}

impl RecorderConfig {
    pub(crate) fn replay_clip_seconds(&self) -> u64 {
        self.replay_seconds + self.post_roll_seconds
    }

    pub(crate) fn segment_dir(&self) -> PathBuf {
        self.paths.replay_segments_dir.clone()
    }

    pub(crate) fn clip_dir(&self) -> PathBuf {
        self.paths.clips_dir.clone()
    }

    pub(crate) fn screenshot_dir(&self) -> PathBuf {
        self.paths.screenshots_dir.clone()
    }

    pub(crate) fn settings_path(&self) -> PathBuf {
        self.paths.settings_path.clone()
    }

    pub(crate) fn log_dir(&self) -> PathBuf {
        self.paths.logs_dir.clone()
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
            paths: AppPaths::installed_defaults(),
            audio: AudioConfig::default(),
            voice_trigger: VoiceTriggerConfig::default(),
        }
    }
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            system_audio: true,
            microphone: false,
            render_device_id: None,
            capture_device_id: None,
            output_sample_rate: DEFAULT_AUDIO_SAMPLE_RATE,
            output_channels: DEFAULT_AUDIO_CHANNELS,
            bitrate: "160k".to_string(),
        }
    }
}

impl Default for VoiceTriggerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            python_path: PathBuf::from(r".venv-openwakeword\Scripts\python.exe"),
            script_path: PathBuf::from(r"scripts\monitor_openwakeword_mic.py"),
            model_path: PathBuf::from(r"models\wakeword\clip_that_v2.onnx"),
            threshold: 0.65,
            cooldown_seconds: 3.0,
            device: None,
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
                recorder.paths =
                    AppPaths::from_output_dir(crate::paths::absolute_path(PathBuf::from(value))?);
            }
            "--no-system-audio" => {
                recorder.audio.system_audio = false;
                index += 1;
            }
            "--microphone" => {
                recorder.audio.microphone = true;
                index += 1;
            }
            "--audio-output-device-id" => {
                recorder.audio.render_device_id =
                    Some(next_value(&args, &mut index, "--audio-output-device-id")?);
            }
            "--audio-input-device-id" => {
                recorder.audio.capture_device_id =
                    Some(next_value(&args, &mut index, "--audio-input-device-id")?);
            }
            "--audio-sample-rate" => {
                recorder.audio.output_sample_rate =
                    parse_next_u32(&args, &mut index, "--audio-sample-rate")?;
            }
            "--audio-channels" => {
                recorder.audio.output_channels =
                    parse_next_u16(&args, &mut index, "--audio-channels")?;
            }
            "--audio-bitrate" => {
                recorder.audio.bitrate = next_value(&args, &mut index, "--audio-bitrate")?;
            }
            "--voice-trigger" => {
                recorder.voice_trigger.enabled = true;
                index += 1;
            }
            "--voice-trigger-python" => {
                recorder.voice_trigger.python_path =
                    PathBuf::from(next_value(&args, &mut index, "--voice-trigger-python")?);
            }
            "--voice-trigger-script" => {
                recorder.voice_trigger.script_path =
                    PathBuf::from(next_value(&args, &mut index, "--voice-trigger-script")?);
            }
            "--voice-trigger-model" => {
                recorder.voice_trigger.model_path =
                    PathBuf::from(next_value(&args, &mut index, "--voice-trigger-model")?);
            }
            "--voice-trigger-threshold" => {
                recorder.voice_trigger.threshold =
                    parse_next_f32(&args, &mut index, "--voice-trigger-threshold")?;
            }
            "--voice-trigger-cooldown-seconds" => {
                recorder.voice_trigger.cooldown_seconds =
                    parse_next_f32(&args, &mut index, "--voice-trigger-cooldown-seconds")?;
            }
            "--voice-trigger-device" => {
                recorder.voice_trigger.device =
                    Some(next_value(&args, &mut index, "--voice-trigger-device")?);
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

    if !config.audio.system_audio && !config.audio.microphone {
        return Err("At least one audio source must be enabled".into());
    }

    if config.audio.output_sample_rate == 0 {
        return Err("--audio-sample-rate must be greater than 0".into());
    }

    if !matches!(config.audio.output_channels, 1 | 2) {
        return Err("--audio-channels must be 1 or 2".into());
    }

    if config.audio.bitrate.trim().is_empty() {
        return Err("--audio-bitrate must not be empty".into());
    }

    if config.voice_trigger.enabled {
        if !config.voice_trigger.python_path.exists() {
            return Err(format!(
                "--voice-trigger-python does not exist: {}",
                config.voice_trigger.python_path.display()
            )
            .into());
        }

        if !config.voice_trigger.script_path.exists() {
            return Err(format!(
                "--voice-trigger-script does not exist: {}",
                config.voice_trigger.script_path.display()
            )
            .into());
        }

        if !config.voice_trigger.model_path.exists() {
            return Err(format!(
                "--voice-trigger-model does not exist: {}",
                config.voice_trigger.model_path.display()
            )
            .into());
        }

        if !(0.0..=1.0).contains(&config.voice_trigger.threshold) {
            return Err("--voice-trigger-threshold must be between 0 and 1".into());
        }

        if config.voice_trigger.cooldown_seconds < 0.0 {
            return Err("--voice-trigger-cooldown-seconds must not be negative".into());
        }
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

fn parse_next_u16(args: &[String], index: &mut usize, flag: &str) -> AppResult<u16> {
    let value = next_value(args, index, flag)?;
    parse_u16(&value, flag)
}

fn parse_next_f32(args: &[String], index: &mut usize, flag: &str) -> AppResult<f32> {
    let value = next_value(args, index, flag)?;
    parse_f32(&value, flag)
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

fn parse_u16(value: &str, flag: &str) -> AppResult<u16> {
    value
        .parse::<u16>()
        .map_err(|error| format!("{flag} must be a positive integer: {error}").into())
}

fn parse_f32(value: &str, flag: &str) -> AppResult<f32> {
    value
        .parse::<f32>()
        .map_err(|error| format!("{flag} must be a number: {error}").into())
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
  --output-dir <path>
  --no-system-audio
  --microphone
  --audio-output-device-id <wasapi-device-id>
  --audio-input-device-id <wasapi-device-id>
  --audio-sample-rate <hz>     (default: 48000)
  --audio-channels <channels>  (default: 2)
  --audio-bitrate <bitrate>    (default: 160k)
  --voice-trigger
  --voice-trigger-python <path>             (default: .venv-openwakeword\\Scripts\\python.exe)
  --voice-trigger-script <path>             (default: scripts\\monitor_openwakeword_mic.py)
  --voice-trigger-model <path>              (default: models\\wakeword\\clip_that_v2.onnx)
  --voice-trigger-threshold <0..1>          (default: 0.65)
  --voice-trigger-cooldown-seconds <secs>   (default: 3)
  --voice-trigger-device <sounddevice-id-or-name>"
}
