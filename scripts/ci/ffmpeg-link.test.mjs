import assert from 'node:assert/strict';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

const script = readFileSync(new URL('./build-ffmpeg.sh', import.meta.url), 'utf8');

// Evaluate the actual Bash array assignment: whitespace inside --extra-ldflags
// must survive as ONE configure argument, not a bare -static argument.
function windowsConfigureArgs() {
  const branch = script.split('platform=win32')[1]?.split(';;')[0];
  assert.ok(branch, 'Windows configure branch exists');
  const assignment = branch.match(/extra=\([\s\S]*?\)/)?.[0];
  assert.ok(assignment, 'Windows configure array exists');
  const result = spawnSync('bash', ['-c', `${assignment}\nprintf '%s\\0' "\${extra[@]}"`], { encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.split('\0').filter(Boolean);
}

test('Windows selects static toolchain runtimes independently of FFmpeg threading', () => {
  const args = windowsConfigureArgs();
  assert.ok(args.includes('--disable-pthreads'));
  assert.ok(args.includes('--enable-w32threads'));
  const ldargs = args.filter(arg => arg.startsWith('--extra-ldflags='));
  assert.equal(ldargs.length, 1);
  const flags = ldargs[0].slice('--extra-ldflags='.length).split(/\s+/);
  assert.ok(flags.includes('-static'), 'static-libgcc alone still permits the default pthread DLL import');
  assert.ok(flags.includes('-static-libgcc'));
  assert.ok(!flags.includes('-Wl,-Bdynamic'), 'do not restore dynamic default runtime lookup');
  assert.ok(!args.includes('-static'), 'link flags must remain inside the configure argument');
  assert.match(script, /VERSION=8\.1\.3\n/);
  assert.match(script, /SHA256=7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3\n/);
});

test('static runtime notices are copied from installed packages and missing licenses fail closed', () => {
  const helper = script.match(/^copy_windows_runtime_notices\(\) \{[\s\S]*?^\}/m)?.[0];
  assert.ok(helper, 'build must preserve installed runtime license texts');
  assert.match(script, /copy_windows_runtime_notices "\/ucrt64\/share\/licenses" "\$BASE\/install\/notices"/);
  const parent = fileURLToPath(new URL('../../.work/ci/', import.meta.url));
  mkdirSync(parent, { recursive: true });
  const root = mkdtempSync(join(parent, 'link-notices-test-'));
  const src = join(root, 'source');
  const dest = join(root, 'notices');
  const invoke = () => spawnSync('bash', ['-c', `set -euo pipefail\n${helper}\ncopy_windows_runtime_notices "$1" "$2"`, 'test', src, dest], { encoding: 'utf8' });
  try {
    for (const runtime of ['gcc-libs', 'libgcc']) {
      rmSync(src, { recursive: true, force: true });
      rmSync(dest, { recursive: true, force: true });
      const files = ['winpthreads/COPYING', 'crt/COPYING', 'crt/COPYING.MinGW-w64.txt',
        'crt/COPYING.MinGW-w64-runtime.txt', `${runtime}/COPYING3`, `${runtime}/COPYING.RUNTIME`];
      for (const file of files) {
        mkdirSync(join(src, file, '..'), { recursive: true });
        writeFileSync(join(src, file), `fixture license ${file}\n`);
      }
      let result = invoke();
      assert.equal(result.status, 0, result.stderr);
      for (const file of files) assert.equal(readFileSync(join(dest, file), 'utf8'), readFileSync(join(src, file), 'utf8'));
      result = invoke();
      assert.equal(result.status, 0, result.stderr);
      assert.equal(existsSync(join(dest, runtime, runtime)), false, 'rerun must not nest notices');
      rmSync(join(src, 'winpthreads/COPYING'));
      assert.notEqual(invoke().status, 0, 'missing winpthreads notice must fail');
      writeFileSync(join(src, 'winpthreads/COPYING'), 'restored fixture');
      rmSync(join(src, runtime, 'COPYING.RUNTIME'));
      assert.notEqual(invoke().status, 0, 'missing GCC runtime exception must fail');
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
