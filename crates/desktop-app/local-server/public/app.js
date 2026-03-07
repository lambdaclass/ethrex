const API = '/api';
const PROGRAM_NAMES = { 'evm-l2': 'EVM L2', 'zk-dex': 'ZK-DEX' };

// Open links in system browser via local-server API
function openExternal(url) {
  fetch(`${API}/open-url`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ url }),
  }).catch(() => {
    // Fallback for regular browser
    window.open(url, '_blank');
  });
}
// Intercept all target="_blank" clicks
document.addEventListener('click', (e) => {
  const a = e.target.closest('a[target="_blank"]');
  if (a) {
    e.preventDefault();
    openExternal(a.href);
  }
});
function programDisplayName(slug) { return PROGRAM_NAMES[slug] || slug; }
let currentDeploymentId = null;
let eventSource = null;
let logEventSource = null;
let allLogLines = [];

// Launch wizard state
let launchStep = 1;
let programs = [];
let selectedProgram = null;
let launchMode = 'local';
let launchDeploymentId = null;
let buildLogLines = [];
let deployEventSource = null;
let deployEvents = [];
let deployStartTime = null;
let phaseStartTime = null;
let currentPhase = 'configured';
let phaseDurations = {};
let elapsedInterval = null;

// ============================================================
// Navigation
// ============================================================
const pageTitles = { deployments: 'My L2s', launch: 'Launch L2', hosts: 'Remote Hosts', detail: 'L2 Details' };

document.querySelectorAll('.nav-link').forEach(btn => {
  btn.addEventListener('click', () => showView(btn.dataset.view));
});

function showView(name) {
  document.querySelectorAll('.view').forEach(v => v.classList.remove('active'));
  document.querySelectorAll('.nav-link').forEach(b => b.classList.remove('active'));
  const view = document.getElementById(`view-${name}`);
  if (view) view.classList.add('active');
  const navBtn = document.querySelector(`.nav-link[data-view="${name}"]`);
  if (navBtn) navBtn.classList.add('active');

  // Update header
  const titleEl = document.getElementById('page-title');
  if (titleEl) titleEl.textContent = pageTitles[name] || name;
  const launchBtn = document.getElementById('header-launch-btn');
  if (launchBtn) launchBtn.style.display = name === 'deployments' ? '' : 'none';

  if (name === 'deployments') loadDeployments();
  if (name === 'hosts') loadHosts();
  if (name === 'launch') { loadPrograms(); launchGoStep(1); }
}

// ============================================================
// Health Check
// ============================================================
async function checkHealth() {
  try {
    const res = await fetch(`${API}/health`);
    const data = await res.json();
    // Sidebar status
    const dot = document.getElementById('server-status-dot');
    const text = document.getElementById('server-status-text');
    // Footer status
    const fDot = document.getElementById('footer-engine-dot');
    const fText = document.getElementById('footer-engine-text');
    if (data.status === 'ok') {
      dot.className = 'dot ok';
      text.textContent = 'Engine running';
      fDot.className = 'footer-dot ok';
      fText.textContent = 'Engine running';
    } else {
      dot.className = 'dot';
      text.textContent = 'Error';
      fDot.className = 'footer-dot';
      fText.textContent = 'Engine error';
    }
  } catch {
    const dot = document.getElementById('server-status-dot');
    const text = document.getElementById('server-status-text');
    const fDot = document.getElementById('footer-engine-dot');
    const fText = document.getElementById('footer-engine-text');
    dot.className = 'dot';
    text.textContent = 'Offline';
    fDot.className = 'footer-dot';
    fText.textContent = 'Engine offline';
  }
}

// ============================================================
// Launch Wizard
// ============================================================
const LOCAL_STEPS = [
  { phase: 'checking_docker', label: 'Checking Docker' },
  { phase: 'building', label: 'Building Docker Images' },
  { phase: 'l1_starting', label: 'Starting L1 Node' },
  { phase: 'deploying_contracts', label: 'Deploying Contracts' },
  { phase: 'l2_starting', label: 'Starting L2 Node' },
  { phase: 'starting_prover', label: 'Starting Prover' },
  { phase: 'starting_tools', label: 'Starting Tools (Explorer, Bridge)' },
  { phase: 'running', label: 'Running' },
];

const REMOTE_STEPS = [
  { phase: 'pulling', label: 'Pulling Docker Images' },
  { phase: 'l1_starting', label: 'Starting L1 Node' },
  { phase: 'deploying_contracts', label: 'Deploying Contracts' },
  { phase: 'l2_starting', label: 'Starting L2 Node' },
  { phase: 'starting_prover', label: 'Starting Prover' },
  { phase: 'running', label: 'Running' },
];

const PHASE_ESTIMATES = {
  checking_docker: { min: 1, max: 5 },
  building: { min: 120, max: 600 },
  pulling: { min: 30, max: 180 },
  l1_starting: { min: 5, max: 30 },
  deploying_contracts: { min: 30, max: 120 },
  l2_starting: { min: 10, max: 60 },
  starting_prover: { min: 5, max: 15 },
  starting_tools: { min: 10, max: 60 },
};

function formatDuration(s) {
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const sec = s % 60;
  return sec > 0 ? `${m}m ${sec}s` : `${m}m`;
}

function formatEstimate(phase) {
  const est = PHASE_ESTIMATES[phase];
  if (!est || est.max <= 10) return '';
  return `~${formatDuration(est.min)}\u2013${formatDuration(est.max)}`;
}

async function loadPrograms() {
  try {
    const res = await fetch(`${API}/store/programs`);
    programs = await res.json();
  } catch {
    programs = [];
  }
  renderPrograms();
}

function renderPrograms() {
  const grid = document.getElementById('programs-grid');
  const search = (document.getElementById('program-search')?.value || '').toLowerCase();
  const catFilter = document.getElementById('category-filter')?.value || '';

  // Populate category filter
  const catSelect = document.getElementById('category-filter');
  if (catSelect && catSelect.options.length <= 1) {
    const cats = [...new Set(programs.map(p => p.category).filter(Boolean))];
    cats.forEach(c => {
      const opt = document.createElement('option');
      opt.value = c; opt.textContent = c;
      catSelect.appendChild(opt);
    });
  }

  const filtered = programs.filter(p => {
    const matchSearch = p.name.toLowerCase().includes(search) ||
      (p.description || '').toLowerCase().includes(search) ||
      (p.program_id || '').toLowerCase().includes(search);
    const matchCat = !catFilter || p.category === catFilter;
    return matchSearch && matchCat;
  });

  if (filtered.length === 0) {
    grid.innerHTML = '<p class="empty-state">No apps found.</p>';
    return;
  }

  grid.innerHTML = filtered.map(p => `
    <div class="program-card">
      <div class="program-card-header">
        <div class="program-icon">${esc((p.name || '?').charAt(0).toUpperCase())}</div>
        <div style="min-width:0">
          <div class="program-card-title">${esc(p.name)}</div>
          <div class="program-card-id">${esc(p.program_id || p.id)}</div>
        </div>
      </div>
      <div class="program-card-badges">
        ${p.category ? `<span class="badge-category">${esc(p.category)}</span>` : ''}
        ${p.is_official ? '<span class="badge-official">Official</span>' : ''}
        ${p.use_count ? `<span class="badge-deploys">${p.use_count} deployments</span>` : ''}
      </div>
      <div class="program-card-desc">${esc(p.description || 'No description')}</div>
      <button class="btn-select" onclick="selectProgram('${p.id}')">Select</button>
    </div>
  `).join('');
}

function filterPrograms() { renderPrograms(); }

function selectProgram(id) {
  selectedProgram = programs.find(p => p.id === id);
  if (!selectedProgram) return;
  document.getElementById('launch-name').value = `${selectedProgram.name} L2`;
  document.getElementById('launch-chain-id').value = Math.floor(Math.random() * 90000) + 10000;
  launchGoStep(2);
}

