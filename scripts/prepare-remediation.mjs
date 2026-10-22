import {spawnSync} from 'node:child_process';
import {dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
const project=resolve(dirname(fileURLToPath(import.meta.url)),'..');
export function prepareRemediation({root=project,verify=false}={}) {
 const result=spawnSync(process.platform==='win32'?'python':'python3',[resolve(root,'scripts/prepare-remediation.py'),...(verify?['--verify']:[])],{cwd:root,encoding:'utf8',windowsHide:true,timeout:120000,maxBuffer:4*1024*1024});
 if(result.error||result.status!==0)throw Error(`Dependency reconstruction ${verify?'verification':'preparation'} failed: ${result.error?.message??result.stderr}`);
 return JSON.parse(result.stdout);
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url))console.log(JSON.stringify(prepareRemediation({verify:process.argv.includes('--verify')}),null,2));
