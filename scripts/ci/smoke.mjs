// Test-only native runner. Never package the instrumented executable/app.
import {resolve, dirname, relative, isAbsolute, sep, join, basename} from 'node:path';
import {closeSync, openSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync, readdirSync, cpSync, copyFileSync, existsSync, realpathSync, chmodSync, lstatSync, rmSync} from 'node:fs';
import {spawn, spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {machine} from 'node:os';
import {fileURLToPath} from 'node:url';

export const requiredChecks = [
  'bundled UI rendered', 'native bridge active', 'metadata probe entered queue',
  'real snapshot rendered in UI', 'selection update preserves trim end',
  'explicit null resets end boundary', 'scoped asset returns WAV bytes',
  'scoped decoded preview loads', 'native webview seek', 'real helper export completed',
  'completed row selection preserves export', 'exact-target queue removal',
  'crowded queue scroll stays inside workspace', 'error banner preserves footer',
];
export function validateReport(report) {
  if (!report || report.passed !== true || !Array.isArray(report.checks) || report.checks.some(c => !c || c.ok !== true)) {
    throw Error(`Native smoke report failed: ${report?.error ?? 'missing/failed checks'}`);
  }
  for (const name of requiredChecks) {
    if (report.checks.filter(c => c.name === name && c.ok === true).length !== 1) throw Error(`Missing or duplicate native check: ${name}`);
  }
  if (typeof report.output !== 'string' || !report.output || typeof report.userAgent !== 'string' || !report.userAgent) throw Error('Missing native output/userAgent');
  return report;
}

export function assertNativeHost() {
  if (!['win32/x64','darwin/arm64'].includes(`${process.platform}/${process.arch}`) || machine().toLowerCase()!==(process.arch==='x64'?'x86_64':'arm64')) throw Error('Native smoke requires Windows x64 or macOS arm64 (no emulation)');
  if (process.env.GITHUB_ACTIONS==='true' && (process.env.RUNNER_OS!==(process.platform==='win32'?'Windows':'macOS') || process.env.RUNNER_ARCH!==(process.arch==='x64'?'X64':'ARM64'))) throw Error('Hosted runner OS/architecture mismatch');
}

// File-backed logs avoid pipe deadlock from surviving webview subprocesses.
// On timeout kill only this launched process tree; no broad process-name kills.
export function runProcess(exe,args,{cwd,log,timeoutMs,env=process.env}) {
  if (!Number.isFinite(timeoutMs) || timeoutMs<=0) throw Error('Invalid process timeout');
  return new Promise((accept,reject)=>{
    const fd=openSync(log,'a');
    let child;
    try {child=spawn(exe,args,{cwd,env,stdio:['ignore',fd,fd],windowsHide:true,detached:process.platform!=='win32',shell:false});}
    catch(error) {closeSync(fd);reject(error);return;}
    closeSync(fd);
    let timedOut=false;
    const timer=setTimeout(()=>{
      timedOut=true;
      if (process.platform==='win32') {
        spawnSync('taskkill.exe',['/PID',String(child.pid),'/T','/F'],{windowsHide:true,timeout:10000,stdio:'ignore'});
        child.kill('SIGKILL');
      } else {try {process.kill(-child.pid,'SIGKILL');} catch {child.kill('SIGKILL');}}
    },timeoutMs);
    child.once('error',error=>{clearTimeout(timer);reject(error);});
    child.once('exit',(exitCode,signal)=>{
      clearTimeout(timer);
      const result={exitCode,signal,timedOut};
      if (timedOut || exitCode!==0) reject(Object.assign(Error(`${basename(exe)} ${timedOut?'timeout':`exit=${exitCode} signal=${signal}`} (see ${log})`),{result}));
      else accept(result);
    });
  });
}

const digest = b => createHash('sha256').update(b).digest('hex');
const readJSON = path => JSON.parse(readFileSync(path,'utf8'));
const saveJSON = (path,value) => {mkdirSync(dirname(path),{recursive:true});writeFileSync(path,JSON.stringify(value,null,2)+'\n');};
function files(dir) {
  return readdirSync(dir,{withFileTypes:true}).sort((a,b)=>a.name.localeCompare(b.name)).flatMap(e=>{
    const path=join(dir,e.name);
    if (e.isSymbolicLink()) throw Error(`Unexpected symlink in smoke/package resources: ${path}`);
    return e.isDirectory()?files(path):[path];
  });
}
function record(path) {const b=readFileSync(path);return {bytes:b.length,sha256:digest(b)};}
export function verifyDeliverables(dir) {
  const records=readJSON(join(dir,'checksums.json'));
  if (!Array.isArray(records) || !records.length) throw Error('Collected package checksums required');
  const seen=new Set();
  for (const entry of records) {
    if (typeof entry.path!=='string' || !contained(dir,resolve(dir,entry.path)) || seen.has(entry.path)) throw Error('Unsafe or duplicate collected package path');
    seen.add(entry.path);
    const path=resolve(dir,entry.path);
    if (!contained(realpathSync(dir),realpathSync(path))) throw Error('Unsafe collected package symlink');
    const actual=record(path);
    if (actual.bytes!==entry.bytes || actual.sha256!==entry.sha256) throw Error(`Collected package checksum mismatch: ${entry.path}`);
  }
  const actual=files(dir).map(p=>relative(dir,p).replaceAll('\\','/')).filter(p=>p!=='checksums.json');
  if (actual.length!==records.length || actual.some(p=>!seen.has(p))) throw Error('Collected package file inventory mismatch');
  return records;
}

export function contained(base, path) {
  const rel = relative(resolve(base), resolve(path));
  return rel !== '' && !isAbsolute(rel) && rel !== '..' && !rel.startsWith('..' + sep);
}

export function inspectWav(b) {
  if (b.length < 12 || b.toString('ascii',0,4) !== 'RIFF' || b.toString('ascii',8,12) !== 'WAVE' || b.readUInt32LE(4) + 8 !== b.length) throw Error('Invalid/truncated RIFF WAV');
  let fmt, data;
  for (let offset=12; offset < b.length;) {
    if (offset+8 > b.length) throw Error('truncated WAV chunk');
    const name=b.toString('ascii',offset,offset+4), size=b.readUInt32LE(offset+4), start=offset+8, end=start+size;
    if (end+(size%2)>b.length) throw Error('truncated WAV chunk payload');
    if (name==='fmt ') {if (fmt) throw Error('duplicate WAV format'); fmt=b.subarray(start,end);}
    if (name==='data') {if (data) throw Error('duplicate WAV data'); data=b.subarray(start,end);}
    offset=end+(size%2);
  }
  if (!fmt || fmt.length<16 || !data) throw Error('Missing WAV format/data');
  const encoding=fmt.readUInt16LE(0), channels=fmt.readUInt16LE(2), rate=fmt.readUInt32LE(4), byteRate=fmt.readUInt32LE(8), block=fmt.readUInt16LE(12), bits=fmt.readUInt16LE(14);
  const pcm=encoding===1 || (encoding===0xfffe && fmt.length>=40 && fmt.readUInt16LE(16)===22 && fmt.readUInt16LE(18)===24 && fmt.subarray(24,40).toString('hex')==='0100000000001000800000aa00389b71');
  if (!pcm || channels!==1 || rate!==44100 || bits!==24 || block!==3 || byteRate!==132300) throw Error('Unexpected WAV format: requires PCM 44100Hz/24-bit/mono');
  if (data.length!==132300) throw Error(`Unexpected trimmed WAV size: ${data.length}, expected one second`);
  let square=0, crossings=0, previous=0, peak=0;
  for (let i=0; i<data.length; i+=3) {
    const sample=data.readIntLE(i,3); square+=sample*sample; peak=Math.max(peak,Math.abs(sample));
    if (previous<=0 && sample>0) crossings++; previous=sample;
  }
  const rms=Math.sqrt(square/(data.length/3))/8388608;
  if (rms<0.001 || peak>8388607 || crossings<430 || crossings>450) throw Error(`WAV is silent or not the expected 440Hz tone: rms=${rms}, crossings=${crossings}`);
  return {encoding,channels,sampleRate:rate,bitsPerSample:bits,dataBytes:data.length,frames:data.length/block,duration:data.length/byteRate,rms,positiveZeroCrossings:crossings};
}

// Production outputs are already collected by native.mjs before this entry point.
// An entirely separate Cargo target directory prevents smoke-feature replacement
// of either the production executable or bundles, including on a failed smoke.
export async function main() {
  assertNativeHost();
  const root=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
  const ci=join(root,'.work/ci'), evidence=join(ci,'evidence');
  mkdirSync(evidence,{recursive:true});
  const summary={passed:false,platform:process.platform,arch:process.arch,startedAt:new Date().toISOString(),testOnly:true};
  const finalPath=join(evidence,'native-smoke.json');
  // Invalidate prior success before doing anything that can fail.
  saveJSON(finalPath,summary);
  rmSync(join(evidence,'native-smoke-report.json'),{force:true});
  let preservationCheck, sourceCheck;
  try {
    if (!process.env.DITTO_FIXTURE_FFMPEG || !isAbsolute(process.env.DITTO_FIXTURE_FFMPEG)) throw Error('DITTO_FIXTURE_FFMPEG must name the validated absolute test-only fixture generator');
    const fixture=realpathSync(process.env.DITTO_FIXTURE_FFMPEG);
    if (process.env.CARGO_BUILD_TARGET) throw Error('Native smoke does not support CARGO_BUILD_TARGET/cross compilation');
    const seconds=(name,fallback,max)=>{
      const value=Number(process.env[name]??fallback);
      if (!Number.isFinite(value)||value<=0||value>max) throw Error(`Invalid ${name}: must be >0 and <=${max} seconds`);
      return value*1000;
    };
    const smokeTimeout=seconds('DITTO_SMOKE_TIMEOUT',180,1800), buildTimeout=seconds('DITTO_SMOKE_BUILD_TIMEOUT',1200,3600);
    const deliverables=join(ci,'deliverables');
    const packageRecords=verifyDeliverables(deliverables);
    const packageEvidence=join(evidence,'packages.json'), packages=readJSON(packageEvidence);
    if (packages.platform!==process.platform || packages.arch!==process.arch || !packages.packages?.length) throw Error('Native production collection must precede smoke');
    const production=resolve(root,packages.application.path);
    if (!contained(join(ci,'target/release'),production) || record(production).sha256!==packages.application.sha256) throw Error('Production application mismatch');
    for (const name of packages.packages) if (!packageRecords.some(r=>r.path===name)) throw Error('Collected native package missing');
    const originalApp=join(ci,'target/release/bundle/macos/Ditto Converter.app');
    const productionFiles=()=>process.platform==='darwin'?files(originalApp):[production,...files(join(root,'src-tauri/resources'))];
    const protectedFiles=[...files(deliverables),packageEvidence,...productionFiles()];
    const before=protectedFiles.map(path=>({path,...record(path)}));
    preservationCheck=()=>{
      summary.productionPreserved={passed:false,files:before.length};
      verifyDeliverables(deliverables);
      const currentFiles=[...files(deliverables),packageEvidence,...productionFiles()];
      if (JSON.stringify(currentFiles)!==JSON.stringify(protectedFiles) || before.some(r=>record(r.path).sha256!==r.sha256)) throw Error('Production artifacts changed during smoke');
      summary.productionPreserved={passed:true,files:before.length};
    };
    const runs=join(ci,'smoke-runs');mkdirSync(runs,{recursive:true});
    const work=mkdtempSync(join(runs,'run-'));
    const data=join(work,'data'), output=join(work,'output'), tmp=join(work,'tmp');
    for (const path of [data,output,tmp]) mkdirSync(path,{recursive:true});
    summary.work=relative(root,work).replaceAll('\\','/');
    const env={...process.env,TMPDIR:tmp,TMP:tmp,TEMP:tmp,DITTO_DATA_DIR:data,DITTO_SMOKE_INPUT:join(work,'source.wav'),DITTO_SMOKE_OUTPUT:output,DITTO_SMOKE_ONLINE:'0',DITTO_ISOLATE_TMP:'1'};
    delete env.DITTO_FFMPEG;delete env.DITTO_FFPROBE;
    const command=async(exe,args,label,timeoutMs=120000)=>{
      const log=join(work,`${label}.log`);
      const result=await runProcess(exe,args,{cwd:root,env,log,timeoutMs});
      return {log:relative(root,log).replaceAll('\\','/'),...result};
    };
    // Fixture binary is invoked for tests only and is never staged as a helper.
    summary.fixtureProcess=await command(fixture,['-nostdin','-v','error','-f','lavfi','-i','sine=frequency=440:duration=2','-ar','48000','-ac','2','-c:a','pcm_s16le','-y',env.DITTO_SMOKE_INPUT],'fixture');
    summary.source={...record(env.DITTO_SMOKE_INPUT),duration:2};
    sourceCheck=()=>{
      if (record(env.DITTO_SMOKE_INPUT).sha256!==summary.source.sha256) throw Error('Original source changed during native smoke');
      summary.source.preserved=true;
    };
    if (!existsSync(join(root,'ui/dist/index.html'))) throw Error('Production frontend must already be built');
    const target=join(ci,'smoke-target');
    env.CARGO_TARGET_DIR=target;
    const args=['build','--locked','--release','--package','ditto-converter','--features','desktop,custom-protocol,smoke','--target-dir',target];
    summary.build={command:['cargo',...args],...await command('cargo',args,'build',buildTimeout)};
    const instrumented=join(target,'release',process.platform==='win32'?'ditto-converter.exe':'ditto-converter');
    let executable, resources;
    if (process.platform==='darwin') {
      const app=join(work,'Ditto Converter.app');
      cpSync(originalApp,app,{recursive:true});
      // Use the collected production executable's bundle-relative location.
      if (!contained(originalApp,production)) throw Error('Production executable is outside the application bundle');
      executable=join(app,relative(originalApp,production));
      if (lstatSync(executable).isSymbolicLink() || !contained(realpathSync(app),realpathSync(executable))) throw Error('Unsafe test executable path');
      copyFileSync(instrumented,executable);chmodSync(executable,0o755);
      resources=join(app,'Contents/Resources/resources');
      // Replacing the main executable invalidates the copied resource seal.
      // Only this disposable test copy is ad-hoc signed; no identities/bypasses.
      summary.signing=await command('/usr/bin/codesign',['--force','--sign','-',app],'sign-test-copy');
      summary.signatureVerification=await command('/usr/bin/codesign',['--verify','--deep','--strict',app],'verify-test-copy');
    } else {
      const app=join(work,'application');mkdirSync(app,{recursive:true});
      executable=join(app,'ditto-converter.exe');copyFileSync(instrumented,executable);
      resources=join(app,'resources');
      cpSync(join(root,'src-tauri/resources'),resources,{recursive:true});
    }
    const engine=readJSON(join(resources,'offline-engine.json'));
    if (engine.platform!==process.platform || engine.arch!==process.arch || !Array.isArray(engine.files) || engine.files.length!==2) throw Error('Test copy has wrong native helper manifest');
    for (const name of ['ffmpeg','ffprobe']) {
      const file=name+(process.platform==='win32'?'.exe':'');
      const entry=engine.files.find(f=>f.name===file);
      if (!entry || record(join(resources,'bin',file)).sha256!==entry.sha256) throw Error(`Test copy helper mismatch: ${file}`);
    }
    summary.application={path:relative(root,executable).replaceAll('\\','/'),...record(executable)};
    let launchError;
    try {summary.process=await command(executable,[],'native-ui',smokeTimeout);}
    catch(error) {summary.process=error.result;launchError=error;}
    // A zero exit is never sufficient. The fresh isolated data directory must
    // contain the report written by the real feature-gated native IPC command.
    const reportPath=join(data,'native-smoke.json');
    if (existsSync(reportPath)) {
      summary.report=readJSON(reportPath);
      copyFileSync(reportPath,join(evidence,'native-smoke-report.json'));
    }
    if (launchError) throw launchError;
    if (!summary.report) throw Error('Native process exited without the mandatory native-smoke.json report');
    const report=validateReport(summary.report);
    if (!isAbsolute(report.output) || !contained(realpathSync(output),realpathSync(report.output)) || lstatSync(report.output).isSymbolicLink()) throw Error('Reported export escaped smoke output directory');
    if (files(output).length!==1 || !report.output.toLowerCase().endsWith('.wav')) throw Error('Unexpected export/partial files in output directory');
    summary.export={...record(report.output),...inspectWav(readFileSync(report.output))};
    summary.decode=await command(fixture,['-nostdin','-v','error','-xerror','-i',report.output,'-map','0:a:0','-f','null','-'],'decode-export');
    sourceCheck();preservationCheck();
    summary.passed=true;
  } catch(error) {
    summary.error=String(error.stack??error);
  } finally {
    for (const check of [sourceCheck,preservationCheck]) if (check) {
      try {check();} catch(error) {summary.passed=false;summary.error=[summary.error,String(error)].filter(Boolean).join('\n');}
    }
    summary.finishedAt=new Date().toISOString();
    saveJSON(finalPath,summary);
  }
  if (!summary.passed) throw Error(`Native smoke failed: ${summary.error}\nEvidence: ${finalPath}`);
  console.log(`PASS: real native IPC/render/decoded preview/seek/trim/export; source and production artifacts unchanged. Evidence: ${finalPath}`);
  return summary;
}

if (process.argv[1] && resolve(process.argv[1])===fileURLToPath(import.meta.url)) {
  main().catch(error=>{console.error(String(error.stack??error));process.exitCode=1;});
}