function launchGoStep(step) {
  launchStep = step;
  document.querySelectorAll('.launch-step').forEach(el => el.style.display = 'none');
  document.getElementById(`launch-step${step}`).style.display = 'block';

  // Update step indicator
  const indicator = document.getElementById('step-indicator');
  const stepLabels = ['Select App', 'Configure', 'Deploy'];
  indicator.innerHTML = [1, 2, 3].map((n, i) => `
    ${i > 0 ? `<div class="step-line${n <= step ? ' done' : ''}"></div>` : ''}
    <div class="step-item">
      <div class="step-circle${n === step ? ' active' : (n < step ? ' done' : '')}">${n < step ? '\u2713' : n}</div>
      <span class="step-label${n === step ? ' active' : (n < step ? ' done' : '')}">${stepLabels[i]}</span>
    </div>
  `).join('');

  if (step === 2 && selectedProgram) {
    // Update description
    document.getElementById('step2-desc').innerHTML = `Set up your L2 chain powered by <strong>${esc(selectedProgram.name)}</strong>.`;

    // Selected program card with app config inside
    const pid = selectedProgram.program_id || selectedProgram.id;
    let configHtml = '<h4>App Configuration</h4>';
    if (pid === 'zk-dex') {
      configHtml += '<p>ZK Circuits: SP1 (DEX order matching + settlement)<br>Verification: SP1 Verifier Contract<br>Genesis: Custom L2 genesis with DEX pre-deploys</p>';
    } else if (pid === 'evm-l2') {
      configHtml += '<p>Circuits: Standard EVM execution<br>Verification: Default Verifier Contract<br>Genesis: Standard L2 genesis</p>';
    } else {
      configHtml += `<p>Custom guest program: ${esc(pid)}<br>Verification: Default Verifier Contract</p>`;
    }

    document.getElementById('selected-program-info').innerHTML = `
      <div class="selected-program-top">
        <div class="program-icon">${esc(selectedProgram.name.charAt(0).toUpperCase())}</div>
        <div class="info">
          <h3>${esc(selectedProgram.name)}</h3>
          <div class="id">${esc(pid)}</div>
        </div>
        <button class="btn-change" onclick="launchGoStep(1)">Change</button>
      </div>
      <div class="app-config-box">${configHtml}</div>
    `;

    checkDocker();
  }
}

function setLaunchMode(mode) {
  launchMode = mode;
  document.querySelectorAll('.mode-card').forEach(b => {
    b.classList.toggle('active', b.dataset.mode === mode);
  });
  document.getElementById('remote-host-area').style.display = mode === 'remote' ? 'block' : 'none';
  document.getElementById('docker-status-area').style.display = mode === 'local' ? 'block' : 'none';
  document.getElementById('manual-rpc-area').style.display = mode === 'manual' ? 'block' : 'none';
  document.getElementById('l1-node-area').style.display = mode === 'local' ? 'block' : 'none';
  document.getElementById('deploy-dir-area').style.display = mode === 'manual' ? 'none' : 'block';

  const btn = document.getElementById('launch-deploy-btn');
  btn.textContent = mode === 'manual' ? 'Create L2 Config' : 'Deploy L2';

  if (mode === 'local') checkDocker();
  if (mode === 'remote') loadHostsForLaunch();
}

async function checkDocker() {
  const area = document.getElementById('docker-status-area');
  area.innerHTML = '<div class="docker-status checking">Checking Docker...</div>';
  try {
    const res = await fetch(`${API}/deployments/docker/status`);
    const data = await res.json();
    area.innerHTML = data.available
      ? '<div class="docker-status ok">\u2713 Docker is running</div>'
      : '<div class="docker-status error">\u2717 Docker is not running. <a href="https://www.docker.com/products/docker-desktop/" target="_blank" style="color:inherit;font-weight:600;text-decoration:underline">Download Docker Desktop</a></div>';
  } catch {
    area.innerHTML = '<div class="docker-status error">\u2717 Could not check Docker status</div>';
  }
}

async function loadHostsForLaunch() {
  try {
    const res = await fetch(`${API}/hosts`);
    const data = await res.json();
    const hosts = data.hosts || data || [];
    const sel = document.getElementById('launch-host-select');
    sel.innerHTML = '<option value="">Select a server...</option>' +
      hosts.map(h => `<option value="${h.id}">${esc(h.name)} (${esc(h.hostname)})</option>`).join('');
  } catch { /* ignore */ }
}

async function handleLaunchDeploy() {
  const name = document.getElementById('launch-name').value.trim();
  if (!name) { showLaunchError('L2 name is required'); return; }
  if (!selectedProgram) { showLaunchError('Please select a program first'); return; }

  const btn = document.getElementById('launch-deploy-btn');
  btn.disabled = true;
  btn.textContent = 'Deploying...';
  hideLaunchError();

  try {
    const rpcUrl = launchMode === 'manual' ? (document.getElementById('launch-rpc-url')?.value || '').trim() : undefined;
    const body = {
      programSlug: selectedProgram.program_id || selectedProgram.id,
      name,
      chainId: parseInt(document.getElementById('launch-chain-id').value) || undefined,
      rpcUrl: rpcUrl || undefined,
      config: {
        mode: launchMode,
        l1Image: launchMode === 'local' ? (document.getElementById('launch-l1-image')?.value || 'ethrex') : undefined,
        deployDir: (document.getElementById('launch-deploy-dir')?.value || '').trim() || undefined,
      },
    };
    if (launchMode === 'remote') {
      const hostId = document.getElementById('launch-host-select').value;
      if (hostId) body.hostId = hostId;
    }
    const deployDir = document.getElementById('launch-deploy-dir')?.value?.trim();
    if (deployDir) body.deployDir = deployDir;

    // 1. Create deployment
    const res = await fetch(`${API}/deployments`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!res.ok) { const err = await res.json(); throw new Error(err.error || 'Failed to create'); }
    const data = await res.json();
    launchDeploymentId = data.deployment?.id || data.id;

    if (launchMode === 'manual') {
      showDeploymentDetail(launchDeploymentId);
      btn.disabled = false;
      btn.textContent = 'Create L2 Config';
      return;
    }

    // 2. Start provision (returns immediately, runs in background)
    let provRes;
    if (launchMode === 'remote') {
      const hostId = document.getElementById('launch-host-select').value;
      provRes = await fetch(`${API}/deployments/${launchDeploymentId}/provision`, { method: 'POST', headers: {'Content-Type':'application/json'}, body: JSON.stringify(hostId ? {hostId} : {}) });
    } else {
      provRes = await fetch(`${API}/deployments/${launchDeploymentId}/provision`, { method: 'POST' });
    }
    if (!provRes.ok) {
      const err = await provRes.json().catch(() => ({}));
      throw new Error(err.error || 'Failed to start provisioning');
    }

    // 3. Switch to progress view
    launchGoStep(3);
    startDeployProgress(launchDeploymentId);
  } catch (err) {
    console.error('Deploy error:', err);
    showLaunchError(err.message);
  } finally {
    btn.disabled = false;
    btn.textContent = 'Deploy L2';
  }
}

function showLaunchError(msg) {
  const el = document.getElementById('launch-error');
  el.textContent = msg; el.style.display = 'block';
}
function hideLaunchError() {
  document.getElementById('launch-error').style.display = 'none';
}

