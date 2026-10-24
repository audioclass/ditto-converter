#!/usr/bin/env python3
"""CI-only display wrapper. No tool downloads; never creates acceptance evidence."""
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import time


def save(path, value):
    path = Path(path)
    temporary = path.with_suffix('.tmp')
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')
    temporary.replace(path)


def inventory():
    # lstart rejects visibly different identities, but has only second resolution.
    result = subprocess.run(['ps', '-axo', 'pid=,ppid=,pgid=,stat=,lstart='],
                            capture_output=True, text=True, timeout=2, check=True)
    rows = {}
    for line in result.stdout.splitlines():
        fields = line.split(None, 4)
        if len(fields) == 5:
            rows[int(fields[0])] = (int(fields[1]), int(fields[2]), fields[3], fields[4])
    return rows


def alive(pid):
    row = inventory().get(pid)
    return row is not None and not row[2].startswith('Z')


def supervise(command, log, seconds, grace=3):
    """Track owned descendants including detached smoke process groups.

    Sampling is best effort, not an OS containment boundary: a child that double
    forks and reparents between samples cannot be proven owned. Descendant
    lstart comparisons and numeric signals are not atomic kernel handles; coarse
    timestamps and sample-to-signal races remain. This test driver does not launch
    arbitrary commands from media metadata.
    """
    cancelled = False
    def interrupt(_sig, _frame):
        nonlocal cancelled
        cancelled = True
    previous = {s: signal.signal(s, interrupt) for s in (signal.SIGTERM, signal.SIGINT)}
    child = None
    tracked = {}
    status = {'exit': 1, 'timedOut': False, 'forcedKill': False,
              'cancelled': False, 'cleanupVerified': False}
    try:
        with Path(log).open('ab') as output:
            child = subprocess.Popen(command, stdout=output, stderr=output,
                                     stdin=subprocess.DEVNULL, start_new_session=True)
            status['pid'] = child.pid
            deadline = time.monotonic() + seconds
            # Capture before poll/wait can reap even a rapidly exited leader.
            # An unreaped direct child pins its PID, including while a zombie.
            initial = inventory().get(child.pid)
            if initial is None:
                raise OSError('Cannot establish direct-child identity')
            tracked[child.pid] = initial[3]
            def sample():
                rows = inventory()
                owned = {pid for pid, identity in tracked.items()
                         if pid in rows and rows[pid][3] == identity}
                while True:
                    more = {pid for pid, row in rows.items()
                            if pid != child.pid and row[0] in owned and
                            (pid not in tracked or tracked[pid] == row[3])}
                    if more <= owned:
                        break
                    owned |= more
                for pid in owned:
                    tracked.setdefault(pid, rows[pid][3])
                return rows
            def send(sig):
                rows = sample()
                for pid, identity in tracked.items():
                    if pid in rows and rows[pid][3] == identity:
                        try:
                            os.kill(pid, sig)
                        except ProcessLookupError:
                            pass
            while True:
                sample()
                # Sample before reaping the leader to retain descendant ownership.
                if child.poll() is not None:
                    # Never signal or re-adopt the released leader PID, even if
                    # a replacement shares ps's coarse, second-resolution lstart.
                    tracked.pop(child.pid, None)
                    break
                if cancelled or time.monotonic() >= deadline:
                    status['cancelled'] = cancelled
                    status['timedOut'] = not cancelled
                    break
                time.sleep(.05)
            send(signal.SIGTERM)
            end = time.monotonic() + grace
            while time.monotonic() < end:
                rows = sample()
                if not any(pid in rows and rows[pid][3] == ident and not rows[pid][2].startswith('Z')
                           for pid, ident in tracked.items()):
                    break
                time.sleep(.05)
            rows = sample()
            if any(pid in rows and rows[pid][3] == ident and not rows[pid][2].startswith('Z')
                   for pid, ident in tracked.items()):
                status['forcedKill'] = True
                send(signal.SIGKILL)
            try:
                status['exit'] = child.wait(timeout=2)
            except subprocess.TimeoutExpired:
                status['exit'] = 1
            end = time.monotonic() + 1
            while time.monotonic() < end:
                rows = inventory()
                if not any(pid in rows and rows[pid][3] == ident and not rows[pid][2].startswith('Z')
                           for pid, ident in tracked.items()):
                    status['cleanupVerified'] = True
                    break
                time.sleep(.05)
    except (OSError, subprocess.SubprocessError) as error:
        status['error'] = str(error)
        if child is not None:
            # Inventory failure is never cleanup proof. Only Popen's unreaped
            # direct child remains safe to kill; a numeric process group does not.
            try:
                if child.returncode is None:
                    child.kill()
            except ProcessLookupError:
                pass
            try:
                child.wait(timeout=2)
            except subprocess.TimeoutExpired:
                pass
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)
    status['cancelled'] = status['cancelled'] or cancelled
    if status['timedOut'] or status['cancelled'] or not status['cleanupVerified']:
        status['exit'] = status['exit'] or 1
    return status


