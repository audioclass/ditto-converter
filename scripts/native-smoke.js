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
    const src = window.__TAURI_INTERNALS__.convertFileSrc(path);
    const response=await fetch(src);
    const media=await response.blob();
    check('scoped asset returns WAV bytes',response.ok&&media.size>1000,{status:response.status,type:media.type,size:media.size});
    // Match production: only Linux uses Blob playback; native targets use assets.
    const previewSrc = /Linux/.test(navigator.userAgent) ? URL.createObjectURL(media) : src;
    const audio = new Audio();
    audio.preload = 'auto';
    await new Promise((resolve, reject) => {
      const timer=setTimeout(()=>reject(new Error('preview metadata timeout')),8000);
      audio.onloadedmetadata = () => {clearTimeout(timer);resolve();};
      audio.onerror = () => {clearTimeout(timer);reject(new Error('asset preview media failed ' + JSON.stringify({path,src:audio.src,code:audio.error?.code,message:audio.error?.message,policyViolations})));};
      audio.src = previewSrc; audio.load();
    });
    check('scoped decoded preview loads', audio.duration > 1.9 && audio.duration < 2.1, audio.duration);
    const ranges = values => Array.from({length:values.length},(_,i)=>[values.start(i),values.end(i)]);
    const mediaState = () => ({src:audio.src,readyState:audio.readyState,networkState:audio.networkState,currentTime:audio.currentTime,duration:audio.duration,seekable:ranges(audio.seekable),buffered:ranges(audio.buffered),error:audio.error?.message,policyViolations});
    // loadedmetadata alone does not establish a seekable range in WKWebView.
    // Wait for actual decoded data and the target range; never accept seek=0.
    const readyStarted=Date.now();
    while(audio.readyState<3||!ranges(audio.seekable).some(([start,end])=>start<=0.75&&end>=0.75)){
      if(audio.error||Date.now()-readyStarted>8000)throw new Error('preview not seekable: '+JSON.stringify(mediaState()));
      await delay(25);
    }
    await new Promise((resolve,reject)=>{
      const timer=setTimeout(()=>reject(new Error('seek timeout')),5000);
      audio.onseeked=()=>{clearTimeout(timer);resolve();};
      audio.currentTime = 0.75;
    });
    check('native webview seek', Math.abs(audio.currentTime-0.75)<0.02, mediaState());
    audio.pause(); audio.removeAttribute('src'); audio.load();
    if(previewSrc!==src)URL.revokeObjectURL(previewSrc);
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
    // Exercise the real renderer in the native webview at a short container
    // height. Only this opt-in test build constrains the app container or
    // supplies controlled error text; production gains no test hooks.
    await invoke('add_files', {paths: Array(11).fill(cfg.input)});
    const layoutStarted = Date.now();
    while (document.querySelectorAll('.queue-row').length !== 11) {
      if (Date.now()-layoutStarted > 5000) throw new Error('layout queue render timeout');
      await delay(50);
    }
    const appRoot = document.getElementById('app');
    const banner = document.getElementById('error-banner');
    const previousHeight = appRoot.style.height;
    const previousFlex = appRoot.style.flex;
    const previousError = document.getElementById('error-text').textContent;
    const previousHidden = banner.hidden;
    try {
      appRoot.style.height = '600px';
      appRoot.style.flex = '0 0 600px';
      await delay(50);
      const measure = () => {
        const rect = selector => document.querySelector(selector).getBoundingClientRect();
        const main = rect('main'), workspace = rect('.workspace'), queue = rect('.queue-body'), footer = rect('.export-bar');
        const queueElement = document.querySelector('.queue-body');
        queueElement.scrollTop = queueElement.scrollHeight;
        const scroll = queueElement.scrollTop;
        const footerHits = [];
        for (let y=footer.top+5;y<Math.min(footer.bottom,innerHeight);y+=10) {
          for (let x=40;x<workspace.right;x+=30) {
            if (document.elementFromPoint(x,y)?.closest('.queue-row')) footerHits.push({x,y});
          }
        }
        return {mainBottom:main.bottom,workspaceBottom:workspace.bottom,queueBottom:queue.bottom,footerTop:footer.top,footerBottom:footer.bottom,rootBottom:appRoot.getBoundingClientRect().bottom,scroll,footerHits};
      };
      let layout = measure();
      check('crowded queue scroll stays inside workspace', layout.scroll>0&&layout.workspaceBottom<=layout.mainBottom+1&&layout.queueBottom<=layout.footerTop+1&&layout.footerBottom<=layout.rootBottom+1&&!layout.footerHits.length, layout);
      document.getElementById('error-text').textContent = 'Controlled layout diagnostic. '.repeat(40);
      banner.hidden = false;
      await delay(50);
      layout = measure();
      check('error banner preserves footer', layout.footerBottom<=layout.rootBottom+1&&layout.queueBottom<=layout.footerTop+1&&!layout.footerHits.length, layout);
    } finally {
      appRoot.style.height = previousHeight;
      appRoot.style.flex = previousFlex;
      document.getElementById('error-text').textContent = previousError;
      banner.hidden = previousHidden;
      for (const job of (await invoke('snapshot')).jobs) await invoke('remove_job', {id:job.id});
    }
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
