#!/usr/bin/env python3
"""Exercise the real PipeWire engine through a temporary *silent* null sink.

Does not change default output. Unloads only the module this script creates.
Requires pactl, pw-play, pw-record, NumPy and SciPy.
"""
import json
import signal
import subprocess
import time
from pathlib import Path

import numpy as np
from scipy.io import wavfile


def main():
    out = Path("target/dsp-check")
    out.mkdir(parents=True, exist_ok=True)
    source = out / "reference.wav"
    if not source.exists():
        raise SystemExit("Run tools/measure_dsp.py first")
    existing = subprocess.check_output(["pactl", "list", "short", "sinks"], text=True)
    if "dolby_atmos_rust" in existing or "opendolby_test_speaker" in existing:
        raise SystemExit("Stop the existing Dolby engine before running the isolated test")
    default = subprocess.check_output(["pactl", "get-default-sink"], text=True).strip()
    module = subprocess.check_output([
        "pactl", "load-module", "module-null-sink", "sink_name=opendolby_test_speaker",
        "channels=2", "rate=48000",
    ], text=True).strip()
    children = []
    handles = []
    try:
        log = (out / "pipewire-engine.log").open("w")
        handles.append(log)
        engine = subprocess.Popen(["target/release/dolby-rs", "run", "--profile", "dynamic", "--target-sink", "opendolby_test_speaker"], stdout=log, stderr=log)
        children.append(engine)
        for _ in range(40):
            if engine.poll() is not None:
                raise RuntimeError("Engine exited, see pipewire-engine.log")
            sinks = subprocess.check_output(["pactl", "list", "short", "sinks"], text=True)
            if "dolby_atmos_rust" in sinks:
                break
            time.sleep(0.1)
        else:
            raise RuntimeError("Virtual sink did not appear")
        dest = out / "pipewire-capture.wav"
        recordlog = (out / "pipewire-record.log").open("w")
        handles.append(recordlog)
        recorder = subprocess.Popen([
            "pw-record", "--target", "opendolby_test_speaker", "--properties", "stream.capture.sink=true",
            "--rate", "48000", "--channels", "2", "--format", "f32", "--sample-count", "240000", str(dest),
        ], stdout=recordlog, stderr=recordlog)
        children.append(recorder)
        time.sleep(0.4)
        player = subprocess.Popen(["pw-play", "--target", "dolby_atmos_rust", "--volume", "0.1", str(source)], stdout=subprocess.DEVNULL, stderr=log)
        children.append(player)
        recorder.wait(timeout=12)
        # PipeWire 1.6.11 pw-cat.c exits the sample-limit branch without setting
        # data.drained, so a complete recording can return EXIT_FAILURE (1).
        # Accept that only with an empty error log and the exact requested frames.
        if recorder.returncode not in (0, 1) or (recorder.returncode == 1 and (out / "pipewire-record.log").stat().st_size):
            raise RuntimeError(f"Recording failed with code {recorder.returncode}, see pipewire-record.log")
        rate, data = wavfile.read(dest)
        assert rate == 48000 and data.shape == (240000, 2), "Incomplete recording"
        settled = data[rate:rate*4].astype(np.float64)
        powers = np.sqrt(np.mean(settled**2, axis=0))
        blocks = settled.reshape(-1, 4800, 2)
        silent_blocks = int(np.sum(np.sqrt(np.mean(blocks**2, axis=(1, 2))) < 1e-7))
        links = subprocess.check_output(["pw-link", "-l"], text=True)
        (out / "pipewire-links.txt").write_text(links)
        report = {"rate": rate, "frames": len(data), "recorder_exit_code": recorder.returncode, "channel_rms": powers.tolist(),
                  "silent_100ms_blocks_after_settle": silent_blocks,
                  "finite": bool(np.isfinite(data).all()), "default_unchanged": default == subprocess.check_output(["pactl", "get-default-sink"], text=True).strip()}
        (out / "pipewire-smoke.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report, indent=2))
        assert report["finite"] and report["default_unchanged"]
        assert np.all(powers > 1e-5), "No measurable stereo output"
        assert silent_blocks == 0, "Sustained gaps in actual streaming output"
        assert "opendolby_test_speaker:playback" in links, "Missing explicit test-speaker routing"
    finally:
        for child in reversed(children):
            if child.poll() is None:
                child.send_signal(signal.SIGTERM)
                try:
                    child.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
        for handle in handles:
            handle.close()
        subprocess.run(["pactl", "unload-module", module], check=True)


if __name__ == "__main__":
    main()
