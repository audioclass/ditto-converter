import test from 'node:test';
import assert from 'node:assert/strict';

import {mkdtempSync,readFileSync,rmSync,writeFileSync,mkdirSync} from 'node:fs';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';

test('workflow isolates native smoke after production collection and preserves diagnostics',()=>{
  const workflow=readFileSync(new URL('../../.github/workflows/native.yml',import.meta.url),'utf8');
  assert.ok(workflow.indexOf('node scripts/ci/smoke.mjs')>workflow.indexOf('node scripts/ci/native.mjs collect'));
  assert.match(workflow,/node --test[^\n]*scripts\/ci\/smoke.test.mjs/);
  assert.match(workflow,/\.work\/ci\/smoke-runs\/\*\/\*\.log/);
});

mkdirSync(fileURLToPath(new URL('../../.work',import.meta.url)),{recursive:true});
test('collected production packages must be present and hash-correct before smoke',async()=>{
  const {verifyDeliverables}=await import('./smoke.mjs');
  const dir=mkdtempSync(fileURLToPath(new URL('../../.work/ci-smoke-unit-',import.meta.url)));
  try {
    writeFileSync(join(dir,'package.exe'),'real file for checksum unit test');
    const b=readFileSync(join(dir,'package.exe'));
    const records=[{path:'package.exe',bytes:b.length,sha256:createHash('sha256').update(b).digest('hex')}];
    writeFileSync(join(dir,'checksums.json'),JSON.stringify(records));
    assert.equal(verifyDeliverables(dir).length,1);
    writeFileSync(join(dir,'package.exe'),'changed');assert.throws(()=>verifyDeliverables(dir),/mismatch/);
    records[0].path='../outside.exe';writeFileSync(join(dir,'checksums.json'),JSON.stringify(records));
    assert.throws(()=>verifyDeliverables(dir),/Unsafe/);
  } finally {rmSync(dir,{recursive:true,force:true});}
});

