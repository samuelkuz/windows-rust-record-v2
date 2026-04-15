#!/usr/bin/env python3
"""Run an openWakeWord model against the microphone for false-positive testing."""

from __future__ import annotations

import argparse
import csv
import queue
import sys
import time
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path


SAMPLE_RATE = 16_000


@dataclass
class Detection:
    local_time: str
    elapsed_seconds: float
    event: str
    label: str
    score: float


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Monitor a microphone with an openWakeWord model and log threshold "
            "detections for long false-positive tests."
        )
    )
    parser.add_argument(
        "--model",
        type=Path,
        help="Path to the trained openWakeWord model, for example models/wakeword/clip_that.onnx.",
    )
    parser.add_argument(
        "--threshold",
        type=float,
        default=0.65,
        help="Score required to count as a detection. Default: 0.65.",
    )
    parser.add_argument(
        "--duration-minutes",
        type=float,
        default=30.0,
        help="How long to monitor the microphone. Use 0 to run until Ctrl+C. Default: 30.",
    )
    parser.add_argument(
        "--cooldown-seconds",
        type=float,
        default=3.0,
        help="Minimum time between logged detections. Default: 3.",
    )
    parser.add_argument(
        "--chunk-ms",
        type=int,
        default=80,
        help="Prediction chunk size in milliseconds. Default: 80.",
    )
    parser.add_argument(
        "--device",
        help="Optional sounddevice input device id or name substring.",
    )
    parser.add_argument(
        "--list-devices",
        action="store_true",
        help="List available audio devices and exit.",
    )
    parser.add_argument(
        "--label",
        help=(
            "Prediction label to score. Defaults to the model's highest-scoring "
            "label for each frame."
        ),
    )
    parser.add_argument(
        "--vad-threshold",
        type=float,
        default=0.0,
        help="Optional Silero VAD gate from 0 to 1. Default: 0 disables VAD.",
    )
    parser.add_argument(
        "--patience-frames",
        type=int,
        default=0,
        help=(
            "Require this many consecutive above-threshold 80 ms frames before "
            "openWakeWord returns a non-zero score. Default: 0 disables patience."
        ),
    )
    parser.add_argument(
        "--print-above",
        type=float,
        default=None,
        help="Also print non-trigger scores at or above this value.",
    )
    parser.add_argument(
        "--log-above",
        type=float,
        default=None,
        help="Also write non-trigger scores at or above this value to the CSV as NEAR events.",
    )
    parser.add_argument(
        "--show-peaks",
        action="store_true",
        help="Print the highest score seen in each status interval, even if it did not trigger.",
    )
    parser.add_argument(
        "--show-levels",
        action="store_true",
        help="Print microphone RMS/peak levels in each status interval.",
    )
    parser.add_argument(
        "--status-interval-seconds",
        type=float,
        default=60.0,
        help="Print progress every N seconds. Default: 60.",
    )
    parser.add_argument(
        "--csv",
        type=Path,
        help="CSV path for detection events. Defaults to test-audio/mic-monitor-<timestamp>.csv.",
    )
    parser.add_argument(
        "--session-note",
        default="",
        help="Optional note stored in the CSV header, such as 'YouTube and Discord on'.",
    )
    return parser.parse_args()


def import_dependencies():
    try:
        import numpy as np
        import sounddevice as sd
        from openwakeword.model import Model
    except ImportError as error:
        print(
            "Missing Python dependency. Install with:\n"
            "  .\\.venv-openwakeword\\Scripts\\python.exe -m pip install -r .\\scripts\\openwakeword-test-requirements.txt\n"
            f"\nOriginal error: {error}",
            file=sys.stderr,
        )
        raise SystemExit(2) from error

    return np, sd, Model


def model_label_from_path(path: Path) -> str:
    return path.stem


def default_csv_path() -> Path:
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    return Path("test-audio") / f"mic-monitor-{stamp}.csv"


def write_csv_header(
    csv_path: Path,
    model_path: Path,
    threshold: float,
    cooldown_seconds: float,
    session_note: str,
) -> None:
    csv_path.parent.mkdir(parents=True, exist_ok=True)
    with csv_path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.writer(handle)
        writer.writerow(["session_start", datetime.now().isoformat(timespec="seconds")])
        writer.writerow(["model", str(model_path)])
        writer.writerow(["threshold", threshold])
        writer.writerow(["cooldown_seconds", cooldown_seconds])
        writer.writerow(["session_note", session_note])
        writer.writerow([])
        writer.writerow(["local_time", "elapsed_seconds", "event", "label", "score"])


