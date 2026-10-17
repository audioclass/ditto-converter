import './style.css';
import { convertFileSrc, invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { open, confirm } from '@tauri-apps/plugin-dialog';
import { activeStates, formatTime as time, parseTime, queueView, selectionUpdates, validTrim } from './logic';
import type { Job, OnlineEntry, Snapshot } from './model';
import { previewBlob } from './preview';

const native = isTauri();
const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;
const escape = (value: unknown) => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
let snapshot: Snapshot = { jobs: [], output_dir: null, running: false, online_available: false, component_status: 'Not installed' };
let focusId: string | null = null;
let inspectorKey = '';
let results: OnlineEntry[] = [];
const chosenResults = new Set<number>();
let previewId: string | null = null;
let previewToken = 0;
let previewLoading = false;
let previewObjectUrl: string | null = null;
let installing = false;
let onlineBusy = false;
let polling = false;
let closePending = false;
let lastNotice:string|null=null;
const pending = new Set<string>();
const audio = new Audio();
audio.preload = 'metadata';

$('app').innerHTML = `
  <header class="app-header">
    <div class="brand"><span class="looper" aria-hidden="true"></span><div><h1>ditto<span> converter</span></h1><p>A little preparation. More playing.</p></div></div>
    <button id="online-settings" class="quiet"><span id="pack-dot" class="dot"></span> Online tools</button>
  </header>
  <div id="desktop-notice" class="notice" role="status" ${native ? 'hidden' : ''}>Desktop preview only · Open the Ditto Converter desktop app to add files, listen and convert. No native actions run in this browser.</div>
  <div id="error-banner" class="error-banner" role="alert" hidden><span id="error-text"></span><button id="dismiss-error" aria-label="Dismiss error">×</button></div>
  <main>
    <section class="workspace" aria-labelledby="queue-heading">
      <div class="workspace-heading"><div><p class="eyebrow">PREPARE YOUR NEXT SESSION</p><h2 id="queue-heading">Your queue <span id="queue-count" class="count">0</span></h2></div><div class="source-actions"><button id="add-online">Add online</button><button id="add-files" class="cream" data-native>+ Add files</button></div></div>
      <div class="queue-toolbar"><label><input id="select-all" type="checkbox" data-native aria-label="Select all queue items"><span id="selection-label">Select all</span></label><div><button id="retry-failed" class="text" data-native>Retry failed</button><button id="remove-selected" class="text" data-native>Remove selected</button></div></div>
      <div id="drop-zone" class="queue-body"><div class="column-head"><span>TRACK / SOURCE</span><span>LENGTH</span><span>STATUS</span></div><ol id="queue" aria-label="Ordered conversion queue"></ol><div id="empty" class="empty"><span class="empty-loop" aria-hidden="true"></span><h3>Start with a track.</h3><p>Drop audio files here, or add a few from your library.<br>Listen, trim if you like, then convert.</p><button id="browse-empty" class="quiet" data-native>Choose audio files</button><small>MP3 · M4A · FLAC · WAV · and more</small></div><div id="drop-overlay" hidden>Drop to add to your queue</div></div>
      <div class="queue-foot"><span id="queue-summary" aria-live="polite">No tracks added</span><span>Originals stay untouched</span></div>
    </section>
    <aside class="inspector" aria-labelledby="inspector-heading">
      <div class="inspector-heading"><p class="eyebrow">LISTEN & TRIM</p><h2 id="inspector-heading">Make it yours.</h2></div>
      <div id="inspector-empty" class="inspector-empty"><span class="mini-record" aria-hidden="true"></span><h3>A closer listen.</h3><p>Select a track in your queue to preview it and keep just the part you need.</p></div>
      <div id="inspector-track" hidden>
        <p id="inspect-source" class="eyebrow"></p><h3 id="inspect-title"></h3><p id="inspect-meta" class="muted"></p>
        <div class="transport"><button id="play" class="play" data-native aria-label="Play preview">▶</button><button id="stop" data-native aria-label="Stop preview">■</button><div><span id="playhead">0:00.000</span><small id="full-duration">/ —</small></div></div>
        <label class="sr-only" for="seek">Preview position</label><input id="seek" type="range" min="0" max="1" step="0.01" value="0" disabled>
        <p id="preview-status" class="hint" aria-live="polite">Preview loads only when you press play.</p>
        <div class="trim-heading"><h4>Keep a section</h4><button id="reset-trim" class="text" data-native>Reset</button></div>
        <form id="trim-form"><div class="trim-grid"><label>IN<input id="trim-start" inputmode="decimal" autocomplete="off" spellcheck="false" value="0:00.000"><button type="button" id="set-in" class="text" data-native>Set from playhead</button></label><label>OUT<input id="trim-end" inputmode="decimal" autocomplete="off" spellcheck="false" placeholder="Full track"><button type="button" id="set-out" class="text" data-native>Set from playhead</button></label></div><p id="trim-error" class="inline-error" role="alert"></p><button id="save-trim" type="submit" class="wide" data-native>Apply trim</button></form>
        <p id="kept-duration" class="kept"></p><p class="hint">Nondestructive. Your source file is never changed.</p>
      </div>
      <div class="profile"><span class="profile-label">DITTO+ EXPORT PROFILE</span><strong>WAV <span> / </span> 44.1 kHz <span> / </span> 24-bit <span> / </span> Mono</strong><p>Manufacturer-listed format.<br>Generated files have not been pedal-tested.</p></div>
    </aside>
  </main>
  <footer class="export-bar"><div class="destination"><span class="eyebrow">SAVE CONVERTED FILES TO</span><div><span id="output-path" title="Choose an output folder">Choose an output folder</span><button id="choose-folder" class="text" data-native>Change…</button></div></div><button id="open-output" class="quiet" data-native>Open folder</button><button id="convert" class="primary" data-native>Convert selected <span aria-hidden="true">→</span></button></footer>
  <dialog id="online-dialog" aria-labelledby="online-title"><div class="dialog-head"><div><p class="eyebrow">ONLINE SOURCES</p><h2 id="online-title">Find your backing track.</h2></div><button id="close-online" class="close" aria-label="Close online sources">×</button></div><p class="muted">Use only sources you have permission to access and convert.</p><div id="online-unavailable" class="setup-note">Online tools are optional. Set them up to import links or search. Local files always work independently.<button id="setup-from-online" class="text">Manage online tools →</button></div><div class="tabs" role="tablist" aria-label="Online source mode"><button id="link-tab" role="tab" aria-selected="true" aria-controls="link-panel">Paste a link</button><button id="search-tab" role="tab" aria-selected="false" aria-controls="search-panel">Search</button></div><section id="link-panel" role="tabpanel" aria-labelledby="link-tab"><form id="link-form"><label for="online-url">YouTube or SoundCloud link</label><div class="input-action"><input id="online-url" type="url" required placeholder="https://…"><button id="import-link" class="cream" data-native>Review in queue</button></div></form><p class="hint">Playlist titles are fetched first. Select or remove tracks in your queue before converting; importing a link does not start a batch download.</p></section><section id="search-panel" role="tabpanel" aria-labelledby="search-tab" hidden><form id="search-form"><label for="provider">Source</label><div class="input-action"><select id="provider"><option value="youtube">YouTube</option><option value="soundcloud">SoundCloud</option></select><input id="search-query" required placeholder="Artist, song, backing track…" aria-label="Search tracks"><button id="search-button" class="cream" data-native>Search</button></div></form><div id="search-results" aria-live="polite"></div><button id="add-results" class="wide" data-native disabled>Add selected to queue</button><p class="hint">To confirm a result: add it, select it in the queue, then press Play. Playback is a separate, explicit media download.</p></section><p id="online-status" role="status" class="hint"></p></dialog>
  <dialog id="component-dialog" aria-labelledby="component-title"><div class="dialog-head"><div><p class="eyebrow">OPTIONAL COMPONENTS</p><h2 id="component-title">Online tools.</h2></div><button id="close-components" class="close" aria-label="Close online tools">×</button></div><p class="muted">Managed yt-dlp and its JavaScript runtime enable supported YouTube and SoundCloud sources. Setup is optional and never changes your local workflow.</p><div class="component-state"><span id="component-label">Not installed</span><progress id="component-progress" hidden></progress><pre id="component-detail"></pre></div><p class="hint">The app verifies its approved component bundle before activation. If a trusted bundle is unavailable, setup fails safely. No browser cookies or account credentials are read.</p><div class="dialog-actions"><button id="refresh-components" data-native>Refresh status</button><button id="cancel-components" data-native disabled>Cancel setup</button><button id="install-components" class="cream" data-native>Set up online tools</button></div></dialog>
`;

const cancelMetadata = document.createElement('button');
cancelMetadata.id='cancel-metadata';cancelMetadata.textContent='Cancel metadata request';cancelMetadata.hidden=true;
$('online-status').after(cancelMetadata);
cancelMetadata.onclick=()=>void action('cancel-metadata',()=>command('cancel_metadata'));
const repairComponents=document.createElement('button');repairComponents.id='repair-components';repairComponents.textContent='Repair failed setup';repairComponents.hidden=true;
$('install-components').before(repairComponents);
repairComponents.onclick=()=>void action('repair-components',async()=>{
  if(!await confirm('Quarantine the failed or interrupted online setup and download the approved tools again? Local files and exports are not changed.',{title:'Repair online tools?',kind:'warning'}))return;
  await command('repair_components');installing=true;updateControls();
  try{await command('install_components');await componentInfo();}finally{installing=false;}
});
function showError(error: unknown) {
  $('error-text').textContent = error instanceof Error ? error.message : typeof error === 'string' ? error : JSON.stringify(error);
  $('error-banner').hidden = false;
}
async function command<T = unknown>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (!native) throw new Error('This action is available only in the desktop app.');
  return invoke<T>(name, args);
}
async function refresh() {
  if (!native || polling) return;
  polling = true;
  try { applySnapshot(await command<Snapshot>('snapshot')); }
  catch (e) { showError(e); }
  finally { polling = false; }
}
async function action(key: string, task: () => Promise<unknown>) {
  if (pending.has(key)) return;
  pending.add(key); updateControls();
  try { await task(); await refresh(); }
  catch (e) { showError(e); }
  finally { pending.delete(key); updateControls(); }
}
function applySnapshot(next: Snapshot) {
  if(next.notice&&next.notice!==lastNotice){showError(next.notice);lastNotice=next.notice;}
  snapshot = next;
  if (focusId && !next.jobs.some(j => j.id === focusId)) { focusId = null; stopPreview(true); }
  renderQueue(); renderInspector(); updateControls();
}
function renderQueue() {
  const view = queueView(snapshot.jobs);
  const focusKey = (document.activeElement as HTMLElement)?.dataset.focusKey;
  const scroll = $('drop-zone').scrollTop;
  $('queue-count').textContent = String(view.ordered.length);
  $('empty').hidden = view.ordered.length > 0;
  $('queue').innerHTML = view.ordered.map((job, index) => {
    const active = activeStates.has(job.state);
    const progress = job.progress === null ? '' : `${Math.round(Math.max(0, Math.min(1, job.progress)) * 100)}%`;
    return `<li class="queue-row ${job.id === focusId ? 'focused' : ''} ${job.state === 'failed' ? 'has-error' : ''}" data-id="${escape(job.id)}"><label class="row-check"><input type="checkbox" data-select="${escape(job.id)}" data-focus-key="check-${escape(job.id)}" aria-label="Select ${escape(job.title)}" ${job.selected ? 'checked' : ''} ${!native || active || snapshot.running ? 'disabled' : ''}></label><span class="position">${String(index + 1).padStart(2, '0')}</span><button class="track-info" data-inspect="${escape(job.id)}" data-focus-key="inspect-${escape(job.id)}" aria-pressed="${job.id === focusId}"><strong>${escape(job.title)}</strong><small>${job.source.kind === 'local' ? 'LOCAL FILE' : escape(job.source.provider).toUpperCase()}${job.trim_start || job.trim_end !== null ? ' · TRIMMED' : ''}</small></button><span class="track-duration">${time(job.duration).replace(/\.\d{3}$/, '')}</span><div class="job-status"><span class="state state-${escape(job.state)}">${escape(job.state === 'acquiring' ? 'Downloading' : job.state.charAt(0).toUpperCase() + job.state.slice(1))}${active && progress ? ` ${progress}` : ''}</span>${active ? `<progress ${job.progress === null ? '' : `value="${Math.max(0, Math.min(1, job.progress))}" max="1"`} aria-label="${escape(job.state)} progress"></progress>` : ''}</div><div class="row-actions">${active ? `<button data-cancel="${escape(job.id)}" aria-label="Cancel ${escape(job.title)}">Cancel</button>` : ['failed', 'cancelled'].includes(job.state) ? `<button data-retry="${escape(job.id)}" aria-label="Retry ${escape(job.title)}">Retry</button>` : ''}<button data-remove="${escape(job.id)}" class="remove" aria-label="Remove ${escape(job.title)}" title="Remove from queue (keeps exported file)">×</button></div>${job.error ? `<p class="job-error">${escape(job.error)}</p>` : ''}${job.output ? `<p class="job-output" title="${escape(job.output)}">Saved: ${escape(job.output)}</p>` : ''}</li>`;
  }).join('');
  if (focusKey) Array.from($('queue').querySelectorAll<HTMLElement>('[data-focus-key]')).find(el => el.dataset.focusKey === focusKey)?.focus({ preventScroll: true });
  $('drop-zone').scrollTop = scroll;
  $('queue-summary').textContent = view.ordered.length ? `${view.complete} of ${view.ordered.length} complete · ${view.selected} selected${snapshot.running ? ' · Working' : ''}` : 'No tracks added';
  const all = $<HTMLInputElement>('select-all');
  all.checked = view.ordered.length > 0 && view.selected === view.ordered.length;
  all.indeterminate = view.selected > 0 && view.selected < view.ordered.length;
  $('selection-label').textContent = view.selected ? `${view.selected} selected` : 'Select all';
}
function currentJob() { return snapshot.jobs.find(j => j.id === focusId); }
function renderInspector(force = false) {
  const job = currentJob();
  $('inspector-empty').hidden = !!job;
  $('inspector-track').hidden = !job;
  if (!job) { inspectorKey = ''; return; }
  const key = JSON.stringify([job.id, job.title, job.duration, job.trim_start, job.trim_end, job.state]);
  if (!force && key === inspectorKey) return;
  // Do not replace a timestamp while the user is editing it.
  if (!force && ['trim-start', 'trim-end'].includes(document.activeElement?.id ?? '') && inspectorKey) return;
  inspectorKey = key;
  $('inspect-title').textContent = job.title;
  $('inspect-source').textContent = job.source.kind === 'local' ? 'LOCAL FILE' : job.source.provider;
  $('inspect-meta').textContent = `${time(job.duration)} · ${job.state}`;
  $('full-duration').textContent = `/ ${time(job.duration)}`;
  $<HTMLInputElement>('trim-start').value = time(job.trim_start);
  $<HTMLInputElement>('trim-end').value = job.trim_end === null ? '' : time(job.trim_end);
  $('kept-duration').textContent = `Kept length  ${time((job.trim_end ?? job.duration) === null ? null : (job.trim_end ?? job.duration)! - job.trim_start)}`;
  $('trim-error').textContent = '';
}
function updateControls() {
  document.querySelectorAll<HTMLButtonElement | HTMLInputElement>('[data-native]').forEach(el => { el.disabled = !native; });
  const view = queueView(snapshot.jobs);
  $<HTMLButtonElement>('convert').disabled = !native || snapshot.running || previewLoading || (onlineBusy && snapshot.jobs.some(j=>j.selected&&j.source.kind==='online')) || view.pending === 0 || !snapshot.output_dir || pending.has('start');
  $('convert').innerHTML = snapshot.running ? 'Converting…' : `Convert ${view.pending || 'selected'}${view.pending === 1 ? ' track' : view.pending ? ' tracks' : ''} <span aria-hidden="true">→</span>`;
  $<HTMLButtonElement>('remove-selected').disabled = !native || !view.selected || pending.has('remove');
  $<HTMLButtonElement>('retry-failed').disabled = !native || !snapshot.jobs.some(j => ['failed', 'cancelled'].includes(j.state)) || pending.has('retry');
  $<HTMLInputElement>('select-all').disabled = !native || snapshot.running || !snapshot.jobs.length || pending.has('selection');
  $<HTMLButtonElement>('open-output').disabled = !native || !snapshot.output_dir;
  $<HTMLButtonElement>('choose-folder').disabled = !native || snapshot.running;
  const job = currentJob();
  const editable = native && !!job && !snapshot.running && ['queued','failed','cancelled'].includes(job.state);
  ['trim-start', 'trim-end', 'save-trim', 'reset-trim', 'set-in', 'set-out'].forEach(id => { ($<HTMLInputElement>(id)).disabled = !editable; });
  for (const id of ['set-in', 'set-out']) $<HTMLButtonElement>(id).disabled = !editable || previewId !== job?.id;
  $<HTMLButtonElement>('play').disabled = !native || !job || snapshot.running || previewLoading || job.state === 'cancelling';
  $<HTMLButtonElement>('stop').disabled = !native || (!previewId && !previewLoading);
  $<HTMLInputElement>('seek').disabled = !native || !previewId || !Number.isFinite(audio.duration);
  $('output-path').textContent = snapshot.output_dir ?? 'Choose an output folder';
  $('output-path').title = snapshot.output_dir ?? 'Choose an output folder';
  $('pack-dot').classList.toggle('ready', snapshot.online_available);
  $('online-unavailable').hidden = snapshot.online_available;
  ['import-link', 'search-button'].forEach(id => { $<HTMLButtonElement>(id).disabled = !native || !snapshot.online_available || onlineBusy; });
  $<HTMLButtonElement>('add-results').disabled = !native || !snapshot.online_available || onlineBusy || !chosenResults.size;
  cancelMetadata.hidden=!onlineBusy;
  cancelMetadata.disabled=!native;
  $('component-label').textContent = snapshot.component_status;
  const packBusy = installing || /installing|downloading|verifying|extracting|cancelling/i.test(snapshot.component_status);
  $('component-progress').hidden = !packBusy;
  $<HTMLButtonElement>('install-components').disabled = !native || packBusy || snapshot.online_available;
  $<HTMLButtonElement>('cancel-components').disabled = !native || !packBusy;
  repairComponents.hidden=snapshot.online_available||!/failed|did not complete|stale|interrupted|quarantined/i.test(snapshot.component_status);
  repairComponents.disabled=!native||packBusy||snapshot.running;
}
function stopPreview(clear = false) {
  if (previewLoading && native) { void command('cancel_preview').catch(showError); clear = true; }
  audio.pause();
  if (audio.readyState) audio.currentTime = currentJob()?.trim_start ?? 0;
  if (clear) { ++previewToken; audio.removeAttribute('src'); audio.load(); if(previewObjectUrl)URL.revokeObjectURL(previewObjectUrl); previewObjectUrl=null; previewId = null; previewLoading = false; }
  $('play').textContent = '▶'; $('play').setAttribute('aria-label', 'Play preview');
  $('playhead').textContent = time(currentJob()?.trim_start ?? 0);
  $<HTMLInputElement>('seek').value = String(currentJob()?.trim_start ?? 0);
  if (clear) $('preview-status').textContent = 'Preview loads only when you press play.';
  updateControls();
}
async function playPreview() {
  const job = currentJob();
  if (!job) return;
  if (!audio.paused && previewId === job.id) { audio.pause(); $('play').textContent = '▶'; $('play').setAttribute('aria-label', 'Play preview'); return; }
  if (previewId !== job.id) {
    const token = ++previewToken;
    previewLoading = true; updateControls();
    $('preview-status').textContent = job.source.kind === 'online' ? 'Preparing preview · this explicitly downloads the selected audio…' : 'Preparing a local preview…';
    try {
      const path = await command<string>('preview_job', { id: job.id });
      if (token !== previewToken || focusId !== job.id) return;
      // Backend can return a scoped asset URL, or a local path requiring conversion.
      const src = /^(asset:|https?:|blob:)/.test(path) ? path : convertFileSrc(path);
      if (/Linux/.test(navigator.userAgent)) {
        const blob = await previewBlob(src);
        if (token !== previewToken || focusId !== job.id) return;
        if(previewObjectUrl)URL.revokeObjectURL(previewObjectUrl);
        previewObjectUrl=URL.createObjectURL(blob);audio.src=previewObjectUrl;
      } else { audio.src=src; }
      previewId = job.id;
      await new Promise<void>((resolve, reject) => {
        const timeout = window.setTimeout(() => { audio.onloadedmetadata = audio.onerror = null; reject(new Error('Preview loading timed out. Try this track again.')); }, 30000);
        audio.onloadedmetadata = () => { clearTimeout(timeout); resolve(); };
        audio.onerror = () => { clearTimeout(timeout); reject(new Error('The preview could not be decoded by this desktop audio player.')); };
        audio.load();
      });
      if (token !== previewToken) return;
      audio.currentTime = job.trim_start;
      const seek = $<HTMLInputElement>('seek');
      seek.max = String(Number.isFinite(audio.duration) ? audio.duration : job.duration ?? 1);
      $('preview-status').textContent = 'Preview ready · playback stops at your OUT point.';
    } catch (error) {
      if (token === previewToken) { previewId = null; audio.removeAttribute('src'); if(previewObjectUrl)URL.revokeObjectURL(previewObjectUrl);previewObjectUrl=null; $('preview-status').textContent = 'Preview unavailable. Check the error above, then try again.'; }
      throw error;
    } finally { previewLoading = false; updateControls(); }
  }
  const end = job.trim_end ?? job.duration ?? audio.duration;
  if (audio.currentTime < job.trim_start || audio.currentTime >= end) audio.currentTime = job.trim_start;
  await audio.play(); $('play').textContent = 'Ⅱ'; $('play').setAttribute('aria-label', 'Pause preview');
}
async function saveTrim(startOverride?: number, endOverride?: number | null) {
  const job = currentJob(); if (!job) return;
  const start = startOverride ?? parseTime($<HTMLInputElement>('trim-start').value);
  const endText = $<HTMLInputElement>('trim-end').value.trim();
  const end = endOverride !== undefined ? endOverride : endText ? parseTime(endText) : null;
  if (start === null || (endText && endOverride === undefined && end === null) || !validTrim(start, end, job.duration)) {
    $('trim-error').textContent = 'Use seconds or m:ss.mmm. IN must be before OUT and within the track.'; return;
  }
  await action('trim', async () => { await command('update_job', { id: job.id, trim_start: start, trim_end: end }); inspectorKey = ''; });
}
async function pickFiles() {
  const selected = await open({ multiple: true, directory: false, title: 'Add audio files', filters: [{ name: 'Audio', extensions: ['mp3', 'm4a', 'aac', 'flac', 'wav', 'aif', 'aiff', 'ogg', 'opus', 'wma', 'mp4', 'webm'] }, { name: 'All files', extensions: ['*'] }] });
  if (selected) await command('add_files', { paths: Array.isArray(selected) ? selected : [selected] });
}
function modal(id: string) { const dialog = $<HTMLDialogElement>(id); if (!dialog.open) dialog.showModal(); }
function renderResults() {
  $('search-results').innerHTML = results.length ? results.map((entry, i) => `<label class="search-result"><input type="checkbox" data-result="${i}" ${chosenResults.has(i) ? 'checked' : ''} ${entry.available ? '' : 'disabled'}><span><strong>${escape(entry.title)}</strong><small>${escape(entry.provider)} · ${time(entry.duration)}${entry.available ? '' : ' · Unavailable'}</small></span></label>`).join('') : '<p class="hint">No results. Try a different title or provider.</p>';
  updateControls();
}
async function componentInfo() {
  const info = await command<unknown>('component_info');
  $('component-detail').textContent = typeof info === 'string' ? info : JSON.stringify(info, null, 2);
}

