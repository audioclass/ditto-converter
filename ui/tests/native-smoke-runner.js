// Run only at /tests/native-smoke.html. Test fixtures never enter the app build.
export async function runSmoke() {
  const results = [];
  const check = (name, value) => { results.push({ name, pass: !!value }); if (!value) throw new Error(name); };
  const until = async test => { for (let i = 0; i < 60; i++) { if (test()) return; await new Promise(r => setTimeout(r, 50)); } throw new Error('UI update timeout'); };
  await until(() => document.querySelectorAll('.queue-row').length === 3);
  check('provider order preserved', document.querySelector('.queue-row').dataset.id === 'first');
  check('source title escaped', document.querySelector('.track-info strong').textContent === '<unsafe> & first fixture' && !document.querySelector('unsafe'));
  document.querySelector('[data-inspect=first]').click();
  document.getElementById('trim-start').value = '1:00'; document.getElementById('trim-end').value = '0:10'; document.getElementById('save-trim').click();
  check('reversed trim rejected', document.getElementById('trim-error').textContent.includes('IN must be before OUT'));
  document.getElementById('trim-start').value = '0:05.250'; document.getElementById('trim-end').value = '1:00'; document.getElementById('save-trim').click();
  await until(() => window.smokeCalls.some(c => c.command === 'update_job' && c.args.trim_start === 5.25 && c.args.trim_end === 60));
  check('trim exact IPC', true);
  await until(() => !document.getElementById('select-all').disabled);
  document.getElementById('select-all').click(); await until(() => document.getElementById('selection-label').textContent === 'Select all');
  check('deselect-all disables conversion', document.getElementById('convert').disabled);
  await until(() => !document.getElementById('select-all').disabled);
  document.getElementById('select-all').click(); await until(() => !document.getElementById('convert').disabled);
  document.getElementById('add-online').click(); document.getElementById('search-tab').click(); document.getElementById('search-query').value = 'fixture'; document.getElementById('search-button').click();
  await until(() => !!document.querySelector('[data-result]')); document.querySelector('[data-result]').click(); document.getElementById('add-results').click(); await until(() => document.querySelectorAll('.queue-row').length === 4);
  check('search only explicit metadata command', window.smokeCalls.some(c => c.command === 'search' && c.args.query === 'fixture'));
  check('add-results never auto-previews', !window.smokeCalls.some(c => c.command === 'preview_job'));
  document.getElementById('close-online').click(); document.getElementById('convert').click(); await until(() => !!document.querySelector('button[data-cancel]'));
  check('convert explicit start command', window.smokeCalls.some(c => c.command === 'start_queue'));
  check('progress from backend', document.querySelector('.job-status progress').value === .42);
  document.querySelector('button[data-cancel]').click(); await until(() => !document.querySelector('button[data-cancel]'));
  check('cancel explicit command', window.smokeCalls.some(c => c.command === 'cancel_job'));
  document.querySelector('[data-remove=online]').click(); await until(() => document.querySelectorAll('.queue-row').length === 3);
  check('remove exact target', window.smokeCalls.some(c => c.command === 'remove_job' && c.args.id === 'online'));
  return results;
}
