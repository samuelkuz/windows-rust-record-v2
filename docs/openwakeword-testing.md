# openWakeWord WAV Testing

Use this before integrating voice triggers into the Rust app. The goal is to prove that a trained model can detect "clip that" without firing on ordinary desktop/game audio.

## Setup

Create a virtual environment and install the test dependencies:

```powershell
py -m venv .venv-openwakeword
.\.venv-openwakeword\Scripts\python.exe -m pip install -r .\scripts\openwakeword-test-requirements.txt
```

Download openWakeWord's required feature-extractor models into the virtual environment:

```powershell
.\.venv-openwakeword\Scripts\python.exe -c "from openwakeword.utils import download_models; download_models([])"
```

This one-time step provides files such as `melspectrogram.onnx` and `embedding_model.onnx`. Your custom wake-word model depends on those openWakeWord feature models at runtime.

Put your trained model somewhere like:

```text
models\wakeword\clip_that.onnx
```

## WAV Format

The test script expects WAV files to be:

- 16-bit PCM
- 16 kHz
- mono preferred; stereo is downmixed by the script

If you have FFmpeg available, convert a recording like this:

```powershell
ffmpeg -i input.wav -ac 1 -ar 16000 -sample_fmt s16 test-audio\positive-01.wav
```

## Run

```powershell
.\.venv-openwakeword\Scripts\python.exe .\scripts\test_openwakeword_wavs.py --model .\models\wakeword\clip_that.onnx --threshold 0.65 .\test-audio\*.wav
```

Print frame-level scores at or above the threshold:

```powershell
.\.venv-openwakeword\Scripts\python.exe .\scripts\test_openwakeword_wavs.py --model .\models\wakeword\clip_that.onnx --threshold 0.65 --show-frames .\test-audio\*.wav
```

Write a CSV summary:

```powershell
.\.venv-openwakeword\Scripts\python.exe .\scripts\test_openwakeword_wavs.py --model .\models\wakeword\clip_that.onnx --threshold 0.65 --csv .\test-audio\results.csv .\test-audio\*.wav
```

## Live Microphone False-Positive Test

List microphone devices if you need to choose one:

```powershell
.\.venv-openwakeword\Scripts\python.exe .\scripts\monitor_openwakeword_mic.py --list-devices
```

Run a 30-minute false-positive test:

```powershell
.\.venv-openwakeword\Scripts\python.exe .\scripts\monitor_openwakeword_mic.py --model .\models\wakeword\clip_that.onnx --threshold 0.65 --duration-minutes 30 --session-note "normal desktop use with game/video audio"
```

The monitor writes a CSV event log in `test-audio`, prints every threshold hit, and prints a short summary at the end.

If you want to see near misses while tuning, print scores above a lower value:

```powershell
.\.venv-openwakeword\Scripts\python.exe .\scripts\monitor_openwakeword_mic.py --model .\models\wakeword\clip_that.onnx --threshold 0.65 --duration-minutes 30 --print-above 0.45
```

For short calibration runs, log near misses to the CSV and show interval peaks:

```powershell
.\.venv-openwakeword\Scripts\python.exe .\scripts\monitor_openwakeword_mic.py --model .\models\wakeword\clip_that.onnx --threshold 0.65 --duration-minutes 2 --print-above 0.30 --log-above 0.30 --show-peaks --status-interval-seconds 10
```

During that run, say "clip that" ten times with a few seconds between attempts. If missed attempts usually peak around `0.50` to `0.64`, the threshold is probably too high. If missed attempts stay very low, check the selected microphone, mic gain, pronunciation, and training data.

If your chosen mic is not the default, pass a device id or part of the device name:

```powershell
.\.venv-openwakeword\Scripts\python.exe .\scripts\monitor_openwakeword_mic.py --model .\models\wakeword\clip_that.onnx --threshold 0.65 --device "Microphone"
```

## Suggested Test Set

Record a few positives:

```text
positive-01.wav  say "clip that" normally
positive-02.wav  say "clip that" quietly
positive-03.wav  say "clip that" while game/video audio is playing
```

Record negatives:

```text
negative-01.wav  silence or room noise
negative-02.wav  normal talking, no wake phrase
negative-03.wav  similar phrases like "click that", "skip that", "clip it"
```

For a first prototype, aim for:

```text
Intentional "clip that": 8/10 or better detections
False triggers: 0-1 in 30 minutes of realistic background audio
```
