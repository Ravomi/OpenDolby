#!/usr/bin/env python3
"""Opt-in real desktop check of close/reopen audio lifetime.

Changes the default output briefly, but plays no test sound. Refuses an existing
OpenDolby instance or virtual sink. Restores the original default in finally.
Requires a graphical Umbriel session, pactl, and a local tuning XML.
"""
import argparse
import json
import select
import shutil
import subprocess
import time
from pathlib import Path


def run(*args):
    return subprocess.check_output(args, text=True, timeout=10).strip()


def wait_for(check, label, timeout=10):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        time.sleep(0.1)
    raise AssertionError(f"Timed out: {label}")


def close_panel(pid):
    def window_id():
        watcher = subprocess.Popen(["umbriel", "subscribe", "windows"], text=True,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            if not select.select([watcher.stdout], [], [], 5)[0]:
                raise AssertionError("No compositor window snapshot")
            windows = json.loads(watcher.stdout.readline())["data"]
            return next((w["id"] for w in windows if w.get("pid") == pid), None)
        finally:
            watcher.terminate()
            watcher.communicate(timeout=5)

    run("umbriel", "msg", "window-close", wait_for(window_id, "compositor panel"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--confirm-route-test", action="store_true",
                        help="Allow the temporary default-output change")
    parser.add_argument("--binary", type=Path,
                        default=Path("target/release/dolby-gui"))
    args = parser.parse_args()
    if not args.confirm_route_test:
        parser.error("Pass --confirm-route-test to allow the temporary routing check")
    for tool in ("pactl", "umbriel"):
        if not shutil.which(tool):
            raise SystemExit(f"Missing tool: {tool}")
    binary = str(args.binary.resolve())

    def status():
        return json.loads(run(binary, "--status"))

    initial = status()
    if initial.get("pid") or "dolby_atmos_rust" in run("pactl", "list", "short", "sinks"):
        raise SystemExit("Quit the existing OpenDolby instance before this test")
    original_default = run("pactl", "get-default-sink")
    output = Path("target/acceptance")
    output.mkdir(parents=True, exist_ok=True)
    report = {}
    child = None
    log = (output / "gui-lifecycle.log").open("w")
    try:
        child = subprocess.Popen([binary], stdout=log, stderr=log)
        opened = wait_for(lambda: (s if (s := status()).get("window_open") else None),
                          "initial panel")
        pid = opened["pid"]
        assert pid == child.pid, "The test must own the GUI process"
        run(binary, "--start")
        wait_for(lambda: status().get("running"), "engine start")
        wait_for(lambda: "dolby_atmos_rust" in run("pactl", "list", "short", "sinks"),
                 "virtual sink")
        run("pactl", "set-default-sink", "dolby_atmos_rust")

        close_panel(pid)
        closed = wait_for(lambda: (s if not (s := status()).get("window_open") else None),
                          "panel closure")
        assert closed["running"] and closed["pid"] == pid, "Closing must keep the engine alive"
        assert run("pactl", "get-default-sink") == "dolby_atmos_rust", "Closing changed output"
        report["close_keeps_engine_and_route"] = True

        run(binary)
        reopened = wait_for(lambda: (s if (s := status()).get("window_open") else None),
                            "panel reopen")
        assert reopened["pid"] == pid and reopened["running"], "Reopen must reuse the live engine"
        assert reopened["profile"] == opened["profile"], "Reopen lost the active profile"
        sinks = run("pactl", "list", "short", "sinks").splitlines()
        assert sum(line.split()[1] == "dolby_atmos_rust" for line in sinks) == 1
        report["reopen_reuses_pid_profile_and_single_sink"] = True

        run(binary, "--stop")
        wait_for(lambda: not status().get("running"), "explicit stop")
        wait_for(lambda: "dolby_atmos_rust" not in run("pactl", "list", "short", "sinks"),
                 "virtual sink removal")
        assert run("pactl", "get-default-sink") != "dolby_atmos_rust", "Stop did not restore hardware"
        report["explicit_stop_restores_hardware"] = True
        close_panel(pid)
        child.wait(timeout=10)
        assert not status().get("pid"), "Closing a stopped panel must exit"
        report["closing_stopped_panel_exits"] = True

        child = subprocess.Popen([binary], stdout=log, stderr=log)
        opened = wait_for(lambda: (s if (s := status()).get("window_open") else None),
                          "fresh panel for explicit quit")
        assert opened["pid"] == child.pid, "Fresh launch must own its process"
        run(binary, "--quit")
        child.wait(timeout=10)
        assert not status().get("pid"), "Quit left a background instance"
        report["explicit_quit_exits"] = True
    finally:
        if child is not None and child.poll() is None:
            try:
                run(binary, "--quit")
                child.wait(timeout=10)
            except (subprocess.SubprocessError, OSError):
                child.terminate()
                child.wait(timeout=5)
        subprocess.run(["pactl", "set-default-sink", original_default], check=True, timeout=10)
        log.close()
        report["original_default_restored"] = run("pactl", "get-default-sink") == original_default
        (output / "gui-lifecycle.json").write_text(json.dumps(report, indent=2) + "\n")
    assert report["original_default_restored"]
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
