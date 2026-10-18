import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const config = JSON.parse(readFileSync(new URL('../../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));

test('main window opts into startup workarea fitting without weakening its size constraints', () => {
  const main = config.app.windows.find((window: { label: string }) => window.label === 'main');
  assert.ok(main, 'the main native window must be configured');
  // Tauri excludes Windows side/bottom shadows from its overflow size calculation.
  // Reserve a total 32 physical pixels per axis, not a per-edge or logical margin.
  assert.deepEqual(main.preventOverflow, { width: 32, height: 32 },
    'startup must fit the monitor workarea with a physical decoration/shadow reserve');
  assert.equal(main.width, 1080, 'keep the preferred width on larger workareas');
  assert.equal(main.height, 760, 'keep the preferred height on larger workareas');
  assert.equal(main.minWidth, 780, 'do not silently weaken the minimum width');
  assert.equal(main.minHeight, 600, 'do not silently weaken the minimum height');
});
