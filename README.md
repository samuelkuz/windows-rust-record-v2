# Windows Rust Record

Windows Rust Record is a Windows replay recorder for saving recent gameplay or desktop highlights. It records the primary display into a rolling replay buffer, saves clips on demand from a global hotkey or tray menu, and can capture system audio plus an optional microphone.

This project is still early, but it can be packaged as a zip with the app and FFmpeg binaries included.

## Requirements

- Windows
- A GPU/driver setup supported by FFmpeg `gfxcapture` and `h264_nvenc`, or the app will try the CPU readback fallback

For packaged builds, FFmpeg is bundled in the zip. For source builds, either keep FFmpeg on `PATH` or place `ffmpeg.exe` and `ffprobe.exe` in `vendor\ffmpeg\bin`.

## Download And Run

1. Download `WindowsRustRecord-<version>-windows-x64.zip`.
2. Extract the zip.
3. Run `windows-rust-record-v2.exe`.

The extracted folder should look like this:

```text
WindowsRustRecord-<version>-windows-x64\
  windows-rust-record-v2.exe
  README.md
  ffmpeg\
    bin\
      ffmpeg.exe
      ffprobe.exe
```

Windows may show SmartScreen the first time the app runs because the executable is not signed yet.

## Build A Zip Release

Install Rust and Cargo, then run:

```powershell
.\scripts\package-release.ps1 -FfmpegBinDir C:\path\to\ffmpeg\bin
```

If `ffmpeg.exe` and `ffprobe.exe` are already on `PATH`, this also works:

```powershell
.\scripts\package-release.ps1
```

The release folder and zip are written to `dist`:

```text
dist\WindowsRustRecord-0.1.0-windows-x64\
dist\WindowsRustRecord-0.1.0-windows-x64.zip
```

Only redistribute FFmpeg builds whose license terms you understand and can comply with.

Check FFmpeg from PowerShell:

```powershell
ffmpeg -version
ffprobe -version
```

## Install From Source

Clone the repository and build it:

```powershell
git clone <repo-url>
cd windows-rust-record-v2
cargo build --release
```

Run the release build:

```powershell
.\target\release\windows-rust-record-v2.exe --microphone
```

For development, you can run directly with Cargo:

```powershell
cargo run -- --microphone
```

System audio is enabled by default. `--microphone` adds the default microphone to the mix.

## Basic Use

Start the app:

```powershell
cargo run -- --microphone
```

The app will:

- Start recording a rolling replay buffer.
- Add a tray icon in the Windows notification area.
- Register the default hotkey: `Ctrl+Alt+S`.
- Save clips to `Videos\Windows Rust Record\clips` by default.

If the tray icon is hidden, click the `^` overflow arrow near the Windows clock.

## Tray Menu

Right-click the tray icon to open the menu:

- `Save replay`: save the recent replay buffer after the configured post-roll.
- `Pause / resume`: stop or restart recording.
- `Open clips folder`: open the clips directory in File Explorer.
- `Open settings`: open `settings.toml` in Notepad.
- `Reload settings`: reload the settings file and re-register the hotkey.
- `Toggle start with Windows`: create or remove a Startup-folder launcher.
- `Quit`: stop the recorder and exit.

## Hotkey And Settings

The app creates a TOML settings file at:

```text
%APPDATA%\Windows Rust Record\settings.toml
```

Example:

```text
C:\Users\<you>\AppData\Roaming\Windows Rust Record\settings.toml
```

Example settings:

```text
hotkey = "Ctrl+Alt+S"
start_with_windows = false
```

To change the hotkey:

1. Choose `Open settings` from the tray menu.
2. Edit the `hotkey` value, for example:

```text
hotkey = "Ctrl+Shift+F9"
```

3. Save the file.
4. Choose `Reload settings` from the tray menu.

Supported hotkey keys include single letters/numbers and function keys like `F1` through `F24`. Supported modifiers are `Ctrl`, `Alt`, `Shift`, and `Win`.

## Output Folders

When no `--output-dir` is provided, user data is written to Windows profile folders:

```text
%USERPROFILE%\Videos\Windows Rust Record\clips
%USERPROFILE%\Videos\Windows Rust Record\screenshots
%LOCALAPPDATA%\Windows Rust Record\replay-segments
%LOCALAPPDATA%\Windows Rust Record\logs
%APPDATA%\Windows Rust Record\settings.toml
```

Clips are saved as MP4 files in `clips`.

Replay segments are temporary rolling `.ts` files in `replay-segments`.

Logs are written to `logs\app.log`.

Screenshots from `--screenshot` are written to `screenshots`.

## Recording A Test Clip

You can record and save a short test clip without using the tray menu:

```powershell
cargo run -- --record-test 10 --microphone
```

This records for 10 seconds and saves a replay clip.

## Useful Options

```text
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
--voice-trigger-python <path>
--voice-trigger-script <path>
--voice-trigger-model <path>
--voice-trigger-threshold <0..1>
--voice-trigger-cooldown-seconds <seconds>
--voice-trigger-device <sounddevice-id-or-name>
```

Examples:

Record system audio only:

```powershell
cargo run --
```

Record system audio and microphone:

```powershell
cargo run -- --microphone
```

Record microphone only:

```powershell
cargo run -- --microphone --no-system-audio
```

Enable the prototype openWakeWord voice trigger:

```powershell
cargo run -- --voice-trigger --voice-trigger-model .\models\wakeword\clip_that_v2.onnx
```

The voice trigger currently runs the Python openWakeWord monitor from `.venv-openwakeword` and saves a replay when it detects the wake phrase.

Use a different output folder:

```powershell
cargo run -- --microphone --output-dir C:\Recordings\WindowsRustRecord
```

## Troubleshooting

If the packaged app says FFmpeg is missing, make sure the extracted app folder contains `ffmpeg\bin\ffmpeg.exe` and `ffmpeg\bin\ffprobe.exe`.

If a source build says FFmpeg is missing, make sure `ffmpeg.exe` and `ffprobe.exe` are installed and available on `PATH`, or place them in `vendor\ffmpeg\bin`.

If the global hotkey fails to register, another app may already be using it. Change `hotkey` in `settings.toml`, save the file, and choose `Reload settings` from the tray menu.

If audio is missing, try running with only system audio first:

```powershell
cargo run --
```

Then try adding microphone capture:

```powershell
cargo run -- --microphone
```

If the GPU capture backend fails at startup, the app logs the error and tries the CPU readback fallback.

## Current Limitations

- This is packaged as a zip, not a full installer.
- Settings are edited through a text file, not a full settings window.
- Device selection currently uses WASAPI device IDs rather than a friendly device picker.
- The tray app still runs from a console while developing with Cargo. Release builds run as a Windows tray app without a console window.
