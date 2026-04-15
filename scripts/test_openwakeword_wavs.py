#!/usr/bin/env python3
"""Test an openWakeWord model against local 16 kHz mono WAV files.

This script is intentionally separate from the Rust app. Use it to tune a
custom wake-word model and threshold before wiring voice triggers into the
recorder.
"""

from __future__ import annotations

import argparse
import csv
import glob
import sys
import wave
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


SAMPLE_RATE = 16_000


@dataclass
class FileResult:
    path: Path
    status: str
    label: str
    max_score: float
    first_hit_seconds: float | None
    hit_count: int
    frame_count: int
    error: str | None = None


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Run an openWakeWord .onnx/.tflite model against WAV files and "
            "print max scores plus threshold hits."
        )
    )
    parser.add_argument(
        "--model",
        required=True,
        type=Path,
        help="Path to the trained openWakeWord model, for example models/wakeword/clip_that.onnx.",
    )
    parser.add_argument(
        "wav",
        nargs="+",
        help="WAV file paths or glob patterns, for example test-audio/*.wav.",
    )
    parser.add_argument(
        "--threshold",
        type=float,
        default=0.65,
        help="Score required to count as a detection. Default: 0.65.",
    )
    parser.add_argument(
        "--chunk-ms",
        type=int,
        default=80,
        help="Prediction chunk size in milliseconds. Must divide evenly into 16 kHz. Default: 80.",
    )
    parser.add_argument(
        "--padding-seconds",
        type=int,
        default=1,
        help="Silence padding added before and after each clip. Default: 1.",
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
        "--show-frames",
        action="store_true",
        help="Print every frame at or above --frame-min-score.",
    )
    parser.add_argument(
        "--frame-min-score",
        type=float,
        default=None,
        help="Minimum frame score printed by --show-frames. Defaults to --threshold.",
    )
    parser.add_argument(
        "--csv",
        type=Path,
        help="Optional path to write a CSV summary.",
    )
    return parser.parse_args()


def expand_wavs(patterns: Iterable[str]) -> list[Path]:
    paths: list[Path] = []
    for pattern in patterns:
        matches = glob.glob(pattern)
        if matches:
            paths.extend(Path(match) for match in matches)
        else:
            paths.append(Path(pattern))

    deduped: list[Path] = []
    seen: set[Path] = set()
    for path in paths:
        resolved = path.resolve()
        if resolved not in seen:
            seen.add(resolved)
            deduped.append(path)
    return sorted(deduped)


def load_wav(path: Path) -> np.ndarray:
    with wave.open(str(path), "rb") as wav:
        channels = wav.getnchannels()
        sample_width = wav.getsampwidth()
        sample_rate = wav.getframerate()
        frame_count = wav.getnframes()
        compression = wav.getcomptype()
        raw = wav.readframes(frame_count)

    if compression != "NONE":
        raise ValueError(f"compressed WAV is not supported: comptype={compression}")
    if sample_width != 2:
        raise ValueError(f"expected 16-bit PCM WAV, got sample_width={sample_width} bytes")
    if sample_rate != SAMPLE_RATE:
        raise ValueError(f"expected {SAMPLE_RATE} Hz WAV, got {sample_rate} Hz")
    if channels < 1:
        raise ValueError("WAV has no audio channels")

    audio = np.frombuffer(raw, dtype=np.int16)
    if channels == 1:
        return audio

    audio = audio.reshape(-1, channels).astype(np.int32)
    return np.clip(audio.mean(axis=1), -32768, 32767).astype(np.int16)


def score_file(
    model: object,
    path: Path,
    threshold: float,
    chunk_samples: int,
    padding_seconds: int,
    label: str | None,
    show_frames: bool,
    frame_min_score: float,
) -> FileResult:
    audio = load_wav(path)
    model.reset()
    predictions = model.predict_clip(
        audio,
        padding=padding_seconds,
        chunk_size=chunk_samples,
    )

    scores: list[float] = []
    labels: list[str] = []
    for prediction in predictions:
        if label:
            if label not in prediction:
                available = ", ".join(prediction.keys())
                raise ValueError(f"label '{label}' not found; available labels: {available}")
            labels.append(label)
            scores.append(float(prediction[label]))
        elif prediction:
            best_label, best_score = max(prediction.items(), key=lambda item: item[1])
            labels.append(best_label)
            scores.append(float(best_score))
        else:
            labels.append("")
            scores.append(0.0)

    max_score = max(scores, default=0.0)
    max_label = labels[scores.index(max_score)] if scores else label or ""
    hit_indexes = [index for index, score in enumerate(scores) if score >= threshold]
    first_hit_seconds = None
    if hit_indexes:
        first_hit_seconds = (hit_indexes[0] * chunk_samples / SAMPLE_RATE) - padding_seconds
        first_hit_seconds = max(0.0, first_hit_seconds)

    if show_frames:
        for index, score in enumerate(scores):
            if score >= frame_min_score:
                seconds = (index * chunk_samples / SAMPLE_RATE) - padding_seconds
                seconds = max(0.0, seconds)
                print(f"    frame {index:04d}  t={seconds:7.2f}s  score={score:.4f}")

    return FileResult(
        path=path,
        status="HIT" if hit_indexes else "MISS",
        label=max_label,
        max_score=max_score,
        first_hit_seconds=first_hit_seconds,
        hit_count=len(hit_indexes),
        frame_count=len(scores),
    )


