const API = '/api';
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

// ============================================================
// Navigation
// ============================================================

document.querySelectorAll('.nav-btn').forEach(btn => {
  btn.addEventListener('click', () => showView(btn.dataset.view));
});

function showView(name) {
  document.querySelectorAll('.view').forEach(v => v.classList.remove('active'));
  document.querySelectorAll('.nav-btn').forEach(b => b.classList.remove('active'));
  const view = document.getElementById(`view-${name}`);
  if (view) view.classList.add('active');
  const navBtn = document.querySelector(`[data-view="${name}"]`);
  if (navBtn) navBtn.classList.add('active');

  if (name === 'deployments') loadDeployments();
  if (name === 'hosts') loadHosts();
  if (name === 'launch') { loadPrograms(); launchGoStep(1); }
}

// ============================================================
// API helpers
// ============================================================

async function api(path, options = {}) {
  const resp = await fetch(`${API}${path}`, {
    headers: { 'Content-Type': 'application/json' },
    ...options,
  });
  if (!resp.ok) {
    const err = await resp.json().catch(() => ({ error: resp.statusText }));
    throw new Error(err.error || resp.statusText);
  }
  return resp.json();
}

// ============================================================
// Health check
// ============================================================

async function checkHealth() {
  try {
    await api('/health');
    document.getElementById('server-status').textContent = 'connected';
    document.getElementById('server-status').className = 'status-badge ok';
  } catch {
    document.getElementById('server-status').textContent = 'disconnected';
    document.getElementById('server-status').className = 'status-badge';
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
  { phase: 'starting_tools', label: 'Starting Tools' },
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

function renderStepIndicator() {
  const el = document.getElementById('step-indicator');
  const steps = [
    { num: 1, label: 'Select App', complete: launchStep > 1 },
    { num: 2, label: 'Configure', complete: launchStep > 2 },
    { num: 3, label: 'Deploy', complete: false },
  ];
  el.innerHTML = steps.map((s, i) => `
    ${i > 0 ? '<div class="step-line"></div>' : ''}
    <div class="step-item ${launchStep === s.num ? 'active' : ''} ${s.complete ? 'complete' : ''}"
         onclick="${s.num < launchStep ? `launchGoStep(${s.num})` : ''}">
      <span class="step-num">${s.complete ? '&#10003;' : s.num}</span>
      <span class="step-label">${s.label}</span>
    </div>
  `).join('');
}

function launchGoStep(step) {
  launchStep = step;
  document.getElementById('launch-step1').style.display = step === 1 ? '' : 'none';
  document.getElementById('launch-step2').style.display = step === 2 ? '' : 'none';
  document.getElementById('launch-step3').style.display = step === 3 ? '' : 'none';
  renderStepIndicator();
}

async function loadPrograms() {
  const grid = document.getElementById('programs-grid');
  if (programs.length > 0) { renderPrograms(programs); return; }
  try {
    const data = await api('/store/programs');
    programs = data.programs || data;
    renderPrograms(programs);
  } catch {
    // Fallback: default programs if Platform not available
    programs = [
      { id: 'default-evm', program_id: 'evm-l2', name: 'EVM L2', description: 'Standard EVM Layer 2 chain', category: 'core', is_official: true, use_count: 0 },
      { id: 'default-zk-dex', program_id: 'zk-dex', name: 'ZK-DEX', description: 'ZK-powered decentralized exchange', category: 'defi', is_official: true, use_count: 0 },
    ];
    renderPrograms(programs);
  }
}

function renderPrograms(list) {
  const grid = document.getElementById('programs-grid');
  const search = (document.getElementById('program-search').value || '').toLowerCase();
  const filtered = list.filter(p =>
    !search || p.name.toLowerCase().includes(search) || p.program_id.toLowerCase().includes(search)
  );
  if (filtered.length === 0) {
    grid.innerHTML = '<p class="empty-state">No programs found.</p>';
    return;
  }
  grid.innerHTML = filtered.map(p => `
    <div class="deploy-card program-card" onclick="selectProgram('${esc(p.id)}')">
      <div class="program-header">
        <div class="program-icon">${esc(p.name.charAt(0).toUpperCase())}</div>
        <div>
          <h3>${esc(p.name)}</h3>
          <span class="meta">${esc(p.program_id)}</span>
        </div>
      </div>
      <p class="program-desc">${esc(p.description || 'No description')}</p>
      <div class="meta">
        <span class="tag">${esc(p.category)}</span>
        ${p.is_official ? '<span class="tag official">Official</span>' : ''}
        <span>${p.use_count || 0} deployments</span>
      </div>
    </div>
  `).join('');
}

function filterPrograms() { renderPrograms(programs); }

function selectProgram(id) {
  selectedProgram = programs.find(p => p.id === id);
  if (!selectedProgram) return;
  document.getElementById('launch-name').value = `${selectedProgram.name} L2`;
  document.getElementById('launch-chain-id').value = Math.floor(Math.random() * 90000) + 10000;
  renderSelectedProgram();
  checkDockerStatus();
  launchGoStep(2);
}

function renderSelectedProgram() {
  const el = document.getElementById('selected-program-info');
  if (!selectedProgram) return;
  el.innerHTML = `
    <div class="program-header">
      <div class="program-icon">${esc(selectedProgram.name.charAt(0).toUpperCase())}</div>
      <div>
        <h3>${esc(selectedProgram.name)}</h3>
        <span class="meta">${esc(selectedProgram.program_id)}</span>
      </div>
      <button class="btn-action" onclick="launchGoStep(1)" style="margin-left:auto">Change</button>
    </div>
  `;
}

function setLaunchMode(mode) {
  launchMode = mode;
  document.querySelectorAll('.mode-btn').forEach(b => {
    b.classList.toggle('active', b.dataset.mode === mode);
  });
  document.getElementById('remote-host-area').style.display = mode === 'remote' ? '' : 'none';
  if (mode === 'local') checkDockerStatus();
  if (mode === 'remote') loadLaunchHosts();
}

async function checkDockerStatus() {
  const area = document.getElementById('docker-status-area');
  area.innerHTML = '<div class="info-box">Checking Docker status...</div>';
  try {
    const data = await api('/deployments/docker-status');
    if (data.available) {
      area.innerHTML = '<div class="success-box">Docker is running</div>';
    } else {
      area.innerHTML = '<div class="error-box">Docker is not running. <a href="https://www.docker.com/products/docker-desktop/" target="_blank">Download Docker Desktop</a></div>';
    }
  } catch {
    area.innerHTML = '<div class="info-box">Could not check Docker status</div>';
  }
}

async function loadLaunchHosts() {
  const select = document.getElementById('launch-host-select');
  try {
    const { hosts } = await api('/hosts');
    select.innerHTML = '<option value="">Select a server...</option>' +
      hosts.map(h => `<option value="${h.id}">${esc(h.name)} (${esc(h.username)}@${esc(h.hostname)}:${h.port})</option>`).join('');
  } catch {
    select.innerHTML = '<option value="">No servers available</option>';
  }
}

async function handleLaunchDeploy() {
  const name = document.getElementById('launch-name').value.trim();
  const chainId = document.getElementById('launch-chain-id').value;
  const deployDir = document.getElementById('launch-deploy-dir').value.trim();
  const errEl = document.getElementById('launch-error');

  if (!name) { errEl.textContent = 'L2 name is required'; errEl.style.display = ''; return; }
  if (launchMode === 'remote') {
    const hostId = document.getElementById('launch-host-select').value;
    if (!hostId) { errEl.textContent = 'Please select a remote server'; errEl.style.display = ''; return; }
  }
  errEl.style.display = 'none';
  document.getElementById('launch-deploy-btn').disabled = true;

  try {
    // Create deployment
    const { deployment } = await api('/deployments', {
      method: 'POST',
      body: JSON.stringify({
        name,
        programSlug: selectedProgram.program_id,
        chainId: chainId ? parseInt(chainId) : undefined,
        config: { mode: launchMode, deployDir: deployDir || undefined },
      }),
    });

    launchDeploymentId = deployment.id;
    buildLogLines = [];

    // Show step 3
    document.getElementById('deploy-info-text').textContent =
      `Deploying "${name}" powered by ${selectedProgram.name}...`;
    renderDeployProgress('configured');
    launchGoStep(3);

    // Start provisioning
    if (launchMode === 'remote') {
      const hostId = document.getElementById('launch-host-select').value;
      await api(`/deployments/${deployment.id}/provision`, {
        method: 'POST',
        body: JSON.stringify({ hostId }),
      });
    } else {
      await api(`/deployments/${deployment.id}/provision`, { method: 'POST' });
    }

    // Connect to SSE events
    connectDeployEvents(deployment.id);
  } catch (e) {
    errEl.textContent = e.message;
    errEl.style.display = '';
    document.getElementById('launch-deploy-btn').disabled = false;
  }
}

function connectDeployEvents(id) {
  if (deployEventSource) deployEventSource.close();
  deployEventSource = new EventSource(`${API}/deployments/${id}/events`);
  deployEventSource.onmessage = (e) => {
    try {
      const data = JSON.parse(e.data);
      if (data.event === 'log') {
        buildLogLines.push(data.message || '');
        if (buildLogLines.length > 300) buildLogLines = buildLogLines.slice(-300);
        renderBuildLog();
        return;
      }
      if (data.phase) renderDeployProgress(data.phase);
      if (data.message) {
        const msgEl = document.getElementById('deploy-message');
        msgEl.textContent = data.message;
        msgEl.style.display = '';
      }
      if (data.event === 'error') {
        const errEl = document.getElementById('deploy-error-msg');
        errEl.textContent = 'Error: ' + (data.message || 'Deployment failed');
        errEl.style.display = '';
        deployEventSource.close();
      }
      if (data.phase === 'running') {
        const completeEl = document.getElementById('deploy-complete');
        let html = '<strong>Deployment is running!</strong>';
        if (data.l1Rpc) html += `<br>L1 RPC: <code>${esc(data.l1Rpc)}</code>`;
        if (data.l2Rpc) html += `<br>L2 RPC: <code>${esc(data.l2Rpc)}</code>`;
        if (data.bridgeAddress) html += `<br>Bridge: <code>${esc(data.bridgeAddress)}</code>`;
        completeEl.innerHTML = html;
        completeEl.style.display = '';
        document.getElementById('goto-dashboard-btn').style.display = '';
        document.getElementById('deploy-message').style.display = 'none';
        deployEventSource.close();
      }
    } catch {}
  };
}

function renderDeployProgress(currentPhase) {
  const steps = launchMode === 'remote' ? REMOTE_STEPS : LOCAL_STEPS;
  const currentIdx = steps.findIndex(s => s.phase === currentPhase);
  const el = document.getElementById('deploy-progress-steps');
  el.innerHTML = steps.map((step, i) => {
    const isComplete = i < currentIdx || currentPhase === 'running';
    const isCurrent = step.phase === currentPhase;
    let cls = 'pending';
    if (isComplete) cls = 'complete';
    else if (isCurrent) cls = 'current';
    return `
      <div class="progress-step ${cls}">
        <div class="progress-icon">
          ${isComplete ? '&#10003;' : isCurrent ? '<span class="spinner"></span>' : (i + 1)}
        </div>
        <span class="progress-label">${step.label}</span>
      </div>
    `;
  }).join('');
}

function renderBuildLog() {
  const container = document.getElementById('build-log');
  const countEl = document.getElementById('build-log-count');
  countEl.textContent = buildLogLines.length;
  container.innerHTML = buildLogLines.map(l => `<div class="log-line">${esc(l)}</div>`).join('');
  container.scrollTop = container.scrollHeight;
}

function goToDashboard() {
  if (launchDeploymentId) showDeployment(launchDeploymentId);
}

// ============================================================
// Deployments
// ============================================================

async function loadDeployments() {
  const container = document.getElementById('deployments-list');
  try {
    const { deployments } = await api('/deployments');
    if (deployments.length === 0) {
      container.innerHTML = '<p class="empty-state">No deployments yet. <a href="#" onclick="showView(\'launch\');return false">Launch your first L2</a></p>';
      return;
    }
    container.innerHTML = deployments.map(d => `
      <div class="deploy-card" onclick="showDeployment('${d.id}')">
        <h3>${esc(d.name)}</h3>
        <span class="phase-badge phase-${d.phase}">${d.phase}</span>
        <div class="meta">
          <span>Program: ${esc(d.program_slug)}</span>
          ${d.l2_port ? `<span>L2: :${d.l2_port}</span>` : ''}
          ${d.chain_id ? `<span>Chain: ${d.chain_id}</span>` : ''}
        </div>
      </div>
    `).join('');
  } catch (e) {
    container.innerHTML = `<p class="empty-state">Error: ${esc(e.message)}</p>`;
  }
}

async function showDeployment(id) {
  currentDeploymentId = id;
  try {
    const { deployment: d } = await api(`/deployments/${id}`);
    document.getElementById('detail-name').textContent = d.name;
    const phaseBadge = document.getElementById('detail-phase');
    phaseBadge.textContent = d.phase;
    phaseBadge.className = `phase-badge phase-${d.phase}`;

    // Container status cards
    renderContainerCards(d);

    // Info
    const dl = document.getElementById('detail-info-dl');
    dl.innerHTML = `
      <dt>ID</dt><dd>${d.id}</dd>
      <dt>Program</dt><dd>${d.program_slug}</dd>
      <dt>Status</dt><dd>${d.status}</dd>
      ${d.chain_id ? `<dt>Chain ID</dt><dd>${d.chain_id}</dd>` : ''}
      ${d.l1_port ? `<dt>L1 RPC</dt><dd>http://127.0.0.1:${d.l1_port}</dd>` : ''}
      ${d.l2_port ? `<dt>L2 RPC</dt><dd>http://127.0.0.1:${d.l2_port}</dd>` : ''}
      ${d.bridge_address ? `<dt>Bridge</dt><dd>${d.bridge_address}</dd>` : ''}
      ${d.proposer_address ? `<dt>Proposer</dt><dd>${d.proposer_address}</dd>` : ''}
      ${d.error_message ? `<dt>Error</dt><dd style="color:var(--error)">${esc(d.error_message)}</dd>` : ''}
    `;

    // Endpoints & Tools
    renderEndpoints(d);

    // Actions
    renderActions(d);

    // Config tab
    renderConfig(d);

    showView('detail');
    switchTab('overview');
    connectEvents(id);
  } catch (e) {
    alert('Failed to load deployment: ' + e.message);
  }
}

function renderContainerCards(d) {
  const el = document.getElementById('container-cards');
  if (d.phase === 'configured') { el.innerHTML = ''; return; }
  const services = [
    { key: 'l1', label: 'L1 Node' },
    { key: 'l2', label: 'L2 Node' },
    { key: 'prover', label: 'Prover' },
    { key: 'deployer', label: 'Deployer' },
  ];
  el.innerHTML = `<div class="container-grid">${services.map(s => {
    const running = d.phase === 'running';
    return `<div class="container-card ${running ? 'running' : ''}">
      <div class="container-label">${s.label}</div>
      <div class="container-status">${running ? 'running' : d.phase}</div>
    </div>`;
  }).join('')}</div>`;
}

function renderEndpoints(d) {
  const el = document.getElementById('detail-endpoints');
  const content = document.getElementById('endpoints-content');
  if (d.phase !== 'running') { el.style.display = 'none'; return; }
  el.style.display = '';
  let html = '';
  if (d.l1_port) html += `<div class="endpoint-item"><span>L1 RPC</span><a href="http://127.0.0.1:${d.l1_port}" target="_blank">:${d.l1_port}</a></div>`;
  if (d.l2_port) html += `<div class="endpoint-item"><span>L2 RPC</span><a href="http://127.0.0.1:${d.l2_port}" target="_blank">:${d.l2_port}</a></div>`;
  if (d.tools_l1_explorer_port) html += `<div class="endpoint-item"><span>L1 Explorer</span><a href="http://127.0.0.1:${d.tools_l1_explorer_port}" target="_blank">:${d.tools_l1_explorer_port}</a></div>`;
  if (d.tools_l2_explorer_port) html += `<div class="endpoint-item"><span>L2 Explorer</span><a href="http://127.0.0.1:${d.tools_l2_explorer_port}" target="_blank">:${d.tools_l2_explorer_port}</a></div>`;
  if (d.tools_bridge_ui_port) html += `<div class="endpoint-item"><span>Bridge UI</span><a href="http://127.0.0.1:${d.tools_bridge_ui_port}" target="_blank">:${d.tools_bridge_ui_port}</a></div>`;
  content.innerHTML = html || '<span class="meta">No endpoints available</span>';
}

function renderActions(d) {
  const container = document.getElementById('detail-actions');
  const buttons = [];
  if (d.phase === 'configured') {
    buttons.push(`<button class="btn-primary" onclick="doAction('${d.id}', 'provision')">Deploy (Docker)</button>`);
  }
  if (d.phase === 'running') {
    buttons.push(`<button class="btn-action" onclick="doAction('${d.id}', 'stop')">Stop</button>`);
  }
  if (d.phase === 'stopped') {
    buttons.push(`<button class="btn-primary" onclick="doAction('${d.id}', 'start')">Start</button>`);
    buttons.push(`<button class="btn-danger" onclick="doAction('${d.id}', 'destroy')">Destroy</button>`);
  }
  if (d.phase === 'error') {
    buttons.push(`<button class="btn-danger" onclick="doAction('${d.id}', 'destroy')">Clean Up</button>`);
  }
  buttons.push(`<button class="btn-danger" onclick="deleteDeployment('${d.id}')">Delete Record</button>`);
  container.innerHTML = buttons.join('');
}

function renderConfig(d) {
  const el = document.getElementById('config-content');
  let config = {};
  try { config = d.config ? JSON.parse(d.config) : {}; } catch {}
  el.innerHTML = `
    <h3>Deployment Configuration</h3>
    <dl>
      <dt>Mode</dt><dd>${config.mode || 'local'}</dd>
      <dt>Program</dt><dd>${d.program_slug || 'evm-l2'}</dd>
      <dt>Docker Project</dt><dd>${d.docker_project || 'N/A'}</dd>
      ${config.deployDir ? `<dt>Deploy Directory</dt><dd>${esc(config.deployDir)}</dd>` : ''}
    </dl>
  `;
}

async function doAction(id, action) {
  try {
    await api(`/deployments/${id}/${action}`, { method: 'POST' });
    showDeployment(id);
  } catch (e) {
    alert('Error: ' + e.message);
  }
}

async function deleteDeployment(id) {
  if (!confirm('Delete this deployment record?')) return;
  try {
    await api(`/deployments/${id}`, { method: 'DELETE' });
    disconnectEvents();
    showView('deployments');
  } catch (e) {
    alert('Failed: ' + e.message);
  }
}

// ============================================================
// Tabs (Detail view)
// ============================================================

function switchTab(name) {
  document.querySelectorAll('.tab-btn').forEach(b => b.classList.toggle('active', b.dataset.tab === name));
  document.querySelectorAll('.tab-panel').forEach(p => p.style.display = 'none');
  const panel = document.getElementById(`tab-${name}`);
  if (panel) panel.style.display = '';
  if (name === 'logs') reloadLogs();
}

// ============================================================
// Log Viewer
// ============================================================

async function reloadLogs() {
  if (!currentDeploymentId) return;
  const service = document.getElementById('log-service').value;
  const viewer = document.getElementById('log-viewer');
  allLogLines = [];
  try {
    const resp = await fetch(`${API}/deployments/${currentDeploymentId}/logs?service=${service}&tail=200`);
    const data = await resp.json();
    if (data.logs) {
      allLogLines = data.logs.split('\n').filter(Boolean);
    }
    renderLogs();
  } catch {}
}

function renderLogs() {
  const viewer = document.getElementById('log-viewer');
  const search = (document.getElementById('log-search').value || '').toLowerCase();
  const filtered = search ? allLogLines.filter(l => l.toLowerCase().includes(search)) : allLogLines;
  viewer.innerHTML = filtered.map(l => `<div class="log-line">${esc(l)}</div>`).join('');
  document.getElementById('log-line-count').textContent = `${filtered.length} / ${allLogLines.length} lines`;
  if (document.getElementById('auto-scroll').checked) {
    viewer.scrollTop = viewer.scrollHeight;
  }
}

function filterLogs() { renderLogs(); }

function toggleStream() {
  const btn = document.getElementById('stream-btn');
  if (logEventSource) {
    logEventSource.close();
    logEventSource = null;
    btn.textContent = 'Stream';
    btn.className = 'btn-action';
    return;
  }
  const service = document.getElementById('log-service').value;
  const url = `${API}/deployments/${currentDeploymentId}/logs?service=${service}&follow=true`;
  logEventSource = new EventSource(url);
  logEventSource.onmessage = (e) => {
    try {
      const data = JSON.parse(e.data);
      if (data.line) {
        allLogLines.push(data.line);
        if (allLogLines.length > 3000) allLogLines = allLogLines.slice(-3000);
        renderLogs();
      }
    } catch {}
  };
  logEventSource.onerror = () => {
    logEventSource.close();
    logEventSource = null;
    btn.textContent = 'Stream';
    btn.className = 'btn-action';
  };
  btn.textContent = 'Stop';
  btn.className = 'btn-danger';
}

// ============================================================
// SSE Events (Detail view)
// ============================================================

function connectEvents(id) {
  disconnectEvents();
  eventSource = new EventSource(`${API}/deployments/${id}/events`);
  eventSource.onmessage = (e) => {
    try {
      const data = JSON.parse(e.data);
      if (data.phase) {
        const phaseBadge = document.getElementById('detail-phase');
        phaseBadge.textContent = data.phase;
        phaseBadge.className = `phase-badge phase-${data.phase}`;
        if (data.phase === 'running' || data.phase === 'error') {
          setTimeout(() => showDeployment(id), 500);
        }
      }
    } catch {}
  };
}

function disconnectEvents() {
  if (eventSource) { eventSource.close(); eventSource = null; }
  if (logEventSource) { logEventSource.close(); logEventSource = null; }
}

// ============================================================
// Hosts
// ============================================================

async function loadHosts() {
  const container = document.getElementById('hosts-list');
  try {
    const { hosts } = await api('/hosts');
    if (hosts.length === 0) {
      container.innerHTML = '<p class="empty-state">No remote hosts configured.</p>';
      return;
    }
    container.innerHTML = hosts.map(h => `
      <div class="host-card">
        <h3>${esc(h.name)}</h3>
        <div class="meta">${esc(h.username)}@${esc(h.hostname)}:${h.port} (${h.status})</div>
        <div class="actions">
          <button class="btn-action" onclick="testHost('${h.id}')">Test</button>
          <button class="btn-danger" onclick="deleteHost('${h.id}')">Delete</button>
        </div>
      </div>
    `).join('');
  } catch (e) {
    container.innerHTML = `<p class="empty-state">Error: ${esc(e.message)}</p>`;
  }
}

document.getElementById('host-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const fd = new FormData(e.target);
  try {
    await api('/hosts', {
      method: 'POST',
      body: JSON.stringify({
        name: fd.get('name'),
        hostname: fd.get('hostname'),
        username: fd.get('username'),
        port: parseInt(fd.get('port')) || 22,
        authMethod: 'key',
        privateKey: fd.get('privateKey') || undefined,
      }),
    });
    e.target.reset();
    loadHosts();
  } catch (err) {
    alert('Failed: ' + err.message);
  }
});

async function testHost(id) {
  try {
    const result = await api(`/hosts/${id}/test`, { method: 'POST' });
    alert(result.ok ? `SSH OK, Docker: ${result.docker ? 'available' : 'not found'}` : 'Connection failed');
    loadHosts();
  } catch (e) {
    alert('Test failed: ' + e.message);
  }
}

async function deleteHost(id) {
  if (!confirm('Delete this host?')) return;
  try {
    await api(`/hosts/${id}`, { method: 'DELETE' });
    loadHosts();
  } catch (e) {
    alert('Failed: ' + e.message);
  }
}

// ============================================================
// Utils
// ============================================================

function esc(str) {
  if (!str) return '';
  const div = document.createElement('div');
  div.textContent = String(str);
  return div.innerHTML;
}

// ============================================================
// Init
// ============================================================

checkHealth();
loadDeployments();
setInterval(checkHealth, 10000);
