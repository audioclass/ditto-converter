import {appendFileSync,copyFileSync,cpSync,existsSync,mkdirSync,readFileSync,readdirSync,rmSync,statSync,writeFileSync} from 'node:fs';
import {dirname,join,resolve,relative,basename} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import os from 'node:os';

export const root=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const ci=join(root,'.work/ci'), evidence=join(ci,'evidence'), engine=join(ci,'ffmpeg');
const version='8.1.3', sourceHash='7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3';
const sourceURL=`https://ffmpeg.org/releases/ffmpeg-${version}.tar.xz`;
const suffix=process.platform==='win32'?'.exe':'';
const digest=b=>createHash('sha256').update(b).digest('hex');
const json=(p,data)=>{mkdirSync(dirname(p),{recursive:true});writeFileSync(p,JSON.stringify(data,null,2)+'\n');};
export function binaryArchitecture(b){
 if(b.length>=64&&b.subarray(0,2).toString()==='MZ'){
  const pe=b.readUInt32LE(60);
  if(pe+6<=b.length&&b.subarray(pe,pe+4).equals(Buffer.from('PE\0\0'))){
   return ({34404:'x64',43620:'arm64',332:'x86'})[b.readUInt16LE(pe+4)]??'unknown-pe';
  }
 }
 if(b.length>=8&&b.readUInt32LE(0)===0xfeedfacf){
  return ({16777228:'arm64',16777223:'x64'})[b.readUInt32LE(4)]??'unknown-mach-o';
 }
 return 'unknown';
}
export function assertHost(platform,arch){
 if(!['win32/x64','darwin/arm64'].includes(`${platform}/${arch}`))throw Error('Unsupported native CI target');
 if(process.platform!==platform||process.arch!==arch||os.machine().toLowerCase()!==(arch==='x64'?'x86_64':'arm64'))throw Error(`Actual host ${process.platform}/${process.arch}/${os.machine()} does not match ${platform}/${arch}`);
 if(process.env.GITHUB_ACTIONS==='true'&&(process.env.RUNNER_OS!==(platform==='win32'?'Windows':'macOS')||process.env.RUNNER_ARCH!==(arch==='x64'?'X64':'ARM64')))throw Error('Runner OS/architecture mismatch');
}
function nativeHost(){assertHost(process.platform,process.arch);}
function run(exe,args,options={}){
 const r=spawnSync(exe,args,{cwd:root,encoding:'utf8',windowsHide:true,timeout:120000,maxBuffer:64*1024*1024,...options});
 if(r.error||r.status!==0)throw Error(`${exe} ${args.join(' ')} failed: ${r.error??r.status}\n${r.stdout??''}\n${r.stderr??''}`);
 return `${r.stdout??''}${r.stderr??''}`;
}
function env(values){
 if(!process.env.GITHUB_ENV)throw Error('GITHUB_ENV required');
 for(const [k,v] of Object.entries(values)){
  if(/[\r\n]/.test(v))throw Error('Invalid environment value');
  appendFileSync(process.env.GITHUB_ENV,`${k}=${v}\n`);
 }
}
function record(path){const b=readFileSync(path);return {path:relative(root,path).replaceAll('\\','/'),bytes:b.length,sha256:digest(b)};}
function walk(dir){return readdirSync(dir,{withFileTypes:true}).flatMap(e=>e.isDirectory()?walk(join(dir,e.name)):e.isFile()?[join(dir,e.name)]:[]);}
function bootstrap(platform,arch){
 assertHost(platform,arch);
 for(const d of ['tmp','cargo','rustup','target','npm-cache','evidence'])mkdirSync(join(ci,d),{recursive:true});
 env({CARGO_HOME:join(ci,'cargo'),RUSTUP_HOME:join(ci,'rustup'),CARGO_TARGET_DIR:join(ci,'target'),npm_config_cache:join(ci,'npm-cache'),TMPDIR:join(ci,'tmp'),TMP:join(ci,'tmp'),TEMP:join(ci,'tmp')});
 json(join(evidence,'runner.json'),{platform,arch,machine:os.machine(),release:os.release(),node:process.version,image:process.env.ImageOS,imageVersion:process.env.ImageVersion,runnerOS:process.env.RUNNER_OS,runnerArch:process.env.RUNNER_ARCH,commit:process.env.GITHUB_SHA,run:process.env.GITHUB_RUN_ID,attempt:process.env.GITHUB_RUN_ATTEMPT,expected:platform==='win32'?['x64 NSIS installer','x64 app executable']:['arm64 .app archive','arm64 DMG'],signing:'No trusted signing identity or notarization; macOS may use mandatory ad-hoc code signatures'});
}
function stage(){
 nativeHost();mkdirSync(evidence,{recursive:true});
 const archive=join(engine,`source/ffmpeg-${version}.tar.xz`);
 if(digest(readFileSync(archive))!==sourceHash)throw Error('Source archive mismatch');
 const resources=join(root,'src-tauri/resources'), bin=join(resources,'bin');
 rmSync(bin,{recursive:true,force:true});mkdirSync(bin,{recursive:true});
 const files=[];
 for(const name of ['ffmpeg','ffprobe']){
  const file=name+suffix, src=join(engine,'install/bin',file), b=readFileSync(src);
  if(binaryArchitecture(b)!==process.arch)throw Error(`${file}: wrong binary architecture`);
  const license=run(src,['-L']), conf=run(src,['-buildconf']);
  if(!license.includes('GNU Lesser General Public License')||/--enable-(gpl|version3|nonfree)/.test(conf))throw Error('Unapproved helper license/configuration');
  for(const [label,args] of Object.entries({version:['-version'],buildconf:['-buildconf'],license:['-L']}))writeFileSync(join(evidence,`${name}-${label}.txt`),run(src,args));
  const linkage=process.platform==='darwin'?run('otool',['-L',src]):run('C:\\msys64\\ucrt64\\bin\\objdump.exe',['-p',src]);
  writeFileSync(join(evidence,`${name}-linkage.txt`),linkage);
  if(process.platform==='darwin'){
   const imports=linkage.split('\n').slice(1).map(s=>s.trim().split(' ')[0]).filter(Boolean);
   if(imports.some(s=>!s.startsWith('/usr/lib/')&&!s.startsWith('/System/Library/')))throw Error('Non-system helper dylib would be missing in bundle');
  }else{
   const imports=[...linkage.matchAll(/DLL Name:\s*(\S+)/g)].map(m=>m[1]);
   const system=/^(?:api-ms-win-[\w.-]+|ext-ms-win-[\w.-]+|kernel32|user32|advapi32|bcrypt|crypt32|msvcrt|ucrtbase|ole32|oleaut32|secur32|shell32|ws2_32|ntdll|gdi32|winmm|shlwapi|psapi)\.dll$/i;
   if(imports.length===0||imports.some(s=>!system.test(s)))throw Error(`Unapproved/missing runtime DLLs: ${imports.join(', ')}`);
  }
  copyFileSync(src,join(bin,file));files.push({name:file,bytes:b.length,sha256:digest(b),binary_arch:binaryArchitecture(b)});
 }
 cpSync(join(engine,'install/notices'),join(resources,'notices/ffmpeg'),{recursive:true});
 const manifest={platform:process.platform,arch:process.arch,version,source_url:sourceURL,source_sha256:sourceHash,license:'LGPL-2.1-or-later; corresponding unmodified source and exact build scripts/configuration accompany CI artifact',files};
 json(join(resources,'offline-engine.json'),manifest);
 json(join(engine,'installation-manifest.json'),{...manifest,remove:'Remove .work/ci/ffmpeg and generated src-tauri/resources/bin + notices/ffmpeg + offline-engine.json; disposable hosted runner is discarded after job.'});
 for(const feature of ['decoders','demuxers','encoders','muxers','filters','protocols'])writeFileSync(join(evidence,`${feature}.txt`),run(join(bin,'ffmpeg'+suffix),['-hide_banner','-'+feature]));
 env({DITTO_FFMPEG:join(bin,'ffmpeg'+suffix),DITTO_FFPROBE:join(bin,'ffprobe'+suffix)});
}
function fixture(path){
 nativeHost();const exe=resolve(path);
 if(binaryArchitecture(readFileSync(exe))!==process.arch)throw Error('Wrong fixture FFmpeg architecture');
 // Full-feature TEST ONLY binary: never copied into application resources/artifacts.
 const encoders=run(exe,['-hide_banner','-encoders']);
 for(const required of ['libmp3lame','libopus','libvorbis','ffv1','pcm_s24le','aac'])if(!encoders.includes(required))throw Error(`Fixture FFmpeg lacks ${required}`);
 writeFileSync(join(evidence,'fixture-ffmpeg.txt'),run(exe,['-version'])+'\n'+encoders);
 env({DITTO_FIXTURE_FFMPEG:exe});
}
function prepare(){
 nativeHost();
 const config={bundle:{icon:[`../.work/ci/icons/icon.${process.platform==='win32'?'ico':'icns'}`]}};
 json(join(ci,'tauri-native.json'),config);
 env({TAURI_CONFIG:JSON.stringify(config)});
}
function inventories(){
 nativeHost();
 const meta=JSON.parse(run('cargo',['metadata','--locked','--format-version','1','--features','desktop,custom-protocol']));
 if(meta.resolve.nodes.some(n=>n.id.includes('ditto-converter')&&n.features.includes('smoke')))throw Error('Production resolved smoke feature');
 const dest=join(root,'src-tauri/resources/notices/dependencies');mkdirSync(dest,{recursive:true});
 const packages=meta.packages.map(p=>{
  const base=dirname(p.manifest_path), dir=join(dest,'cargo',`${p.name}-${p.version}`), files=[];
  for(const f of readdirSync(base))if(/^(LICENSE|LICENCE|COPYING|COPYRIGHT|NOTICE)([._-]|$)/i.test(f)&&statSync(join(base,f)).isFile()){mkdirSync(dir,{recursive:true});copyFileSync(join(base,f),join(dir,f));files.push(f);}
  if(p.license_file&&existsSync(join(base,p.license_file))){mkdirSync(dir,{recursive:true});copyFileSync(join(base,p.license_file),join(dir,basename(p.license_file)));if(!files.includes(basename(p.license_file)))files.push(basename(p.license_file));}
  return {name:p.name,version:p.version,license:p.license,source:p.source,notices:files,review_required:!p.license||files.length===0};
 });
 const lock=JSON.parse(readFileSync(join(root,'package-lock.json'),'utf8'));
 const npm=Object.entries(lock.packages).filter(([p])=>p).map(([path,p])=>{
  const base=join(root,path), files=[], dir=join(dest,'npm',path.replaceAll('/','__'));
  if(existsSync(base))for(const f of readdirSync(base))if(/^(LICENSE|LICENCE|COPYING|COPYRIGHT|NOTICE)([._-]|$)/i.test(f)&&statSync(join(base,f)).isFile()){mkdirSync(dir,{recursive:true});copyFileSync(join(base,f),join(dir,f));files.push(f);}
  return {path,version:p.version,license:p.license,integrity:p.integrity,dev:p.dev??false,optional:p.optional??false,installed:existsSync(base),notices:files,review_required:existsSync(base)&&(!p.license||files.length===0)};
 });
 const inventory={scope:'Resolved Cargo desktop dependency superset and npm lock/build dependency superset; inventory is not a legal clearance or proof all entries ship',cargo:packages,npm};
 json(join(dest,'inventory.json'),inventory);json(join(evidence,'dependency-inventory.json'),inventory);
 for(const [name,exe,args] of [['rust','rustc',['-vV']],['cargo','cargo',['-V']],['rustup','rustup',['show']],['python',process.platform==='win32'?'python':'python3',['--version']]])writeFileSync(join(evidence,`${name}-version.txt`),run(exe,args));
 copyFileSync(join(root,'Cargo.lock'),join(evidence,'Cargo.lock'));copyFileSync(join(root,'package-lock.json'),join(evidence,'package-lock.json'));
}
function collect(){
 nativeHost();const out=join(ci,'deliverables');rmSync(out,{recursive:true,force:true});mkdirSync(out,{recursive:true});
 const release=join(ci,'target/release'), bundle=join(release,'bundle');
 const macApp=join(bundle,'macos/Ditto Converter.app');
 const executable=process.platform==='darwin'?run('/usr/libexec/PlistBuddy',['-c','Print :CFBundleExecutable',join(macApp,'Contents/Info.plist')]).trim():null;
 if(process.platform==='darwin'&&(!executable||basename(executable)!==executable))throw Error('Invalid application executable');
 const app=process.platform==='win32'?join(release,'ditto-converter.exe'):join(macApp,'Contents/MacOS',executable);
 if(binaryArchitecture(readFileSync(app))!==process.arch)throw Error('Packaged application architecture mismatch');
 const manifests=process.platform==='darwin'?walk(join(macApp,'Contents/Resources')).filter(p=>basename(p)==='offline-engine.json'):[join(root,'src-tauri/resources/offline-engine.json')];
 if(manifests.length!==1)throw Error('Expected one packaged offline engine manifest');
 const resource=dirname(manifests[0]);
 const manifest=JSON.parse(readFileSync(manifests[0],'utf8'));
 if(manifest.platform!==process.platform||manifest.arch!==process.arch)throw Error('Packaged platform mismatch');
 for(const f of manifest.files)if(digest(readFileSync(join(resource,'bin',f.name)))!==f.sha256)throw Error('Packaged helper mismatch');
 const packages=[];
 if(process.platform==='win32'){
  for(const f of readdirSync(join(bundle,'nsis')))if(f.endsWith('.exe')){
   const installer=join(bundle,'nsis',f), extracted=join(ci,'nsis-check');
   writeFileSync(join(evidence,'nsis-extract.txt'),run('7z',['x','-y',`-o${extracted}`,installer]));
   const extractedFiles=walk(extracted);
   if(!extractedFiles.some(p=>basename(p)===basename(app)&&digest(readFileSync(p))===digest(readFileSync(app))))throw Error('Installer missing exact native application');
   for(const helper of manifest.files)if(!extractedFiles.some(p=>basename(p)===helper.name&&digest(readFileSync(p))===helper.sha256))throw Error('Installer missing exact audited helper');
   if(!extractedFiles.some(p=>basename(p)==='offline-engine.json'&&digest(readFileSync(p))===digest(readFileSync(manifests[0]))))throw Error('Installer missing audited manifest');
   copyFileSync(installer,join(out,f));packages.push(f);
  }
 }else{
  for(const f of readdirSync(join(bundle,'dmg')))if(f.endsWith('.dmg')){
   const dmg=join(bundle,'dmg',f);writeFileSync(join(evidence,'dmg-verify.txt'),run('hdiutil',['verify',dmg]));
   copyFileSync(dmg,join(out,f));packages.push(f);
  }
  run('tar',['-czf',join(out,'Ditto-Converter-arm64.app.tar.gz'),'-C',join(bundle,'macos'),'Ditto Converter.app']);packages.push('Ditto-Converter-arm64.app.tar.gz');
 }
 if(packages.length!==(process.platform==='win32'?1:2))throw Error('Expected native package set not produced');
 cpSync(join(engine,`source/ffmpeg-${version}.tar.xz`),join(out,`ffmpeg-${version}-corresponding-source.tar.xz`));
 cpSync(join(engine,'evidence'),join(out,'ffmpeg-build-evidence'),{recursive:true});
 cpSync(join(engine,'installation-manifest.json'),join(out,'ffmpeg-installation-manifest.json'));
 cpSync(join(root,'scripts/ci'),join(out,'rebuild-scripts'),{recursive:true});
 cpSync(join(root,'src-tauri/resources/notices'),join(out,'notices'),{recursive:true});
 copyFileSync(join(root,'LICENSE'),join(out,'LICENSE'));
 writeFileSync(join(out,'README-LGPL.txt'),`Ditto Converter original application code is MIT licensed; see LICENSE. Third-party components retain their licenses, notices and source rights. FFmpeg/ffprobe ${version} are independent offline executables, LGPL-2.1-or-later, not MIT. Exact unmodified corresponding FFmpeg source, its COPYING/notice texts, configure-command, config.h/config.mak, and rebuild scripts are in this download. Source SHA256: ${sourceHash}. Source provenance: ${sourceURL}. Run build-ffmpeg.sh from scripts/ci under the source project root on the matching native host. No GPL/nonfree/version3 components enabled; internal FFmpeg libraries static, platform system libraries dynamic. Users may replace the independent helper executables with compatible builds. See binary linkage and dependency inventory evidence. Full-feature fixture generator is test-only and NOT redistributed. Unsigned development packages; no installer/UI/hardware acceptance inferred. One-day Actions artifact retention is NOT a durable public source distribution mechanism. Any further distribution must retain corresponding source/notices with binaries at the same download location, beyond Actions expiration.\n`);
 cpSync(evidence,join(out,'evidence'),{recursive:true});
 json(join(evidence,'packages.json'),{platform:process.platform,arch:process.arch,application:{...record(app),binary_arch:binaryArchitecture(readFileSync(app))},packages});
 copyFileSync(join(evidence,'packages.json'),join(out,'evidence/packages.json'));
 json(join(out,'checksums.json'),walk(out).map(p=>({...record(p),path:relative(out,p).replaceAll('\\','/')})));
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)){
 const [command,...args]=process.argv.slice(2);
 const commands={bootstrap,stage,fixture,prepare,inventories,collect};
 if(!Object.hasOwn(commands,command))throw Error('Expected bootstrap, stage, fixture, prepare, inventories or collect');
 commands[command](...args);
}
