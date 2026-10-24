#!/usr/bin/env python3
"""Offline tiny-helper driver; publishes only bounded JSON/text diagnostics."""
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys


def bounded(command, seconds, grace=0):
    """TERM first, then KILL; never claim restoration after a timeout."""
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               start_new_session=True)
    timed_out = False
    forced_kill = False
    try:
        out, err = process.communicate(timeout=seconds)
    except subprocess.TimeoutExpired:
        timed_out = True
        os.killpg(process.pid, signal.SIGTERM)
        try:
            out, err = process.communicate(timeout=grace)
        except subprocess.TimeoutExpired:
            forced_kill = True
            os.killpg(process.pid, signal.SIGKILL)
            out, err = process.communicate()
    return {"exit": process.returncode, "timedOut": timed_out,
            "forcedKill": forced_kill}, out, err


def main():
    root = Path(__file__).resolve().parents[2]
    evidence = root / "display-preflight-evidence"
    evidence.mkdir(exist_ok=False)
    receipt = {"scope": "display-preflight-only", "fullSmokeExecuted": False,
               "os": platform.system(), "arch": platform.machine(),
               "imageOS": os.environ.get("ImageOS", ""),
               "imageVersion": os.environ.get("ImageVersion", ""),
               "sourceSHA": os.environ.get("GITHUB_SHA", ""),
               "event": os.environ.get("GITHUB_EVENT_NAME", ""),
               "branch": os.environ.get("GITHUB_REF_NAME", "")}
    code = 1
    try:
        assert receipt["os"] == "Darwin" and receipt["arch"] == "arm64"
        assert receipt["event"] == "push"
        assert receipt["branch"] == "diagnostic/macos-display-preflight-20261024-a1"
        assert os.environ.get("GITHUB_RUN_ATTEMPT") == "1", "No automatic/manual rerun"
        assert receipt["imageOS"] and receipt["imageVersion"], "Missing image provenance"
        status, out, _ = bounded(["git", "rev-parse", "HEAD"], 5)
        receipt["checkoutSHA"] = out.decode().strip()
        assert status["exit"] == 0 and not status["timedOut"]
        assert len(receipt["sourceSHA"]) == 40 and receipt["checkoutSHA"] == receipt["sourceSHA"], "Checkout SHA mismatch"
        status, out, _ = bounded(["/usr/bin/sw_vers", "-productVersion"], 5)
        assert status["exit"] == 0 and not status["timedOut"]
        receipt["osVersion"] = out.decode().strip()
        assert receipt["osVersion"].split(".")[0] == "15"
        status, out, _ = bounded(["/usr/bin/xcrun", "swiftc", "--version"], 5)
        receipt["swiftVersion"] = out.decode(errors="replace")[:4000]
        assert status["exit"] == 0 and not status["timedOut"]
        executable = root / "display-preflight-helper"
        status, _, err = bounded(["/usr/bin/xcrun", "swiftc", "-swift-version", "5",
            "-module-cache-path", str(root / "display-preflight-module-cache"),
            str(root / "scripts/ci/display-preflight.swift"),
            "-o", str(executable)], 60)
        receipt["compile"] = status
        (evidence / "compile.txt").write_text(err.decode(errors="replace")[:24000])
        assert status["exit"] == 0 and not status["timedOut"], "Helper compilation failed"
        status, _, _ = bounded([str(executable), str(evidence / "display.json")], 105, 15)
        receipt["helper"] = status
        if status["timedOut"]:
            receipt["timeoutRestoration"] = "unverified_until_explicit_report_readback"
        assert not status["timedOut"], "Helper deadline exceeded"
        report = json.loads((evidence / "display.json").read_text())
        assert report["switchAttempts"] in (0, 1)
        if report["switchAttempts"]:
            assert report["restoration"] == "verified", "Restoration not verified"
        assert status["exit"] == 0, report["result"]
        assert report["fullSmokeExecuted"] is False and report["fullSmokePass"] is False
        assert report["result"] in ("already_sufficient_no_change",
                                   "temporary_mode_logical_workarea_verified")
        code = 0
    except (AssertionError, OSError, ValueError) as error:
        receipt["failure"] = str(error)[:1000]
    finally:
        receipt["driverExit"] = code
        (evidence / "runner.json").write_text(json.dumps(receipt, indent=2, sort_keys=True))
    return code


if __name__ == "__main__":
    sys.exit(main())
