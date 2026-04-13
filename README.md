# Windows Rust Record

Windows Rust Record is a Windows replay recorder for saving recent gameplay or desktop highlights. It records the primary display into a rolling replay buffer, saves clips on demand from a global hotkey or tray menu, and can capture system audio plus an optional microphone.

This project is still early, but it is usable from source with Cargo.

## Requirements

- Windows
- Rust and Cargo
- FFmpeg and FFprobe available on `PATH`
- A GPU/driver setup supported by FFmpeg `gfxcapture` and `h264_nvenc`, or the app will try the CPU readback fallback

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
- Save clips to `clips` under the project folder by default.

If the tray icon is hidden, click the `^` overflow arrow near the Windows clock.

## Tray Menu

Right-click the tray icon to open the menu:

- `Save replay`: save the recent replay buffer after the configured post-roll.
- `Pause / resume`: stop or restart recording.
- `Open clips folder`: open the clips directory in File Explorer.
- `Open settings`: open `settings.txt` in Notepad.
- `Reload settings`: reload the settings file and re-register the hotkey.
- `Toggle start with Windows`: create or remove a Startup-folder launcher.
- `Quit`: stop the recorder and exit.

## Hotkey And Settings

The app creates a settings file at:

```text
settings.txt
```

By default, this is under the project folder:

```text
C:\Users\samku\Coding\windows-rust-record-v2\settings.txt
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

When no `--output-dir` is provided, output is written under the project folder:

```text
clips
replay-segments
logs
screenshots
settings.txt
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

Use a different output folder:

```powershell
cargo run -- --microphone --output-dir C:\Recordings\WindowsRustRecord
```

## Troubleshooting

If the app says FFmpeg is missing, make sure `ffmpeg.exe` and `ffprobe.exe` are installed and available on `PATH`.

If the global hotkey fails to register, another app may already be using it. Change `hotkey` in `settings.txt`, save the file, and choose `Reload settings` from the tray menu.

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

- This is not packaged as an installer yet.
- Settings are edited through a text file, not a full settings window.
- Device selection currently uses WASAPI device IDs rather than a friendly device picker.
- The tray app still runs from a console while developing with Cargo.