def helper_outcome(status, report):
    restoration = report.get('restoration', 'unverified') if report else 'unverified_missing_report'
    if status['forcedKill'] or status['timedOut']:
        restoration = 'unverified_after_outer_timeout_or_kill'
    passed = bool(report and status['exit'] == 0 and status['cleanupVerified'] and
                  not status['forcedKill'] and not status['timedOut'] and
                  report.get('switchAttempts') in (0, 1) and
                  report.get('result') == 'smoke_child_exit_zero' and
                  restoration in ('verified', 'not_needed'))
    return {'passed': passed, 'restoration': restoration}


def main():
    if len(sys.argv) > 1 and sys.argv[1] == '--child':
        receipt = Path(sys.argv[2])
        status = supervise(sys.argv[4:], receipt.with_suffix('.log'), float(sys.argv[3]))
        save(receipt, status)
        return status['exit'] if 0 <= status['exit'] <= 255 else 1
    root = Path(__file__).resolve().parents[2]
    evidence = root / '.work/ci/evidence/display-smoke'
    evidence.mkdir(parents=True, exist_ok=True)
    work = root / '.work/ci/display-smoke'
    work.mkdir(parents=True, exist_ok=True)
    receipt = {'scope': 'CI-only-display-smoke-wrapper', 'nativeAcceptance': 'see unchanged native-smoke.json only',
               'sourceSHA': os.environ.get('GITHUB_SHA', ''), 'os': platform.system(),
               'arch': platform.machine(), 'imageOS': os.environ.get('ImageOS', ''),
               'imageVersion': os.environ.get('ImageVersion', ''),
               'runID': os.environ.get('GITHUB_RUN_ID', ''),
               'runAttempt': os.environ.get('GITHUB_RUN_ATTEMPT', ''),
               'event': os.environ.get('GITHUB_EVENT_NAME', ''),
               'branch': os.environ.get('GITHUB_REF_NAME', ''),
               'osVersion': platform.mac_ver()[0],
               'restoration': 'unverified', 'driverExit': 1}
    try:
        if platform.system() != 'Darwin' or platform.machine() != 'arm64':
            raise ValueError('Requires native macOS arm64')
        for name in ['display-smoke.swift', 'display-smoke.py', 'smoke.mjs']:
            receipt.setdefault('sources', {})[name] = hashlib.sha256((root/'scripts/ci'/name).read_bytes()).hexdigest()
        revision = subprocess.run(['git','rev-parse','HEAD'],capture_output=True,text=True,timeout=5,check=True).stdout.strip()
        receipt['checkoutSHA'] = revision
        if os.environ.get('GITHUB_ACTIONS') == 'true' and revision != receipt['sourceSHA']:
            raise ValueError('Checkout SHA mismatch')
        version = subprocess.run(['/usr/bin/xcrun','swiftc','--version'],capture_output=True,text=True,timeout=10,check=True)
        receipt['swiftVersion'] = version.stdout[:4000]
        executable = work/'display-smoke-helper'
        compile_status = supervise(['/usr/bin/xcrun','swiftc','-swift-version','5','-module-cache-path',str(work/'module-cache'),str(root/'scripts/ci/display-smoke.swift'),'-o',str(executable)], evidence/'compile.txt',60)
        receipt['compile'] = compile_status
        if compile_status['exit'] != 0:
            raise ValueError('Helper compilation failed')
        # 40 min child, 41 min helper, plus 20 s TERM/restoration grace.
        report = evidence/'display.json'
        report.unlink(missing_ok=True)
        status = supervise([str(executable),str(report),sys.executable,str(root/'scripts/ci/display-smoke.py')], evidence/'helper.log',2460,20)
        receipt['helper'] = status
        display = json.loads(report.read_text()) if report.exists() else None
        outcome = helper_outcome(status, display)
        receipt.update(outcome)
        if not outcome['passed']:
            raise ValueError('Display wrapper failed; see display/child reports')
        # Reference, never synthesize or weaken, the unchanged smoke acceptance.
        native = root/'.work/ci/evidence/native-smoke.json'
        receipt['nativeSmokeReportSHA256'] = hashlib.sha256(native.read_bytes()).hexdigest()
        receipt['driverExit'] = 0
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        receipt['error'] = str(error)[:4000]
    finally:
        save(evidence/'runner.json', receipt)
    return receipt['driverExit']


if __name__ == '__main__':
    sys.exit(main())