def print_result(result: FileResult) -> None:
    if result.error:
        print(f"ERROR  {result.path}  {result.error}")
        return

    first_hit = "-" if result.first_hit_seconds is None else f"{result.first_hit_seconds:.2f}s"
    print(
        f"{result.status:5}  max={result.max_score:.4f}  "
        f"first={first_hit:>7}  hits={result.hit_count:3}/{result.frame_count:<3}  "
        f"label={result.label}  {result.path}"
    )


def write_csv(path: Path, results: list[FileResult]) -> None:
    with path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(
            handle,
            fieldnames=[
                "path",
                "status",
                "label",
                "max_score",
                "first_hit_seconds",
                "hit_count",
                "frame_count",
                "error",
            ],
        )
        writer.writeheader()
        for result in results:
            writer.writerow(
                {
                    "path": str(result.path),
                    "status": result.status,
                    "label": result.label,
                    "max_score": f"{result.max_score:.6f}",
                    "first_hit_seconds": (
                        "" if result.first_hit_seconds is None else f"{result.first_hit_seconds:.3f}"
                    ),
                    "hit_count": result.hit_count,
                    "frame_count": result.frame_count,
                    "error": result.error or "",
                }
            )


def main() -> int:
    args = parse_args()

    if not args.model.exists():
        print(f"Model not found: {args.model}", file=sys.stderr)
        return 2

    chunk_samples = SAMPLE_RATE * args.chunk_ms // 1000
    if chunk_samples <= 0 or SAMPLE_RATE * args.chunk_ms % 1000 != 0:
        print("--chunk-ms must convert to a whole number of 16 kHz samples", file=sys.stderr)
        return 2

    wavs = expand_wavs(args.wav)
    if not wavs:
        print("No WAV files matched.", file=sys.stderr)
        return 2

    frame_min_score = args.threshold if args.frame_min_score is None else args.frame_min_score
    try:
        global np
        from openwakeword.model import Model
        import numpy as np
    except ImportError as error:
        print(
            "Missing Python dependency. Install with:\n"
            "  py -m venv .venv-openwakeword\n"
            "  .\\.venv-openwakeword\\Scripts\\python.exe -m pip install -r .\\scripts\\openwakeword-test-requirements.txt\n"
            f"\nOriginal error: {error}",
            file=sys.stderr,
        )
        return 2

    model = Model(
        wakeword_models=[str(args.model)],
        inference_framework="onnx" if args.model.suffix.lower() == ".onnx" else "tflite",
        vad_threshold=args.vad_threshold,
    )

    print(f"model:     {args.model}")
    print(f"threshold: {args.threshold:.3f}")
    print(f"chunk:     {args.chunk_ms} ms ({chunk_samples} samples)")
    print()

    results: list[FileResult] = []
    for wav_path in wavs:
        try:
            result = score_file(
                model=model,
                path=wav_path,
                threshold=args.threshold,
                chunk_samples=chunk_samples,
                padding_seconds=args.padding_seconds,
                label=args.label,
                show_frames=args.show_frames,
                frame_min_score=frame_min_score,
            )
        except Exception as error:  # noqa: BLE001 - this is a test harness; keep going.
            result = FileResult(
                path=wav_path,
                status="ERROR",
                label=args.label or "",
                max_score=0.0,
                first_hit_seconds=None,
                hit_count=0,
                frame_count=0,
                error=str(error),
            )
        print_result(result)
        results.append(result)

    if args.csv:
        write_csv(args.csv, results)
        print()
        print(f"Wrote CSV summary: {args.csv}")

    errors = sum(1 for result in results if result.error)
    hits = sum(1 for result in results if result.status == "HIT")
    print()
    print(f"Summary: {hits} hit(s), {len(results) - hits - errors} miss(es), {errors} error(s)")
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