// Importing the runner must not build or launch an application.
test('process runner executes real children and rejects exit failure and timeout',async()=>{
  const {runProcess}=await import('./smoke.mjs');
  const dir=mkdtempSync(fileURLToPath(new URL('../../.work/ci-smoke-unit-',import.meta.url)));
  try {
    const log=join(dir,'child.log');
    const result=await runProcess(process.execPath,['-e',"console.log('real child')"],{cwd:dir,log,timeoutMs:5000});
    assert.equal(result.exitCode,0);assert.match(readFileSync(log,'utf8'),/real child/);
    await assert.rejects(runProcess(process.execPath,['-e','process.exit(7)'],{cwd:dir,log,timeoutMs:5000}),/exit=7/);
    await assert.rejects(runProcess(process.execPath,['-e','setInterval(()=>{},1000)'],{cwd:dir,log,timeoutMs:100}),/timeout/);
  } finally {rmSync(dir,{recursive:true,force:true});}
});
test('runner rejects unsupported Linux before writing native acceptance evidence',async()=>{
  const {assertNativeHost}=await import('./smoke.mjs');
  if(process.platform==='linux')assert.throws(()=>assertNativeHost(),/Windows x64 or macOS arm64/);
});
test('required checks stay aligned with the real embedded offline smoke script',async()=>{
  const {requiredChecks}=await import('./smoke.mjs');
  const script=readFileSync(new URL('../native-smoke.js',import.meta.url),'utf8').split('if(cfg.online)')[0];
  const actual=[...script.matchAll(/\bcheck\('([^']+)'/g)].map(match=>match[1]);
  const nativeSizes = JSON.parse(script.match(/for \(const \[width,height\] of (\[\[.*?\]\])\)/)[1]);
  actual.push(...nativeSizes.map(([width,height])=>`actual native ${width}x${height} crowded/error/footer layout`));
  assert.deepEqual(requiredChecks,actual);
});

test('native smoke requires crowded-queue and short error-layout regressions', async () => {
  const {requiredChecks} = await import('./smoke.mjs');
  assert.ok(requiredChecks.includes('crowded queue scroll stays inside workspace'));
  assert.ok(requiredChecks.includes('error banner preserves footer'));
  const script = readFileSync(new URL('../native-smoke.js', import.meta.url), 'utf8');
  assert.match(script, /appRoot\.style\.flex = '0 0 600px'/);
});

test('native smoke previews direct assets on Windows/macOS, matching production',()=>{
  const script=readFileSync(new URL('../native-smoke.js',import.meta.url),'utf8');
  assert.match(script,/const src = window\.__TAURI_INTERNALS__\.convertFileSrc\(path\)/);
  assert.match(script,/const previewSrc = \/Linux\/\.test\(navigator\.userAgent\) \? URL\.createObjectURL\(media\) : src/);
  assert.match(script,/audio\.src = previewSrc/);
  assert.doesNotMatch(script,/new Audio\(URL\.createObjectURL\(media\)\)/);
});

test('report validation requires every existing offline native harness check', async () => {
  const {validateReport, requiredChecks} = await import('./smoke.mjs');
  const report = {passed:true, output:'/export/source.wav', userAgent:'native webview', checks:requiredChecks.map(name=>({name,ok:true}))};
  assert.equal(validateReport(report), report);
  assert.throws(()=>validateReport({...report, checks:[]}), /Missing/);
  assert.throws(()=>validateReport({...report, passed:false}), /failed/);
  assert.throws(()=>validateReport({...report, checks:[...report.checks,{name:'extra',ok:false}]}), /failed/);
  assert.throws(()=>validateReport({...report, output:undefined}), /output/);
  for (const name of requiredChecks) {
    assert.throws(()=>validateReport({...report,checks:report.checks.filter(c=>c.name!==name)}), /Missing/);
    assert.throws(()=>validateReport({...report,checks:[...report.checks,{name,ok:true}]}), /duplicate/);
  }
});

function wav({extensible=false, junk=true}={}) {
  const samples=44100, data=Buffer.alloc(samples*3);
  for(let i=0;i<samples;i++) data.writeIntLE(Math.round(Math.sin(i*2*Math.PI*440/44100)*1000000),i*3,3);
  const fmt=Buffer.alloc(extensible?40:16);
  fmt.writeUInt16LE(extensible?0xfffe:1,0); fmt.writeUInt16LE(1,2); fmt.writeUInt32LE(44100,4);
  fmt.writeUInt32LE(132300,8); fmt.writeUInt16LE(3,12); fmt.writeUInt16LE(24,14);
  if(extensible){fmt.writeUInt16LE(22,16);fmt.writeUInt16LE(24,18);fmt.writeUInt32LE(4,20);Buffer.from('0100000000001000800000aa00389b71','hex').copy(fmt,24);}
  const chunk=(name,b)=>{const h=Buffer.alloc(8);h.write(name);h.writeUInt32LE(b.length,4);return Buffer.concat([h,b,...(b.length%2?[Buffer.alloc(1)]:[])]);};
  const body=Buffer.concat([Buffer.from('WAVE'),...(junk?[chunk('JUNK',Buffer.from('odd'))]:[]),chunk('fmt ',fmt),chunk('data',data)]);
  const header=Buffer.alloc(8);header.write('RIFF');header.writeUInt32LE(body.length,4);return Buffer.concat([header,body]);
}
test('independent WAV inspection walks chunks and validates PCM content, not extension',async()=>{
  const {inspectWav}=await import('./smoke.mjs');
  for(const extensible of [false,true]) {
    const result=inspectWav(wav({extensible}));assert.equal(result.frames,44100);assert.equal(result.duration,1);assert.ok(result.rms>0);
  }
  const b=wav();assert.throws(()=>inspectWav(b.subarray(0,b.length-1)),/RIFF|truncated/);
  const silent=wav();silent.fill(0,silent.length-132300);assert.throws(()=>inspectWav(silent),/silent|tone/);
  const bad=wav({junk:false});bad.writeUInt32LE(48000,24);assert.throws(()=>inspectWav(bad),/format/);
  const short=wav();short.writeUInt32LE(5,4);assert.throws(()=>inspectWav(short),/RIFF/);
});
test('independent validator accepts an actual FFmpeg two-second-source trimmed export', {skip:!process.env.DITTO_FIXTURE_FFMPEG}, async()=>{
  const {runProcess,inspectWav}=await import('./smoke.mjs');
  const dir=mkdtempSync(fileURLToPath(new URL('../../.work/ci-smoke-unit-',import.meta.url)));
  const source=join(dir,'source.wav'),output=join(dir,'export.wav'),log=join(dir,'ffmpeg.log');
  const hash=path=>createHash('sha256').update(readFileSync(path)).digest('hex');
  try {
    await runProcess(process.env.DITTO_FIXTURE_FFMPEG,['-nostdin','-v','error','-f','lavfi','-i','sine=frequency=440:duration=2','-ar','48000','-ac','2','-c:a','pcm_s16le',source],{cwd:dir,log,timeoutMs:30000});
    const original=hash(source);
    await runProcess(process.env.DITTO_FIXTURE_FFMPEG,['-nostdin','-v','error','-i',source,'-ss','0.25','-t','1','-ar','44100','-ac','1','-c:a','pcm_s24le',output],{cwd:dir,log,timeoutMs:30000});
    const result=inspectWav(readFileSync(output));assert.equal(result.duration,1);assert.equal(result.positiveZeroCrossings,440);assert.equal(hash(source),original);
  } finally {rmSync(dir,{recursive:true,force:true});}
});

test('path containment rejects sibling prefixes and traversal',async()=>{
  const {contained}=await import('./smoke.mjs');
  assert.equal(contained('/a/test','/a/test/output.wav'),true);
  assert.equal(contained('/a/test','/a/test-escape/output.wav'),false);
  assert.equal(contained('/a/test','/a/test/../outside.wav'),false);
});