// ============================================================
// Deploy Progress (SSE)
// ============================================================
function startDeployProgress(id) {
  currentPhase = 'configured';
  buildLogLines = [];
  deployEvents = [];
  phaseDurations = {};
  deployStartTime = Date.now();
  phaseStartTime = Date.now();

  const deployName = document.getElementById('launch-name')?.value || selectedProgram?.deployName || 'L2';
  document.getElementById('deploy-info-text').innerHTML = `Your L2 <strong>${esc(deployName)}</strong> powered by <strong>${esc(selectedProgram?.name || 'L2')}</strong> is being deployed...`;
  document.getElementById('deploy-error-msg').style.display = 'none';
  document.getElementById('deploy-complete').style.display = 'none';
  document.getElementById('goto-dashboard-btn').style.display = 'none';

  renderProgressSteps();
  startElapsedTimer();

  if (deployEventSource) deployEventSource.close();
  deployEventSource = new EventSource(`${API}/deployments/${id}/events`);

  deployEventSource.onmessage = (e) => {
    try {
      const data = JSON.parse(e.data);
      if (data.event === 'log') {
        buildLogLines.push(data.message || '');
        if (buildLogLines.length > 200) buildLogLines = buildLogLines.slice(-200);
        renderBuildLog();
        return;
      }

      deployEvents.push(data);
      if (data.phase && data.phase !== currentPhase) {
        if (currentPhase !== 'configured') {
          phaseDurations[currentPhase] = Math.floor((Date.now() - phaseStartTime) / 1000);
        }
        currentPhase = data.phase;
        phaseStartTime = Date.now();
      }
      if (data.message) {
        document.getElementById('deploy-message').textContent = data.message;
        document.getElementById('deploy-message').style.display = 'block';
      }
      renderProgressSteps();

      if (data.event === 'error') {
        document.getElementById('deploy-error-msg').textContent = data.message || 'Deployment failed';
        document.getElementById('deploy-error-msg').style.display = 'block';
        document.getElementById('deploy-message').style.display = 'none';
        stopElapsedTimer();
        deployEventSource.close();
      }
      if (data.phase === 'running') {
        stopElapsedTimer();
        showDeployComplete(data);
        deployEventSource.close();
      }
    } catch { /* ignore */ }
  };

  deployEventSource.onerror = () => {
    if (currentPhase === 'running') deployEventSource.close();
  };
}

function renderProgressSteps() {
  const container = document.getElementById('deploy-progress-steps');
  const steps = launchMode === 'remote' ? REMOTE_STEPS : LOCAL_STEPS;
  const currentIdx = steps.findIndex(s => s.phase === currentPhase);
  const hasError = document.getElementById('deploy-error-msg').style.display !== 'none';
  const isTerminal = currentPhase === 'running' || hasError;

  // Elapsed bar (uses elements already in HTML)
  const totalElapsed = Math.floor((Date.now() - deployStartTime) / 1000);
  const elapsedEl = document.getElementById('deploy-elapsed');
  const stepCountEl = document.getElementById('deploy-step-count');
  if (elapsedEl) elapsedEl.textContent = formatDuration(totalElapsed);
  if (stepCountEl) {
    if (currentPhase === 'running') {
      stepCountEl.textContent = 'Complete';
    } else if (!hasError) {
      stepCountEl.textContent = `Step ${currentIdx + 1} of ${steps.length - 1}`;
    }
  }

  container.innerHTML = steps.map((step, i) => {
    const isComplete = i < currentIdx || currentPhase === 'running';
    const isCurrent = step.phase === currentPhase;
    const cls = isComplete ? 'done' : isCurrent ? 'active' : '';
    const elapsed = isCurrent && !isTerminal ? Math.floor((Date.now() - phaseStartTime) / 1000) : null;
    const completedDur = phaseDurations[step.phase];
    const estimate = formatEstimate(step.phase);

    let timeHtml = '';
    if (isCurrent && !isTerminal && elapsed !== null) {
      timeHtml = `<span style="color:var(--blue-600)">${formatDuration(elapsed)}</span>`;
      if (estimate) timeHtml += ` <span style="color:var(--gray-400)">(${estimate})</span>`;
    } else if (isComplete && completedDur !== undefined) {
      timeHtml = `<span style="color:var(--green-600)">${formatDuration(completedDur)}</span>`;
    } else if (!isComplete && !isCurrent && estimate) {
      timeHtml = `<span style="color:var(--gray-300)">${estimate}</span>`;
    }

    let iconHtml;
    if (isComplete) iconHtml = '\u2713';
    else if (isCurrent && !isTerminal) iconHtml = '<div style="width:12px;height:12px;border:2px solid white;border-top-color:transparent;border-radius:50%" class="animate-spin"></div>';
    else iconHtml = i + 1;

    return `<div class="progress-step ${cls}">
      <div class="step-icon">${iconHtml}</div>
      <div class="step-label">${step.label}</div>
      <div class="step-time">${timeHtml}</div>
    </div>`;
  }).join('');

  renderEventLog();
}

function renderBuildLog() {
  document.getElementById('build-log-count').textContent = buildLogLines.length;
  const container = document.getElementById('build-log');
  container.innerHTML = buildLogLines.map(l =>
    `<div style="white-space:pre-wrap;word-break:break-all">${esc(l)}</div>`
  ).join('');
  container.scrollTop = container.scrollHeight;
}

function renderEventLog() {
  const countEl = document.getElementById('event-log-count');
  const logEl = document.getElementById('event-log');
  if (countEl) countEl.textContent = deployEvents.length;
  if (logEl) {
    logEl.innerHTML = deployEvents.map(e =>
      `<div><span class="event-time">${new Date(e.timestamp).toLocaleTimeString()}</span> <span class="event-type ${e.event === 'error' ? 'error' : 'ok'}">[${e.event}]</span> ${esc(e.message || e.phase || '')}</div>`
    ).join('');
  }
}

function startElapsedTimer() { stopElapsedTimer(); elapsedInterval = setInterval(() => renderProgressSteps(), 1000); }
function stopElapsedTimer() { if (elapsedInterval) { clearInterval(elapsedInterval); elapsedInterval = null; } }

function showDeployComplete(data) {
  document.getElementById('deploy-message').style.display = 'none';
  const el = document.getElementById('deploy-complete');
  let html = '<p style="font-weight:600;margin-bottom:8px">Deployment is running!</p>';
  if (data.l1Rpc) html += `<p>L1 RPC: <code style="background:var(--green-100);padding:2px 6px;border-radius:4px">${esc(data.l1Rpc)}</code></p>`;
  if (data.l2Rpc) html += `<p>L2 RPC: <code style="background:var(--green-100);padding:2px 6px;border-radius:4px">${esc(data.l2Rpc)}</code></p>`;
  if (data.bridgeAddress) html += `<p>Bridge: <code style="background:var(--green-100);padding:2px 6px;border-radius:4px;font-size:11px">${esc(data.bridgeAddress)}</code></p>`;
  el.innerHTML = html;
  el.style.display = 'block';
  document.getElementById('goto-dashboard-btn').style.display = 'inline-block';
}

function goToDashboard() {
  if (launchDeploymentId) showDeploymentDetail(launchDeploymentId);
}

