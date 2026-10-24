// Source-order contract only: this is not native desktop/lock execution evidence.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const main = readFileSync(new URL('../../src-tauri/src/main.rs', import.meta.url), 'utf8');

function assertStartupRecoveryOrder(source) {
  const setup = source.slice(source.indexOf('.setup(|app| {'));
  const acquisition = setup.match(/fs2::FileExt::try_lock_exclusive\(&lock\)\.map_err\([\s\S]*?\)\?;/);
  assert.ok(acquisition, 'startup must propagate exclusive-lock acquisition failure');
  const heldAt = acquisition.index + acquisition[0].length;
  for (const operation of [
    'std::fs::read_dir(&journals)',
    'ditto_core::recover_staging(&path)',
    'std::fs::remove_dir_all(&work)',
    'NativePipeline::new(',
    'Coordinator::new(',
  ]) {
    assert.ok(setup.indexOf(operation) >= heldAt, `${operation} must follow successful lock acquisition`);
  }
  const recoveryAt = setup.indexOf('ditto_core::recover_staging(&path)');
  for (const operation of ['NativePipeline::new(', 'Coordinator::new(']) {
    assert.ok(setup.indexOf(operation) > recoveryAt, `${operation} must follow recovery`);
  }
  const managedAt = setup.indexOf('app.manage(AppState {');
  assert.ok(managedAt > heldAt, 'application state must retain the acquired lock');
  assert.match(setup.slice(managedAt), /_lock:\s*lock\s*,/);
  assert.doesNotMatch(setup.slice(heldAt, managedAt), /drop\(lock\)|(?:FileExt::)?unlock\(&lock\)/,
    'the lock cannot be released before application state takes ownership');
}

test('source contract: startup holds the instance lock before recovery, session cleanup and workers', () => {
  assertStartupRecoveryOrder(main);
});

test('source contract detects recovery moved ahead of the lock', () => {
  const reordered = main.replace('.setup(|app| {', '.setup(|app| { ditto_core::recover_staging(&path);');
  assert.throws(() => assertStartupRecoveryOrder(reordered), /must follow successful lock acquisition/);
});

test('source contract detects worker creation before recovery', () => {
  const reordered = main.replace('let mut recovery_errors', 'Coordinator::new(pipeline.clone(), work); let mut recovery_errors');
  assert.notEqual(reordered, main, 'mutation must find the startup recovery boundary');
  assert.throws(() => assertStartupRecoveryOrder(reordered), /must follow recovery/);
});

test('source contract detects early lock release or loss of state ownership', () => {
  const released = main.replace('let mut recovery_errors', 'drop(lock); let mut recovery_errors');
  assert.notEqual(released, main, 'mutation must find the startup recovery boundary');
  assert.throws(() => assertStartupRecoveryOrder(released), /cannot be released/);
  assert.throws(() => assertStartupRecoveryOrder(main.replace(/_lock:\s*lock\s*,/, '_lock: different_lock,')));
});
