// Embedded only by the opt-in Rust `smoke` feature. Exercises real IPC/media.
(async () => {
  if (window.__dittoSmokeStarted) return;
  window.__dittoSmokeStarted = true;
  const cfg = __SMOKE_CONFIG__;
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const checks = [];
  const policyViolations=[];
  document.addEventListener('securitypolicyviolation',e=>policyViolations.push({blocked:e.blockedURI,directive:e.violatedDirective}));
  const check = (name, ok, detail) => { checks.push({name, ok: !!ok, detail}); if (!ok) throw new Error(name + ': ' + JSON.stringify(detail)); };
  const delay = ms => new Promise(r => setTimeout(r, ms));
  try {
    await delay(500);
    check('bundled UI rendered', document.body.innerText.includes('Your queue'), document.title);
    check('native bridge active', !document.body.innerText.includes('Desktop preview only'), true);
    await invoke('set_output_dir', {path: cfg.output});
    await invoke('add_files', {paths: [cfg.input]});
    let s = await invoke('snapshot');
    check('metadata probe entered queue', s.jobs.length === 1 && s.jobs[0].duration > 1.9 && s.jobs[0].state === 'queued', s.jobs);
    const id = s.jobs[0].id;
    await delay(850);
    check('real snapshot rendered in UI', document.body.innerText.includes(s.jobs[0].title), document.body.innerText.slice(0,500));
    await invoke('update_job', {id, trim_start: 0.25, trim_end: 1.25});
    await invoke('update_job', {id, selected: false});
    s = await invoke('snapshot');
    check('selection update preserves trim end', s.jobs[0].trim_end === 1.25, s.jobs[0]);
    await invoke('update_job', {id, selected: true, trim_end: null});
    check('explicit null resets end boundary', (await invoke('snapshot')).jobs[0].trim_end === null, true);
    await invoke('update_job', {id, trim_end: 1.25});
    const path = await invoke('preview_job', {id});
    const response=await fetch(window.__TAURI_INTERNALS__.convertFileSrc(path));
    const media=await response.blob();
    check('scoped asset returns WAV bytes',response.ok&&media.size>1000,{status:response.status,type:media.type,size:media.size});
    const audio = new Audio(URL.createObjectURL(media));
    await new Promise((resolve, reject) => {audio.onloadedmetadata = resolve; audio.onerror = () => reject(new Error('asset preview media failed ' + JSON.stringify({path,src:audio.src,code:audio.error?.code,message:audio.error?.message,policyViolations}))); setTimeout(()=>reject(new Error('preview metadata timeout')),8000);});
    check('scoped decoded preview loads', audio.duration > 1.9 && audio.duration < 2.1, audio.duration);
    audio.currentTime = 0.75;
    await new Promise((resolve,reject)=>{audio.onseeked=resolve;setTimeout(()=>reject(new Error('seek timeout')),5000)});
    check('native webview seek', Math.abs(audio.currentTime-0.75)<0.02, audio.currentTime);
    audio.pause(); audio.removeAttribute('src'); audio.load();
    await invoke('start_queue');
    const started = Date.now();
    do {await delay(50);s=await invoke('snapshot');if(Date.now()-started>10000)throw new Error('conversion timeout')}while(s.running);
    check('real helper export completed', s.jobs[0].state==='complete' && s.jobs[0].output, s.jobs[0]);
    const output=s.jobs[0].output;
    await invoke('update_job',{id,selected:false});
    const completedSelection=(await invoke('snapshot')).jobs[0];
    check('completed row selection preserves export',!completedSelection.selected&&completedSelection.output===output,true);
    await invoke('remove_job', {id});
    check('exact-target queue removal', (await invoke('snapshot')).jobs.length===0, true);
    if(cfg.online){
      await invoke('install_components');
      const info=await invoke('component_info');
      check('app-managed optional setup verified and activated',(await invoke('snapshot')).online_available,info);
      let ended=false,removed=null,importError=null;
      const importStart=Date.now();
      const importing=invoke('import_url',{url:'https://www.youtube.com/playlist?list=PL6B3937A5D230E335'}).catch(e=>{importError=String(e)}).finally(()=>{ended=true});
      let firstRows=null;
      while(!ended){
        const state=await invoke('snapshot');
        if(state.jobs.length){
          if(firstRows===null)firstRows=Date.now()-importStart;
          check('enumeration never auto-acquires media',state.jobs.every(j=>['queued','failed'].includes(j.state)),state.jobs.length);
          if(removed===null){removed=state.jobs[0].id;await invoke('remove_job',{id:removed});}
        }
        await delay(70);
      }
      await importing;if(importError)throw new Error(importError);
      const reviewed=await invoke('snapshot');
      check('live playlist progressively enters review queue',reviewed.jobs.length>0,{rows:reviewed.jobs.length,firstRowsMilliseconds:firstRows});
      check('removed playlist row is not resurrected',removed!==null&&!reviewed.jobs.some(j=>j.id===removed),{removedId:removed});
      const found=await invoke('search',{provider:'soundcloud',query:'lostin powers'});
      check('live SoundCloud search returns metadata',found.length>0,{results:found.length,firstTitle:found[0]?.title});
      check('provider review stays acquisition-free',(await invoke('snapshot')).jobs.every(j=>['queued','failed'].includes(j.state)),true);
    }
    await invoke('smoke_report',{report:{passed:true,checks,output,userAgent:navigator.userAgent}});
  } catch(error) {
    await invoke('smoke_report',{report:{passed:false,checks,error:String(error),userAgent:navigator.userAgent}});
  }
})();
