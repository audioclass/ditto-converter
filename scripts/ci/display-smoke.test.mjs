import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync,existsSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
const root=fileURLToPath(new URL('../../',import.meta.url));
const swift=()=>readFileSync(new URL('./display-smoke.swift',import.meta.url),'utf8');
test('Swift has no second cleanup signal owner or blocking child wait',()=>{
  assert.doesNotMatch(swift(),/\b(?:kill|killpg|terminate|waitUntilExit)\s*\(/,
    'only Python supervisors may signal owned children; Swift must restore and return');
});
// Source-derived control-flow model ONLY: no Swift/Foundation, PID reuse or native
// signals run here. Old stopping grace is already 9s; completion occurs in pump.
function afterPumpModel({completed=false,cancelled=false,deadline=false}={}) {
  const s=swift();
  const body=s.slice(s.indexOf('func runSmoke()'),s.indexOf('func experiment()'));
  assert.ok(body.includes('pump(0.05)'));
  const signals=[];
  if(/kill\(child.processIdentifier, SIGKILL\)/.test(body))signals.push('raw-kill-after-pump');
  if(/child.terminate\(\)/.test(body)&&(cancelled||deadline))signals.push('duplicate-terminate');
  const delegated=/if interrupted \|\| deadlineExceeded \{([\s\S]*?)\n    \}/.exec(body)?.[1];
  assert.ok(body.indexOf('pump(0.05)')<body.indexOf('if interrupted || deadlineExceeded'));
  assert.ok(body.indexOf('if interrupted || deadlineExceeded')<body.indexOf('guard !child.isRunning'));
  assert.ok(body.indexOf('guard !child.isRunning')<body.indexOf('child.terminationStatus'));
  if(cancelled||deadline) {
    assert.ok(delegated,'missing fail-and-return branch after pump');
    for(const value of ['smoke_abandoned_cleanup_delegated','outer_python_supervisor','unverified_until_outer_report'])assert.ok(delegated.includes(value));
    assert.match(delegated,/report\["cleanupVerified"\] = false/);
    assert.match(delegated,/exitCode = 1/);
    assert.match(delegated,/checkpoint\(\)\s*return/);
    assert.doesNotMatch(delegated,/terminationStatus|terminationReason|processIdentifier/);
    return {signals,exit:1,cleanupVerified:false,terminalReads:0};
  }
  assert.match(body,/guard !child.isRunning else \{[\s\S]*?return\s*\}\s*report\["smokeSupervisorExit"\] = child.terminationStatus/);
  return {signals,exit:completed?0:null,terminalReads:completed?1:0};
}
test('source-derived late completion cannot reach old raw kill after pump',()=>{
  const signals=/kill\(child.processIdentifier, SIGKILL\)/.test(swift())?['raw-kill-after-pump']:[];
  assert.deepEqual(signals,[],'completion during pump must not reach a numeric signal sink');
  assert.deepEqual(afterPumpModel({completed:true}),{signals:[],exit:0,terminalReads:1});
});
test('source-derived cancellation/deadline wins over simultaneous zero completion',()=>{
  for(const completed of [false,true])for(const stop of [{cancelled:true},{deadline:true}]) {
    assert.deepEqual(afterPumpModel({completed,...stop}),{signals:[],exit:1,cleanupVerified:false,terminalReads:0});
  }
});
test('abandoned Process stays retained through restoration and process exit',()=>{
  const s=swift();
  assert.match(s,/let smokeSupervisor = Process\(\)/);
  assert.match(s,/let child = smokeSupervisor/);
  assert.match(s,/withExtendedLifetime\(smokeSupervisor\) \{\s*exit\(reportWriteFailed \? 3 : exitCode\)\s*\}/);
});
test('known terminal nonzero supervisor status is retained only after fresh stopped guard',()=>{
  const s=swift();
  assert.match(s,/guard !child.isRunning else \{[\s\S]*?return\s*\}[\s\S]*?if child.terminationStatus != 0 \{ exitCode = child.terminationStatus \}/);
});
test('cancellation delivered during deferred restoration cannot retain success',()=>{
  assert.match(swift(),/experiment\(\)\nreport\["finalScreens"\] = screensRecord\(\)\nif interrupted \{[\s\S]*?exitCode = 1[\s\S]*?\n\}\ncheckpoint\(\)/);
});
test('macOS wrapper lives across actual smoke child and restores the exact object',()=>{
  const path=new URL('./display-smoke.swift',import.meta.url);
  assert.ok(existsSync(path),'missing same-caller wrapper');
  const s=readFileSync(path,'utf8');
  assert.match(s,/defer \{/);assert.match(s,/CGDisplaySetDisplayMode\(display, original, nil\)/);
  assert.match(s,/while child.isRunning/);assert.match(s,/pump\(0.05\)/);
  assert.doesNotMatch(s,/waitUntilExit/);assert.match(s,/nativeFit\(\)/);
  assert.match(s,/sameMode\(actual, chosen\)/);assert.match(s,/switchAttempts.*= 1/);
});
test('workflow only wraps macOS, invokes portable tests, keeps existing upload roots',()=>{
  const w=readFileSync(new URL('../../.github/workflows/native.yml',import.meta.url),'utf8');
  assert.match(w,/if: runner.os == 'Windows'\n        run: node scripts\/ci\/smoke.mjs/);
  assert.match(w,/if: runner.os == 'macOS'\n        timeout-minutes: 45\n        run: python scripts\/ci\/display-smoke.py/);
  assert.match(w,/node --test[^\n]*scripts\/ci\/display-smoke.test.mjs/);
  assert.doesNotMatch(w,/include-hidden-files|display-preflight.yml/);
});
test('real portable supervisor exit, timeout, cancellation, descendant and receipt regressions', {skip:process.platform==='win32'},()=>{
  const r=spawnSync(process.env.PYTHON??'python3',['scripts/ci/test-display-smoke.py'],{cwd:root,encoding:'utf8',timeout:30000});
  assert.equal(r.status,0,r.stdout+r.stderr);
});
