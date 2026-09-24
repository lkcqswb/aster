const $ = (id) => document.getElementById(id);
const escapeHTML = (value) => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const json = (v) => escapeHTML(JSON.stringify(v, null, 2));
const finished = new Set(['passed','failed','unverified','error','cancelled','limit','interrupted']);
const state = {runs:[], tasks:[], config:null, selected:null, run:null, tab:'checks', timer:null, signature:'', view:'runs'};
const number = (n) => Number(n || 0).toLocaleString();
const seconds = (ms) => `${(ms / 1000).toFixed(1)}s`;
const badge = (status) => `<span class="status ${escapeHTML(status)}"><span class="dot ${escapeHTML(status)}"></span>${escapeHTML(status)}</span>`;

async function api(path, body) {
  const response = await fetch(path, body === undefined ? {} : {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)});
  const data = await response.json();
  if (!response.ok) throw new Error(data.error || 'The request failed.');
  return data;
}
function toast(message) {
  $('toast').textContent = message; $('toast').hidden = false;
  clearTimeout(toast.timer); toast.timer = setTimeout(() => $('toast').hidden = true, 5000);
}
function view(name) {
  state.view = name;
  for (const item of ['runs','tasks','tools']) $(`${item}-view`).hidden = item !== name;
  document.querySelectorAll('[data-view]').forEach(button => button.classList.toggle('active', button.dataset.view === name));
  $('breadcrumb').textContent = {runs:'Runs',tasks:'Task library',tools:'Tools & limits'}[name];
}
function renderRuns() {
  $('run-count').textContent = state.runs.length;
  $('history-select').innerHTML = state.runs.map(r=>`<option value="${r.id}" ${r.id===state.selected?'selected':''}>${escapeHTML(r.title)} · ${escapeHTML(r.status)} · ${r.id.slice(0,6)}</option>`).join('');
  $('metric-runs').textContent = state.runs.length;
  const checked = state.runs.filter(r => ['passed','failed'].includes(r.status));
  const passed = checked.filter(r => r.status === 'passed').length;
  $('metric-passed').textContent = checked.length ? `${passed} / ${checked.length}` : '—';
  $('metric-passed-detail').textContent = checked.length ? 'Passed / completed checked runs' : 'No checked runs yet';
  $('metric-tools').textContent = number(state.runs.reduce((n,r) => n+r.tool_calls, 0));
  $('metric-tokens').textContent = number(state.runs.reduce((n,r) => n+r.output_tokens, 0));
  $('run-list').innerHTML = state.runs.length ? state.runs.map(r => `<button class="run-item ${state.selected===r.id?'current':''}" data-run="${r.id}"><span class="dot ${escapeHTML(r.status)}"></span><span><strong>${escapeHTML(r.title)}</strong><small>${r.id.slice(0,6)} · ${r.provider==='demo'?'demo':escapeHTML(r.model)}<br>${new Date(r.created_at).toLocaleTimeString([], {hour:'2-digit',minute:'2-digit'})}</small></span></button>`).join('') : '<p class="muted empty-small">Your first run starts here.</p>';
  $('new-run').disabled = state.runs.some(r => !finished.has(r.status));
}
function eventHTML(event) {
  const icons = {start:'↗',request:'◎',message:'≡',tool_call:'⌘',tool_result:'↳',checkpoint:'◇',verify:'◉',check:'✓',finish:'■',error:'!',stop:'!'};
  if (event.type === 'usage') return '';
  let detail = '';
  if (event.type === 'start') detail = `<div class="event-subtitle">${escapeHTML(event.provider === 'demo' ? 'Scripted demo · no model calls' : event.model)} · isolated workspace</div>`;
  if (event.type === 'request') detail = `<div class="event-subtitle">Up to ${number(event.max_output_tokens)} output tokens</div>`;
  if (event.type === 'message') detail = `<p class="event-message">${escapeHTML(event.text)}</p>`;
  if (event.type === 'tool_call') detail = `<div class="event-code"><pre>${json(event.arguments)}</pre></div>`;
  if (event.type === 'tool_result') detail = `<details data-detail="e${event.seq}"><summary>${event.is_error?'Tool error':'Result recorded'} · ${event.duration_ms}ms</summary><div class="event-code"><pre>${json(event.result)}</pre></div></details>`;
  if (event.type === 'checkpoint') detail = `<span class="checkpoint-note">${event.parent_id?`Restored from ${escapeHTML(event.parent_id.slice(0,6))}`:`Workspace + conversation · turn ${event.step}`}</span>`;
  if (event.type === 'check') detail = `<div class="event-subtitle">${event.passed?'Passed':'Failed'} · ${escapeHTML(event.path)}</div>`;
  if (event.type === 'finish') detail = `<div class="event-subtitle">${escapeHTML(event.status)}</div>`;
  const icon = event.type === 'check' && !event.passed ? '×' : icons[event.type] || '·';
  return `<div class="event ${escapeHTML(event.type)}"><span class="event-icon">${icon}</span><div class="event-title"><span>${escapeHTML(event.title)}</span><time>+${seconds(event.elapsed_ms)}</time></div>${detail}</div>`;
}
function renderInspector() {
  const run = state.run; if (!run) return;
  const open = new Set([...$('inspector-content').querySelectorAll('details[open]')].map(e=>e.dataset.detail));
  let html = '';
  if (state.tab === 'checks') {
    const count = run.checks.filter(c=>c.passed).length;
    if (!run.checks.length) {
      html = `<div class="section-label">INDEPENDENT VERIFICATION</div><p class="pending-note">${finished.has(run.status) ? run.status==='unverified'?'This task has no success checks. Its result is unverified.':'This run ended before verification. No success is claimed.' : 'Checks will run after the agent finishes its work.'}</p>`;
      html += run.task.checks.map(c=>`<div class="check-row"><span class="check-symbol">·</span><div><strong>${escapeHTML(c.name || c.type)}</strong><small>${escapeHTML(c.path)}</small></div></div>`).join('');
    } else {
      html = `<div class="check-summary"><span class="big-check">${count===run.checks.length?'✓':'!'}</span><div><strong>${count} of ${run.checks.length} checks passed</strong><p>Measured from the output files.</p></div></div>`;
      html += run.checks.map((c,i)=>`<div class="check-row"><span class="check-symbol ${c.passed?'':'fail'}">${c.passed?'✓':'×'}</span><div><strong>${escapeHTML(c.name)}</strong><small>${escapeHTML(c.path)}</small>${!c.passed?`<details data-detail="check-${i}" open><summary>View difference</summary><pre>${json(c.actual===undefined?{detail:c.detail}:{expected:c.expected,actual:c.actual})}</pre></details>`:''}</div></div>`).join('');
    }
    html += '<p class="inspector-note">The checker runs outside the agent loop. Passing means these specific checks passed.</p>';
  } else if (state.tab === 'files') {
    html = Object.entries(run.files).map(([name,content])=>`<details class="file-card" data-detail="${escapeHTML(name)}" ${['report.json','service.json'].includes(name)?'open':''}><summary>${escapeHTML(name)} <small>${new TextEncoder().encode(content).length} B</small></summary><pre>${escapeHTML(content)}</pre></details>`).join('') || '<p class="pending-note">No files yet.</p>';
  } else {
    html = `<div class="section-label">TASK INSTRUCTION</div><p class="task-prompt">${escapeHTML(run.task.prompt)}</p>${run.instruction?`<div class="section-label">BRANCH INSTRUCTION</div><p class="task-prompt">${escapeHTML(run.instruction)}</p>`:''}<div class="section-label">MODEL USAGE</div><p class="task-prompt">Input: ${number(run.input_tokens)} tokens<br>Output: ${number(run.output_tokens)} tokens<br>Cache read: ${number(run.cache_read_tokens)} tokens<br>Cache creation: ${number(run.cache_creation_tokens)} tokens</p>`;
  }
  $('inspector-content').innerHTML = html;
  $('inspector-content').querySelectorAll('details').forEach(e=>{if(open.has(e.dataset.detail))e.open=true;});
}
function renderRun() {
  const run = state.run; if (!run) return;
  $('empty-state').hidden = true; $('run-workspace').hidden = false;
  $('run-id').textContent = `RUN / ${run.id} ${run.provider==='demo'?'· SCRIPTED DEMO':''}`;
  $('run-title').textContent = run.title;
  $('run-status').innerHTML = badge(run.status);
  $('run-model').textContent = run.model;
  $('run-time').textContent = seconds(run.duration_ms);
  $('run-lineage').textContent = run.parent_id ? `↳ ${run.parent_id.slice(0,6)} · checkpoint ${run.parent_checkpoint}` : '';
  $('stop-run').hidden = finished.has(run.status);
  $('replay-run').disabled = state.runs.some(r => !finished.has(r.status));
  $('run-error').hidden = !run.error; $('run-error').textContent = run.error || '';
  const signature = `${run.id}:${run.events.length}:${run.status}`;
  if (signature !== state.signature) {
    const timeline = $('timeline');
    const atBottom = timeline.scrollHeight-timeline.scrollTop-timeline.clientHeight < 45;
    const open = new Set([...timeline.querySelectorAll('details[open]')].map(e=>e.dataset.detail));
    timeline.innerHTML = run.events.map(eventHTML).join('');
    timeline.querySelectorAll('details').forEach(e=>e.open=open.has(e.dataset.detail));
    if (atBottom && !finished.has(run.status)) timeline.scrollTop = timeline.scrollHeight;
    state.signature = signature;
  }
  $('event-count').textContent = `${run.events.filter(e=>e.type!=='usage').length} events`;
  $('final-answer').hidden = !run.final;
  $('final-answer').innerHTML = `<b>AGENT’S FINAL MESSAGE</b>${escapeHTML(run.final)}`;
  $('check-count').textContent = run.task.checks.length;
  $('file-count').textContent = Object.keys(run.files).length;
  const budgets = [['Agent turns',run.steps,run.limits.max_steps],['Tool calls',run.tool_calls,run.limits.max_tools],['Output tokens',run.output_tokens,run.limits.total_output_tokens],['Time (seconds)',Math.round(run.duration_ms/1000),run.limits.max_seconds]];
  $('budget').innerHTML = budgets.map(([label,n,max])=>`<div class="budget-row"><div><span>${label}</span><span>${number(n)} / ${number(max)}</span></div><progress value="${Math.min(n,max)}" max="${max}" aria-label="${label}"></progress></div>`).join('');
  renderInspector();
}
async function refresh() {
  clearTimeout(state.timer);
  try {
    state.runs = await api('/api/runs'); renderRuns();
    if (!state.selected && state.runs.length) state.selected = state.runs[0].id;
    if (state.selected) {
      const selected = state.selected;
      const result = await api(`/api/runs/${selected}`);
      if (state.selected===selected) {state.run=result;renderRun();renderRuns();}
    }
    if (state.runs.some(r=>!finished.has(r.status))) state.timer = setTimeout(refresh, 900);
  } catch (error) { toast(error.message); }
}
async function selectRun(id) {
  state.selected = id; state.signature=''; view('runs');
  try {const result=await api(`/api/runs/${id}`);if(state.selected===id){state.run=result;renderRun();renderRuns();}} catch(e){toast(e.message);}
}
function updateForm() {
  const custom = $('task-select').value==='custom';
  $('custom-fields').hidden = !custom;
  const task = state.tasks.find(t=>t.id===$('task-select').value);
  $('task-description').textContent = custom?'Your instruction, starting files, and success criteria.':task?.description || '';
  if (custom) $('provider-select').value='minimax';
  const demo = $('provider-select').value==='demo';
  $('model-input').disabled=demo;
  $('provider-hint').textContent = demo?'A deterministic script exercises real tools. It does not interpret custom instructions.':state.config.minimax_ready?'Connected through your local configuration. This run will call MiniMax.':'Add your MiniMax key to the private .env file and restart Aster.';
  $('launch-run').disabled = (!demo && !state.config.minimax_ready) || (custom && demo);
}
function showNewRun(taskId) {
  if(state.runs.some(r=>!finished.has(r.status))) return toast('Wait for the active run, or stop it first.');
  $('task-select').value = taskId || 'revenue-audit';
  $('provider-select').value = taskId==='failure-lab'?'demo':state.config.minimax_ready?'minimax':'demo';
  $('form-error').hidden=true;updateForm();$('run-dialog').showModal();
}
function limits() {
  return {max_steps:Number($('max-steps').value),max_seconds:Number($('max-seconds').value),max_output_tokens:Number($('max-output').value),total_output_tokens:Number($('total-output').value)};
}
async function launch(config, dialog, errorElement, button) {
  button.disabled=true;errorElement.hidden=true;
  try {const result=await api('/api/runs', config);dialog.close();state.selected=result.id;state.signature='';view('runs');await refresh();}
  catch(error){errorElement.textContent=error.message;errorElement.hidden=false;}
  finally{button.disabled=false;}
}
async function init() {
  try {
    [state.config,state.tasks]=await Promise.all([api('/api/config'),api('/api/tasks')]);
    $('connection').textContent=state.config.minimax_ready?'MiniMax connected':'Demo ready';
    $('model-input').value=state.config.model;
    $('task-select').innerHTML=state.tasks.map(t=>`<option value="${t.id}">${escapeHTML(t.title)}</option>`).join('')+'<option value="custom">＋ Create a custom task</option>';
    $('task-cards').innerHTML=state.tasks.map((t,i)=>`<article class="task-card"><div class="eyebrow">0${i+1} / ${escapeHTML(t.tag)}</div><span class="card-icon">${['▥','⌘','◎'][i]}</span><h3>${escapeHTML(t.title)}</h3><p>${escapeHTML(t.description)}</p><p>${Object.keys(t.files).length} starting file · ${t.checks.length} checks</p><button class="secondary" data-task="${t.id}">Try this task ↗</button></article>`).join('')+'<article class="task-card"><div class="eyebrow">YOUR NEXT EXPERIMENT</div><span class="card-icon">＋</span><h3>Bring your own task</h3><p>Add text files, write an instruction, and define what success looks like.</p><button class="secondary" data-task="custom">Create a task ↗</button></article>';
    $('tool-cards').innerHTML=state.config.tools.map(t=>`<article class="tool-card"><div class="section-label">REGISTERED TOOL</div><h3>${escapeHTML(t.name)}</h3><p>${escapeHTML(t.description)}</p></article>`).join('');
    await refresh();
  } catch(error){$('connection').textContent='Server unavailable';toast(error.message);}
}
document.querySelectorAll('[data-view]').forEach(e=>e.addEventListener('click',()=>view(e.dataset.view)));
$('history-select').addEventListener('change',e=>selectRun(e.target.value));
$('run-list').addEventListener('click',e=>{const button=e.target.closest('[data-run]');if(button)selectRun(button.dataset.run);});
$('task-cards').addEventListener('click',e=>{const button=e.target.closest('[data-task]');if(button)showNewRun(button.dataset.task);});
$('new-run').addEventListener('click',()=>showNewRun());$('empty-start').addEventListener('click',()=>showNewRun());
$('task-select').addEventListener('change',updateForm);$('provider-select').addEventListener('change',updateForm);
document.querySelectorAll('.close-dialog').forEach(e=>e.addEventListener('click',()=>e.closest('dialog').close()));
document.querySelectorAll('[data-tab]').forEach(e=>e.addEventListener('click',()=>{state.tab=e.dataset.tab;document.querySelectorAll('[data-tab]').forEach(t=>{t.classList.toggle('selected',t===e);t.setAttribute('aria-selected',t===e?'true':'false');});renderInspector();}));
$('run-form').addEventListener('submit',async event=>{
  event.preventDefault();
  const config={task_id:$('task-select').value,provider:$('provider-select').value,limits:limits()};
  if(config.provider==='minimax')config.model=$('model-input').value.trim();
  if(config.task_id==='custom'){
    try{config.task={title:$('custom-title').value,prompt:$('custom-prompt').value,files:JSON.parse($('custom-files').value),checks:JSON.parse($('custom-checks').value)};}
    catch(error){$('form-error').textContent='Starting files and checks must be valid JSON.';$('form-error').hidden=false;return;}
  }
  await launch(config,$('run-dialog'),$('form-error'),$('launch-run'));
});
$('stop-run').addEventListener('click',async()=>{try{await api(`/api/runs/${state.selected}/cancel`,{});toast('Stopping this run…');await refresh();}catch(e){toast(e.message);}});
$('export-run').addEventListener('click',async()=>{
  try{const run=await api(`/api/runs/${state.selected}/export`);const url=URL.createObjectURL(new Blob([JSON.stringify(run,null,2)],{type:'application/json'}));const link=document.createElement('a');link.href=url;link.download=`aster-${run.id}.json`;link.click();setTimeout(()=>URL.revokeObjectURL(url),1000);toast('Trace exported.');}catch(e){toast(e.message);}
});
$('replay-run').addEventListener('click',()=>{
  const run=state.run;if(!run)return;
  $('checkpoint-select').innerHTML=run.checkpoints.map(step=>`<option value="${step}">${step===0?'Beginning of this run':`After agent turn ${step}`}</option>`).join('');
  $('replay-instruction').value='';$('replay-instruction').disabled=run.provider==='demo';
  $('replay-model').value=run.model;$('replay-model').disabled=run.provider==='demo';
  $('replay-hint').textContent=run.provider==='demo'?'This script replays the same behavior. Additional instructions require a live-model run.':'Full conversation state and files are restored. Choose an earlier checkpoint to try a different instruction.';
  $('replay-error').hidden=true;$('replay-dialog').showModal();
});
$('replay-form').addEventListener('submit',async e=>{
  e.preventDefault();await launch({parent_id:state.selected,checkpoint:Number($('checkpoint-select').value),instruction:$('replay-instruction').value,model:$('replay-model').value,limits:state.run.limits},$('replay-dialog'),$('replay-error'),e.submitter);
});
document.addEventListener('keydown',e=>{if((e.metaKey||e.ctrlKey)&&e.key==='Enter'&&$('run-dialog').open){e.preventDefault();$('run-form').requestSubmit($('launch-run'));}});
init();
