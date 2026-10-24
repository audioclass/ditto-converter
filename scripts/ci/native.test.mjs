import test from 'node:test';
import assert from 'node:assert/strict';
import {existsSync,readFileSync} from 'node:fs';
import {join} from 'node:path';
import {binaryArchitecture,assertHost,root} from './native.mjs';
import * as native from './native.mjs';

test('NSIS identity comparison permits only the pinned Tauri bundle marker patch',()=>{
 const original=Buffer.from('PE bytes\0__TAURI_BUNDLE_TYPE_VAR_UNK\0unchanged payload');
 const snapshot=Buffer.from(original);
 assert.equal(typeof native.nsisPatchedApplication,'function');
 const expected=native.nsisPatchedApplication(original);
 assert.equal(expected.toString(),'PE bytes\0__TAURI_BUNDLE_TYPE_VAR_NSS\0unchanged payload');
 assert.deepEqual(original,snapshot);
 assert.throws(()=>native.nsisPatchedApplication(Buffer.from('missing marker')),/marker/);
 assert.throws(()=>native.nsisPatchedApplication(Buffer.concat([original,original])),/marker/);
 const unrelated=Buffer.from(expected);unrelated[0]^=1;
 assert.notDeepEqual(expected,unrelated);
});

test('structured command output parses stdout JSON despite Cargo stderr diagnostics',()=>{
 const metadata={packages:[],resolve:{nodes:[]}};
 const output=native.run(process.execPath,['-e',`process.stdout.write(${JSON.stringify(JSON.stringify(metadata))});process.stderr.write(' Downloading crates ...\\n');`],{stdoutOnly:true});
 assert.deepEqual(JSON.parse(output),metadata);
});
test('stdout-only commands retain stdout and stderr diagnostics on failure',()=>{
 assert.throws(()=>native.run(process.execPath,['-e',"process.stdout.write('partial metadata');process.stderr.write('cargo download failed');process.exitCode=7;"],{stdoutOnly:true}),error=>{
  assert.match(error.message,/failed: 7/);
  assert.match(error.message,/partial metadata/);
  assert.match(error.message,/cargo download failed/);
  return true;
 });
});
test('default command output retains both streams for helper audits and evidence',()=>{
 assert.equal(native.run(process.execPath,['-e',"process.stdout.write('license');process.stderr.write('configuration');"]),'licenseconfiguration');
});

