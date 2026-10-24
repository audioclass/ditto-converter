"""Portable process tests, not macOS/CoreGraphics acceptance."""
import importlib.util
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import threading
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]

class SupervisorTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location('display_smoke', ROOT / 'scripts/ci/display-smoke.py')
        assert spec is not None and spec.loader is not None
        self.assertTrue(spec.origin and Path(spec.origin).exists(), 'missing real supervisor')
        self.m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.m)
        (ROOT / '.work').mkdir(exist_ok=True)
        self.tmp = tempfile.TemporaryDirectory(dir=ROOT / '.work')
        self.addCleanup(self.tmp.cleanup)
        self.dir = Path(self.tmp.name)

    def run_child(self, script, timeout=3.0):
        return self.m.supervise([sys.executable, '-c', script], self.dir / 'child.log', timeout, .2)

    def replay_ownership(self, initial_failure=False, missing_root=False, late_failure=False,
                         replacement_start='DIFFERENT_START'):
        # Execute the real function; only its OS boundary is simulated. Never signal
        # real PIDs: reuse cannot be forced deterministically or safely on the host.
        events, signals = [], []
        living = {702}
        clock = [0.0]
        class Child:
            pid = 700
            returncode = None
            def poll(self):
                events.append('reap')
                self.returncode = 0
                return 0
            def wait(self, timeout):
                events.append('wait')
                self.returncode = 0
                return 0
            def kill(self):
                events.append('direct-kill')
                if self.returncode is None:
                    signals.append((700, signal.SIGKILL, 'original'))
        child = Child()
        def inventory():
            events.append('inventory')
            if initial_failure or (late_failure and child.returncode is not None):
                raise OSError('inventory unavailable')
            if child.returncode is None:
                if missing_root:
                    return {}
                # Rapid exit: already a zombie, but not reaped, so PID is pinned.
                return {700: (1, 700, 'Z', 'ORIGINAL_START'),
                        702: (700, 702, 'S', 'OWNED_START')}
            rows = {700: (1, 700, 'S', replacement_start),
                    701: (700, 700, 'S', 'UNRELATED_CHILD'),
                    703: (701, 700, 'S', 'UNRELATED_GRANDCHILD')}
            if 702 in living:
                rows[702] = (1, 702, 'S', 'OWNED_START')
            return rows
        def kill(pid, sig):
            identity = 'unrelated' if pid in (700, 701, 703) else 'owned'
            signals.append((pid, sig, identity))
            if pid == 702:
                living.discard(pid)
        def monotonic():
            clock[0] += .05
            return clock[0]
        with mock.patch.object(self.m.subprocess, 'Popen', return_value=child), \
             mock.patch.object(self.m, 'inventory', side_effect=inventory), \
             mock.patch.object(self.m, 'Path') as path, \
             mock.patch.object(self.m.os, 'kill', side_effect=kill), \
             mock.patch.object(self.m.os, 'killpg', side_effect=lambda pid, sig: signals.append((pid, sig, 'group'))), \
             mock.patch.object(self.m.signal, 'signal'), \
             mock.patch.object(self.m.time, 'monotonic', side_effect=monotonic), \
             mock.patch.object(self.m.time, 'sleep'):
            path.return_value.open.return_value.__enter__.return_value = io.BytesIO()
            result = self.m.supervise(['in-memory-only'], 'not-created', 3, .2)
        return result, events, signals

    def test_reaped_leader_replacement_and_unrelated_descendants_never_adopted(self):
        result, events, signals = self.replay_ownership()
        self.assertEqual(signals, [(702, signal.SIGTERM, 'owned')],
                         'replacement identities must never receive TERM or KILL')
        self.assertLess(events.index('inventory'), events.index('reap'))
        self.assertTrue(result['cleanupVerified'])
        self.assertEqual(result['exit'], 0)
        self.assertFalse(result['forcedKill'])

    def test_reaped_leader_replacement_with_same_coarse_start_is_not_owned(self):
        # Coverage beyond the review's different-start proof: releasing a PID
        # invalidates root authority even when ps cannot distinguish its lstart.
        result, events, signals = self.replay_ownership(replacement_start='ORIGINAL_START')
        self.assertEqual(signals, [(702, signal.SIGTERM, 'owned')])
        self.assertTrue(result['cleanupVerified'])
        self.assertEqual(result['exit'], 0)

    def test_inventory_failure_after_reap_never_signals_recycled_group(self):
        result, events, signals = self.replay_ownership(late_failure=True)
        self.assertEqual(signals, [], 'reaped PID is not authority to kill a group')
        self.assertFalse(result['cleanupVerified'])
        self.assertNotEqual(result['exit'], 0)
        self.assertIn('inventory unavailable', result['error'])

    def test_initial_inventory_failure_cleans_only_unreaped_direct_child(self):
        result, events, signals = self.replay_ownership(initial_failure=True)
        self.assertEqual(signals, [(700, signal.SIGKILL, 'original')])
        self.assertLess(events.index('inventory'), events.index('direct-kill'))
        self.assertLess(events.index('direct-kill'), events.index('wait'))
        self.assertNotIn('reap', events)
        self.assertFalse(result['cleanupVerified'])
        self.assertNotEqual(result['exit'], 0)

    def test_missing_initial_leader_identity_fails_before_pid_release(self):
        result, events, signals = self.replay_ownership(missing_root=True)
        self.assertEqual(signals, [(700, signal.SIGKILL, 'original')])
        self.assertNotIn('reap', events)
        self.assertIn('Cannot establish direct-child identity', result['error'])
        self.assertFalse(result['cleanupVerified'])
        self.assertNotEqual(result['exit'], 0)

    def test_real_child_and_nonzero(self):
        self.assertEqual(self.run_child("print('child')")['exit'], 0)
        self.assertEqual(self.run_child('raise SystemExit(7)')['exit'], 7)
        self.assertIn('child', (self.dir / 'child.log').read_text())

    def test_timeout_kills_term_ignoring_child_and_is_never_success(self):
        t = time.monotonic()
        r = self.run_child('import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(30)', .2)
        self.assertTrue(r['timedOut'])
        self.assertTrue(r['forcedKill'])
        self.assertNotEqual(r['exit'], 0)
        self.assertLess(time.monotonic()-t, 4)

    def test_detached_descendant_is_cleaned(self):
        marker = self.dir / 'descendant'
        script = "import subprocess,sys,time; p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)'],start_new_session=True); open(sys.argv[1],'w').write(str(p.pid)); time.sleep(30)"
        code = 'import sys; sys.argv=[' + repr('fixture') + ',' + repr(str(marker)) + '];' + script
        r = self.run_child(code, .5)
        pid = int(marker.read_text())
        self.assertTrue(r['timedOut'])
        self.assertTrue(r['cleanupVerified'])
        self.assertFalse(self.m.alive(pid))

    def test_cancellation_receipt_and_child_cleanup(self):
        receipt = self.dir / 'status.json'
        p = subprocess.Popen([sys.executable, str(ROOT/'scripts/ci/display-smoke.py'), '--child', str(receipt), '30', sys.executable, '-c', 'import time; time.sleep(30)'])
        try:
            time.sleep(.3)
            p.send_signal(signal.SIGTERM)
            self.assertNotEqual(p.wait(timeout=6), 0)
            r = json.loads(receipt.read_text())
            self.assertTrue(r['cancelled'])
            self.assertTrue(r['cleanupVerified'])
            self.assertFalse(self.m.alive(r['pid']))
        finally:
            if p.poll() is None:
                p.kill(); p.wait(timeout=2)

    def test_helper_timeout_never_claims_restoration_even_with_optimistic_report(self):
        status = {'exit': 0, 'timedOut': True, 'forcedKill': False, 'cleanupVerified': True}
        report = {'restoration': 'verified', 'switchAttempts': 1, 'result': 'smoke_child_exit_zero'}
        result = self.m.helper_outcome(status, report)
        self.assertFalse(result['passed'])
        self.assertIn('unverified', result['restoration'])

    def test_helper_requires_report_and_preserves_failure_and_restore_failure(self):
        status = {'exit': 0, 'timedOut': False, 'forcedKill': False, 'cleanupVerified': True}
        self.assertFalse(self.m.helper_outcome(status, None)['passed'])
        report = {'restoration': 'verified', 'switchAttempts': 1, 'result': 'smoke_child_exit_zero'}
        self.assertTrue(self.m.helper_outcome(status, report)['passed'])
        self.assertFalse(self.m.helper_outcome({**status, 'exit': 7}, report)['passed'])
        self.assertFalse(self.m.helper_outcome(status, {**report, 'restoration': 'FAILED'})['passed'])
        self.assertFalse(self.m.helper_outcome(status, {**report, 'switchAttempts': 2})['passed'])

    def test_cancellation_during_descendant_cleanup_cannot_be_success(self):
        # Keep an observed detached descendant alive through the cleanup grace.
        script = "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c','import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(30)'],start_new_session=True); time.sleep(.2)"
        timer = threading.Timer(.4, lambda: os.kill(os.getpid(), signal.SIGTERM))
        timer.start()
        try:
            result = self.m.supervise([sys.executable, '-c', script], self.dir/'cleanup.log', 3, .6)
            self.assertTrue(result['cancelled'])
            self.assertNotEqual(result['exit'], 0)
        finally:
            timer.cancel()
            timer.join()

    def test_nonzero_helper_exit_delegates_observed_live_descendant_cleanup(self):
        # Portable fixture for outer Python ownership, NOT execution of Swift.
        marker = self.dir / 'abandoned-descendant'
        descendant = 'import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(30)'
        script = ('import subprocess,sys,time; '
                  f'p=subprocess.Popen([sys.executable,"-c",{descendant!r}],start_new_session=True); '
                  f'open({str(marker)!r},"w").write(str(p.pid)); '
                  'time.sleep(.3); raise SystemExit(7)')
        result = self.m.supervise([sys.executable, '-c', script], self.dir/'abandoned.log', 3, .2)
        self.assertEqual(result['exit'], 7, 'cleanup must retain original nonzero helper exit')
        self.assertTrue(result['forcedKill'])
        self.assertTrue(result['cleanupVerified'])
        self.assertFalse(result['timedOut'])
        self.assertFalse(self.m.alive(int(marker.read_text())))

    def test_delegated_swift_report_never_becomes_pass_after_outer_cleanup(self):
        status = {'exit': 1, 'timedOut': False, 'forcedKill': False, 'cleanupVerified': True}
        report = {'restoration': 'verified', 'switchAttempts': 1,
                  'result': 'smoke_abandoned_cleanup_delegated',
                  'cleanupOwner': 'outer_python_supervisor',
                  'cleanup': 'unverified_until_outer_report', 'cleanupVerified': False}
        self.assertFalse(self.m.helper_outcome(status, report)['passed'])
        self.assertFalse(self.m.helper_outcome({**status, 'exit': 0}, report)['passed'])

    def test_launch_error_is_persisted(self):
        receipt = self.dir/'error.json'
        r = subprocess.run([sys.executable,str(ROOT/'scripts/ci/display-smoke.py'),'--child',str(receipt),'1','/nonexistent/ditto-child'],timeout=5)
        self.assertNotEqual(r.returncode,0)
        self.assertIn('error', json.loads(receipt.read_text()))

if __name__ == '__main__':
    unittest.main(verbosity=2)
