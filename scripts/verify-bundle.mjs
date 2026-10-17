import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {resolve,dirname} from 'node:path';
export function nativeMagic(bytes,platform){
 if(platform==='linux')return bytes.subarray(0,4).equals(Buffer.from([0x7f,0x45,0x4c,0x46]));
 if(platform==='win32')return bytes.subarray(0,2).toString()==='MZ';
 if(platform==='darwin')return ['cffaedfe','cefaedfe','cafebabe','bebafeca'].includes(bytes.subarray(0,4).toString('hex'));
 return false;
}
export function verifyBundle(){
 const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
 const resources=resolve(root,'src-tauri/resources');
 const manifest=JSON.parse(readFileSync(resolve(resources,'offline-engine.json'),'utf8'));
 if(manifest.platform!==process.platform||manifest.arch!==process.arch)throw new Error(`No audited offline helper pair staged for ${process.platform}/${process.arch}. The current pair is ${manifest.platform}/${manifest.arch}; refusing a broken cross-platform package.`);
 const files=['ffmpeg','ffprobe'].map(n=>n+(process.platform==='win32'?'.exe':''));
 for(const name of files){
  const path=resolve(resources,'bin',name);const bytes=readFileSync(path);const record=manifest.files.find(r=>r.name===name);
  if(!record||bytes.length!==record.bytes||createHash('sha256').update(bytes).digest('hex')!==record.sha256)throw new Error(`${name}: exact helper manifest mismatch`);
  if(!nativeMagic(bytes,process.platform))throw new Error(`${name}: helper is not a native binary for this platform`);
  const result=spawnSync(path,['-L'],{encoding:'utf8',timeout:15000,windowsHide:true});
  if(result.status!==0||!`${result.stdout}${result.stderr}`.replace(/\s+/g,' ').includes('GNU Lesser General Public License'))throw new Error(`${name}: native LGPL helper smoke failed`);
  const conf=spawnSync(path,['-buildconf'],{encoding:'utf8',timeout:15000,windowsHide:true});
  if(conf.status!==0||/--enable-(gpl|nonfree|version3)/.test(`${conf.stdout}${conf.stderr}`))throw new Error(`${name}: unapproved FFmpeg configuration`);
 }
 console.log(`Verified exact ${manifest.platform}/${manifest.arch} FFmpeg ${manifest.version} offline pair; native runtime/helper tests remain separate.`);
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url))verifyBundle();
