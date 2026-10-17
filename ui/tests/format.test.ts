import { test } from 'node:test';
import assert from 'node:assert/strict';
const logic = await import('../src/logic.ts') as Record<string, any>;
test('display timestamps round once before splitting minutes and milliseconds', () => {
  assert.equal(typeof logic.formatTime, 'function', 'formatTime is not implemented');
  assert.equal(logic.formatTime(null), '—');
  assert.equal(logic.formatTime(NaN), '—');
  assert.equal(logic.formatTime(0), '0:00.000');
  assert.equal(logic.formatTime(59.9996), '1:00.000');
  assert.equal(logic.formatTime(62.25), '1:02.250');
  assert.equal(logic.formatTime(-1), '0:00.000');
});