// Resume watching an in-progress deployment from the list
async function resumeDeployProgress(id) {
  launchDeploymentId = id;

  try {
    // Fetch deployment info + stored event history
    const [statusRes, depRes, histRes] = await Promise.all([
      fetch(`${API}/deployments/${id}/status`),
      fetch(`${API}/deployments`),
      fetch(`${API}/deployments/${id}/events/history`),
    ]);
    const statusData = await statusRes.json();
    const depData = await depRes.json();
    const histData = await histRes.json();
    const depList = depData.deployments || depData || [];
    const dep = depList.find(d => d.id === id);
    const storedEvents = histData.events || [];

    // Restore state from stored events
    selectedProgram = selectedProgram || { name: dep?.program_name || programDisplayName(dep?.program_slug) || 'L2', id: dep?.program_slug || '' };
    currentPhase = statusData.phase || dep?.phase || 'building';
    buildLogLines = [];
    deployEvents = [];
    phaseDurations = {};
    deployStartTime = dep?.created_at ? new Date(dep.created_at).getTime() : Date.now();

    // Rebuild state from DB events: extract logs, phase transitions, durations
    let lastPhaseTime = deployStartTime;
    let lastPhase = 'configured';
    for (const ev of storedEvents) {
      if (ev.event_type === 'log') {
        buildLogLines.push(ev.message || '');
      } else {
        deployEvents.push({
          event: ev.event_type,
          phase: ev.phase,
          message: ev.message,
          timestamp: ev.created_at,
        });
        if (ev.phase && ev.phase !== lastPhase) {
          if (lastPhase !== 'configured') {
            phaseDurations[lastPhase] = Math.floor((ev.created_at - lastPhaseTime) / 1000);
          }
          lastPhase = ev.phase;
          lastPhaseTime = ev.created_at;
        }
      }
    }
    if (buildLogLines.length > 500) buildLogLines = buildLogLines.slice(-500);
    phaseStartTime = lastPhaseTime;

    // Show launch view at step 3
    showView('launch');
    launchGoStep(3);

    const deployName = dep?.name || 'L2';
    const appName = dep?.program_name || dep?.program_slug || 'L2';
    document.getElementById('deploy-info-text').innerHTML = `Your L2 <strong>${esc(deployName)}</strong> powered by <strong>${esc(appName)}</strong> is being deployed...`;
    document.getElementById('deploy-error-msg').style.display = 'none';
    document.getElementById('deploy-complete').style.display = 'none';
    document.getElementById('goto-dashboard-btn').style.display = 'none';

    renderProgressSteps();
    renderBuildLog();

    // If still active, connect SSE for live updates + start timer
    if (histData.isActive) {
      startElapsedTimer();

      if (deployEventSource) deployEventSource.close();
      deployEventSource = new EventSource(`${API}/deployments/${id}/events`);

      deployEventSource.onmessage = (e) => {
        try {
          const data = JSON.parse(e.data);
          if (data.event === 'log') {
            buildLogLines.push(data.message || '');
            if (buildLogLines.length > 500) buildLogLines = buildLogLines.slice(-500);
            renderBuildLog();
            return;
          }
          deployEvents.push(data);
          if (data.phase && data.phase !== currentPhase) {
            if (currentPhase !== 'configured') {
              phaseDurations[currentPhase] = Math.floor((Date.now() - phaseStartTime) / 1000);
            }
            currentPhase = data.phase;
            phaseStartTime = Date.now();
          }
          if (data.message) {
            document.getElementById('deploy-message').textContent = data.message;
            document.getElementById('deploy-message').style.display = 'block';
          }
          renderProgressSteps();

          if (data.event === 'error') {
            document.getElementById('deploy-error-msg').textContent = data.message || 'Deployment failed';
            document.getElementById('deploy-error-msg').style.display = 'block';
            document.getElementById('deploy-message').style.display = 'none';
            stopElapsedTimer();
            deployEventSource.close();
          }
          if (data.phase === 'running') {
            stopElapsedTimer();
            showDeployComplete(data);
            deployEventSource.close();
          }
        } catch { /* ignore */ }
      };

      deployEventSource.onerror = () => {
        if (currentPhase === 'running') deployEventSource.close();
      };
    } else {
      // Not active -- show final state
      if (currentPhase === 'running') {
        showDeployComplete(statusData);
      } else if (currentPhase === 'error') {
        document.getElementById('deploy-error-msg').textContent = statusData.error || dep?.error_message || 'Deployment failed';
        document.getElementById('deploy-error-msg').style.display = 'block';
      }
    }
  } catch (err) {
    console.error('Failed to resume deploy progress:', err);
  }
}

// ============================================================
// Deployments List
// ============================================================
let expandedDeploymentId = null;
let containerPollInterval = null;

async function loadDeployments() {
  try {
    const res = await fetch(`${API}/deployments`);
    const data = await res.json();
    const list = data.deployments || data || [];
    const container = document.getElementById('deployments-list');

    if (list.length === 0) {
      container.innerHTML = `<div class="empty-state">
        <p style="margin-bottom:12px">No L2s launched yet.</p>
        <button class="btn-primary" onclick="showView('launch')">Launch your first L2</button>
      </div>`;
      return;
    }

    container.innerHTML = `
      <table class="data-table">
        <thead>
          <tr>
            <th style="width:40px;padding-left:20px"></th>
            <th>Name</th>
            <th>Status</th>
            <th>Ports</th>
            <th>Phase</th>
            <th style="text-align:right">Actions</th>
          </tr>
        </thead>
        <tbody>
          ${list.map(d => renderDeploymentRow(d)).join('')}
        </tbody>
      </table>`;
  } catch {
    document.getElementById('deployments-list').innerHTML = '<p class="empty-state">Failed to load deployments</p>';
  }
}

function renderDeploymentRow(d) {
  const isExpanded = expandedDeploymentId === d.id;
  const statusClass = d.phase === 'running' ? 'running' : d.phase === 'error' ? 'error'
    : ['building','pulling','l1_starting','deploying_contracts','l2_starting','starting_prover','starting_tools','checking_docker'].includes(d.phase) ? 'building' : 'stopped';
  const ports = [d.l1_port ? `L1:${d.l1_port}` : '', d.l2_port ? `L2:${d.l2_port}` : ''].filter(Boolean).join(' · ') || '-';

  return `
    <tr class="deploy-row" data-id="${d.id}">
      <td>
        <button class="expand-btn" onclick="event.stopPropagation(); toggleDeployExpand('${d.id}')" style="background:none;border:none;cursor:pointer;padding:4px 4px 4px 8px;color:var(--text-muted);display:flex;align-items:center">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" style="transition:transform 0.15s;${isExpanded ? 'transform:rotate(90deg)' : ''}">
            <polyline points="9 18 15 12 9 6"/>
          </svg>
        </button>
      </td>
      <td onclick="${isDeploying(d.phase) ? `resumeDeployProgress('${d.id}')` : `showDeploymentDetail('${d.id}')`}" style="cursor:pointer">
        <div class="name-cell">
          <div class="icon-box">${esc((d.name || '?').charAt(0))}</div>
          <div>
            <div class="name-text">${esc(d.name)}</div>
            <div style="font-size:11px;color:var(--text-muted)">${esc(d.program_name || programDisplayName(d.program_slug) || d.program_id)}</div>
          </div>
        </div>
      </td>
      <td>
        <div class="status-cell">
          <span class="status-dot ${statusClass}"></span>
          <span>${statusLabel(d.phase)}</span>
        </div>
      </td>
      <td style="font-size:12px;color:var(--text-secondary);font-family:monospace">${ports}</td>
      <td>${renderPhaseBadge(d.phase)}</td>
      <td>
        <div class="actions-cell">
          ${isDeploying(d.phase) ? `
            <button class="icon-btn" title="View Progress" onclick="event.stopPropagation(); resumeDeployProgress('${d.id}')" style="color:var(--yellow-600)">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>
            </button>` : ''}
          ${d.phase === 'running' ? `
            <button class="icon-btn" title="Stop" onclick="event.stopPropagation(); stopDeploy('${d.id}')">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
            </button>` : ''}
          ${d.phase === 'error' ? `
            <button class="icon-btn" title="View Error" onclick="event.stopPropagation(); resumeDeployProgress('${d.id}')" style="color:var(--red-500,#ef4444)">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
            </button>
            <button class="icon-btn" title="Retry" onclick="event.stopPropagation(); retryDeploy('${d.id}')" style="color:var(--green-500,#22c55e)">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/></svg>
            </button>` : ''}
          ${d.phase === 'stopped' ? `
            <button class="icon-btn" title="Start" onclick="event.stopPropagation(); startDeploy('${d.id}')">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="5 3 19 12 5 21 5 3"/></svg>
            </button>` : ''}
          <button class="icon-btn" title="Details" onclick="event.stopPropagation(); showDeploymentDetail('${d.id}')">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>
          </button>
          <button class="icon-btn danger" title="Delete" onclick="event.stopPropagation(); deleteDeploy('${d.id}')">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
          </button>
        </div>
      </td>
    </tr>
    ${isExpanded ? `<tr class="container-row"><td colspan="6" style="padding:0"><div id="containers-${d.id}" class="container-expand-area">Loading containers...</div></td></tr>` : ''}`;
}

function isDeploying(phase) {
  return ['checking_docker','building','pulling','l1_starting','deploying_contracts','l2_starting','starting_prover','starting_tools'].includes(phase);
}

function statusLabel(phase) {
  const map = {
    configured: 'Created', checking_docker: 'Checking...', building: 'Building',
    pulling: 'Pulling', l1_starting: 'Starting', deploying_contracts: 'Deploying',
    l2_starting: 'Starting', starting_prover: 'Starting', starting_tools: 'Starting',
    running: 'Running', stopped: 'Stopped', error: 'Error',
  };
  return map[phase] || phase;
}

