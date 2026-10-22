import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdirSync,mkdtempSync,writeFileSync,rmSync,cpSync,readFileSync,readdirSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {join,resolve} from 'node:path';
import * as native from './native.mjs';
const scratch=resolve('.work/remediation-package-tests');mkdirSync(scratch,{recursive:true});
const digest=b=>createHash('sha256').update(b).digest('hex');
function pairedFixture(t){
 const root=mkdtempSync(join(scratch,'pair-'));t.after(()=>rmSync(root,{recursive:true,force:true}));
 const notices=join(root,'notices'),sources=join(root,'sources'),out=join(root,'deliverables'),lock=join(root,'Cargo.lock');
 mkdirSync(notices);mkdirSync(sources);mkdirSync(out);writeFileSync(lock,'fixture locked dependency graph');
 const pins=JSON.parse(readFileSync(join(native.root,'scripts/ci/notice-pins.json'),'utf8')).mplSources.map(p=>({...p,sha256:digest(p.name)}));
 const cargo=pins.map(p=>({name:p.name,version:p.version,license:p.license,review_required:false,notices:[{file:'LICENSE',sha256:digest('MPL fixture notice')}]}));
 for(const item of cargo){const dir=join(notices,'cargo',`${item.name}-${item.version}`);mkdirSync(dir,{recursive:true});writeFileSync(join(dir,'LICENSE'),'MPL fixture notice');}
 const mpl_sources=pins.map(p=>({...p,file:`${p.name}-${p.version}.crate`,bytes:Buffer.byteLength(p.name)}));
 for(const source of mpl_sources)writeFileSync(join(sources,source.file),source.name);
 writeFileSync(join(sources,'source-manifest.json'),JSON.stringify(mpl_sources));writeFileSync(join(sources,'README-MPL.txt'),'fixture source rights');
 const report={schema:1,target:'aarch64-apple-darwin',cargo_lock_sha256:digest(readFileSync(lock)),cargo,mpl_sources,failures:[]};
 const save=()=>writeFileSync(join(notices,'native-notice-report.json'),JSON.stringify(report));save();
 return {notices,sources,out,lock,pins,report,save};
}
test('Mac collection pairs all five exact MPL archives from a target/lock-verified full report',t=>{
 const f=pairedFixture(t);
 writeFileSync(join(f.sources,'stale.crate'),'not in this report');
 assert.equal(typeof native.pairNativeNotices,'function');
 native.pairNativeNotices(f.notices,'aarch64-apple-darwin',f.lock,f.sources,f.out,f.pins);
 const paired=join(f.out,'mpl-corresponding-source');
 assert.deepEqual(readdirSync(paired).sort(),[...f.report.mpl_sources.map(s=>s.file),'source-manifest.json','README-MPL.txt'].sort());
 for(const source of f.report.mpl_sources)assert.deepEqual(readFileSync(join(paired,source.file)),readFileSync(join(f.sources,source.file)));
 for(const name of ['source-manifest.json','README-MPL.txt'])assert.deepEqual(readFileSync(join(paired,name)),readFileSync(join(f.sources,name)));
 const production=readFileSync(join(native.root,'scripts/ci/native.mjs'),'utf8');
 assert.match(production,/pairNativeNotices\(join\(stagedNotices,'dependencies'\),process.platform==='win32'\?'x86_64-pc-windows-msvc':'aarch64-apple-darwin',join\(root,'Cargo.lock'\),join\(ci,'mpl-corresponding-source'\),out\)/);
});
test('Mac pairing rejects missing, stale or unresolved full reports before copying',async t=>{
 for(const problem of ['missing report','wrong target','wrong lock','unresolved','missing notice','changed notice'])await t.test(problem,t=>{
  const f=pairedFixture(t);
  if(problem==='missing report')rmSync(join(f.notices,'native-notice-report.json'));
  if(problem==='wrong target'){f.report.target='x86_64-pc-windows-msvc';f.save();}
  if(problem==='wrong lock')writeFileSync(f.lock,'different lock');
  if(problem==='unresolved'){f.report.failures=['unresolved rights'];f.save();}
  const notice=join(f.notices,'cargo',`${f.report.cargo[0].name}-${f.report.cargo[0].version}`,'LICENSE');
  if(problem==='missing notice')rmSync(notice);
  if(problem==='changed notice')writeFileSync(notice,'changed');
  assert.throws(()=>native.pairNativeNotices(f.notices,'aarch64-apple-darwin',f.lock,f.sources,f.out,f.pins));
  assert.deepEqual(readdirSync(f.out),[]);
 });
});
test('Mac pairing fails closed on unpaired or changed MPL source sets',async t=>{
 for(const problem of ['absent list','empty list','missing entry','duplicate entry','wrong source identity','wrong filename','wrong checksum','missing archive','changed archive','missing manifest','changed manifest','missing rights'])await t.test(problem,t=>{
  const f=pairedFixture(t),first=f.report.mpl_sources[0];
  if(problem==='absent list')delete f.report.mpl_sources;
  if(problem==='empty list')f.report.mpl_sources=[];
  if(problem==='missing entry')f.report.mpl_sources.pop();
  if(problem==='duplicate entry')f.report.mpl_sources[1]=first;
  if(problem==='wrong source identity')first.version='999.0.0';
  if(problem==='wrong filename')first.file='../outside.crate';
  if(problem==='wrong checksum')first.sha256=digest('impostor');
  if(problem==='missing archive')rmSync(join(f.sources,first.file));
  if(problem==='changed archive')writeFileSync(join(f.sources,first.file),'impostor');
  if(problem==='missing manifest')rmSync(join(f.sources,'source-manifest.json'));
  if(problem==='changed manifest')writeFileSync(join(f.sources,'source-manifest.json'),'[]');
  if(problem==='missing rights')rmSync(join(f.sources,'README-MPL.txt'));
  f.save();
  assert.throws(()=>native.pairNativeNotices(f.notices,'aarch64-apple-darwin',f.lock,f.sources,f.out,f.pins));
  assert.deepEqual(readdirSync(f.out),[],'validate complete pairing before writing any deliverable');
 });
});
test('new Mac package must contain byte-exact staged notices and no dependency source or SDK payload',t=>{
 assert.equal(typeof native.verifyMacNoticePayload,'function','Mac notice packaging verifier not implemented');
 const root=mkdtempSync(join(scratch,'case-'));t.after(()=>rmSync(root,{recursive:true,force:true}));
 const staged=join(root,'staged'),resource=join(root,'resources');mkdirSync(join(staged,'dependencies/cargo/objc2-web-kit-0.3.2'),{recursive:true});
 const tail='dependencies/cargo/objc2-web-kit-0.3.2/Apple-WebKit-WKWebView-BSD.txt';writeFileSync(join(staged,tail),'exact authentic fixture notice');
 cpSync(staged,join(resource,'notices'),{recursive:true});
 native.verifyMacNoticePayload(staged,resource);
 writeFileSync(join(resource,'notices',tail),'wrong');assert.throws(()=>native.verifyMacNoticePayload(staged,resource),/notice/);
 cpSync(staged,join(resource,'notices'),{recursive:true,force:true});
 for(const name of ['dependency.rs','dependency.crate','Header.h','Framework.tbd','rustdoc/index.html']){
  const path=join(resource,name);mkdirSync(join(path,'..'),{recursive:true});writeFileSync(path,'must not ship');
  assert.throws(()=>native.verifyMacNoticePayload(staged,resource),/source|SDK|rustdoc/);rmSync(path);
 }
});