def append_detection(csv_path: Path, detection: Detection) -> None:
    with csv_path.open("a", newline="", encoding="utf-8") as handle:
        writer = csv.writer(handle)
        writer.writerow(
            [
                detection.local_time,
                f"{detection.elapsed_seconds:.3f}",
                detection.event,
                detection.label,
                f"{detection.score:.6f}",
            ]
        )


def resolve_device(sd, device: str | None):
    if not device:
        return None

    try:
        return int(device)
    except ValueError:
        pass

    matches = []
    for index, candidate in enumerate(sd.query_devices()):
        if candidate.get("max_input_channels", 0) > 0 and device.lower() in candidate["name"].lower():
            matches.append((index, candidate["name"]))

    if len(matches) == 1:
        return matches[0][0]
    if not matches:
        raise ValueError(f"No input device matched: {device}")

    formatted = "\n".join(f"  {index}: {name}" for index, name in matches)
    raise ValueError(f"Multiple input devices matched '{device}':\n{formatted}")


def print_devices(sd) -> None:
    print("Input devices:")
    for index, device in enumerate(sd.query_devices()):
        if device.get("max_input_channels", 0) > 0:
            default = " default" if index == sd.default.device[0] else ""
            print(f"  {index:2d}: {device['name']}{default}")


def score_prediction(prediction: dict[str, float], label: str | None) -> tuple[str, float]:
    if label:
        if label not in prediction:
            available = ", ".join(prediction.keys())
            raise ValueError(f"label '{label}' not found; available labels: {available}")
        return label, float(prediction[label])

    if not prediction:
        return "", 0.0

    best_label, best_score = max(prediction.items(), key=lambda item: item[1])
    return best_label, float(best_score)