async function toggleDeployExpand(id) {
  if (expandedDeploymentId === id) {
    expandedDeploymentId = null;
    if (containerPollInterval) { clearInterval(containerPollInterval); containerPollInterval = null; }
    loadDeployments();
    return;
  }
  expandedDeploymentId = id;
  loadDeployments();
  loadContainersForDeploy(id);
  if (containerPollInterval) clearInterval(containerPollInterval);
  containerPollInterval = setInterval(() => loadContainersForDeploy(id), 5000);
}

async function loadContainersForDeploy(id) {
  try {
    const res = await fetch(`${API}/deployments/${id}/status`);
    const data = await res.json();
    const el = document.getElementById(`containers-${id}`);
    if (!el) return;

    const containers = data.containers || [];
    if (containers.length === 0) {
      el.innerHTML = '<div class="container-empty">No containers running</div>';
      return;
    }

    el.innerHTML = `
      <table class="container-table">
        <thead>
          <tr>
            <th></th>
            <th>Service</th>
            <th>State</th>
            <th>Ports</th>
            <th>Image</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          ${containers.map(c => {
            const state = (c.State || c.state || '').toLowerCase();
            const stateClass = state === 'running' ? 'running' : state === 'exited' ? 'stopped' : 'building';
            const service = c.Service || c.service || c.Name || c.name || '-';
            const friendlyName = {
              'tokamak-app-l1': 'L1 Node', 'tokamak-app-l2': 'L2 Node',
              'tokamak-app-deployer': 'Deployer', 'tokamak-app-prover': 'Prover',
              'frontend-l1': 'L1 Explorer', 'backend-l1': 'L1 Explorer Backend',
              'frontend-l2': 'L2 Explorer', 'backend-l2': 'L2 Explorer Backend',
              'db': 'Explorer DB', 'db-init': 'DB Init', 'redis-db': 'Redis',
              'proxy': 'Explorer Proxy', 'function-selectors': 'Function Selectors',
              'bridge-ui': 'Bridge UI',
            }[service] || service;
            const ports = formatContainerPorts(c.Ports || c.ports || '');
            const image = (c.Image || c.image || '-').split('/').pop();
            const status = c.Status || c.status || state;
            const isMainService = service.startsWith('tokamak-app-');
            return `
              <tr>
                <td><span class="status-dot ${stateClass}" style="margin:0"></span></td>
                <td style="font-weight:500">${esc(friendlyName)}</td>
                <td><span style="font-size:12px;color:${state === 'running' ? 'var(--green-600)' : 'var(--text-muted)'}">${esc(status)}</span></td>
                <td style="font-size:11px;font-family:monospace;color:var(--text-secondary)">${esc(ports)}</td>
                <td style="font-size:11px;color:var(--text-muted)">${esc(image)}</td>
                <td>${isMainService && service !== 'tokamak-app-deployer' ? (state === 'running'
                  ? `<button class="icon-btn" title="Stop" onclick="event.stopPropagation(); serviceAction('${id}','${service}','stop')" style="padding:2px 4px">
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>
                    </button>`
                  : `<button class="icon-btn" title="Start" onclick="event.stopPropagation(); serviceAction('${id}','${service}','start')" style="padding:2px 4px;color:var(--green-500,#22c55e)">
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polygon points="5 3 19 12 5 21 5 3"/></svg>
                    </button>`) : ''}</td>
              </tr>`;
          }).join('')}
        </tbody>
      </table>`;
  } catch {
    const el = document.getElementById(`containers-${id}`);
    if (el) el.innerHTML = '<div class="container-empty">Failed to load containers</div>';
  }
}

async function serviceAction(deployId, service, action) {
  try {
    await fetch(`${API}/deployments/${deployId}/service/${service}/${action}`, { method: 'POST' });
    // Refresh container list after a brief delay
    setTimeout(() => loadContainersForDeploy(deployId), 1000);
  } catch (e) {
    console.error(`Service ${action} failed:`, e);
  }
}

function formatContainerPorts(ports) {
  if (!ports) return '-';
  if (typeof ports === 'string') {
    const matches = ports.match(/(\d+)->\d+\/tcp/g);
    if (matches) return matches.map(m => m.replace('->',':').replace('/tcp','')).join(', ');
    return ports.length > 30 ? ports.substring(0, 27) + '...' : ports;
  }
  return '-';
}

async function stopDeploy(id) {
  try {
    await fetch(`${API}/deployments/${id}/stop`, { method: 'POST' });
    loadDeployments();
  } catch (e) { console.error('Stop failed:', e); }
}

async function startDeploy(id) {
  try {
    await fetch(`${API}/deployments/${id}/start`, { method: 'POST' });
    loadDeployments();
  } catch (e) { console.error('Start failed:', e); }
}

async function retryDeploy(id) {
  try {
    // Destroy existing containers (ignore errors if not provisioned yet)
    await fetch(`${API}/deployments/${id}/destroy`, { method: 'POST' }).catch(() => {});

    // Fetch deployment info for display
    const depRes = await fetch(`${API}/deployments`);
    const depData = await depRes.json();
    const depList = depData.deployments || depData || [];
    const dep = depList.find(d => d.id === id);
    if (dep) {
      selectedProgram = { name: dep.program_name || programDisplayName(dep.program_slug) || 'L2', id: dep.program_slug || '', deployName: dep.name || 'L2' };
    }

    const resp = await fetch(`${API}/deployments/${id}/provision`, { method: 'POST' });
    if (resp.ok) {
      launchDeploymentId = id;
      showView('launch');
      launchGoStep(3);
      startDeployProgress(id);
    } else {
      const err = await resp.json().catch(() => ({}));
      console.error('Retry failed:', err.error || 'Unknown error');
      loadDeployments();
    }
  } catch (e) {
    console.error('Retry failed:', e);
    console.error('Retry failed:', e.message);
  }
}

async function deleteDeploy(id) {
  try {
    await fetch(`${API}/deployments/${id}`, { method: 'DELETE' });
    if (expandedDeploymentId === id) expandedDeploymentId = null;
    loadDeployments();
  } catch (e) { console.error('Delete failed:', e); }
}

function renderPhaseBadge(phase) {
  const labels = {
    configured: 'Not deployed', checking_docker: 'Checking Docker', building: 'Building',
    pulling: 'Pulling Images', l1_starting: 'Starting L1', deploying_contracts: 'Deploying',
    l2_starting: 'Starting L2', starting_prover: 'Starting Prover', starting_tools: 'Starting Tools',
    running: 'Running', stopped: 'Stopped', error: 'Error',
  };
  const animating = ['checking_docker','building','pulling','l1_starting','deploying_contracts','l2_starting','starting_prover','starting_tools'];
  const label = labels[phase] || phase;
  const dot = animating.includes(phase) ? '<span class="dot pulse"></span>' : (phase === 'running' ? '<span class="dot"></span>' : '');
  return `<span class="phase-badge phase-${phase}">${dot}${label}</span>`;
}

// ============================================================
// Deployment Detail
// ============================================================
let detailPollInterval = null;
let detailDeployment = null;
let detailStatus = null;
let detailMonitoring = null;
let detailTab = 'overview';

async function showDeploymentDetail(id) {
  currentDeploymentId = id;
  showView('detail');
  detailDeployment = null;
  detailStatus = null;
  detailMonitoring = null;
  detailTab = 'overview';
  if (detailPollInterval) { clearInterval(detailPollInterval); detailPollInterval = null; }
  if (logEventSource) { logEventSource.close(); logEventSource = null; }

  try {
    const res = await fetch(`${API}/deployments/${id}`);
    const data = await res.json();
    detailDeployment = data.deployment || data;
    renderDetail();
    startDetailPolling();
  } catch {
    document.getElementById('view-detail').innerHTML = '<p class="empty-state">Failed to load deployment</p>';
  }
}

function renderDetail() {
  const d = detailDeployment;
  if (!d) return;
  document.getElementById('detail-name').textContent = d.name;
  document.getElementById('detail-phase').innerHTML = renderPhaseBadge(d.phase);

  // Mode badge
  const config = parseDeployConfig(d);
  const modeLabels = { local: 'Local', remote: 'Remote', manual: 'Manual' };
  document.getElementById('detail-mode-badge').innerHTML =
    `<span class="mode-badge ${config.mode}">${modeLabels[config.mode] || config.mode}</span>`;

  renderDetailTab();
}