$('dismiss-error').onclick = () => { $('error-banner').hidden = true; };
$('add-files').onclick = $('browse-empty').onclick = () => void action('files', pickFiles);
$('choose-folder').onclick = () => void action('folder', async () => {
  const path = await open({ directory: true, multiple: false, title: 'Save converted files to…', defaultPath: snapshot.output_dir ?? undefined });
  if (typeof path === 'string') await command('set_output_dir', { path });
});
$('open-output').onclick = () => void action('open', () => command('open_output'));
$('convert').onclick = () => {stopPreview(true);void action('start', () => command('start_queue'));};
$('select-all').onchange = () => {
  const selected = $<HTMLInputElement>('select-all').checked;
  void action('selection', async () => { for (const update of selectionUpdates(snapshot.jobs, selected)) await command('update_job', update); });
};
$('remove-selected').onclick = () => void action('remove', async () => {
  const ids = snapshot.jobs.filter(j => j.selected).map(j => j.id);
  for (const id of ids) await command('remove_job', { id });
});
$('retry-failed').onclick = () => void action('retry', async () => {
  for (const job of snapshot.jobs.filter(j => ['failed', 'cancelled'].includes(j.state))) await command('retry_job', { id: job.id });
});
$('queue').addEventListener('change', event => {
  const target = event.target as HTMLInputElement;
  if (target.dataset.select) void action(`select-${target.dataset.select}`, () => command('update_job', { id: target.dataset.select, selected: target.checked }));
});
$('queue').addEventListener('click', event => {
  const target = (event.target as HTMLElement).closest<HTMLButtonElement>('button'); if (!target) return;
  if (target.dataset.inspect) { if (focusId !== target.dataset.inspect) { stopPreview(true); focusId = target.dataset.inspect; } renderQueue(); renderInspector(true); updateControls(); }
  if (target.dataset.remove) void action(`remove-${target.dataset.remove}`, () => command('remove_job', { id: target.dataset.remove }));
  if (target.dataset.retry) void action(`retry-${target.dataset.retry}`, () => command('retry_job', { id: target.dataset.retry }));
  if (target.dataset.cancel) void action(`cancel-${target.dataset.cancel}`, () => command('cancel_job', { id: target.dataset.cancel }));
});
$('play').onclick = () => void action('preview', playPreview);
$('stop').onclick = () => stopPreview();
$('seek').oninput = () => { audio.currentTime = Number($<HTMLInputElement>('seek').value); $('playhead').textContent = time(audio.currentTime); };
audio.addEventListener('timeupdate', () => {
  $('playhead').textContent = time(audio.currentTime);
  $<HTMLInputElement>('seek').value = String(audio.currentTime);
  const end = currentJob()?.trim_end;
  if (end !== null && end !== undefined && audio.currentTime >= end) stopPreview();
});
audio.addEventListener('ended', () => stopPreview());
$('trim-form').onsubmit = event => { event.preventDefault(); void saveTrim(); };
$('reset-trim').onclick = () => void saveTrim(0, null);
$('set-in').onclick = () => { $<HTMLInputElement>('trim-start').value = time(audio.currentTime); };
$('set-out').onclick = () => { $<HTMLInputElement>('trim-end').value = time(audio.currentTime); };
$('add-online').onclick = () => modal('online-dialog');
$('close-online').onclick = () => $<HTMLDialogElement>('online-dialog').close();
$('close-components').onclick = () => $<HTMLDialogElement>('component-dialog').close();
$('online-settings').onclick = $('setup-from-online').onclick = () => { $<HTMLDialogElement>('online-dialog').close(); modal('component-dialog'); if (native) void action('component-info', componentInfo); };
for (const mode of ['link', 'search']) {
  $(`${mode}-tab`).onclick = () => {
    for (const other of ['link', 'search']) { $(`${other}-tab`).setAttribute('aria-selected', String(mode === other)); $(`${other}-panel`).hidden = mode !== other; }
  };
}
$('link-form').onsubmit = event => {
  event.preventDefault(); if (onlineBusy || !native || !snapshot.online_available) return;
  onlineBusy = true; updateControls(); $('online-status').textContent = 'Reading titles and playlist order… no batch media download.';
  void action('import', async () => {
    try { await command('import_url', { url: $<HTMLInputElement>('online-url').value.trim() }); $('online-status').textContent = 'Added to the queue. Review and select tracks before Convert.'; }
    finally { onlineBusy = false; }
  });
};
$('search-form').onsubmit = event => {
  event.preventDefault(); if (onlineBusy || !native || !snapshot.online_available) return;
  onlineBusy = true; updateControls(); $('online-status').textContent = 'Searching metadata…';
  void action('search', async () => {
    try {
      results = await command<OnlineEntry[]>('search', { provider: $<HTMLSelectElement>('provider').value, query: $<HTMLInputElement>('search-query').value.trim() });
      chosenResults.clear(); renderResults(); $('online-status').textContent = `${results.length} results · select tracks to review in your queue.`;
    } finally { onlineBusy = false; }
  });
};
$('search-results').onchange = event => { const input = event.target as HTMLInputElement; const index = Number(input.dataset.result); if (input.checked) chosenResults.add(index); else chosenResults.delete(index); updateControls(); };
$('add-results').onclick = () => {
  onlineBusy = true; updateControls();
  void action('add-online', async () => {
    try { await command('add_online', { entries: results.filter((_, i) => chosenResults.has(i)) }); chosenResults.clear(); renderResults(); $('online-status').textContent = 'Added. Select the track in the queue and press Play to confirm it.'; }
    finally { onlineBusy = false; }
  });
};
$('refresh-components').onclick = () => void action('component-info', componentInfo);
$('install-components').onclick = () => {
  installing = true; updateControls();
  void action('install', async () => { try { await command('install_components'); await componentInfo(); } finally { installing = false; } });
};
$('cancel-components').onclick = () => void action('cancel-install', () => command('cancel_components'));
// Never let browser drag/drop navigate away from this dedicated task surface.
window.addEventListener('dragover', event => event.preventDefault());
window.addEventListener('drop', event => event.preventDefault());
window.addEventListener('keydown', event => {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'o' && native) { event.preventDefault(); void action('files', pickFiles); }
});