def main() -> int:
    args = parse_args()
    np, sd, Model = import_dependencies()

    if args.list_devices:
        print_devices(sd)
        return 0

    if not args.model:
        print("--model is required unless --list-devices is used.", file=sys.stderr)
        return 2
    if not args.model.exists():
        print(f"Model not found: {args.model}", file=sys.stderr)
        return 2

    chunk_samples = SAMPLE_RATE * args.chunk_ms // 1000
    if chunk_samples <= 0 or SAMPLE_RATE * args.chunk_ms % 1000 != 0:
        print("--chunk-ms must convert to a whole number of 16 kHz samples", file=sys.stderr)
        return 2

    try:
        device = resolve_device(sd, args.device)
    except ValueError as error:
        print(error, file=sys.stderr)
        return 2

    model = Model(
        wakeword_models=[str(args.model)],
        inference_framework="onnx" if args.model.suffix.lower() == ".onnx" else "tflite",
        vad_threshold=args.vad_threshold,
    )

    label = args.label or model_label_from_path(args.model)
    csv_path = args.csv or default_csv_path()
    write_csv_header(
        csv_path=csv_path,
        model_path=args.model,
        threshold=args.threshold,
        cooldown_seconds=args.cooldown_seconds,
        session_note=args.session_note,
    )

    audio_queue: queue.Queue[object] = queue.Queue()

    def audio_callback(indata, frames, time_info, status):  # noqa: ANN001
        if status:
            audio_queue.put(("status", str(status)))
        audio_queue.put(indata.copy().reshape(-1))

    duration_seconds = None if args.duration_minutes == 0 else args.duration_minutes * 60.0
    start = time.monotonic()
    last_status = start
    last_detection = -1_000_000.0
    frame_count = 0
    max_score = 0.0
    interval_max_score = 0.0
    interval_max_label = label
    interval_rms_sum = 0.0
    interval_level_frames = 0
    interval_peak_level = 0.0
    detections: list[Detection] = []
    patience = {label: args.patience_frames} if args.patience_frames > 0 else {}
    thresholds = {label: args.threshold} if args.patience_frames > 0 else {}

    print(f"model:      {args.model}")
    print(f"label:      {label}")
    print(f"threshold:  {args.threshold:.3f}")
    print(f"cooldown:   {args.cooldown_seconds:.1f}s")
    print(f"duration:   {'until Ctrl+C' if duration_seconds is None else f'{args.duration_minutes:g} minutes'}")
    print(f"csv:        {csv_path}")
    print()
    print("Listening. Use your computer normally; press Ctrl+C to stop early.")
    print("When a trigger prints, note what was happening at that clock time.")
    print()

    try:
        with sd.InputStream(
            samplerate=SAMPLE_RATE,
            blocksize=chunk_samples,
            channels=1,
            dtype="int16",
            device=device,
            callback=audio_callback,
        ):
            while True:
                now = time.monotonic()
                elapsed = now - start
                if duration_seconds is not None and elapsed >= duration_seconds:
                    break

                timeout = 0.25
                try:
                    item = audio_queue.get(timeout=timeout)
                except queue.Empty:
                    item = None

                if isinstance(item, tuple) and item[0] == "status":
                    print(f"audio status: {item[1]}", file=sys.stderr)
                    continue
                if item is None:
                    pass
                else:
                    audio = np.asarray(item, dtype=np.int16)
                    audio_float = audio.astype(np.float32) / 32768.0
                    rms_level = float(np.sqrt(np.mean(audio_float * audio_float)))
                    peak_level = float(np.max(np.abs(audio_float))) if audio_float.size else 0.0
                    interval_rms_sum += rms_level
                    interval_level_frames += 1
                    interval_peak_level = max(interval_peak_level, peak_level)

                    prediction = model.predict(
                        audio,
                        patience=patience,
                        threshold=thresholds,
                    )
                    frame_count += 1
                    predicted_label, score = score_prediction(prediction, label)
                    max_score = max(max_score, score)
                    if score > interval_max_score:
                        interval_max_score = score
                        interval_max_label = predicted_label

                    if args.print_above is not None and score >= args.print_above:
                        local_time = datetime.now().strftime("%H:%M:%S")
                        print(
                            f"near  {local_time}  elapsed={elapsed:8.1f}s  "
                            f"score={score:.4f}  label={predicted_label}"
                        )

                    if args.log_above is not None and score >= args.log_above and score < args.threshold:
                        append_detection(
                            csv_path,
                            Detection(
                                local_time=datetime.now().isoformat(timespec="seconds"),
                                elapsed_seconds=elapsed,
                                event="NEAR",
                                label=predicted_label,
                                score=score,
                            ),
                        )

                    if score >= args.threshold and now - last_detection >= args.cooldown_seconds:
                        detection = Detection(
                            local_time=datetime.now().isoformat(timespec="seconds"),
                            elapsed_seconds=elapsed,
                            event="HIT",
                            label=predicted_label,
                            score=score,
                        )
                        detections.append(detection)
                        append_detection(csv_path, detection)
                        last_detection = now
                        print(
                            f"HIT   {detection.local_time}  "
                            f"elapsed={detection.elapsed_seconds:8.1f}s  "
                            f"score={detection.score:.4f}  label={detection.label}"
                        )

                now = time.monotonic()
                if now - last_status >= args.status_interval_seconds:
                    elapsed = now - start
                    remaining = "unknown" if duration_seconds is None else f"{max(duration_seconds - elapsed, 0):.0f}s"
                    peak = (
                        f" interval_peak={interval_max_score:.4f}/{interval_max_label}"
                        if args.show_peaks
                        else ""
                    )
                    levels = ""
                    if args.show_levels:
                        avg_rms = (
                            interval_rms_sum / interval_level_frames
                            if interval_level_frames
                            else 0.0
                        )
                        levels = f" rms={avg_rms:.4f} peak={interval_peak_level:.4f}"
                    print(
                        f"status elapsed={elapsed:.0f}s remaining={remaining} "
                        f"detections={len(detections)} max_score={max_score:.4f}{peak}{levels}"
                    )
                    interval_max_score = 0.0
                    interval_max_label = label
                    interval_rms_sum = 0.0
                    interval_level_frames = 0
                    interval_peak_level = 0.0
                    last_status = now
    except KeyboardInterrupt:
        print()
        print("Stopped by user.")

    total_elapsed = time.monotonic() - start
    print()
    print("Summary")
    print(f"  elapsed:    {total_elapsed / 60.0:.2f} minutes")
    print(f"  frames:     {frame_count}")
    print(f"  detections: {len(detections)}")
    print(f"  max_score:  {max_score:.4f}")
    print(f"  csv:        {csv_path}")

    if len(detections) <= 1 and total_elapsed >= 30 * 60:
        print("  prototype false-positive check: PASS")
    elif total_elapsed >= 30 * 60:
        print("  prototype false-positive check: REVIEW")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
