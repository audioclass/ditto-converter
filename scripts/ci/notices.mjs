// Supplemental native notice/source gate. Never synthesize license grants.
import {mkdirSync,readFileSync,writeFileSync,readdirSync,existsSync,realpathSync,statSync} from 'node:fs';
import {dirname,join,resolve,relative,basename,isAbsolute} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {prepareRemediation} from '../prepare-remediation.mjs';
const digest=b=>createHash('sha256').update(b).digest('hex');
const key=p=>`${p.name}@${p.version}`;
const json=(p,d)=>{mkdirSync(dirname(p),{recursive:true});writeFileSync(p,JSON.stringify(d,null,2)+'\n');};
const safe=s=>{if(!/^[A-Za-z0-9_.+-]+$/.test(s)||s==='.'||s==='..')throw Error(`Unsafe filename: ${s}`);return s;};
function inside(base,p){const r=relative(realpathSync(base),realpathSync(p));if(isAbsolute(r)||r==='..'||r.startsWith('../')||r.startsWith('..\\'))throw Error(`Notice escapes source: ${p}`);return p;}
export function parseCargoTree(text){return new Set(text.split(/\r?\n/).filter(Boolean).map(line=>{const m=/^([\w-]+) v([^\s]+)(?:\s|$)/.exec(line);if(!m)throw Error(`Invalid cargo tree line: ${line}`);return `${m[1]}@${m[2]}`;}));}
async function network(url){const res=await fetch(url,{redirect:'error',signal:AbortSignal.timeout(30000)});if(!res.ok)throw Error(`Notice HTTP ${res.status}: ${url}`);const chunks=[];let size=0;for await(const chunk of res.body){size+=chunk.length;if(size>16*1024*1024)throw Error('Notice/source exceeds size limit');chunks.push(chunk);}return Buffer.concat(chunks);}
async function pinnedBytes(pin,fetcher,cacheDir,root){
 if(!/^[a-f0-9]{64}$/.test(pin.sha256))throw Error('Invalid SHA256 pin');
 const u=new URL(pin.url);
 if(u.protocol!=='https:'||u.username||u.password||u.search||u.hash||!((u.hostname==='raw.githubusercontent.com'&&/^\/[^/]+\/[^/]+\/[a-f0-9]{40}\//.test(u.pathname))||(u.hostname==='static.crates.io'&&/^\/crates\/[^/]+\/[^/]+\.crate$/.test(u.pathname))))throw Error(`Non-immutable notice URL: ${pin.url}`);
 if(pin.local_file){
  if(!root||!/^scripts\/remediation\/notices\/[A-Za-z0-9_.+-]+$/.test(pin.local_file))throw Error('Unsafe local notice provenance');
  const b=readFileSync(inside(root,join(root,pin.local_file)));
  if(digest(b)!==pin.sha256)throw Error(`SHA256 mismatch: ${pin.local_file}`);
  return b;
 }
 const cache=cacheDir?join(cacheDir,pin.sha256):null;
 const b=cache&&existsSync(cache)?readFileSync(cache):Buffer.from(await fetcher(pin.url));
 if(digest(b)!==pin.sha256)throw Error(`SHA256 mismatch: ${pin.url}`);
 if(cache){mkdirSync(cacheDir,{recursive:true});writeFileSync(cache,b);}return b;
}
/** included = cargo tree normal,build; runtime = normal,no-proc-macro.
 * Superset packages are not copied/cleared. MPL sources conservatively also
 * cover native build/proc-macro inputs (five small archives, not a vendor tree).
 * This gate checks notice availability, NOT all upstream redistribution rights.
 */
export async function collectNotices({root,destination,sourceDestination,metadata,included,runtime=new Set(),pins={schema:1,crates:[],mplSources:[]},fetcher=network,cacheDir,target=null}){
 if(pins.schema!==1)throw Error('Unsupported notice pin schema');
 const cargo=[],failures=[],seen=new Set(),sources=[];
 for(const p of metadata.packages){if(!included.has(key(p)))continue;
  if(seen.has(key(p)))throw Error(`Ambiguous package provenance: ${key(p)}`);seen.add(key(p));
  const item={name:p.name,version:p.version,license:p.license,source:p.source,classification:runtime.has(key(p))?'native-runtime':'native-build-or-proc-macro',notices:[],review_required:true};cargo.push(item);
  try{
   const dir=join(destination,'cargo',safe(`${p.name}-${p.version}`)),base=dirname(p.manifest_path);
   mkdirSync(dir,{recursive:true});
   const copy=(file,name)=>{const b=readFileSync(file);if(!b.length)throw Error('Empty license notice');writeFileSync(join(dir,safe(name)),b);item.notices.push({file:name,sha256:digest(b)});};
   const pin=pins.crates.find(n=>key(n)===key(p));
   if(!pin&&!p.source&&metadata.workspace_members.includes(p.id)&&p.license==='MIT'){copy(inside(root,join(root,'LICENSE')),'LICENSE');item.notice_provenance='Original workspace application code; root LICENSE';}
   else {
    if(existsSync(base)){
     for(const f of readdirSync(base))if(/^(LICENSE|LICENCE|COPYING|COPYRIGHT|NOTICE)([._-]|$)/i.test(f)&&statSync(join(base,f)).isFile())copy(inside(base,join(base,f)),f);
     if(p.license_file){const f=inside(base,resolve(base,p.license_file));if(!item.notices.some(n=>n.file===basename(f)))copy(f,basename(f));}
    }
    if(pin){
     const vcs=JSON.parse(readFileSync(join(base,'.cargo_vcs_info.json'),'utf8'));
     if(p.repository!==pin.repository||p.license!==pin.license||vcs.git.sha1!==pin.commit||vcs.path_in_vcs!==pin.path_in_vcs)throw Error(`Pin provenance mismatch: ${key(p)}`);
     if(pin.source_variant){
      const variant=pin.source_variant;
      if(p.source!==null||resolve(p.manifest_path)!==resolve(root,variant.output,'Cargo.toml'))throw Error(`Patched source provenance mismatch: ${key(p)}`);
      const verified=prepareRemediation({root,verify:true});
      if(verified.name!==p.name||verified.version!==p.version||verified.archive_sha256!==pin.registry_archive_sha256||Object.entries(variant).some(([k,v])=>verified[k]!==v))throw Error(`Patched variant provenance mismatch: ${key(p)}`);
      item.source_variant=verified;
     }else if(p.source!=='registry+https://github.com/rust-lang/crates.io-index')throw Error(`Pin provenance mismatch: ${key(p)}`);
     if(pin.registry_archive_sha256){
      const blocks=readFileSync(join(root,'Cargo.lock'),'utf8').split(/^\[\[package\]\]\s*$/m).slice(1).map(block=>Object.fromEntries([...block.matchAll(/^(name|version|source|checksum) = "([^"\\\r\n]*)"\s*$/gm)].map(m=>[m[1],m[2]])));
      const entries=blocks.filter(n=>key(n)===key(p));
      if(entries.length!==1||(pin.source_variant?(entries[0].source||entries[0].checksum):(entries[0].source!==p.source||entries[0].checksum!==pin.registry_archive_sha256)))throw Error(`Cargo.lock provenance mismatch: ${key(p)}`);
     }
     item.upstream={repository:pin.repository,commit:pin.commit,path_in_vcs:pin.path_in_vcs,status:pin.status,reason:pin.reason,selected_license:pin.selected_license};
     for(const n of pin.notices){const u=new URL(n.url);const expected=pin.repository.replace('https://github.com/','https://raw.githubusercontent.com/')+'/'+pin.commit+'/';if(!n.license_text_provenance&&!n.url.startsWith(expected))throw Error('Notice repository provenance mismatch');const b=await pinnedBytes(n,fetcher,cacheDir,root);writeFileSync(join(dir,safe(n.filename)),b);item.notices.push({file:n.filename,sha256:digest(b),url:u.href,local_file:n.local_file,license_text_provenance:n.license_text_provenance});}
     if(pin.status!=='complete')throw Error(`unresolved upstream notice/rights: ${key(p)}: ${pin.reason??''}`);
    }
   }
   if(!p.license||!item.notices.length)throw Error(`Missing notices or license metadata: ${key(p)}`);
   item.review_required=false;
  }catch(e){item.error=e.message;failures.push(`${key(p)}: ${e.message}`);}
 }
 for(const k of included)if(!seen.has(k))failures.push(`Included crate absent from metadata: ${k}`);
 for(const p of metadata.packages.filter(p=>included.has(key(p))&&/\bMPL-2\.0\b/.test(p.license??''))){
  try{
   const pin=pins.mplSources?.find(n=>key(n)===key(p));if(!pin||pin.license!==p.license||pin.source!==p.source)throw Error(`Missing MPL source pin: ${key(p)}`);
   if(!sourceDestination)throw Error('MPL source destination required');
   const lock=readFileSync(join(root,'Cargo.lock'),'utf8');
   const blocks=lock.split(/^\[\[package\]\]\s*$/m).slice(1).map(block=>Object.fromEntries([...block.matchAll(/^(name|version|source|checksum) = "([^"\\\r\n]*)"\s*$/gm)].map(m=>[m[1],m[2]])));
   const matches=blocks.filter(n=>key(n)===key(p)&&n.source===p.source);
   if(matches.length!==1)throw Error(`Missing/ambiguous Cargo.lock source entry: ${key(p)}`);
   const checksum=matches[0].checksum;
   if(checksum!==pin.sha256)throw Error(`MPL archive/Cargo checksum mismatch: ${key(p)}`);
   const expected=`https://static.crates.io/crates/${p.name}/${p.name}-${p.version}.crate`;if(pin.url!==expected)throw Error('MPL source provenance mismatch');
   const b=await pinnedBytes(pin,fetcher,cacheDir),file=safe(`${p.name}-${p.version}.crate`);mkdirSync(sourceDestination,{recursive:true});writeFileSync(join(sourceDestination,file),b);sources.push({...pin,file,classification:runtime.has(key(p))?'native-runtime':'conservative-native-build-or-proc-macro',bytes:b.length});
  }catch(e){failures.push(e.message);}
 }
 const report={schema:1,target,cargo_lock_sha256:existsSync(join(root,'Cargo.lock'))?digest(readFileSync(join(root,'Cargo.lock'))):null,scope:'Target-filtered native normal/build dependency notices; runtime excludes build and proc-macro subtrees. Not blanket legal clearance.',cargo,mpl_sources:sources,failures};
 json(join(destination,'native-notice-report.json'),report);
 if(sourceDestination){json(join(sourceDestination,'source-manifest.json'),sources);writeFileSync(join(sourceDestination,'README-MPL.txt'),'The unmodified exact MPL-2.0 crate source archives in this directory accompany the native binaries at the same download location. Archive checksums match Cargo.lock/registry checksums. File-level notices remain in archives. option-ext is in the native runtime graph; other archives conservatively cover native build/proc-macro source inputs. Extract .crate files as gzip tar. Source remains under MPL-2.0, not the application MIT license. No application restrictions override recipients\' MPL source rights. Retain this directory and notices beside all redistributed binaries, beyond ephemeral CI retention. No source modifications are made by this notice collector.\n');}
 if(failures.length)throw Object.assign(Error(`Native notice gate failed:\n${failures.join('\n')}`),{report});return report;
}
function run(args,root){const r=spawnSync('cargo',args,{cwd:root,encoding:'utf8',maxBuffer:64*1024*1024,windowsHide:true});if(r.error||r.status!==0)throw Error(r.error?.message??r.stderr);return r.stdout;}
export function verifyNoticeReport(directory,target,lockPath){
 const report=JSON.parse(readFileSync(join(directory,'native-notice-report.json'),'utf8'));
 if(report.schema!==1||report.target!==target||report.cargo_lock_sha256!==digest(readFileSync(lockPath))||!Array.isArray(report.failures)||report.failures.length||!Array.isArray(report.cargo)||!report.cargo.length)throw Error('Native notice report is missing, unresolved or for a different build');
 for(const item of report.cargo){
  if(item.review_required!==false||!Array.isArray(item.notices)||!item.notices.length)throw Error('Unresolved native notice');
  for(const notice of item.notices){const file=join(directory,'cargo',safe(`${item.name}-${item.version}`),safe(notice.file));if(digest(readFileSync(inside(directory,file)))!==notice.sha256)throw Error('Native notice checksum mismatch');}
 }
 return report;
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)){
 const [target,dest,sourceDest]=process.argv.slice(2);if(!['aarch64-apple-darwin','x86_64-pc-windows-msvc'].includes(target)||!dest||!sourceDest)throw Error('Usage: node scripts/ci/notices.mjs <native-target> <notice-dir> <MPL-source-dir>');
 const root=resolve(dirname(fileURLToPath(import.meta.url)),'../..'),common=['--locked','-p','ditto-converter','--features','desktop,custom-protocol','--target',target,'--prefix','none','--format','{p}'];
 const metadata=JSON.parse(run(['metadata','--locked','--format-version','1','--features','desktop,custom-protocol','--filter-platform',target],root));
 await collectNotices({root,target,destination:resolve(root,dest),sourceDestination:resolve(root,sourceDest),metadata,included:parseCargoTree(run(['tree',...common,'--edges','normal,build'],root)),runtime:parseCargoTree(run(['tree',...common,'--edges','normal,no-proc-macro'],root)),pins:JSON.parse(readFileSync(new URL('./notice-pins.json',import.meta.url),'utf8')),cacheDir:join(root,'.work/notice-cache')});
}
