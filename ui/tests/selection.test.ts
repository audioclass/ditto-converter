import { test } from 'node:test';
import assert from 'node:assert/strict';
const logic = await import('../src/logic.ts') as Record<string, any>;
test('selection updates target stable ids and never change active jobs', () => {
  assert.equal(typeof logic.selectionUpdates, 'function', 'selectionUpdates is not implemented');
  const jobs = [{id:'queued',state:'queued',selected:false}, {id:'busy',state:'converting',selected:false}, {id:'done',state:'complete',selected:true}];
  assert.deepEqual(logic.selectionUpdates(jobs, true), [{id:'queued',selected:true}]);
  assert.deepEqual(logic.selectionUpdates(jobs, false), [{id:'done',selected:false}]);
  assert.equal(jobs[0].selected, false);
});
