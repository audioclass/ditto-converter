import { test } from 'node:test';
import assert from 'node:assert/strict';
// Dynamic import lets RED assert missing behavior rather than a loader error.
const logic = await import('../src/logic.ts').catch(() => ({})) as Record<string, any>;
test('timestamps accept seconds and colon notation, rejecting malformed input', () => {
  assert.equal(typeof logic.parseTime, 'function', 'parseTime is not implemented');
  assert.equal(logic.parseTime('1:02.250'), 62.25);
  assert.equal(logic.parseTime('90.5'), 90.5);
  assert.equal(logic.parseTime('1:60'), null);
  assert.equal(logic.parseTime('-1'), null);
  assert.equal(logic.parseTime(''), null);
  assert.equal(logic.parseTime('Infinity'), null);
});
test('queue derivation preserves provider order and excludes terminal jobs from conversion', () => {
  assert.equal(typeof logic.queueView, 'function', 'queueView is not implemented');
  const jobs = [
    { id: 'b', position: 2, selected: true, state: 'complete' },
    { id: 'a', position: 1, selected: true, state: 'queued' },
    { id: 'c', position: 3, selected: false, state: 'queued' },
  ];
  assert.deepEqual(logic.queueView(jobs), { ordered: [jobs[1], jobs[0], jobs[2]], selected: 2, pending: 1, complete: 1, active: false });
  assert.equal(jobs[0].id, 'b');
});