function parseDeployConfig(d) {
  try { return d.config ? (typeof d.config === 'string' ? JSON.parse(d.config) : d.config) : { mode: 'local' }; }
  catch { return { mode: 'local' }; }
}

function switchTab(tab) {
  detailTab = tab;
  document.querySelectorAll('.tab-btn').forEach(btn => btn.classList.toggle('active', btn.dataset.tab === tab));
  renderDetailTab();
}

function renderDetailTab() {
  document.querySelectorAll('.tab-panel').forEach(p => p.classList.remove('active'));
  const panel = document.getElementById(`tab-${detailTab}`);
  if (panel) panel.classList.add('active');
  if (detailTab === 'overview') renderOverviewTab();
  if (detailTab === 'logs') renderLogsTab();
  if (detailTab === 'config') renderConfigTab();
}

function renderOverviewTab() {
  const d = detailDeployment;
  if (!d) return;
  const isProvisioned = !!d.docker_project;
  const isRunning = d.phase === 'running';
  const isStopped = d.phase === 'stopped';
  const isError = d.phase === 'error';
  const isDeploying = ['checking_docker','building','l1_starting','deploying_contracts','l2_starting','starting_prover','starting_tools'].includes(d.phase);

  // Containers
  const containerCards = document.getElementById('container-cards');
  if (detailStatus?.containers?.length > 0) {
    const services = ['tokamak-app-l1','tokamak-app-l2','tokamak-app-prover','tokamak-app-deployer'];
    const labels = {'tokamak-app-l1':'L1 Node','tokamak-app-l2':'L2 Node','tokamak-app-prover':'Prover','tokamak-app-deployer':'Deployer'};
    containerCards.innerHTML = '<div class="container-cards">' + services.map(svc => {
      const c = detailStatus.containers.find(c => c.Service === svc || c.Name?.includes(svc.replace('tokamak-app-','')));
      const state = c ? c.State : 'not started';
      return `<div class="container-card ${state === 'running' ? 'running' : state === 'exited' ? 'exited' : ''}">
        <h4>${labels[svc] || svc}</h4>
        <div class="status ${state === 'running' ? 'running' : 'stopped'}">${state}</div>
      </div>`;
    }).join('') + '</div>';
  } else {
    containerCards.innerHTML = '';
  }

  // Endpoints
  const endpointsEl = document.getElementById('detail-endpoints');
  if (isProvisioned) {
    endpointsEl.style.display = 'block';
    let html = '<h3 style="margin-bottom:16px">Endpoints</h3><div class="endpoints-grid">';
    html += `<div class="endpoint-card"><div class="label">L1 RPC</div><code>${d.l1_port ? `http://127.0.0.1:${d.l1_port}` : 'Not assigned'}</code>${detailMonitoring?.l1?.healthy ? '<span class="health ok">Connected</span>' : ''}</div>`;
    html += `<div class="endpoint-card"><div class="label">L2 RPC</div><code>${d.l2_port ? `http://127.0.0.1:${d.l2_port}` : 'Not assigned'}</code>${detailMonitoring?.l2?.healthy ? '<span class="health ok">Connected</span>' : ''}</div>`;
    if (isRunning) {
      html += `<div class="endpoint-card"><div class="label">L1 Explorer</div><a href="http://127.0.0.1:${d.tools_l1_explorer_port||8083}" target="_blank">http://127.0.0.1:${d.tools_l1_explorer_port||8083}</a></div>`;
      html += `<div class="endpoint-card"><div class="label">L2 Explorer</div><a href="http://127.0.0.1:${d.tools_l2_explorer_port||8082}" target="_blank">http://127.0.0.1:${d.tools_l2_explorer_port||8082}</a></div>`;
      html += `<div class="endpoint-card"><div class="label">Bridge UI</div><a href="http://127.0.0.1:${d.tools_bridge_ui_port||3000}" target="_blank">http://127.0.0.1:${d.tools_bridge_ui_port||3000}</a></div>`;
    }
    html += '</div>';
    if (isRunning) {
      html += `<div style="display:flex;gap:8px;margin-top:16px;padding-top:16px;border-top:1px solid var(--gray-200);align-items:center">
        <button class="btn-secondary" onclick="toolsAction('build')">Build Tools</button>
        <button class="btn-secondary" onclick="toolsAction('restart')">Restart Tools</button>
        <button class="btn-secondary" onclick="toolsAction('stop-tools')">Stop Tools</button>
        <span style="font-size:12px;color:var(--gray-400);margin-left:8px">Blockscout, Bridge UI</span>
      </div>`;
    }
    document.getElementById('endpoints-content').innerHTML = html;
  } else {
    endpointsEl.style.display = 'none';
  }

  // Dynamic content
  let dynamicEl = document.querySelector('#tab-overview .overview-dynamic');
  if (!dynamicEl) {
    dynamicEl = document.createElement('div');
    dynamicEl.className = 'overview-dynamic';
    document.getElementById('tab-overview').appendChild(dynamicEl);
  }

  let html = '';

  // Error
  if (d.error_message) html += `<div class="error-box" style="margin-bottom:24px">${esc(d.error_message)}</div>`;

  // Actions
  html += '<div style="display:flex;gap:8px;margin-bottom:24px">';
  if (!isProvisioned) html += '<button class="btn-primary" onclick="deployAction(\'provision\')">Deploy</button>';
  if (isStopped) html += '<button class="btn-green" onclick="deployAction(\'start\')">Start</button>';
  if (isRunning || isDeploying) html += '<button class="btn-orange" onclick="deployAction(\'stop\')">Stop</button>';
  if (isProvisioned) html += '<button class="btn-danger" onclick="deployAction(\'destroy\')">Destroy</button>';
  if (isError) html += '<button class="btn-primary" onclick="deployAction(\'provision\')">Retry Deploy</button>';
  html += '</div>';

  // Contracts
  if (d.bridge_address || d.proposer_address) {
    html += '<div class="card" style="margin-bottom:24px"><h3 style="margin-bottom:12px">Contracts</h3><dl class="info-grid">';
    if (d.bridge_address) html += `<dt>Bridge</dt><dd style="font-size:11px">${esc(d.bridge_address)}</dd>`;
    if (d.proposer_address) html += `<dt>OnChainProposer</dt><dd style="font-size:11px">${esc(d.proposer_address)}</dd>`;
    html += '</dl></div>';
  }

  // Monitoring
  if (detailMonitoring && (detailMonitoring.l1 || detailMonitoring.l2)) {
    html += '<div class="card" style="margin-bottom:24px"><h3 style="margin-bottom:12px">Chain Info</h3><div style="display:grid;grid-template-columns:1fr 1fr;gap:12px">';
    if (detailMonitoring.l1) html += `<div style="padding:12px;background:var(--gray-50);border-radius:8px"><div style="font-weight:500;margin-bottom:4px">L1</div><div style="font-size:13px"><span style="color:var(--gray-500)">Block:</span> <span style="font-family:monospace">${detailMonitoring.l1.blockNumber ?? 'N/A'}</span></div><div style="font-size:13px"><span style="color:var(--gray-500)">Chain ID:</span> <span style="font-family:monospace">${detailMonitoring.l1.chainId ?? 'N/A'}</span></div></div>`;
    if (detailMonitoring.l2) html += `<div style="padding:12px;background:var(--gray-50);border-radius:8px"><div style="font-weight:500;margin-bottom:4px">L2</div><div style="font-size:13px"><span style="color:var(--gray-500)">Block:</span> <span style="font-family:monospace">${detailMonitoring.l2.blockNumber ?? 'N/A'}</span></div><div style="font-size:13px"><span style="color:var(--gray-500)">Chain ID:</span> <span style="font-family:monospace">${detailMonitoring.l2.chainId ?? 'N/A'}</span></div></div>`;
    html += '</div></div>';
  }

  // Info with Edit
  html += `<div class="card" style="margin-bottom:24px">
    <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:12px">
      <h3>Settings</h3>
      <button class="btn-back" style="font-size:13px" onclick="toggleSettingsEdit()">Edit</button>
    </div>
    <div id="settings-display">
      <dl class="info-grid">
        <dt>Chain ID</dt><dd>${d.chain_id || 'Not set'}</dd>
        <dt>L1 RPC URL</dt><dd style="font-size:11px">${d.rpc_url || (d.l1_port ? `http://127.0.0.1:${d.l1_port}` : 'Not set')}</dd>
        <dt>Docker Project</dt><dd style="font-size:11px">${d.docker_project || 'Not provisioned'}</dd>
        <dt>Created</dt><dd>${new Date(d.created_at).toLocaleString()}</dd>
      </dl>
    </div>
    <div id="settings-edit" style="display:none">
      <label>L2 Name<input type="text" id="edit-name" value="${esc(d.name)}"></label>
      <label>Chain ID<input type="number" id="edit-chain-id" value="${d.chain_id || ''}"></label>
      <label>L1 RPC URL<input type="text" id="edit-rpc-url" value="${esc(d.rpc_url || '')}"></label>
      <div style="display:flex;gap:8px;margin-top:12px">
        <button class="btn-primary" onclick="saveSettings()">Save</button>
        <button class="btn-secondary" onclick="toggleSettingsEdit()">Cancel</button>
      </div>
    </div>
  </div>`;

  // Danger zone
  html += `<div class="danger-zone"><h3>Danger Zone</h3><button class="btn-danger" onclick="deleteDeployment('${d.id}')">Remove L2</button></div>`;

  dynamicEl.innerHTML = html;
}

async function deployAction(action) {
  if (!currentDeploymentId) return;
  // Show loading state on all action buttons
  const btns = document.querySelectorAll('#tab-overview .overview-dynamic button');
  btns.forEach(b => { b.disabled = true; b.style.opacity = '0.5'; });
  const actionLabels = { stop: 'Stopping...', start: 'Starting...', destroy: 'Destroying...', provision: 'Deploying...' };
  const statusEl = document.querySelector('#tab-overview .overview-dynamic');
  if (statusEl) {
    let indicator = document.getElementById('action-status');
    if (!indicator) { indicator = document.createElement('div'); indicator.id = 'action-status'; statusEl.prepend(indicator); }
    indicator.textContent = actionLabels[action] || 'Processing...';
    indicator.style.cssText = 'padding:8px 12px;background:var(--gray-100);border-radius:6px;margin-bottom:12px;font-size:13px;color:var(--text-secondary)';
  }
  try {
    const res = await fetch(`${API}/deployments/${currentDeploymentId}/${action}`, { method: 'POST' });
    if (!res.ok) { const e = await res.json(); throw new Error(e.error); }
    const data = await res.json();
    // If destroyed/deleted, go back to list
    if (data.deleted || action === 'destroy') {
      showView('deployments');
      return;
    }
    if (data.deployment) detailDeployment = data.deployment;
    else { const r2 = await fetch(`${API}/deployments/${currentDeploymentId}`); const d2 = await r2.json(); detailDeployment = d2.deployment || d2; }
    const indicator = document.getElementById('action-status');
    if (indicator) indicator.remove();
    renderDetail();
  } catch (err) {
    const indicator = document.getElementById('action-status');
    if (indicator) { indicator.textContent = `Failed: ${err.message}`; indicator.style.color = 'var(--red-500, #ef4444)'; }
    btns.forEach(b => { b.disabled = false; b.style.opacity = '1'; });
  }
}

async function toolsAction(action) {
  if (!currentDeploymentId) return;
  const ep = action === 'stop-tools' ? 'stop-tools' : action === 'restart' ? 'restart-tools' : 'build-tools';
  try { await fetch(`${API}/deployments/${currentDeploymentId}/${ep}`, { method: 'POST' }); } catch {}
}

async function deleteDeployment(id) {
  try { await fetch(`${API}/deployments/${id}`, { method: 'DELETE' }); showView('deployments'); }
  catch (err) { console.error('Failed to remove:', err.message); }
}

function startDetailPolling() {
  if (detailPollInterval) clearInterval(detailPollInterval);
  fetchDetailStatus();
  detailPollInterval = setInterval(fetchDetailStatus, 10000);
}

async function fetchDetailStatus() {
  if (!currentDeploymentId || !detailDeployment?.docker_project) return;
  try {
    const [sRes, mRes] = await Promise.all([
      fetch(`${API}/deployments/${currentDeploymentId}/status`),
      fetch(`${API}/deployments/${currentDeploymentId}/monitoring`),
    ]);
    if (sRes.ok) detailStatus = await sRes.json();
    if (mRes.ok) detailMonitoring = await mRes.json();
    if (detailTab === 'overview') renderOverviewTab();
  } catch {}
}

// ============================================================
// Logs Tab
// ============================================================
function renderLogsTab() {
  const panel = document.getElementById('tab-logs');
  if (!detailDeployment?.docker_project) {
    panel.innerHTML = '<div class="card"><p style="color:var(--gray-500)">Deploy your L2 first to see logs.</p></div>';
    return;
  }
  if (!panel.querySelector('.log-controls')) {
    panel.innerHTML = `<div class="card">
      <h3 style="margin-bottom:16px">Logs</h3>
      <div class="log-controls">
        <select id="log-service" onchange="reloadLogs()">
          <option value="">All Services</option>
          <option value="tokamak-app-l1">L1 Node</option>
          <option value="tokamak-app-l2">L2 Node</option>
          <option value="tokamak-app-prover">Prover</option>
          <option value="tokamak-app-deployer">Deployer</option>
          <option value="bridge-ui">Bridge UI</option>
          <option value="backend-l1">Explorer L1</option>
          <option value="backend-l2">Explorer L2</option>
        </select>
        <input type="text" id="log-search" placeholder="Search logs..." oninput="filterLogs()">
        <button class="stream-btn inactive" id="stream-btn" onclick="toggleStream()">Stream</button>
        <label class="checkbox-label"><input type="checkbox" id="auto-scroll" checked> Auto-scroll</label>
      </div>
      <div id="log-viewer" class="log-container" style="height:400px"></div>
      <div id="log-line-count" class="log-count"></div>
    </div>`;
  }
  reloadLogs();
}

async function reloadLogs() {
  if (!currentDeploymentId) return;
  const service = document.getElementById('log-service')?.value || '';
  try {
    const res = await fetch(`${API}/deployments/${currentDeploymentId}/logs?service=${service}&tail=200`);
    const data = await res.json();
    allLogLines = data.logs ? data.logs.split('\n').filter(Boolean) : [];
    renderLogLines();
  } catch {}
}

function filterLogs() { renderLogLines(); }

function renderLogLines() {
  const search = (document.getElementById('log-search')?.value || '').toLowerCase();
  const filtered = search ? allLogLines.filter(l => l.toLowerCase().includes(search)) : allLogLines;
  const viewer = document.getElementById('log-viewer');
  if (!viewer) return;

  if (filtered.length === 0) {
    viewer.innerHTML = `<div style="text-align:center;padding:40px;color:var(--gray-500)">${allLogLines.length === 0 ? 'No logs available' : 'No matching lines'}</div>`;
  } else {
    viewer.innerHTML = filtered.map(l => {
      if (search) {
        const re = new RegExp(`(${search.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')})`, 'gi');
        return `<div class="log-line">${l.replace(re, '<mark style="background:#fde047;color:black">$1</mark>')}</div>`;
      }
      return `<div class="log-line">${esc(l)}</div>`;
    }).join('');
  }

  const count = document.getElementById('log-line-count');
  if (count) count.textContent = `${filtered.length} / ${allLogLines.length} lines`;
  if (document.getElementById('auto-scroll')?.checked && viewer) viewer.scrollTop = viewer.scrollHeight;
}

