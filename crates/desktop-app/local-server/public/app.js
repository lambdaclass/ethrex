const API = '/api';
let currentDeploymentId = null;
let eventSource = null;

// ============================================================
// Navigation
// ============================================================

document.querySelectorAll('.nav-btn').forEach(btn => {
  btn.addEventListener('click', () => showView(btn.dataset.view));
});

function showView(name) {
  document.querySelectorAll('.view').forEach(v => v.classList.remove('active'));
  document.querySelectorAll('.nav-btn').forEach(b => b.classList.remove('active'));
  document.getElementById(`view-${name}`).classList.add('active');
  const navBtn = document.querySelector(`[data-view="${name}"]`);
  if (navBtn) navBtn.classList.add('active');

  if (name === 'deployments') loadDeployments();
  if (name === 'hosts') loadHosts();
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
// Deployments
// ============================================================

async function loadDeployments() {
  const container = document.getElementById('deployments-list');
  try {
    const { deployments } = await api('/deployments');
    if (deployments.length === 0) {
      container.innerHTML = '<p class="empty-state">No deployments yet. Create one to get started.</p>';
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

    const dl = document.getElementById('detail-info-dl');
    dl.innerHTML = `
      <dt>ID</dt><dd>${d.id}</dd>
      <dt>Program</dt><dd>${d.program_slug}</dd>
      <dt>Status</dt><dd>${d.status}</dd>
      <dt>Deploy Method</dt><dd>${d.deploy_method || 'docker'}</dd>
      ${d.chain_id ? `<dt>Chain ID</dt><dd>${d.chain_id}</dd>` : ''}
      ${d.l1_port ? `<dt>L1 RPC</dt><dd>http://127.0.0.1:${d.l1_port}</dd>` : ''}
      ${d.l2_port ? `<dt>L2 RPC</dt><dd>http://127.0.0.1:${d.l2_port}</dd>` : ''}
      ${d.bridge_address ? `<dt>Bridge</dt><dd>${d.bridge_address}</dd>` : ''}
      ${d.proposer_address ? `<dt>Proposer</dt><dd>${d.proposer_address}</dd>` : ''}
      ${d.docker_project ? `<dt>Docker Project</dt><dd>${d.docker_project}</dd>` : ''}
      ${d.error_message ? `<dt>Error</dt><dd style="color:var(--error)">${esc(d.error_message)}</dd>` : ''}
      ${d.tools_l2_explorer_port ? `<dt>L2 Explorer</dt><dd><a href="http://127.0.0.1:${d.tools_l2_explorer_port}" target="_blank" style="color:var(--primary)">:${d.tools_l2_explorer_port}</a></dd>` : ''}
      ${d.tools_bridge_ui_port ? `<dt>Bridge UI</dt><dd><a href="http://127.0.0.1:${d.tools_bridge_ui_port}" target="_blank" style="color:var(--primary)">:${d.tools_bridge_ui_port}</a></dd>` : ''}
    `;

    renderActions(d);
    showView('detail');
    connectEvents(id);
  } catch (e) {
    alert('Failed to load deployment: ' + e.message);
  }
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

async function doAction(id, action) {
  try {
    const { deployment } = await api(`/deployments/${id}/${action}`, { method: 'POST' });
    showDeployment(id);
  } catch (e) {
    addLogLine(`Error: ${e.message}`, 'error');
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
// Create Deployment
// ============================================================

document.getElementById('create-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const fd = new FormData(e.target);
  const data = {
    name: fd.get('name'),
    programSlug: fd.get('programSlug'),
    chainId: fd.get('chainId') ? parseInt(fd.get('chainId')) : undefined,
    config: fd.get('deployDir') ? { deployDir: fd.get('deployDir') } : undefined,
  };
  try {
    const { deployment } = await api('/deployments', { method: 'POST', body: JSON.stringify(data) });
    e.target.reset();
    showDeployment(deployment.id);
  } catch (err) {
    alert('Failed: ' + err.message);
  }
});

// ============================================================
// SSE Events & Logs
// ============================================================

function connectEvents(id) {
  disconnectEvents();
  eventSource = new EventSource(`${API}/deployments/${id}/events`);
  eventSource.onmessage = (e) => {
    try {
      const data = JSON.parse(e.data);
      if (data.event === 'phase') {
        addLogLine(`[${data.phase}] ${data.message}`, 'phase');
        const phaseBadge = document.getElementById('detail-phase');
        phaseBadge.textContent = data.phase;
        phaseBadge.className = `phase-badge phase-${data.phase}`;
        // Refresh detail when entering terminal states
        if (data.phase === 'running' || data.phase === 'error') {
          setTimeout(() => showDeployment(id), 500);
        }
      } else if (data.event === 'log') {
        addLogLine(data.message, 'info');
      } else if (data.event === 'error') {
        addLogLine(`ERROR: ${data.message}`, 'error');
      } else if (data.event === 'waiting') {
        addLogLine(data.message, 'info');
      }
    } catch {}
  };
  eventSource.onerror = () => {
    addLogLine('SSE connection lost, will retry...', 'info');
  };
}

function disconnectEvents() {
  if (eventSource) {
    eventSource.close();
    eventSource = null;
  }
}

function addLogLine(text, type = 'info') {
  const container = document.getElementById('log-container');
  const line = document.createElement('div');
  line.className = `log-line log-${type}`;
  const ts = new Date().toLocaleTimeString();
  line.textContent = `[${ts}] ${text}`;
  container.appendChild(line);
  container.scrollTop = container.scrollHeight;
}

function clearLogs() {
  document.getElementById('log-container').innerHTML = '';
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
  div.textContent = str;
  return div.innerHTML;
}

// ============================================================
// Init
// ============================================================

checkHealth();
loadDeployments();
setInterval(checkHealth, 10000);
