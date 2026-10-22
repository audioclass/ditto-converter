import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdirSync,mkdtempSync,readFileSync,writeFileSync,cpSync,rmSync} from 'node:fs';
import {resolve,join} from 'node:path';
import {createHash} from 'node:crypto';
import {collectNotices} from './notices.mjs';
const root=resolve('.'),scratch=join(root,'.work/remediation-notice-tests');mkdirSync(scratch,{recursive:true});
const hash=b=>createHash('sha256').update(b).digest('hex');
const reviewed=['block2','dispatch2','objc2','objc2-app-kit','objc2-core-foundation','objc2-encode','objc2-exception-helper','objc2-foundation','objc2-quartz-core','objc2-web-kit'];
const pins=()=>JSON.parse(readFileSync(join(root,'scripts/ci/notice-pins.json')));
function fixture(t,names){
 const work=mkdtempSync(join(scratch,'case-'));t.after(()=>rmSync(work,{recursive:true,force:true}));
 const packages=names.map(name=>{
  const pin=pins().crates.find(p=>p.name===name),base=join(work,'registry',name);mkdirSync(base,{recursive:true});
  writeFileSync(join(base,'.cargo_vcs_info.json'),JSON.stringify({git:{sha1:pin.commit},path_in_vcs:pin.path_in_vcs}));
  return {id:`registry:${name}`,name,version:pin.version,source:'registry+https://github.com/rust-lang/crates.io-index',repository:pin.repository,license:pin.license,manifest_path:join(base,'Cargo.toml')};
 });
 return {root,destination:join(work,'notices'),metadata:{workspace_members:[],packages},included:new Set(packages.map(p=>`${p.name}@${p.version}`)),pins:pins(),fetcher:async()=>readFileSync(join(root,'scripts/remediation/notices/pinned-upstream-LICENSE.md'))};
}
test('notice bytes are not rewritten by Git checkout line-ending conversion',()=>{
 assert.match(readFileSync(join(root,'.gitattributes'),'utf8'),/^scripts\/remediation\/notices\/\* -text$/m);
});
test('only ten reviewed versions are complete with actual granted alternatives',()=>{
 const all=pins().crates;
 for(const name of reviewed){const p=all.find(p=>p.name===name);assert.equal(p.status,'complete',name);assert.equal(p.selected_license,p.license==='MIT'?'MIT':'Zlib');}
 for(const p of all.filter(p=>p.repository==='https://github.com/madsmtm/objc2'&&!reviewed.includes(p.name)))assert.equal(p.status,'unresolved',p.name);
});
test('historical and later MIT grants and Apple BSD are packaged byte-for-byte in exact package dirs',async t=>{
 const f=fixture(t,['block2','objc2','objc2-encode','objc2-foundation','objc2-web-kit']);
 const result=await collectNotices(f);assert.equal(result.failures.length,0);
 for(const p of f.metadata.packages){
  const pin=f.pins.crates.find(n=>n.name===p.name);
  const supplemental=pin.notices.filter(n=>n.local_file);assert.ok(supplemental.length>=1);
  for(const n of supplemental){assert.equal(hash(readFileSync(join(f.destination,'cargo',`${p.name}-${p.version}`,n.filename))),n.sha256);assert.deepEqual(readFileSync(join(root,n.local_file)),readFileSync(join(f.destination,'cargo',`${p.name}-${p.version}`,n.filename)));}
 }
});
test('local notice tampering and registry provenance tampering fail closed',async t=>{
 const f=fixture(t,['block2']);
 const pin=f.pins.crates.find(p=>p.name==='block2');assert.ok(pin.notices.some(n=>n.local_file),'reviewed notice inputs missing');
 pin.notices.find(n=>n.local_file).sha256='0'.repeat(64);await assert.rejects(collectNotices(f),/SHA256/);
 f.pins=pins();f.metadata.packages[0].source=null;await assert.rejects(collectNotices(f),/provenance/);
 f.metadata.workspace_members=[f.metadata.packages[0].id];await assert.rejects(collectNotices(f),/provenance/);
});
test('collector accepts only the exact verified reconstructed path variant, rejects source and path impostors',async t=>{
 const f=fixture(t,['objc2-core-foundation']),p=f.metadata.packages[0];
 p.source=null;p.manifest_path=join(root,'.work/dependency-remediation/objc2-core-foundation-0.3.2/Cargo.toml');
 const r=await collectNotices(f);assert.equal(r.cargo[0].source_variant.variant,'ditto-cfusernotification-no-optional-flag-helpers-v1');
 p.manifest_path=join(root,'.work/dependency-remediation/objc2-core-foundation-0.3.2/Cargo.toml.impostor');await assert.rejects(collectNotices(f),/provenance/);
 p.manifest_path=join(root,'.work/dependency-remediation/objc2-core-foundation-0.3.2/Cargo.toml');p.source='git+https://example.invalid';await assert.rejects(collectNotices(f),/provenance/);
});
test('real build and test entry points prepare the path dependency before Cargo resolution',()=>{
 const cargo=readFileSync(join(root,'Cargo.toml'),'utf8');assert.match(cargo,/\[patch\.crates-io\][\s\S]*objc2-core-foundation\s*=\s*\{\s*path\s*=\s*"\.work\/dependency-remediation\/objc2-core-foundation-0\.3\.2"/);
 const pkg=JSON.parse(readFileSync(join(root,'package.json')));assert.match(pkg.scripts['prebuild:desktop'],/prepare-remediation/);
 assert.match(readFileSync(join(root,'.github/workflows/native.yml'),'utf8'),/Prepare exact reviewed dependency source[\s\S]*prepare-remediation/);

 assert.match(readFileSync(join(root,'scripts/ci/smoke.mjs'),'utf8'),/prepareRemediation/);
});