test('PE machine field, not just MZ, distinguishes x64/arm64/x86',()=>{
 for(const [machine,arch] of [[0x8664,'x64'],[0xaa64,'arm64'],[0x14c,'x86']]){
  const b=Buffer.alloc(128);b.write('MZ');b.writeUInt32LE(64,60);b.write('PE\0\0',64);b.writeUInt16LE(machine,68);
  assert.equal(binaryArchitecture(b),arch);
 }
});
test('thin Mach-O CPU type distinguishes arm64 and Intel',()=>{
 for(const [cpu,arch] of [[0x100000c,'arm64'],[0x1000007,'x64']]){
  const b=Buffer.alloc(32);b.writeUInt32LE(0xfeedfacf);b.writeUInt32LE(cpu,4);assert.equal(binaryArchitecture(b),arch);
 }
});
test('truncated, invalid and ELF inputs never masquerade as native',()=>{
 for(const b of [Buffer.alloc(0),Buffer.from('MZ'),Buffer.from([0x7f,69,76,70]),Buffer.alloc(64)])assert.equal(binaryArchitecture(b),'unknown');
 const b=Buffer.alloc(64);b.write('MZ');b.writeUInt32LE(0xffffffff,60);assert.equal(binaryArchitecture(b),'unknown');
});
test('Linux and mismatched native targets are rejected',()=>{
 assert.throws(()=>assertHost('linux','x64'),/Unsupported/);
 if(process.platform!=='win32')assert.throws(()=>assertHost('win32','x64'),/does not match/);
 if(process.platform!=='darwin')assert.throws(()=>assertHost('darwin','arm64'),/does not match/);
});
test('helper license validation accepts wrapped LGPL text but rejects incompatible builds',()=>{
 assert.equal(typeof native.validateHelperLicense,'function');
 assert.doesNotThrow(()=>native.validateHelperLicense('GNU Lesser General Public\nLicense; version 2.1 or later','--disable-everything --disable-network'));
 for(const option of ['gpl','nonfree','version3'])assert.throws(()=>native.validateHelperLicense('GNU Lesser General Public License',`--enable-${option}`),/Unapproved/);
 assert.throws(()=>native.validateHelperLicense('GNU General Public License','--disable-network'),/Unapproved/);
});
test('workflow pins every action SHA, runner labels, budget controls and no privileged PR context',()=>{
 const y=readFileSync(join(root,'.github/workflows/native.yml'),'utf8');
 const refs=[...y.matchAll(/uses:\s*([^\s]+)\s/g)].map(m=>m[1]);
 assert.ok(refs.length>=3);for(const ref of refs)assert.match(ref,/@[a-f0-9]{40}$/);
 assert.match(y,/runner: windows-2025/);assert.match(y,/runner: macos-15/);
 assert.match(y,/contents: read/);assert.match(y,/persist-credentials: false/);
 assert.match(y,/timeout-minutes: 75/);assert.match(y,/cancel-in-progress: true/);
 assert.doesNotMatch(y,/pull_request_target|secrets\.|actions\/cache|continue-on-error|--all-features/);
 assert.match(y,/npm ci/);assert.match(y,/cargo test --workspace --features desktop --locked/);
 assert.match(y,/node --test .*scripts\/ci\/startup-recovery\.test\.mjs/,'CI must execute the published startup-lock contracts');
 assert.match(y,/node --test .*scripts\/ci\/ffmpeg-link\.test\.mjs/,'CI must execute the published Windows helper-link contracts');
 assert.match(y,/cargo clippy --workspace --all-targets --features desktop --locked/);
 assert.match(y,/DITTO_NATIVE_PACKAGE_UPLOAD_AUTHORIZED == 'true'/);
 const uploads=y.split(/uses: actions\/upload-artifact@/).slice(1);
 for(const part of uploads)assert.match(part,/retention-days: 1/);
});
test('Windows checkout keeps executable shell scripts LF',()=>{
 const path=join(root,'.gitattributes');
 assert.ok(existsSync(path),'Shell checkout must have an explicit LF rule');
 assert.match(readFileSync(path,'utf8'),/^\*\.sh\s+text\s+eol=lf$/m);
 assert.doesNotMatch(readFileSync(join(root,'scripts/ci/build-ffmpeg.sh'),'utf8'),/\r\n/);
});
test('macOS corpus uses the full keg-only FFmpeg fixture generator',()=>{
 const workflow=readFileSync(join(root,'.github/workflows/native.yml'),'utf8');
 assert.match(workflow,/brew install ffmpeg-full/);
 assert.match(workflow,/brew --prefix ffmpeg-full/);
 });
 test('Windows uses actual user-owned storage without changing ownership policy',()=>{
 const workflow=readFileSync(join(root,'.github/workflows/native.yml'),'utf8');
 assert.match(workflow,/workdir: 'C:\\Users\\runneradmin\\ditto-converter-ci'/);
 assert.match(workflow,/working-directory: \$\{\{ matrix.workdir \}\}/);
 assert.match(workflow,/Unexpected Windows runner profile/);
 assert.match(workflow,/robocopy \$env:GITHUB_WORKSPACE \$destination/);
 assert.doesNotMatch(workflow,/\/setowner|takeown|Set-Acl|continue-on-error/);
 assert.match(workflow,/path: \$\{\{ matrix.workdir \}\}\/\.work\/ci\/deliverables\//);
 assert.match(workflow,/cargo test -p ditto-core --lib staging::windows::tests --locked/);
 });
test('macOS uploads remain disabled while build/smoke and notice revalidation execute',()=>{
 const workflow=readFileSync(join(root,'.github/workflows/native.yml'),'utf8');
 assert.match(workflow,/if: runner.os == 'Windows' && github.event_name != 'pull_request' && vars.DITTO_NATIVE_PACKAGE_UPLOAD_AUTHORIZED == 'true'/);
 assert.ok(workflow.indexOf('macOS notice/source revalidation')>workflow.indexOf('node scripts/ci/smoke.mjs'));
 assert.match(workflow,/node scripts\/ci\/notices.mjs aarch64-apple-darwin/);
 assert.match(workflow,/node scripts\/ci\/notices.mjs x86_64-pc-windows-msvc/);
 assert.doesNotMatch(workflow,/continue-on-error/);
});
test('full Mac notices and paired MPL sources are staged after inventory but before package build',()=>{
 const workflow=readFileSync(join(root,'.github/workflows/native.yml'),'utf8');
 const inventory=workflow.indexOf('node scripts/ci/native.mjs inventories');
 const collector=workflow.indexOf('node scripts/ci/notices.mjs aarch64-apple-darwin');
 const build=workflow.indexOf('npm run build:desktop');
 const collect=workflow.indexOf('node scripts/ci/native.mjs collect');
 assert.ok(inventory<collector&&collector<build&&build<collect,'inventory -> full Mac collector -> package build -> exact package collection');
 const stage=workflow.slice(workflow.lastIndexOf('      - name:',collector),workflow.indexOf('      - name:',collector));
 assert.match(stage,/if: runner.os == 'macOS'/);
 assert.match(stage,/aarch64-apple-darwin src-tauri\/resources\/notices\/dependencies .work\/ci\/mpl-corresponding-source/);
 assert.match(stage,/Copy-Item .*native-notice-report.json .work\/ci\/evidence\/native-notice-report.json/);
 assert.match(stage,/exit \$result/);
 assert.equal([...workflow.matchAll(/node scripts\/ci\/notices.mjs aarch64-apple-darwin/g)].length,2,'retain final notice/source revalidation');
 assert.match(workflow,/if: runner.os == 'Windows' && github.event_name != 'pull_request' && vars.DITTO_NATIVE_PACKAGE_UPLOAD_AUTHORIZED == 'true'/);
});
test('native source pin matches existing audited Linux source pin',()=>{
 const a=readFileSync(join(root,'scripts/toolchain/build-ffmpeg.sh'),'utf8');
 const b=readFileSync(join(root,'scripts/ci/build-ffmpeg.sh'),'utf8');
 assert.equal(b.match(/^VERSION=(.+)$/m)[1],a.match(/^VERSION=(.+)$/m)[1]);
 assert.equal(b.match(/^SHA256=(.+)$/m)[1],a.match(/^SHA256=(.+)$/m)[1]);
 assert.match(b,/--disable-network/);assert.match(b,/--disable-autodetect/);assert.match(b,/--enable-w32threads/);
 assert.doesNotMatch(b,/--enable-(gpl|version3|nonfree)/);
});
