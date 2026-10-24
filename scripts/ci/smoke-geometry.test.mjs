import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {createGeometryDiagnostics} from '../smoke-geometry.js';
import {requiredChecks} from './smoke.mjs';

async function resizeLoop({delta=0,defer=0,diagnostics=true}={}) {
  const s=readFileSync(new URL('../native-smoke.js',import.meta.url),'utf8');
  const start=s.indexOf('      for (const [width,height] of [[780,420]');
  const block=s.slice(start,s.indexOf('\n    } finally {',start));
  assert.ok(start>0 && block.length>500);
  let elapsed=0, pending, ticks=0, nativeSnapshot=0;
  const calls=[], checks=[];
  const context={innerWidth:1000,innerHeight:700,Date:{now:()=>elapsed},
    invoke:async(name,args)=>{
      calls.push({name,args});
      if(name==='smoke_geometry')return {snapshot:++nativeSnapshot};
      assert.equal(name,'smoke_window');
      const [w,h]=args.size;
      pending=()=>{context.innerWidth=w;context.innerHeight=h+delta;};
      ticks=defer;if(!ticks){pending();pending=null;}
      return {originalResponse:args.size};
    },
    delay:async(ms)=>{elapsed+=ms;if(pending&&!--ticks){pending();pending=null;}},
    measureLayout:()=>({checks:{fixture:true}}),
    check:(name,ok)=>{assert.equal(ok,true);checks.push(name);}
  };
  const geometry=createGeometryDiagnostics(context.invoke,()=>({inner:[context.innerWidth,context.innerHeight]}),()=>elapsed);
  context.geometry=diagnostics ? geometry : null;
  let error;
  try { await vm.runInNewContext(`(async()=>{${block}\n})()`,context,{timeout:1000}); } catch(e) { error=String(e); }
  return {error,events:geometry.events,calls,checks,elapsed};
}
test('actual resize block retains fresh timeout and rejects 780x392 with original 3s/25ms bound',async()=>{
  const r=await resizeLoop({delta:-28});
  assert.equal(r.error,'Error: native viewport resize 780x420: 780x392');
  assert.equal(r.elapsed,3025);
  assert.equal(r.checks.length,0);
  assert.deepEqual(r.events.map(e=>e.stage),['before-resize-request','immediate-helper-response','timeout']);
  assert.equal(r.events[0].helper.snapshot,1);
  assert.equal(r.events[2].helper.snapshot,2);
  assert.deepEqual(Array.from(r.events[1].helper.originalResponse),[780,420]);
  assert.deepEqual(r.events[2].dom.inner,[780,392]);
});
test('actual resize block keeps six exact assertions and fresh settled observations',async()=>{
  for(const opts of [{},{defer:3},{diagnostics:false}]) {
    const r=await resizeLoop(opts);assert.equal(r.error,undefined);assert.equal(r.checks.length,6);
    assert.deepEqual(r.calls.filter(c=>c.name==='smoke_window').map(c=>Array.from(c.args.size)),[[780,420],[856,420],[780,480],[872,480],[780,600],[1080,760]]);
    if(opts.diagnostics!==false)assert.equal(r.events.filter(e=>e.stage==='settled').length,6);
    else assert.equal(r.calls.filter(c=>c.name==='smoke_geometry').length,0);
  }
  assert.equal(requiredChecks.length,22);
  assert.equal(requiredChecks.filter(n=>!n.startsWith('actual native ')&&!['native startup outer window fits workarea','native layout fixture has genuine trim validation error'].includes(n)).length,14);
});

const load = () => import('../smoke-geometry.js');
const source = () => readFileSync(new URL('../native-smoke.js',import.meta.url),'utf8');
test('macOS diagnostics are feature/target gated with bounded off-main waiting', () => {
  const rust=readFileSync(new URL('../../src-tauri/src/main.rs',import.meta.url),'utf8');
  const cargo=readFileSync(new URL('../../src-tauri/Cargo.toml',import.meta.url),'utf8');
  assert.doesNotMatch(rust,/async fn smoke_window\(/, 'diagnostics must preserve synchronous helper/main-thread dispatch');
  assert.match(rust, /#\[cfg\(all\(feature = "smoke", target_os = "macos"\)\)\]\nmod smoke_geometry;/);
  assert.match(rust, /#\[cfg\(all\(feature = "smoke", target_os = "macos"\)\)\][\s\S]*?generate_handler!\[smoke_geometry\]/);
  assert.match(cargo, /\[target\.'cfg\(target_os = "macos"\)'\.dependencies\]/);
  for(const name of ['objc2','objc2-app-kit','objc2-foundation']) assert.match(cargo,new RegExp(`${name} = \\{[^\\n]*optional = true`));
  const native=readFileSync(new URL('../../src-tauri/src/smoke_geometry.rs',import.meta.url),'utf8');
  assert.match(native,/MainThreadMarker::new\(\)/);
  assert.match(native,/with_webview\(move/);
  assert.match(native,/spawn_blocking\(move \|\|/);
  assert.match(native,/recv_timeout\(Duration::from_millis\(1000\)\)/);
  assert.doesNotMatch(native,/\.recv\(\)|setContent|setFrame|TcpListener|listen\(/);
});
test('embedded smoke retains diagnostics in both reports and original exact polling contract', () => {
  const s=source();
  assert.equal((s.match(/geometryDiagnostics: geometry\?\.events/g)||[]).length,2);
  assert.match(s,/before-first-helper-request/);
  assert.match(s,/immediate-helper-response/);
  assert.match(s,/snapshot\('timeout'/);
  assert.match(s,/snapshot\('settled'/);
  assert.match(s,/while\(innerWidth!==width \|\| innerHeight!==height\)/);
  assert.match(s,/Date\.now\(\)-resizing>3000\)/);
  assert.match(s,/await delay\(25\)/);
});
test('failed diagnostics are retained and rejected, never a successful snapshot', async () => {
  const {createGeometryDiagnostics} = await load();
  const d = createGeometryDiagnostics(async()=>{throw Error('native snapshot timeout');}, ()=>({inner:[780,392]}), ()=>1);
  await assert.rejects(d.snapshot('timeout',[780,420]), /native snapshot timeout/);
  assert.equal(d.events.length,1);
  assert.equal(d.events[0].error,'Error: native snapshot timeout');
  assert.equal(d.events[0].helper,undefined);
});
test('geometry retains ordered before/response/fresh timeout snapshots without accepting mismatch', async () => {
  const {createGeometryDiagnostics} = await load();
  let clock = 0;
  const snapshots = [{native:'before'}, {native:'fresh-timeout'}];
  const d = createGeometryDiagnostics(async () => snapshots.shift(), () => ({inner:[780,392],dpr:2}), () => ++clock);
  await d.snapshot('before-first-helper-request', null);
  d.record('immediate-helper-response', [780,420], {native:'immediate', fits:true});
  await d.snapshot('timeout', [780,420]);
  assert.deepEqual(d.events.map(e=>e.stage), ['before-first-helper-request','immediate-helper-response','timeout']);
  assert.deepEqual(d.events.map(e=>e.sequence), [1,2,3]);
  assert.ok(d.events.every((e,i,a)=>!i || e.monotonicMs>a[i-1].monotonicMs));
  assert.equal(d.events[1].helper.native,'immediate');
  assert.equal(d.events[2].helper.native,'fresh-timeout');
  assert.deepEqual(d.events[2].dom.inner,[780,392]);
  assert.equal(d.events[2].ok,undefined, 'diagnostics cannot supply geometry acceptance');
});