const unlisten: Array<() => void> = [];
if (native) {
  void listen<Snapshot>('queue-changed', event => applySnapshot(event.payload)).then(fn => unlisten.push(fn)).catch(showError);
  void getCurrentWebview().onDragDropEvent(event => {
    $('drop-overlay').hidden = !['enter', 'over'].includes(event.payload.type);
    if (event.payload.type === 'drop') { const paths = event.payload.paths; void action('drop', () => command('add_files', { paths })); }
  }).then(fn => unlisten.push(fn)).catch(showError);
  void getCurrentWindow().onCloseRequested(async event => {
    if (!snapshot.running && !queueView(snapshot.jobs).active && !installing && !onlineBusy && !previewLoading) return;
    event.preventDefault(); if (closePending) return; closePending = true;
    try {
      if (await confirm('Work is still in progress. Quit and cancel active work? Originals and completed exports will be kept.', { title: 'Quit Ditto Converter?', kind: 'warning', okLabel: 'Quit & cancel', cancelLabel: 'Keep working' })) {
        for (const job of snapshot.jobs.filter(j => activeStates.has(j.state))) await command('cancel_job', { id: job.id });
        if (installing) await command('cancel_components');
        await getCurrentWindow().destroy();
      }
    } catch (e) { showError(e); }
    finally { closePending = false; }
  }).then(fn => unlisten.push(fn)).catch(showError);
  void refresh();
}
const timer = window.setInterval(() => { if (native) void refresh(); }, 750);
window.addEventListener('beforeunload', () => { clearInterval(timer); unlisten.forEach(fn => fn()); audio.pause(); });
renderQueue(); updateControls();