function toggleStream() {
  const btn = document.getElementById('stream-btn');
  if (logEventSource) {
    logEventSource.close(); logEventSource = null;
    btn.textContent = 'Stream'; btn.className = 'stream-btn inactive';
    return;
  }
  const service = document.getElementById('log-service')?.value || '';
  const params = new URLSearchParams({ follow: 'true' });
  if (service) params.set('service', service);

  logEventSource = new EventSource(`${API}/deployments/${currentDeploymentId}/logs?${params}`);
  btn.textContent = 'Stop'; btn.className = 'stream-btn active';

  logEventSource.onmessage = (e) => {
    try {
      const data = JSON.parse(e.data);
      if (data.line) { allLogLines.push(data.line); if (allLogLines.length > 2000) allLogLines = allLogLines.slice(-2000); renderLogLines(); }
    } catch {}
  };
  logEventSource.onerror = () => { logEventSource.close(); logEventSource = null; btn.textContent = 'Stream'; btn.className = 'stream-btn inactive'; };
}

// ============================================================
// Config Tab
// ============================================================
function renderConfigTab() {
  const d = detailDeployment;
  if (!d) return;
  const slug = d.program_slug || d.program_id;
  const toml = `# Guest Program Registry Configuration\n# Generated by Tokamak for: ${d.name}\n\ndefault_program = "${slug}"\nenabled_programs = ["${slug}"]`;

  document.getElementById('tab-config').innerHTML = `
    <div class="card" style="margin-bottom:24px">
      <h3 style="margin-bottom:12px">App Configuration</h3>
      <dl class="info-grid">
        <dt>Guest Program</dt><dd>${esc(slug)}</dd>
        <dt>Program Name</dt><dd>${esc(d.program_name || '')}</dd>
      </dl>
    </div>
    <div class="card" style="margin-bottom:24px">
      <h3 style="margin-bottom:12px">Configuration Files</h3>
      <p style="font-size:14px;color:var(--gray-500);margin-bottom:16px">Download configuration files to run an ethrex L2 node with this guest program.</p>
      <button class="btn-secondary" onclick="downloadToml()">Download programs.toml</button>
      <div style="margin-top:16px">
        <p style="font-size:12px;font-weight:500;color:var(--gray-500);margin-bottom:8px">programs.toml</p>
        <pre class="config-pre">${esc(toml)}</pre>
      </div>
    </div>
    <div class="card">
      <h3 style="margin-bottom:12px">Manual Setup</h3>
      <div style="background:var(--gray-50);border-radius:8px;padding:16px;font-size:14px">
        <div style="margin-bottom:12px"><p style="font-weight:500;color:var(--gray-700);margin-bottom:4px">1. Clone ethrex</p><pre class="config-pre">git clone https://github.com/tokamak-network/ethrex.git\ncd ethrex</pre></div>
        <div style="margin-bottom:12px"><p style="font-weight:500;color:var(--gray-700);margin-bottom:4px">2. Run with guest program</p><pre class="config-pre">make -C crates/l2 init-guest-program PROGRAM=${esc(slug)}</pre></div>
        <div style="margin-bottom:12px"><p style="font-weight:500;color:var(--gray-700);margin-bottom:4px">3. Endpoints</p><div style="color:var(--gray-500)"><p>L1 RPC: <code style="background:var(--gray-200);padding:2px 6px;border-radius:4px;font-size:12px">http://localhost:8545</code></p><p>L2 RPC: <code style="background:var(--gray-200);padding:2px 6px;border-radius:4px;font-size:12px">http://localhost:1729</code></p></div></div>
        <div><p style="font-weight:500;color:var(--gray-700);margin-bottom:4px">4. Stop</p><pre class="config-pre">make -C crates/l2 down-guest-program</pre></div>
      </div>
    </div>`;
}

