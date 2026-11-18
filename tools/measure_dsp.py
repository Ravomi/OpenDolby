#!/usr/bin/env python3
"""Offline-only deterministic DSP check. Never plays sound on the speakers.

Requires NumPy and SciPy. Output lives under target/dsp-check, not version control.
"""
import argparse
import json
import subprocess
from pathlib import Path

import numpy as np
from scipy.io import wavfile
from scipy.signal import butter, sosfilt


def rms(x):
    return float(np.sqrt(np.mean(np.square(x.astype(np.float64)))))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--label", default="after")
    parser.add_argument("--binary", default="target/release/dolby-rs")
    parser.add_argument("--check-matching", action="store_true", help="Export matched bypass and assert settled A/B RMS within 0.3 dB")
    args = parser.parse_args()
    out = Path("target/dsp-check")
    out.mkdir(parents=True, exist_ok=True)
    rate = 48000
    rng = np.random.default_rng(2941693)
    samples = rng.normal(size=(rate * 8, 2))
    samples = sosfilt(butter(3, [180, 10000], btype="bandpass", fs=rate, output="sos"), samples, axis=0)
    samples *= 0.035 / rms(samples)
    samples = samples.astype(np.float32)
    source = out / "reference.wav"
    wavfile.write(source, rate, samples)
    report = {"input_rms_dbfs": 20 * np.log10(rms(samples)), "label": args.label}
    for profile in ("dynamic", "music", "movie", "off"):
        dest = out / f"{args.label}-{profile}.wav"
        subprocess.run([str(Path(args.binary).resolve()), "process-wav", str(source), str(dest), "--profile", profile], check=True, stdout=subprocess.DEVNULL)
        _, result = wavfile.read(dest)
        settled = result[rate * 4:]
        reference = samples[rate * 4:]
        report[profile] = {
            "settled_rms_delta_db": float(20 * np.log10(max(rms(settled), 1e-12) / rms(reference))),
            "peak": float(np.max(np.abs(result))),
            "finite": bool(np.isfinite(result).all()),
            "difference_rms": rms(settled - reference),
        }
        if args.check_matching and profile != "off":
            bypass_path = out / f"{args.label}-{profile}-bypass.wav"
            subprocess.run([str(Path(args.binary).resolve()), "process-wav", str(source), str(bypass_path), "--profile", profile, "--bypass"], check=True, stdout=subprocess.DEVNULL)
            _, bypass = wavfile.read(bypass_path)
            delta = float(20 * np.log10(max(rms(settled), 1e-12) / max(rms(bypass[rate * 4:]), 1e-12)))
            report[profile]["matched_wet_vs_bypass_db"] = delta
            assert abs(delta) < 0.3, f"{profile}: unmatched A/B: {delta:.3f} dB"
        assert report[profile]["finite"], f"{profile}: non-finite output"
    path = out / f"{args.label}.json"
    path.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
