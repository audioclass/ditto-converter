import { test } from 'node:test';
import assert from 'node:assert/strict';
const logic = await import('../src/logic.ts') as Record<string, any>;
test('trim validation rejects empty, reversed, and beyond-duration intervals', () => {
  assert.equal(typeof logic.validTrim, 'function', 'validTrim is not implemented');
  assert.equal(logic.validTrim(0, null, null), true);
  assert.equal(logic.validTrim(5, 5, 10), false);
  assert.equal(logic.validTrim(5, 4, 10), false);
  assert.equal(logic.validTrim(0, 11, 10), false);
  assert.equal(logic.validTrim(-1, 5, 10), false);
  assert.equal(logic.validTrim(11, null, 10), false);
  assert.equal(logic.validTrim(1.25, 9.75, 10), true);
});