function downloadToml() {
  const d = detailDeployment; if (!d) return;
  const slug = d.program_slug || d.program_id;
  const blob = new Blob([`default_program = "${slug}"\nenabled_programs = ["${slug}"]\n`], { type: 'application/toml' });
  const a = document.createElement('a'); a.href = URL.createObjectURL(blob); a.download = 'programs.toml'; a.click();
}

// ============================================================
// Remote Hosts
// ============================================================
async function loadHosts() {
  try {
    const res = await fetch(`${API}/hosts`);
    const data = await res.json();
    const list = data.hosts || data || [];
    const container = document.getElementById('hosts-list');
    if (list.length === 0) { container.innerHTML = '<p class="empty-state">No remote hosts configured</p>'; return; }
    container.innerHTML = list.map(h => `
      <div class="host-card">
        <h3>${esc(h.name)}</h3>
        <div class="meta">${esc(h.username)}@${esc(h.hostname)}:${h.port || 22}</div>
        <div class="actions">
          <button class="btn-secondary" onclick="testHost('${h.id}')">Test</button>
          <button class="btn-danger" onclick="removeHost('${h.id}')">Remove</button>
        </div>
      </div>
    `).join('');
  } catch { document.getElementById('hosts-list').innerHTML = '<p class="empty-state">Failed to load hosts</p>'; }
}

document.getElementById('host-form')?.addEventListener('submit', async (e) => {
  e.preventDefault();
  const fd = new FormData(e.target);
  try {
    await fetch(`${API}/hosts`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: fd.get('name'), hostname: fd.get('hostname'), username: fd.get('username'), port: parseInt(fd.get('port')) || 22, privateKey: fd.get('privateKey') }),
    });
    e.target.reset(); loadHosts();
  } catch {}
});

async function testHost(id) {
  try { const r = await fetch(`${API}/hosts/${id}/test`, { method: 'POST' }); const d = await r.json(); alert(d.success ? 'Connection successful!' : `Failed: ${d.error}`); }
  catch { alert('Test failed'); }
}

async function removeHost(id) {
  if (!confirm('Remove this host?')) return;
  try { await fetch(`${API}/hosts/${id}`, { method: 'DELETE' }); loadHosts(); } catch {}
}

// ============================================================
// Settings Edit
// ============================================================
function toggleSettingsEdit() {
  const display = document.getElementById('settings-display');
  const edit = document.getElementById('settings-edit');
  if (!display || !edit) return;
  const showing = edit.style.display !== 'none';
  display.style.display = showing ? 'block' : 'none';
  edit.style.display = showing ? 'none' : 'block';
}

async function saveSettings() {
  if (!currentDeploymentId || !detailDeployment) return;
  const name = document.getElementById('edit-name')?.value?.trim();
  if (!name) return;
  try {
    const body = {
      name,
      chain_id: parseInt(document.getElementById('edit-chain-id')?.value) || null,
      rpc_url: document.getElementById('edit-rpc-url')?.value?.trim() || null,
    };
    const res = await fetch(`${API}/deployments/${currentDeploymentId}`, {
      method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
    });
    if (res.ok) {
      const data = await res.json();
      detailDeployment = data.deployment || data;
      renderDetail();
    }
  } catch {}
}

// ============================================================
// Directory Picker
// ============================================================
async function browseDirPicker() {
  // Simple prompt-based picker (directory browser via /api/fs/browse)
  const current = document.getElementById('launch-deploy-dir')?.value || '';
  try {
    const res = await fetch(`${API}/fs/browse${current ? '?path=' + encodeURIComponent(current) : ''}`);
    if (!res.ok) { alert('Directory browser not available'); return; }
    const data = await res.json();
    const dirs = data.dirs || [];
    const dirList = dirs.map(d => d.name).join('\n');
    const selected = prompt(`Current: ${data.current}\n\nSubdirectories:\n${dirList}\n\nEnter path:`, data.current);
    if (selected) document.getElementById('launch-deploy-dir').value = selected;
  } catch {
    // Fallback: simple prompt
    const path = prompt('Enter deploy directory path:', current);
    if (path) document.getElementById('launch-deploy-dir').value = path;
  }
}

// ============================================================
// Utilities
// ============================================================
function esc(str) {
  const div = document.createElement('div');
  div.textContent = str || '';
  return div.innerHTML;
}

// ============================================================
// Init
// ============================================================
checkHealth();
setInterval(checkHealth, 15000);
loadDeployments();
// Show launch button in header for deployments view
document.getElementById('header-launch-btn').style.display = '';
