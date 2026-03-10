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
let cachedDeployList = [];
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

function resetLaunchForm() {
  const db = document.getElementById('launch-deploy-btn'); if (db) db.textContent = 'Deploy L2';
  document.getElementById('launch-name').value = '';
  document.getElementById('launch-chain-id').value = '';
  document.getElementById('launch-deploy-dir').value = '';
  document.getElementById('launch-l1-image').value = 'ethrex';
  onL1NodeChange();
  const rpcEl = document.getElementById('launch-testnet-rpc'); if (rpcEl) { rpcEl.value = ''; rpcEl.style.borderColor = '#f87171'; }
  const keyEl = document.getElementById('launch-testnet-keychain-key'); if (keyEl) keyEl.value = '';
  const addrEl = document.getElementById('launch-testnet-deployer-addr'); if (addrEl) addrEl.value = '';
  const chainIdEl = document.getElementById('launch-testnet-l1-chainid'); if (chainIdEl) chainIdEl.value = '';
  const etherscanEl = document.getElementById('launch-testnet-etherscan-key'); if (etherscanEl) etherscanEl.value = '';
  const balEl = document.getElementById('testnet-balance-check'); if (balEl) balEl.innerHTML = '';
  const saveEl = document.getElementById('testnet-save-status'); if (saveEl) saveEl.innerHTML = '';
  // Reset role key selectors
  for (const id of ['launch-testnet-committer-key', 'launch-testnet-proof-coordinator-key', 'launch-testnet-bridge-owner-key']) {
    const el = document.getElementById(id); if (el) el.value = '';
  }
  for (const id of ['launch-testnet-committer-addr', 'launch-testnet-proof-coordinator-addr', 'launch-testnet-bridge-owner-addr']) {
    const el = document.getElementById(id); if (el) el.value = '';
  }
  const cfgSummary = document.getElementById('deploy-config-summary'); if (cfgSummary) cfgSummary.style.display = 'none';
  selectedProgram = null;
}

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
  if (name === 'launch') {
    loadPrograms(); launchGoStep(1); launchDeploymentId = null;
    resetLaunchForm();
  }
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

const TESTNET_STEPS = [
  { phase: 'checking_docker', label: 'Checking Docker' },
  { phase: 'building', label: 'Preparing Docker Images' },
  { phase: 'deploying_contracts', label: 'Deploying L1 Contracts' },
  { phase: 'verifying_contracts', label: 'Verifying on Etherscan' },
  { phase: 'l2_starting', label: 'Starting L2 Node' },
  { phase: 'starting_prover', label: 'Starting Prover' },
  { phase: 'starting_tools', label: 'Starting Tools (Explorer, Bridge)' },
  { phase: 'running', label: 'Running' },
];

const TESTNET_L1_VALUES = new Set(['sepolia', 'holesky', 'custom-l1']);
const TESTNET_NETWORKS = {
  sepolia: { chainId: 11155111, name: 'Sepolia', rpcPlaceholder: 'https://sepolia.infura.io/v3/YOUR_KEY' },
  holesky: { chainId: 17000, name: 'Holesky', rpcPlaceholder: 'https://holesky.infura.io/v3/YOUR_KEY' },
  'custom-l1': { chainId: null, name: 'Custom', rpcPlaceholder: 'https://your-l1-rpc-endpoint' },
};

function isTestnetL1() {
  return TESTNET_L1_VALUES.has(document.getElementById('launch-l1-image')?.value || '');
}

const PHASE_ESTIMATES = {
  checking_docker: { min: 1, max: 5 },
  building: { min: 120, max: 600 },
  pulling: { min: 30, max: 180 },
  l1_starting: { min: 5, max: 30 },
  deploying_contracts: { min: 30, max: 120 },
  verifying_contracts: { min: 10, max: 60 },
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
      <div style="display:flex;gap:16px;align-items:flex-start">
        <div style="flex-shrink:0">
          <div style="display:flex;align-items:center;gap:10px">
            <div class="program-icon" style="width:30px;height:30px;font-size:14px">${esc(selectedProgram.name.charAt(0).toUpperCase())}</div>
            <div>
              <h3 style="font-size:13px;font-weight:700">${esc(selectedProgram.name)}</h3>
              <div style="font-size:11px;color:var(--text-muted)">${esc(pid)}</div>
            </div>
          </div>
          <button class="btn-change" onclick="launchGoStep(1)" style="margin-top:4px">Change</button>
        </div>
        <div class="app-config-box" style="padding:6px 12px;margin:0;flex:1;font-size:11px">${configHtml}
          <div id="docker-image-status" style="margin-top:6px;font-size:11px;color:var(--text-muted)">Checking Docker image...</div>
        </div>
      </div>
    `;

    checkDocker();
    checkDockerImage(pid);
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

function onL1NodeChange() {
  const val = document.getElementById('launch-l1-image').value;
  const isTestnet = TESTNET_L1_VALUES.has(val);
  document.getElementById('testnet-fields').style.display = isTestnet ? 'block' : 'none';
  if (isTestnet) {
    const info = TESTNET_NETWORKS[val];
    document.getElementById('launch-testnet-rpc').placeholder = info.rpcPlaceholder;
    document.getElementById('testnet-custom-chainid').style.display = val === 'custom-l1' ? 'block' : 'none';
    document.getElementById('testnet-balance-check').innerHTML = '';
    loadKeychainKeys().then(() => onKeychainKeyChange());
  }
}

// ============================================================
// Keychain Management
// ============================================================

async function loadKeychainKeys() {
  try {
    const res = await fetch(`${API}/keychain/keys`);
    const data = await res.json();
    const keys = data.keys || [];
    const keyOptions = keys.map(k => `<option value="${esc(k)}">${esc(k)}</option>`).join('');

    // Deployer selector
    const sel = document.getElementById('launch-testnet-keychain-key');
    const prev = sel.value;
    sel.innerHTML = '<option value="">Select a key from Keychain...</option>' + keyOptions;
    if (prev && keys.includes(prev)) sel.value = prev;

    // Role selectors (committer, proof-coordinator, bridge-owner)
    const roleIds = ['launch-testnet-committer-key', 'launch-testnet-proof-coordinator-key', 'launch-testnet-bridge-owner-key'];
    for (const id of roleIds) {
      const roleSel = document.getElementById(id);
      if (!roleSel) continue;
      const rolePrev = roleSel.value;
      roleSel.innerHTML = '<option value="">Same as Deployer (default)</option>' + keyOptions;
      if (rolePrev && keys.includes(rolePrev)) roleSel.value = rolePrev;
    }
  } catch (e) {
    console.error('Failed to load keychain keys:', e);
  }
}

async function refreshKeychainKeys() {
  await loadKeychainKeys();
  await onKeychainKeyChange();
}

async function onKeychainKeyChange() {
  const sel = document.getElementById('launch-testnet-keychain-key');
  const addrInput = document.getElementById('launch-testnet-deployer-addr');
  if (!sel.value) { addrInput.value = ''; updateRoleDefaultAddresses(''); return; }
  try {
    const res = await fetch(`${API}/keychain/keys/${encodeURIComponent(sel.value)}`);
    const data = await res.json();
    if (data.address) {
      addrInput.value = data.address;
      updateRoleDefaultAddresses(data.address);
    }
  } catch (e) {
    console.error('Failed to get address for key:', e);
  }
  autoCheckBalances();
}

// Show deployer address in role cards that are set to "Same as Deployer"
function updateRoleDefaultAddresses(deployerAddress) {
  const roleIds = [
    { sel: 'launch-testnet-committer-key', addr: 'launch-testnet-committer-addr' },
    { sel: 'launch-testnet-proof-coordinator-key', addr: 'launch-testnet-proof-coordinator-addr' },
    { sel: 'launch-testnet-bridge-owner-key', addr: 'launch-testnet-bridge-owner-addr' },
  ];
  for (const { sel, addr } of roleIds) {
    const selEl = document.getElementById(sel);
    const addrEl = document.getElementById(addr);
    if (!selEl || !addrEl) continue;
    if (!selEl.value) {
      // "Same as Deployer" selected — show deployer address in lighter style
      addrEl.value = deployerAddress || '';
      addrEl.placeholder = deployerAddress ? '' : '= Deployer';
      addrEl.style.color = '#9ca3af';  // gray to indicate inherited
    }
  }
}

function toggleGuideLocale() {
  document.querySelectorAll('.guide-ko').forEach(el => {
    el.style.display = el.style.display === 'none' ? '' : 'none';
  });
  document.querySelectorAll('.guide-en').forEach(el => {
    el.style.display = el.style.display === 'none' ? '' : 'none';
  });
}

async function registerKeychainKey() {
  try {
    const res = await fetch(`${API}/open-url`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ url: 'keychain-register' }),
    });
    const data = await res.json();
    if (data.ok && data.keyName) {
      await loadKeychainKeys();
      document.getElementById('launch-testnet-keychain-key').value = data.keyName;
      await onKeychainKeyChange();
    }
  } catch (e) {
    console.error('Keychain register failed:', e);
  }
}

async function onRoleKeyChange(role) {
  const idMap = {
    'committer': { sel: 'launch-testnet-committer-key', addr: 'launch-testnet-committer-addr' },
    'proof-coordinator': { sel: 'launch-testnet-proof-coordinator-key', addr: 'launch-testnet-proof-coordinator-addr' },
    'bridge-owner': { sel: 'launch-testnet-bridge-owner-key', addr: 'launch-testnet-bridge-owner-addr' },
  };
  const ids = idMap[role];
  if (!ids) return;
  const sel = document.getElementById(ids.sel);
  const addrInput = document.getElementById(ids.addr);
  if (!sel.value) {
    // "Same as Deployer" — show deployer address in gray
    const deployerAddr = (document.getElementById('launch-testnet-deployer-addr')?.value || '').trim();
    addrInput.value = deployerAddr || '';
    addrInput.placeholder = deployerAddr ? '' : '= Deployer';
    addrInput.style.color = '#9ca3af';
    autoCheckBalances();
    return;
  }
  try {
    const res = await fetch(`${API}/keychain/keys/${encodeURIComponent(sel.value)}`);
    const data = await res.json();
    if (data.address) {
      addrInput.value = data.address;
      addrInput.style.color = '#4b5563';  // darker = own key
    }
  } catch (e) {
    console.error(`Failed to get address for ${role} key:`, e);
  }
  autoCheckBalances();
}

// Auto-check balances when keys change (only if RPC URL and deployer key are set)
function autoCheckBalances() {
  const rpcUrl = (document.getElementById('launch-testnet-rpc')?.value || '').trim();
  const deployerAddr = (document.getElementById('launch-testnet-deployer-addr')?.value || '').trim();
  if (rpcUrl && deployerAddr) checkTestnetBalance();
}

async function checkTestnetBalance() {
  const rpcUrl = (document.getElementById('launch-testnet-rpc')?.value || '').trim();
  const deployerAddr = (document.getElementById('launch-testnet-deployer-addr')?.value || '').trim();
  const el = document.getElementById('testnet-balance-check');

  if (!rpcUrl) { el.innerHTML = '<span style="color:var(--red-600,#dc2626);font-size:11px">Enter L1 RPC URL first.</span>'; return; }
  if (!deployerAddr) { el.innerHTML = '<span style="color:var(--red-600,#dc2626);font-size:11px">Select a Deployer key first.</span>'; return; }

  // Collect all roles with addresses and card status element IDs
  const roles = [
    { role: 'deployer', address: deployerAddr, statusId: 'balance-status-deployer' },
    { role: 'committer', address: (document.getElementById('launch-testnet-committer-addr')?.value || '').trim() || deployerAddr, statusId: 'balance-status-committer' },
    { role: 'proof-coordinator', address: (document.getElementById('launch-testnet-proof-coordinator-addr')?.value || '').trim() || deployerAddr, statusId: 'balance-status-proof-coordinator' },
    { role: 'bridge-owner', address: (document.getElementById('launch-testnet-bridge-owner-addr')?.value || '').trim() || deployerAddr, statusId: 'balance-status-bridge-owner' },
  ];

  // Show loading in each card
  for (const { statusId } of roles) {
    const statusEl = document.getElementById(statusId);
    if (statusEl) statusEl.innerHTML = '<span style="color:var(--gray-400);font-size:10px">Checking...</span>';
  }
  el.innerHTML = '<span style="color:var(--gray-500);font-size:11px">Checking all account balances...</span>';

  try {
    const results = await Promise.all(
      roles.map(async ({ role, address, statusId }) => {
        const res = await fetch(`${API}/deployments/testnet/check-balance`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ rpcUrl, address, role }),
        });
        const data = await res.json();
        return { ...data, statusId, roleKey: role };
      })
    );

    const firstError = results.find(r => r.error);
    if (firstError) {
      el.innerHTML = `<span style="color:var(--red-600,#dc2626);font-size:11px">${esc(firstError.error)}</span>`;
      return;
    }

    // Format gas number for display (e.g. 12960000000 → "12.96B", 25000000 → "25M")
    function formatGas(gas) {
      if (gas >= 1e9) return (gas / 1e9).toFixed(2) + 'B';
      if (gas >= 1e6) return (gas / 1e6).toFixed(0) + 'M';
      if (gas >= 1e3) return (gas / 1e3).toFixed(0) + 'K';
      return String(gas);
    }

    // Update each card with balance status
    for (const r of results) {
      const statusEl = document.getElementById(r.statusId);
      if (!statusEl) continue;
      const balColor = r.sufficient ? '#16a34a' : '#dc2626';
      const icon = r.sufficient ? '✓' : '✗';
      statusEl.innerHTML = `
        <div style="margin-top:6px;padding:6px 8px;background:white;border-radius:6px;border:1px solid #e5e7eb;font-size:11px">
          <table style="width:100%;border-collapse:collapse;font-size:11px">
            <tr>
              <td style="color:#6b7280;padding:1px 0">Balance</td>
              <td style="text-align:right;font-weight:700;color:${balColor}">${esc(r.balanceEth)} ETH</td>
            </tr>
            <tr>
              <td style="color:#6b7280;padding:1px 0">Required (1 month)</td>
              <td style="text-align:right;font-weight:600">~${esc(r.estimatedCostEth)} ETH</td>
            </tr>
            <tr>
              <td style="color:#6b7280;padding:1px 0">Est. Gas</td>
              <td style="text-align:right">${formatGas(r.estimatedGas)} gas</td>
            </tr>
          </table>
          <div style="color:${balColor};font-weight:600;margin-top:3px;font-size:11px">${icon} ${r.sufficient ? 'Sufficient' : 'Insufficient'}</div>
          <details style="margin-top:3px">
            <summary style="font-size:10px;color:#9ca3af;cursor:pointer;user-select:none">${esc(r.gasLabel)}</summary>
            <div style="font-size:10px;color:#6b7280;margin-top:3px;line-height:1.5;padding:4px 6px;background:#f9fafb;border-radius:4px">
              ${esc(r.gasDetail || '')}
              ${r.interval ? `<br><strong>Tx interval:</strong> every ${esc(r.interval)}` : ''}
            </div>
          </details>
        </div>`;
    }

    // Summary
    const allSufficient = results.every(r => r.sufficient);
    const first = results[0];
    let html = `<div style="font-size:11px;padding:6px 10px;background:var(--gray-100,#f3f4f6);border-radius:6px">`;
    if (first.chainId) html += `<span>L1 Chain ID: <code>${esc(String(first.chainId))}</code> · Gas Price: <code>${esc(first.gasPriceGwei)} gwei</code></span> · `;
    html += `<span style="color:${allSufficient ? 'var(--green-600,#16a34a)' : 'var(--red-600,#dc2626)'};font-weight:600">
      ${allSufficient ? '✓ All accounts funded' : '✗ Some accounts need more ETH'}
    </span></div>`;
    el.innerHTML = html;
  } catch (e) {
    el.innerHTML = `<span style="color:var(--red-600,#dc2626);font-size:11px">Connection failed: ${esc(e.message)}</span>`;
  }
}

async function saveTestnetSettings() {
  const name = document.getElementById('launch-name').value.trim();
  if (!name) { showLaunchError('L2 name is required to save settings'); return; }
  if (!selectedProgram) { showLaunchError('Please select a program first'); return; }

  const el = document.getElementById('testnet-save-status');
  el.innerHTML = '<span style="color:var(--gray-500);font-size:11px">Saving...</span>';

  try {
    const network = document.getElementById('launch-l1-image').value;
    const netInfo = TESTNET_NETWORKS[network];
    const body = {
      programSlug: selectedProgram.program_id || selectedProgram.id,
      name,
      chainId: parseInt(document.getElementById('launch-chain-id').value) || undefined,
      config: {
        mode: 'testnet',
        l1Image: network,
        deployDir: (document.getElementById('launch-deploy-dir')?.value || '').trim() || undefined,
        testnet: {
          l1RpcUrl: (document.getElementById('launch-testnet-rpc')?.value || '').trim(),
          keychainKeyName: (document.getElementById('launch-testnet-keychain-key')?.value || '').trim(),
          committerKeychainKey: (document.getElementById('launch-testnet-committer-key')?.value || '').trim() || undefined,
          proofCoordinatorKeychainKey: (document.getElementById('launch-testnet-proof-coordinator-key')?.value || '').trim() || undefined,
          bridgeOwnerKeychainKey: (document.getElementById('launch-testnet-bridge-owner-key')?.value || '').trim() || undefined,
          l1ChainId: netInfo?.chainId || (parseInt(document.getElementById('launch-testnet-l1-chainid')?.value) || undefined),
          network: network === 'custom-l1' ? 'custom' : network,
          etherscanApiKey: (document.getElementById('launch-testnet-etherscan-key')?.value || '').trim() || undefined,
        },
      },
      rpcUrl: (document.getElementById('launch-testnet-rpc')?.value || '').trim(),
    };

    let res;
    if (launchDeploymentId) {
      // Update existing configured deployment
      res = await fetch(`${API}/deployments/${launchDeploymentId}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: body.name, chain_id: body.chainId, rpc_url: body.rpcUrl, config: body.config }),
      });
    } else {
      res = await fetch(`${API}/deployments`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
    }
    if (!res.ok) { const err = await res.json(); throw new Error(err.error || 'Failed to save'); }
    const data = await res.json();
    if (!launchDeploymentId) launchDeploymentId = data.deployment?.id || data.id;

    el.innerHTML = '<span style="color:var(--green-600,#16a34a)">Settings saved to My L2.</span>';
    setTimeout(() => { el.innerHTML = ''; }, 3000);
    loadDeployments();
  } catch (e) {
    el.innerHTML = `<span style="color:var(--red-600,#dc2626)">${esc(e.message)}</span>`;
  }
}

let dockerImageReady = false;

async function checkDockerImage(programSlug) {
  const el = document.getElementById('docker-image-status');
  if (!el) return;
  dockerImageReady = false;
  el.innerHTML = '<span style="color:var(--text-muted)">Checking Docker image...</span>';
  try {
    const res = await fetch(`${API}/deployments/check-image/${encodeURIComponent(programSlug)}`);
    const data = await res.json();
    if (data.exists) {
      dockerImageReady = true;
      el.innerHTML = `<span style="color:#16a34a;font-weight:600">Docker image ready: ${esc(data.image)}</span>`;
    } else {
      el.innerHTML = '<span style="color:#d97706;font-weight:600">Docker image not found — will be built during deployment (~5-10min)</span>';
    }
  } catch {
    el.innerHTML = '<span style="color:var(--text-muted)">Could not check Docker image</span>';
  }
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
    if (isTestnetL1()) {
      const testnetRpc = (document.getElementById('launch-testnet-rpc')?.value || '').trim();
      const keychainKey = (document.getElementById('launch-testnet-keychain-key')?.value || '').trim();
      const network = document.getElementById('launch-l1-image').value;
      if (!testnetRpc) { showLaunchError('L1 RPC URL is required for testnet'); btn.disabled = false; btn.textContent = 'Deploy L2'; return; }
      if (!keychainKey) { showLaunchError('Select a deployer key from Keychain. Click "Open Keychain Access" to register one.'); btn.disabled = false; btn.textContent = 'Deploy L2'; return; }
      const netInfo = TESTNET_NETWORKS[network];
      body.config.mode = 'testnet';
      body.config.testnet = {
        l1RpcUrl: testnetRpc,
        keychainKeyName: keychainKey,
        committerKeychainKey: (document.getElementById('launch-testnet-committer-key')?.value || '').trim() || undefined,
        proofCoordinatorKeychainKey: (document.getElementById('launch-testnet-proof-coordinator-key')?.value || '').trim() || undefined,
        bridgeOwnerKeychainKey: (document.getElementById('launch-testnet-bridge-owner-key')?.value || '').trim() || undefined,
        l1ChainId: netInfo.chainId || (parseInt(document.getElementById('launch-testnet-l1-chainid')?.value) || undefined),
        network: network === 'custom-l1' ? 'custom' : network,
        etherscanApiKey: (document.getElementById('launch-testnet-etherscan-key')?.value || '').trim() || undefined,
      };
      body.rpcUrl = testnetRpc;
    }
    if (launchMode === 'remote') {
      const hostId = document.getElementById('launch-host-select').value;
      if (hostId) body.hostId = hostId;
    }
    const deployDir = document.getElementById('launch-deploy-dir')?.value?.trim();
    if (deployDir) body.deployDir = deployDir;

    // 1. Create or update deployment
    if (launchDeploymentId) {
      // Update existing configured deployment
      const res = await fetch(`${API}/deployments/${launchDeploymentId}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: body.name, chain_id: body.chainId, rpc_url: body.rpcUrl, config: body.config }),
      });
      if (!res.ok) { const err = await res.json(); throw new Error(err.error || 'Failed to update'); }
    } else {
      const res = await fetch(`${API}/deployments`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      if (!res.ok) { const err = await res.json(); throw new Error(err.error || 'Failed to create'); }
      const data = await res.json();
      launchDeploymentId = data.deployment?.id || data.id;
    }

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
// Deploy Config Summary (shown on step 3)
// ============================================================
function renderDeployConfigSummary(deployment) {
  const el = document.getElementById('deploy-config-summary');
  if (!el) return;
  const config = typeof deployment.config === 'string' ? JSON.parse(deployment.config || '{}') : (deployment.config || {});
  const mode = config.mode || launchMode || 'local';
  const testnet = config.testnet || {};

  const rows = [];
  rows.push(['L2 Name', esc(deployment.name || '')]);
  rows.push(['App', esc(deployment.program_name || programDisplayName(deployment.program_slug) || '')]);
  rows.push(['Environment', mode === 'testnet' ? 'Testnet' : mode === 'remote' ? 'Remote' : 'Local (Docker)']);

  if (mode === 'testnet') {
    const networkLabels = { sepolia: 'Sepolia', holesky: 'Holesky', 'custom-l1': 'Custom L1' };
    rows.push(['L1 Network', networkLabels[testnet.network] || testnet.network || config.l1Image || '']);
    if (testnet.l1RpcUrl) rows.push(['L1 RPC URL', '<code>' + esc(testnet.l1RpcUrl) + '</code>']);
    if (testnet.l1ChainId) rows.push(['L1 Chain ID', testnet.l1ChainId]);
    // Role key summary
    if (testnet.keychainKeyName) {
      rows.push(['Deployer', `🔑 ${esc(testnet.keychainKeyName)}`]);
      rows.push(['Committer', testnet.committerKeychainKey ? `🔑 ${esc(testnet.committerKeychainKey)}` : `🔑 ${esc(testnet.keychainKeyName)} <span style="opacity:0.5">(= Deployer)</span>`]);
      rows.push(['Proof Coordinator', testnet.proofCoordinatorKeychainKey ? `🔑 ${esc(testnet.proofCoordinatorKeychainKey)}` : `🔑 ${esc(testnet.keychainKeyName)} <span style="opacity:0.5">(= Deployer)</span>`]);
      rows.push(['Bridge Owner', testnet.bridgeOwnerKeychainKey ? `🔑 ${esc(testnet.bridgeOwnerKeychainKey)}` : `🔑 ${esc(testnet.keychainKeyName)} <span style="opacity:0.5">(= Deployer)</span>`]);
    }
  } else {
    const l1Labels = { ethrex: 'ethrex (Tokamak)', geth: 'Geth', reth: 'Reth' };
    rows.push(['L1 Node', l1Labels[config.l1Image] || config.l1Image || '']);
    // Local mode: show hardcoded dev key roles
    rows.push(['Deployer / Committer', '<code style="font-size:10px">0x3D1e…</code> <span style="opacity:0.5">(dev key)</span>']);
    rows.push(['Proof Coordinator', '<code style="font-size:10px">0xE255…</code> <span style="opacity:0.5">(dev key)</span>']);
    rows.push(['Bridge Owner', '<code style="font-size:10px">0x4417…</code> <span style="opacity:0.5">(dev key)</span>']);
  }
  if (deployment.chain_id) rows.push(['L2 Chain ID', deployment.chain_id]);
  if (deployment.bridge_address) rows.push(['Bridge', '<code>' + esc(deployment.bridge_address) + '</code>']);
  if (deployment.proposer_address) rows.push(['Proposer', '<code>' + esc(deployment.proposer_address) + '</code>']);

  const grid = document.getElementById('deploy-config-grid');
  if (grid) {
    grid.innerHTML = rows.map(([k, v]) =>
      `<div class="config-item"><span class="config-label">${k}</span><span class="config-value">${v}</span></div>`
    ).join('');
  }
  el.style.display = '';
}

// ============================================================
// Deploy Progress (SSE)
// ============================================================
// Track deployed contracts in real-time
let deployedContracts = {};
let lastContractAnnouncement = null;
let buildingImageFound = null; // Set when image is found (skip build)

function parseContractFromLog(line) {
  // Priority 1: Structured JSON output from deployer
  const jsonMatch = line.match(/DEPLOYER_RESULT_JSON:(\{.*\})/);
  if (jsonMatch) {
    try {
      const data = JSON.parse(jsonMatch[1]);
      if (data.status === 'success' && data.contracts) {
        for (const [name, addr] of Object.entries(data.contracts)) {
          if (addr && !deployedContracts[name]) deployedContracts[name] = addr;
        }
        lastContractAnnouncement = null;
        return;
      }
    } catch { /* fall through to legacy parsing */ }
  }

  const addrMatch = line.match(/address=(0x[0-9a-fA-F]{40})/);

  // Detect contract announcement
  if (line.includes('CommonBridge deployed')) lastContractAnnouncement = 'CommonBridge';
  else if (line.includes('OnChainProposer deployed')) lastContractAnnouncement = 'OnChainProposer';
  else if (line.includes('Timelock deployed')) lastContractAnnouncement = 'Timelock';
  else if (line.includes('SP1Verifier deployed')) lastContractAnnouncement = 'SP1Verifier';
  else if (line.includes('SequencerRegistry deployed')) lastContractAnnouncement = 'SequencerRegistry';
  else if (line.includes('GuestProgramRegistry initialized') || line.includes('GuestProgramRegistry deployed')) {
    if (addrMatch) deployedContracts['GuestProgramRegistry'] = addrMatch[1];
    lastContractAnnouncement = null;
    return;
  }

  if (!addrMatch) return;
  const addr = addrMatch[1];

  // SP1Verifier is single-line
  if (lastContractAnnouncement === 'SP1Verifier' && line.includes('SP1Verifier deployed')) {
    deployedContracts['SP1Verifier'] = addr;
    lastContractAnnouncement = null;
    return;
  }

  // Proxy-based contracts: capture Proxy address
  if (lastContractAnnouncement && line.includes('Proxy')) {
    deployedContracts[lastContractAnnouncement] = addr;
    lastContractAnnouncement = null;
  }
}

function startDeployProgress(id) {
  currentPhase = 'configured';
  buildLogLines = [];
  deployEvents = [];
  deployedContracts = {};
  lastContractAnnouncement = null;
  buildingImageFound = null;
  phaseDurations = {};
  deployStartTime = Date.now();
  phaseStartTime = Date.now();

  const deployName = document.getElementById('launch-name')?.value || selectedProgram?.deployName || 'L2';
  document.getElementById('deploy-info-text').innerHTML = `Your L2 <strong>${esc(deployName)}</strong> powered by <strong>${esc(selectedProgram?.name || 'L2')}</strong> is being deployed...`;
  document.getElementById('deploy-error-msg').style.display = 'none';
  document.getElementById('deploy-complete').style.display = 'none';
  document.getElementById('goto-dashboard-btn').style.display = 'none';
  const cancelBtn = document.getElementById('cancel-deploy-btn');
  cancelBtn.style.display = '';
  cancelBtn.disabled = false;
  cancelBtn.textContent = 'Cancel Deployment';

  renderProgressSteps();
  startElapsedTimer();

  // Load and show config summary
  fetch(`${API}/deployments`).then(r => r.json()).then(data => {
    const dep = (data.deployments || data || []).find(d => d.id === id);
    if (dep) renderDeployConfigSummary(dep);
  }).catch(() => {});

  if (deployEventSource) deployEventSource.close();
  deployEventSource = new EventSource(`${API}/deployments/${id}/events`);

  deployEventSource.onmessage = (e) => {
    try {
      const data = JSON.parse(e.data);
      if (data.event === 'log') {
        buildLogLines.push(data.message || '');
        if (buildLogLines.length > 200) buildLogLines = buildLogLines.slice(-200);
        // Parse contract addresses during contract-related phases
        if (['deploying_contracts', 'verifying_contracts', 'l2_starting'].includes(currentPhase)) {
          const prevCount = Object.keys(deployedContracts).length;
          parseContractFromLog(data.message || '');
          if (Object.keys(deployedContracts).length > prevCount) renderProgressSteps();
        }
        renderBuildLog();
        return;
      }

      deployEvents.push(data);
      if (data.imageFound) buildingImageFound = data.imageFound;
      // Capture contract addresses from phase events
      if (data.bridgeAddress && !deployedContracts['CommonBridge']) deployedContracts['CommonBridge'] = data.bridgeAddress;
      if (data.proposerAddress && !deployedContracts['OnChainProposer']) deployedContracts['OnChainProposer'] = data.proposerAddress;
      if (data.timelockAddress && !deployedContracts['Timelock']) deployedContracts['Timelock'] = data.timelockAddress;
      if (data.sp1VerifierAddress && !deployedContracts['SP1Verifier']) deployedContracts['SP1Verifier'] = data.sp1VerifierAddress;
      if (data.guestProgramRegistryAddress && !deployedContracts['GuestProgramRegistry']) deployedContracts['GuestProgramRegistry'] = data.guestProgramRegistryAddress;
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
        document.getElementById('cancel-deploy-btn').style.display = 'none';
        const resumeBtn = document.getElementById('resume-deploy-btn');
        if (resumeBtn) { resumeBtn.style.display = ''; resumeBtn.dataset.id = launchDeploymentId; }
        stopElapsedTimer();
        deployEventSource.close();
      }
      if (data.phase === 'running') {
        document.getElementById('cancel-deploy-btn').style.display = 'none';
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
  const steps = launchMode === 'remote' ? REMOTE_STEPS : (isTestnetL1() ? TESTNET_STEPS : LOCAL_STEPS);
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
    const isBuildingSkipped = step.phase === 'building' && buildingImageFound && isCurrent;
    const cls = (isComplete || isBuildingSkipped) ? 'done' : isCurrent ? 'active' : '';
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

    // If building phase and image was found, treat as quick-complete (no spinner)
    const buildingSkipped = step.phase === 'building' && buildingImageFound && isCurrent && !isTerminal;

    let iconHtml;
    if (isComplete || buildingSkipped) iconHtml = '\u2713';
    else if (isCurrent && !isTerminal) iconHtml = '<div style="width:12px;height:12px;border:2px solid white;border-top-color:transparent;border-radius:50%" class="animate-spin"></div>';
    else iconHtml = i + 1;

    // Override time and class for skipped build
    if (buildingSkipped) {
      timeHtml = `<span style="color:var(--green-600);font-size:10px">${esc(buildingImageFound)}</span>`;
    }

    // Show contract addresses under deploying_contracts step
    let contractsHtml = '';
    if (step.phase === 'deploying_contracts' && Object.keys(deployedContracts).length > 0) {
      const entries = Object.entries(deployedContracts);
      const l1Network = document.getElementById('launch-l1-image')?.value || '';
      const etherscanUrls = { sepolia: 'https://sepolia.etherscan.io', holesky: 'https://holesky.etherscan.io' };
      const explorerBase = etherscanUrls[l1Network] || null;
      contractsHtml = `<div style="margin:4px 0 0 28px;font-size:10px;line-height:1.6">` +
        entries.map(([name, addr]) => {
          const addrDisplay = explorerBase
            ? `<a href="${esc(explorerBase)}/address/${esc(addr)}" target="_blank" style="color:var(--blue-600,#2563eb);text-decoration:none;font-family:monospace;font-size:9px">${esc(addr)} ↗</a>`
            : `<code style="color:var(--text-muted);font-size:9px">${esc(addr)}</code>`;
          return `<div style="display:flex;gap:6px;align-items:center">` +
            `<span style="color:var(--green-600);font-weight:600">\u2713</span>` +
            `<span style="color:var(--text-secondary);min-width:140px">${esc(name)}</span>` +
            addrDisplay +
          `</div>`;
        }).join('') +
        `</div>`;
    }

    return `<div class="progress-step ${cls}">
      <div class="step-icon">${iconHtml}</div>
      <div class="step-label">${step.label}</div>
      <div class="step-time">${timeHtml}</div>
    </div>${contractsHtml}`;
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

    // Save program info (will be restored after showView reset)
    const restoredProgram = { name: dep?.program_name || programDisplayName(dep?.program_slug) || 'L2', id: dep?.program_slug || '' };
    currentPhase = statusData.phase || dep?.phase || 'building';
    buildLogLines = [];
    deployEvents = [];
    deployedContracts = {};
    lastContractAnnouncement = null;
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

    // Show launch view at step 3 (showView resets selectedProgram, so restore after)
    showView('launch');
    selectedProgram = restoredProgram;
    launchGoStep(3);

    const deployName = dep?.name || 'L2';
    const appName = dep?.program_name || dep?.program_slug || 'L2';
    document.getElementById('deploy-info-text').innerHTML = `Your L2 <strong>${esc(deployName)}</strong> powered by <strong>${esc(appName)}</strong> is being deployed...`;
    document.getElementById('deploy-error-msg').style.display = 'none';
    document.getElementById('deploy-complete').style.display = 'none';
    document.getElementById('goto-dashboard-btn').style.display = 'none';

    // Always reset cancel button state
    const cancelBtn = document.getElementById('cancel-deploy-btn');

    renderProgressSteps();
    renderBuildLog();
    if (dep) renderDeployConfigSummary(dep);

    // If still active, connect SSE for live updates + start timer
    if (histData.isActive) {
      // Show cancel button for active deployments
      if (cancelBtn) {
        cancelBtn.style.display = '';
        cancelBtn.disabled = false;
        cancelBtn.textContent = 'Cancel Deployment';
      }
      startElapsedTimer();

      if (deployEventSource) deployEventSource.close();
      deployEventSource = new EventSource(`${API}/deployments/${id}/events`);

      deployEventSource.onmessage = (e) => {
        try {
          const data = JSON.parse(e.data);
          if (data.event === 'log') {
            buildLogLines.push(data.message || '');
            if (buildLogLines.length > 500) buildLogLines = buildLogLines.slice(-500);
            // Parse contract addresses from logs
            if (['deploying_contracts', 'verifying_contracts', 'l2_starting'].includes(currentPhase)) {
              const prevCount = Object.keys(deployedContracts).length;
              parseContractFromLog(data.message || '');
              if (Object.keys(deployedContracts).length > prevCount) renderProgressSteps();
            }
            renderBuildLog();
            return;
          }
          deployEvents.push(data);
          if (data.imageFound) buildingImageFound = data.imageFound;
          // Capture contract addresses from phase events
          if (data.bridgeAddress && !deployedContracts['CommonBridge']) deployedContracts['CommonBridge'] = data.bridgeAddress;
          if (data.proposerAddress && !deployedContracts['OnChainProposer']) deployedContracts['OnChainProposer'] = data.proposerAddress;
          if (data.timelockAddress && !deployedContracts['Timelock']) deployedContracts['Timelock'] = data.timelockAddress;
          if (data.sp1VerifierAddress && !deployedContracts['SP1Verifier']) deployedContracts['SP1Verifier'] = data.sp1VerifierAddress;
          if (data.guestProgramRegistryAddress && !deployedContracts['GuestProgramRegistry']) deployedContracts['GuestProgramRegistry'] = data.guestProgramRegistryAddress;
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
      // Not active -- hide cancel button, show final state
      if (cancelBtn) cancelBtn.style.display = 'none';
      if (currentPhase === 'running') {
        showDeployComplete(statusData);
      } else if (currentPhase === 'error' || currentPhase === 'stopped') {
        // Stopped mid-deploy (e.g. contracts deployed but L2 not started) — auto-resume
        document.getElementById('deploy-info-text').innerHTML = `Resuming deployment <strong>${esc(dep?.name || 'L2')}</strong>...`;
        try {
          const resp = await fetch(`${API}/deployments/${id}/provision`, { method: 'POST' });
          if (resp.ok) {
            launchDeploymentId = id;
            startDeployProgress(id);
          } else {
            const err = await resp.json().catch(() => ({}));
            document.getElementById('deploy-error-msg').textContent = err.error || 'Failed to resume';
            document.getElementById('deploy-error-msg').style.display = 'block';
            const resumeBtn = document.getElementById('resume-deploy-btn');
            if (resumeBtn) { resumeBtn.style.display = ''; resumeBtn.dataset.id = id; }
          }
        } catch (e) {
          document.getElementById('deploy-error-msg').textContent = `Resume failed: ${e.message}`;
          document.getElementById('deploy-error-msg').style.display = 'block';
          const resumeBtn = document.getElementById('resume-deploy-btn');
          if (resumeBtn) { resumeBtn.style.display = ''; resumeBtn.dataset.id = id; }
        }
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
    cachedDeployList = list;
    const container = document.getElementById('deployments-list');

    if (list.length === 0) {
      container.innerHTML = `<div class="empty-state">
        <p style="margin-bottom:12px">No L2s launched yet.</p>
        <button class="btn-primary" onclick="showView('launch')">Launch your first L2</button>
      </div>`;
      return;
    }

    // Reconcile: check live Docker status for deployments with docker_project
    await Promise.all(list.map(async (d) => {
      if (!d.docker_project || isDeploying(d.phase)) return;
      try {
        const statusRes = await fetch(`${API}/deployments/${d.id}/status`);
        const statusData = await statusRes.json();
        const containers = statusData.containers || [];
        // Only check core services (L1/L2/Prover), not shared tools containers
        const coreServices = ['tokamak-app-l1', 'tokamak-app-l2', 'tokamak-app-prover'];
        const coreContainers = containers.filter(c => coreServices.includes(c.Service));
        const anyRunning = coreContainers.some(c => (c.State || c.state) === 'running');
        if (d.phase === 'stopped' && anyRunning) {
          d.phase = 'running'; d.status = 'active';
        } else if (d.phase === 'running' && containers.length > 0 && !anyRunning) {
          d.phase = 'stopped'; d.status = 'configured';
        }
      } catch { /* ignore */ }
    }));

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
  const hasError = !!d.error_message;
  const statusClass = hasError ? 'error' : d.phase === 'running' ? 'running'
    : d.phase === 'configured' ? 'configured'
    : ['building','pulling','l1_starting','deploying_contracts','verifying_contracts','l2_starting','starting_prover','starting_tools','checking_docker'].includes(d.phase) ? 'building' : 'stopped';
  const rowConfig = d.config ? (typeof d.config === 'string' ? JSON.parse(d.config) : d.config) : {};
  const isTestnet = rowConfig.mode === 'testnet';
  const ports = [!isTestnet && d.l1_port ? `L1:${d.l1_port}` : '', d.l2_port ? `L2:${d.l2_port}` : ''].filter(Boolean).join(' · ') || '-';

  return `
    <tr class="deploy-row" data-id="${d.id}">
      <td>
        <button class="expand-btn" onclick="event.stopPropagation(); toggleDeployExpand('${d.id}')" style="background:none;border:none;cursor:pointer;padding:4px 4px 4px 8px;color:var(--text-muted);display:flex;align-items:center">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" style="transition:transform 0.15s;${isExpanded ? 'transform:rotate(90deg)' : ''}">
            <polyline points="9 18 15 12 9 6"/>
          </svg>
        </button>
      </td>
      <td onclick="${isDeploying(d.phase) ? `resumeDeployProgress('${d.id}')` : (isIncomplete(d) || hasError) && d.docker_project ? `resumeDeployProgress('${d.id}')` : (d.phase === 'configured' || (hasError && !d.docker_project) || isIncomplete(d)) ? `editConfiguredDeploy('${d.id}')` : `showDeploymentDetail('${d.id}')`}" style="cursor:pointer">
        <div class="name-cell">
          <div class="icon-box">${esc((d.name || '?').charAt(0))}</div>
          <div>
            <div class="name-text">${esc(d.name)}</div>
            <div style="font-size:11px;color:var(--text-muted)">${esc(d.program_name || programDisplayName(d.program_slug) || d.program_id)}${isTestnet ? ` · <span style="color:var(--blue-500,#3b82f6)">L1 ${esc({sepolia:'Sepolia',holesky:'Holesky',custom:'Custom'}[(rowConfig.testnet||{}).network]||'Testnet')}</span>` : ''}</div>
          </div>
        </div>
      </td>
      <td>
        <div class="status-cell">
          <span class="status-dot ${statusClass}"></span>
          <span>${hasError ? statusLabel(d.phase) + ' ⚠' : statusLabel(d.phase)}</span>
        </div>
      </td>
      <td style="font-size:12px;color:var(--text-secondary);font-family:monospace">${ports}</td>
      <td>${renderPhaseBadge(d.phase, hasError)}</td>
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
          ${hasError && !d.docker_project ? `
            <button class="icon-btn" title="Edit & Retry" onclick="event.stopPropagation(); editConfiguredDeploy('${d.id}')" style="color:var(--green-500,#22c55e)">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>
            </button>` : ''}
          ${hasError && d.docker_project ? `
            <button class="icon-btn" title="Resume Deploy" onclick="event.stopPropagation(); resumeDeployProgress('${d.id}')" style="color:var(--green-600,#16a34a)">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="5 3 19 12 5 21 5 3"/></svg>
            </button>` : ''}
          ${d.phase === 'configured' ? `
            <button class="icon-btn" title="Edit & Deploy" onclick="event.stopPropagation(); editConfiguredDeploy('${d.id}')" style="color:var(--green-600,#16a34a)">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 2L11 13"/><path d="M22 2l-7 20-4-9-9-4 20-7z"/></svg>
            </button>` : ''}
          ${d.phase === 'stopped' && !isIncomplete(d) ? `
            <button class="icon-btn" title="Start" onclick="event.stopPropagation(); startDeploy('${d.id}')">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="5 3 19 12 5 21 5 3"/></svg>
            </button>` : ''}
          ${isIncomplete(d) ? `
            <button class="icon-btn" title="Resume Deploy" onclick="event.stopPropagation(); ${d.docker_project ? `resumeDeployProgress('${d.id}')` : `editConfiguredDeploy('${d.id}')`}" style="color:var(--green-600,#16a34a)">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="5 3 19 12 5 21 5 3"/></svg>
            </button>` : ''}
          <button class="icon-btn" title="${(d.phase === 'configured' || ((hasError || isIncomplete(d)) && !d.docker_project)) ? 'Edit Settings' : (isIncomplete(d) || hasError) && d.docker_project ? 'Resume Deploy' : 'Details'}" onclick="event.stopPropagation(); ${(d.phase === 'configured' || ((hasError || isIncomplete(d)) && !d.docker_project)) ? `editConfiguredDeploy('${d.id}')` : (isIncomplete(d) || hasError) && d.docker_project ? `resumeDeployProgress('${d.id}')` : `showDeploymentDetail('${d.id}')`}">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>
          </button>
          <button class="icon-btn danger" title="Delete" onclick="event.stopPropagation(); deleteDeploy('${d.id}', event)">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
          </button>
        </div>
      </td>
    </tr>
    ${isExpanded ? `<tr class="container-row"><td colspan="6" style="padding:0"><div id="containers-${d.id}" class="container-expand-area">Loading containers...</div></td></tr>` : ''}`;
}

function isDeploying(phase) {
  return ['checking_docker','building','pulling','l1_starting','deploying_contracts','verifying_contracts','l2_starting','starting_prover','starting_tools'].includes(phase);
}

// Deployment was stopped before ever reaching 'running' — incomplete build
function isIncomplete(d) {
  if (d.phase !== 'stopped') return false;
  if (d.ever_running) return false;
  if (d.status === 'active') return false;
  return true;
}

function statusLabel(phase) {
  const map = {
    configured: 'Configured', checking_docker: 'Checking...', building: 'Building',
    pulling: 'Pulling', l1_starting: 'Starting', deploying_contracts: 'Deploying',
    verifying_contracts: 'Verifying', l2_starting: 'Starting', starting_prover: 'Starting', starting_tools: 'Starting',
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

    // Filter out L1 Explorer containers for testnet deployments
    const depInfo = cachedDeployList.find(d => d.id === id);
    const depConfig = depInfo?.config ? (typeof depInfo.config === 'string' ? JSON.parse(depInfo.config) : depInfo.config) : {};
    const isTestnetDep = depConfig.mode === 'testnet';
    const l1ExplorerServices = ['frontend-l1', 'backend-l1'];
    const containers = (data.containers || []).filter(c => {
      if (!isTestnetDep) return true;
      const svc = c.Service || c.service || c.Name || c.name || '';
      return !l1ExplorerServices.includes(svc);
    });
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

async function serviceAction(deployId, service, action, btnEl) {
  // Immediate UI feedback: show Starting.../Stopping...
  if (btnEl) {
    btnEl.disabled = true;
    btnEl.textContent = action === 'start' ? 'Starting...' : 'Stopping...';
  }
  try {
    const resp = await fetch(`${API}/deployments/${deployId}/service/${service}/${action}`, { method: 'POST' });
    if (!resp.ok) {
      const err = await resp.json().catch(() => ({}));
      throw new Error(err.error || `Failed (${resp.status})`);
    }
    // Refresh detail view to reflect new state
    if (currentDeploymentId === deployId) await fetchDetailStatus();
    setTimeout(() => loadContainersForDeploy(deployId), 1000);
  } catch (e) {
    console.error(`Service ${action} failed:`, e);
    const msgEl = document.getElementById('detail-error-msg');
    if (msgEl) { msgEl.textContent = `${service} ${action} failed: ${e.message}`; msgEl.style.display = 'block'; setTimeout(() => msgEl.style.display = 'none', 8000); }
  } finally {
    if (btnEl) { btnEl.disabled = false; btnEl.textContent = action === 'start' ? 'Start' : 'Stop'; }
  }
}

// Track tools pending state so polling re-renders preserve the "Starting/Stopping" label
let _toolsPending = null; // 'starting' | 'stopping' | null
let _toolsPendingTimer = null;

async function toolsAction(deployId, action, btnEl) {
  const isStart = action === 'startTools';
  _toolsPending = isStart ? 'starting' : 'stopping';
  // Auto-clear pending state after 30s (fallback in case polling doesn't detect change)
  clearTimeout(_toolsPendingTimer);
  _toolsPendingTimer = setTimeout(() => { _toolsPending = null; }, 30000);

  if (btnEl) {
    btnEl.disabled = true;
    btnEl.textContent = isStart ? 'Starting...' : 'Stopping...';
  }
  try {
    const endpoint = isStart ? 'restart-tools' : 'stop-tools';
    const resp = await fetch(`${API}/deployments/${deployId}/${endpoint}`, { method: 'POST' });
    if (!resp.ok) {
      const err = await resp.json().catch(() => ({}));
      throw new Error(err.error || `Failed (${resp.status})`);
    }
    // Server responds immediately; poll faster to pick up container state changes
    if (currentDeploymentId === deployId) {
      const fastPoll = setInterval(async () => {
        await fetchDetailStatus();
        // Check if tools state changed — clear pending
        const svcs = ['frontend-l1','frontend-l2','bridge-ui'];
        const anyRunning = (detailStatus?.containers || []).some(c => svcs.includes(c.Service) && (c.State || c.state) === 'running');
        if ((isStart && anyRunning) || (!isStart && !anyRunning)) {
          _toolsPending = null;
          clearTimeout(_toolsPendingTimer);
          clearInterval(fastPoll);
          renderOverviewTab();
        }
      }, 3000);
      // Stop fast polling after 60s
      setTimeout(() => clearInterval(fastPoll), 60000);
    }
  } catch (e) {
    _toolsPending = null;
    clearTimeout(_toolsPendingTimer);
    console.error(`Tools ${action} failed:`, e);
    const msgEl = document.getElementById('detail-error-msg');
    if (msgEl) { msgEl.textContent = `Tools ${isStart ? 'start' : 'stop'} failed: ${e.message}`; msgEl.style.display = 'block'; setTimeout(() => msgEl.style.display = 'none', 8000); }
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

async function resumeDeploy() {
  const resumeBtn = document.getElementById('resume-deploy-btn');
  const id = resumeBtn?.dataset.id;
  if (!id) return;
  resumeBtn.disabled = true;
  resumeBtn.textContent = 'Resuming...';
  try {
    document.getElementById('deploy-error-msg').style.display = 'none';
    resumeBtn.style.display = 'none';
    const resp = await fetch(`${API}/deployments/${id}/provision`, { method: 'POST' });
    if (resp.ok) {
      launchDeploymentId = id;
      startDeployProgress(id);
    } else {
      const err = await resp.json().catch(() => ({}));
      document.getElementById('deploy-error-msg').style.display = 'block';
      document.getElementById('deploy-error-msg').textContent = err.error || 'Failed to resume';
      resumeBtn.style.display = '';
      resumeBtn.disabled = false;
      resumeBtn.textContent = 'Resume Deployment';
    }
  } catch (e) {
    console.error('Resume failed:', e);
    resumeBtn.disabled = false;
    resumeBtn.textContent = 'Resume Deployment';
  }
}

async function cancelDeploy() {
  if (!launchDeploymentId) return;
  const btn = document.getElementById('cancel-deploy-btn');
  btn.disabled = true;
  btn.textContent = 'Cancelling...';
  try {
    await fetch(`${API}/deployments/${launchDeploymentId}/stop`, { method: 'POST' });
    if (deployEventSource) { deployEventSource.close(); deployEventSource = null; }
    stopElapsedTimer();
    document.getElementById('deploy-error-msg').style.display = 'block';
    document.getElementById('deploy-error-msg').textContent = 'Deployment cancelled by user.';
    document.getElementById('deploy-message').style.display = 'none';
    btn.style.display = 'none';
    // Show Resume button
    const resumeBtn = document.getElementById('resume-deploy-btn');
    if (resumeBtn) { resumeBtn.style.display = ''; resumeBtn.disabled = false; resumeBtn.textContent = 'Resume Deployment'; resumeBtn.dataset.id = launchDeploymentId; }
    renderProgressSteps(); // Re-render to stop spinner and show error state
    loadDeployments();
  } catch (e) {
    console.error('Cancel failed:', e);
    btn.disabled = false;
    btn.textContent = 'Cancel Deployment';
  }
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

async function editConfiguredDeploy(id) {
  try {
    const res = await fetch(`${API}/deployments/${id}`);
    const data = await res.json();
    const dep = data.deployment || data;
    const config = dep.config ? (typeof dep.config === 'string' ? JSON.parse(dep.config) : dep.config) : {};

    // Ensure programs are loaded
    if (programs.length === 0) await loadPrograms();

    // Navigate to launch step 2 first (showView resets selectedProgram)
    showView('launch');

    // Restore selectedProgram after showView reset
    const slug = dep.program_slug || 'evm-l2';
    selectedProgram = programs.find(p => (p.program_id || p.id) === slug) || { id: slug, name: programDisplayName(slug) || slug };

    launchGoStep(2);

    // Restore form fields
    document.getElementById('launch-name').value = dep.name || '';
    document.getElementById('launch-chain-id').value = dep.chain_id || '';

    // Restore L1 node selection
    if (config.l1Image) {
      document.getElementById('launch-l1-image').value = config.l1Image;
      onL1NodeChange();
    }

    // Restore testnet config
    if (config.testnet) {
      if (config.testnet.l1RpcUrl) {
        const rpcInput = document.getElementById('launch-testnet-rpc');
        rpcInput.value = config.testnet.l1RpcUrl;
        rpcInput.style.borderColor = '';
      }
      if (config.testnet.keychainKeyName) {
        await loadKeychainKeys();
        document.getElementById('launch-testnet-keychain-key').value = config.testnet.keychainKeyName;
        await onKeychainKeyChange();
      }
      // Restore role keys
      const roleKeyMap = {
        committerKeychainKey: { sel: 'launch-testnet-committer-key', role: 'committer' },
        proofCoordinatorKeychainKey: { sel: 'launch-testnet-proof-coordinator-key', role: 'proof-coordinator' },
        bridgeOwnerKeychainKey: { sel: 'launch-testnet-bridge-owner-key', role: 'bridge-owner' },
      };
      for (const [cfgKey, { sel, role }] of Object.entries(roleKeyMap)) {
        if (config.testnet[cfgKey]) {
          const el = document.getElementById(sel);
          if (el) { el.value = config.testnet[cfgKey]; await onRoleKeyChange(role); }
        }
      }
      if (config.testnet.l1ChainId && document.getElementById('launch-testnet-l1-chainid')) {
        document.getElementById('launch-testnet-l1-chainid').value = config.testnet.l1ChainId;
      }
      if (config.testnet.etherscanApiKey && document.getElementById('launch-testnet-etherscan-key')) {
        document.getElementById('launch-testnet-etherscan-key').value = config.testnet.etherscanApiKey;
      }
    }

    if (config.deployDir) {
      document.getElementById('launch-deploy-dir').value = config.deployDir;
    }

    // Store the existing deployment ID so Deploy uses it instead of creating a new one
    launchDeploymentId = dep.id;

    // Show deployment progress info if partially deployed
    const deployBtn = document.getElementById('launch-deploy-btn');
    const hasContracts = dep.bridge_address && dep.proposer_address;
    const wasDeployed = !!dep.error_message || dep.phase === 'stopped' || dep.bridge_address || dep.docker_project;

    if (wasDeployed) {
      deployBtn.textContent = 'Continue Deploy';
    } else {
      deployBtn.textContent = 'Deploy L2';
    }

    // Show saved contract/deployment info
    const infoEl = document.getElementById('launch-error');
    const infoLines = [];
    if (dep.error_message) {
      infoLines.push(`Last error: ${dep.error_message}`);
    }
    if (dep.bridge_address) infoLines.push(`Bridge: ${dep.bridge_address}`);
    if (dep.proposer_address) infoLines.push(`Proposer: ${dep.proposer_address}`);
    if (dep.bridge_address && dep.proposer_address) {
      infoLines.push('Contracts already deployed — will be reused (no extra gas)');
    } else if (dep.bridge_address && !dep.proposer_address) {
      infoLines.push('Bridge deployed but Proposer missing — contracts will be redeployed');
    }
    if (dep.docker_project) infoLines.push(`Docker project: ${dep.docker_project}`);

    if (infoLines.length > 0) {
      const statusDiv = document.getElementById('testnet-save-status');
      if (statusDiv) {
        statusDiv.innerHTML = infoLines.map(l => `<div style="margin-bottom:2px;color:var(--text-secondary);font-size:11px">${esc(l)}</div>`).join('');
      }
    }
  } catch (e) {
    console.error('Failed to load configured deployment:', e);
  }
}

async function provisionDeploy(id) {
  try {
    const depRes = await fetch(`${API}/deployments`);
    const depData = await depRes.json();
    const depList = depData.deployments || depData || [];
    const dep = depList.find(d => d.id === id);
    const resp = await fetch(`${API}/deployments/${id}/provision`, { method: 'POST' });
    if (resp.ok) {
      launchDeploymentId = id;
      showView('launch');
      // Set selectedProgram after showView reset
      if (dep) {
        selectedProgram = { name: dep.program_name || programDisplayName(dep.program_slug) || 'L2', id: dep.program_slug || '', deployName: dep.name || 'L2' };
      }
      launchGoStep(3);
      startDeployProgress(id);
    } else {
      const err = await resp.json().catch(() => ({}));
      console.error('Provision failed:', err.error || 'Unknown error');
      loadDeployments();
    }
  } catch (e) {
    console.error('Provision failed:', e);
  }
}

async function retryDeploy(id) {
  try {
    // Stop existing containers without deleting DB record
    await fetch(`${API}/deployments/${id}/stop`, { method: 'POST' }).catch(() => {});

    // Fetch deployment info for display
    const depRes = await fetch(`${API}/deployments`);
    const depData = await depRes.json();
    const depList = depData.deployments || depData || [];
    const dep = depList.find(d => d.id === id);
    const resp = await fetch(`${API}/deployments/${id}/provision`, { method: 'POST' });
    if (resp.ok) {
      launchDeploymentId = id;
      showView('launch');
      // Set selectedProgram after showView reset
      if (dep) {
        selectedProgram = { name: dep.program_name || programDisplayName(dep.program_slug) || 'L2', id: dep.program_slug || '', deployName: dep.name || 'L2' };
      }
      launchGoStep(3);
      startDeployProgress(id);
    } else {
      const err = await resp.json().catch(() => ({}));
      console.error('Retry failed:', err.error || 'Unknown error');
      loadDeployments();
    }
  } catch (e) {
    console.error('Retry failed:', e);
  }
}

function showConfirm(message) {
  return new Promise(resolve => {
    const overlay = document.getElementById('confirm-overlay');
    document.getElementById('confirm-msg').textContent = message;
    overlay.style.display = 'flex';
    const ok = document.getElementById('confirm-ok');
    const cancel = document.getElementById('confirm-cancel');
    function cleanup(result) {
      overlay.style.display = 'none';
      ok.removeEventListener('click', onOk);
      cancel.removeEventListener('click', onCancel);
      overlay.removeEventListener('click', onOverlay);
      resolve(result);
    }
    function onOk() { cleanup(true); }
    function onCancel() { cleanup(false); }
    function onOverlay(e) { if (e.target === overlay) cleanup(false); }
    ok.addEventListener('click', onOk);
    cancel.addEventListener('click', onCancel);
    overlay.addEventListener('click', onOverlay);
  });
}

async function deleteDeploy(id, event) {
  const dep = cachedDeployList?.find(d => d.id === id);
  const name = dep?.name || 'this L2';
  if (!await showConfirm(`Delete "${name}"?\n\nThis will remove the deployment record. Docker containers will not be affected.`)) return;
  try {
    await fetch(`${API}/deployments/${id}`, { method: 'DELETE' });
    if (expandedDeploymentId === id) expandedDeploymentId = null;
    loadDeployments();
  } catch (e) { console.error('Delete failed:', e); }
}

function renderPhaseBadge(phase, hasError) {
  const labels = {
    configured: 'Not deployed', checking_docker: 'Checking Docker', building: 'Building',
    pulling: 'Pulling Images', l1_starting: 'Starting L1', deploying_contracts: 'Deploying',
    verifying_contracts: 'Verifying', l2_starting: 'Starting L2', starting_prover: 'Starting Prover', starting_tools: 'Starting Tools',
    running: 'Running', stopped: 'Stopped', error: 'Error',
  };
  const animating = ['checking_docker','building','pulling','l1_starting','deploying_contracts','verifying_contracts','l2_starting','starting_prover','starting_tools'];
  const label = labels[phase] || phase;
  if (hasError && phase !== 'error') {
    return `<span class="phase-badge phase-error" title="Error during: ${label}">${label} - Error</span>`;
  }
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
let detailContracts = null;
let detailTab = 'overview';

async function showDeploymentDetail(id) {
  currentDeploymentId = id;
  showView('detail');
  detailDeployment = null;
  detailStatus = null;
  detailMonitoring = null;
  detailContracts = null;
  detailTab = 'overview';
  if (detailPollInterval) { clearInterval(detailPollInterval); detailPollInterval = null; }
  if (logEventSource) { logEventSource.close(); logEventSource = null; }

  // Reset tab buttons to overview active
  document.querySelectorAll('.tab-btn').forEach(btn => btn.classList.toggle('active', btn.dataset.tab === 'overview'));

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
  const modeLabels = { local: 'Local', remote: 'Remote', testnet: 'Testnet', manual: 'Manual' };
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
  const isDeploying = ['checking_docker','building','l1_starting','deploying_contracts','verifying_contracts','l2_starting','starting_prover','starting_tools'].includes(d.phase);
  // Reconcile: use live container state instead of stale DB phase
  const liveContainers = detailStatus?.containers || [];
  const hasContainers = liveContainers.length > 0;
  const anyContainerRunning = hasContainers && liveContainers.some(c => (c.State || c.state) === 'running');
  // Only trust live status when detailStatus has been fetched; otherwise use DB phase
  const statusFetched = !!detailStatus;
  const isRunning = isDeploying ? false : (statusFetched ? anyContainerRunning : d.phase === 'running');
  const isStopped = !isDeploying && !isRunning && (statusFetched ? !anyContainerRunning : d.phase !== 'error');
  const hasError = !!d.error_message && d.phase !== 'running';
  const isError = hasError || (statusFetched && isProvisioned && !hasContainers && d.phase === 'running');

  // Update header badge to reflect live container status (only when status is fetched)
  if (statusFetched) {
    const livePhase = isDeploying ? d.phase : isRunning ? 'running' : 'stopped';
    document.getElementById('detail-phase').innerHTML = renderPhaseBadge(livePhase, hasError);
  }

  document.getElementById('container-cards').innerHTML = '';
  document.getElementById('detail-endpoints').style.display = 'none';

  let dynamicEl = document.querySelector('#tab-overview .overview-dynamic');
  if (!dynamicEl) {
    dynamicEl = document.createElement('div');
    dynamicEl.className = 'overview-dynamic';
    document.getElementById('tab-overview').appendChild(dynamicEl);
  }

  let html = '';
  if (d.error_message && d.phase !== 'running') html += `<div class="error-box" style="margin-bottom:10px">${esc(d.error_message)}</div>`;

  // Helper: find container state
  const containers = detailStatus?.containers || [];
  function svcState(name) {
    const c = containers.find(c => c.Service === name || c.Name?.includes(name.replace('tokamak-app-','').replace('zk-dex-tools-','')));
    return c ? (c.State || 'stopped') : 'stopped';
  }

  // Helper: render service row
  function svcRow(label, svcName, endpoint, isTools) {
    const state = svcState(svcName);
    const running = state === 'running';
    const dot = `<span style="width:7px;height:7px;border-radius:50%;background:${running ? 'var(--green-500)' : 'var(--text-muted)'};flex-shrink:0"></span>`;
    const stateText = `<span style="font-size:11px;color:${running ? 'var(--green-600)' : 'var(--text-muted)'}">${state}</span>`;
    const openIcon = `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align:-1px;cursor:pointer"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>`;
    const ep = endpoint
      ? (running
        ? `<span style="font-size:11px;font-family:monospace;color:var(--blue-600)">${endpoint}</span> <a href="http://localhost${endpoint}" target="_blank" title="Open in browser" style="color:var(--blue-600)">${openIcon}</a>`
        : `<span style="font-size:11px;font-family:monospace;color:var(--text-muted)">${endpoint}</span>`)
      : '';
    const btn = isTools ? '' : (running
      ? `<button class="btn-secondary" style="padding:2px 8px;font-size:10px" onclick="serviceAction('${d.id}','${svcName}','stop',this)">Stop</button>`
      : (isProvisioned ? `<button class="btn-secondary" style="padding:2px 8px;font-size:10px" onclick="serviceAction('${d.id}','${svcName}','start',this)">Start</button>` : ''));
    return `<div class="svc-row">${dot}<span class="svc-name">${label}</span>${stateText}${ep}<span style="margin-left:auto">${btn}</span></div>`;
  }

  // 2-column layout
  html += '<div class="overview-grid">';

  // LEFT: Services
  html += '<div style="display:flex;flex-direction:column;gap:14px">';
  html += '<div class="card">';
  html += '<h3 style="font-size:13px;margin-bottom:8px">Services</h3>';

  // Detect testnet mode
  const dConfig = d.config ? (typeof d.config === 'string' ? JSON.parse(d.config) : d.config) : {};
  const isTestnetDeploy = dConfig.mode === 'testnet';

  // Core services
  html += '<div style="font-size:10px;font-weight:600;text-transform:uppercase;letter-spacing:0.05em;color:var(--text-muted);margin-bottom:4px">Core</div>';
  if (!isTestnetDeploy) {
    html += svcRow('L1 Node', 'tokamak-app-l1', d.l1_port ? `:${d.l1_port}` : null);
  } else {
    // Testnet: show external L1 info instead of container
    const testnetCfg = dConfig.testnet || {};
    const netNames = { sepolia: 'Sepolia', holesky: 'Holesky', custom: 'Custom' };
    html += `<div class="svc-row">
      <span style="width:7px;height:7px;border-radius:50%;background:var(--blue-500,#3b82f6);flex-shrink:0"></span>
      <span class="svc-name">L1 (${esc(netNames[testnetCfg.network] || 'External')})</span>
      <span style="font-size:11px;color:var(--blue-600,#2563eb)">external</span>
      <span style="font-size:10px;font-family:monospace;color:var(--text-muted);max-width:200px;overflow:hidden;text-overflow:ellipsis" title="${esc(testnetCfg.l1RpcUrl || '')}">${esc((testnetCfg.l1RpcUrl || '').replace(/^https?:\/\//, '').slice(0, 30))}</span>
    </div>`;
  }
  html += svcRow('L2 Node', 'tokamak-app-l2', d.l2_port ? `:${d.l2_port}` : null);
  html += svcRow('Prover', 'tokamak-app-prover', null);

  // Tools services
  const toolsSvcs = ['frontend-l1','frontend-l2','bridge-ui'];
  const anyToolRunning = toolsSvcs.some(s => svcState(s) === 'running');
  let toolsBtnLabel, toolsBtnAction, toolsBtnStyle, toolsBtnDisabled;
  if (_toolsPending === 'starting') {
    toolsBtnLabel = 'Starting...'; toolsBtnAction = 'startTools'; toolsBtnStyle = 'color:var(--blue-500,#3b82f6)'; toolsBtnDisabled = true;
  } else if (_toolsPending === 'stopping') {
    toolsBtnLabel = 'Stopping...'; toolsBtnAction = 'stopTools'; toolsBtnStyle = 'color:var(--blue-500,#3b82f6)'; toolsBtnDisabled = true;
  } else if (anyToolRunning) {
    toolsBtnLabel = 'Stop'; toolsBtnAction = 'stopTools'; toolsBtnStyle = 'color:var(--orange-600,#ea580c)'; toolsBtnDisabled = false;
  } else {
    toolsBtnLabel = 'Start'; toolsBtnAction = 'startTools'; toolsBtnStyle = 'color:var(--green-600,#16a34a)'; toolsBtnDisabled = false;
  }
  const toolsBtn = isProvisioned ? `<button class="btn-secondary" style="padding:2px 8px;font-size:10px;margin-left:auto;${toolsBtnStyle}" ${toolsBtnDisabled ? 'disabled' : ''} onclick="toolsAction('${d.id}','${toolsBtnAction}',this)">${toolsBtnLabel}</button>` : '';
  html += `<div style="display:flex;align-items:center;gap:6px;margin:8px 0 4px;padding-top:8px;border-top:1px solid var(--border-light)"><span style="font-size:10px;font-weight:600;text-transform:uppercase;letter-spacing:0.05em;color:var(--text-muted)">Tools</span>${toolsBtn}</div>`;
  if (_toolsPending) {
    const pendingMsg = _toolsPending === 'starting'
      ? 'Starting tools (Explorer, Dashboard)... This may take a minute.'
      : 'Stopping tools...';
    html += `<div style="font-size:11px;color:var(--blue-600,#2563eb);padding:4px 8px;margin-bottom:4px;background:var(--blue-50,#eff6ff);border-radius:4px">${pendingMsg}</div>`;
  }
  if (!isTestnetDeploy) {
    html += svcRow('L1 Explorer', 'frontend-l1', d.tools_l1_explorer_port ? `:${d.tools_l1_explorer_port}` : null, true);
  } else {
    // Testnet: link to public explorer instead of local L1 Explorer
    const explorerUrls = { sepolia: 'https://sepolia.etherscan.io', holesky: 'https://holesky.etherscan.io' };
    const pubUrl = explorerUrls[(dConfig.testnet || {}).network];
    if (pubUrl) {
      html += `<div class="svc-row">
        <span style="width:7px;height:7px;border-radius:50%;background:var(--blue-500);flex-shrink:0"></span>
        <span class="svc-name">L1 Explorer</span>
        <a href="${esc(pubUrl)}" target="_blank" style="font-size:11px;color:var(--blue-600)">${esc(pubUrl.replace('https://', ''))} ↗</a>
      </div>`;
    }
  }
  html += svcRow('L2 Explorer', 'frontend-l2', d.tools_l2_explorer_port ? `:${d.tools_l2_explorer_port}` : null, true);
  html += svcRow('Dashboard', 'bridge-ui', d.tools_bridge_ui_port ? `:${d.tools_bridge_ui_port}` : null, true);

  // Global actions
  html += '<div style="display:flex;gap:6px;margin-top:10px;padding-top:10px;border-top:1px solid var(--border)">';
  if (!isProvisioned) html += '<button class="btn-primary" style="padding:5px 12px;font-size:12px" onclick="deployAction(\'provision\')">Deploy</button>';
  if (isStopped) html += '<button class="btn-green" style="padding:5px 12px;font-size:12px" onclick="this.disabled=true;this.textContent=\'Starting...\';deployAction(\'start\')">Start All</button>';
  if (isRunning || isDeploying) html += '<button class="btn-orange" style="padding:5px 12px;font-size:12px" onclick="this.disabled=true;this.textContent=\'Stopping...\';deployAction(\'stop\')">Stop All</button>';
  if (isError) html += '<button class="btn-primary" style="padding:5px 12px;font-size:12px" onclick="this.disabled=true;this.textContent=\'Deploying...\';deployAction(\'provision\')">Retry</button>';
  html += '</div>';

  html += '</div>'; // card

  // Contracts — prefer detailContracts (from bridge-ui config.json), fallback to DB
  {
    const contracts = [];
    const src = detailContracts || {};
    const bridge = src.bridge_address || d.bridge_address;
    const proposer = src.on_chain_proposer_address || d.proposer_address;
    const timelock = src.timelock_address || d.timelock_address;
    const sp1Verifier = src.sp1_verifier_address || d.sp1_verifier_address;
    if (bridge) contracts.push({ label: 'CommonBridge', addr: bridge });
    if (proposer) contracts.push({ label: 'OnChainProposer', addr: proposer });
    if (timelock) contracts.push({ label: 'Timelock', addr: timelock });
    if (sp1Verifier) contracts.push({ label: 'SP1 Verifier', addr: sp1Verifier });
    if (contracts.length > 0) {
      const explorerBase = (!isTestnetDeploy && d.tools_l1_explorer_port) ? `http://localhost:${d.tools_l1_explorer_port}` : null;
      const etherscanByNetwork = { sepolia: 'https://sepolia.etherscan.io', holesky: 'https://holesky.etherscan.io' };
      const etherscanByChainId = { 1: 'https://etherscan.io', 11155111: 'https://sepolia.etherscan.io', 17000: 'https://holesky.etherscan.io' };
      const etherscanBase = etherscanByNetwork[(dConfig.testnet || {}).network] || etherscanByChainId[detailMonitoring?.l1?.chainId] || null;
      const linkIcon = `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align:-1px"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>`;
      const l1ContractTitle = isTestnetDeploy ? `L1 Deployed Contracts (${esc({sepolia:'Sepolia',holesky:'Holesky',custom:'Custom'}[(dConfig.testnet||{}).network]||'Testnet')})` : 'L1 Deployed Contracts';
      html += `<div class="card"><h3 style="font-size:13px;margin-bottom:6px">${l1ContractTitle}</h3>`;
      for (const c of contracts) {
        let links = '';
        if (explorerBase) links += `<a href="${explorerBase}/address/${esc(c.addr)}" target="_blank" style="color:var(--blue-600);margin-left:4px" title="Local Explorer">${linkIcon}</a>`;
        if (etherscanBase) links += `<a href="${etherscanBase}/address/${esc(c.addr)}" target="_blank" style="color:var(--blue-600);margin-left:4px" title="Etherscan">${linkIcon}</a>`;
        html += `<div class="endpoint-row"><span class="ep-label">${esc(c.label)}</span><code style="font-size:10px">${esc(c.addr)}</code>${links}</div>`;
      }
      html += '</div>';
    }
  }
  html += '</div>'; // end left

  // RIGHT: Chain Info + Settings
  html += '<div style="display:flex;flex-direction:column;gap:14px">';
  if (detailMonitoring && (detailMonitoring.l1 || detailMonitoring.l2)) {
    html += '<div class="card"><h3 style="font-size:13px;margin-bottom:6px">Chain Info</h3><div style="display:grid;grid-template-columns:1fr 1fr;gap:6px">';
    if (detailMonitoring.l1) html += `<div style="padding:6px 8px;background:var(--bg);border-radius:6px"><div style="font-weight:600;font-size:11px;margin-bottom:2px">L1${isTestnetDeploy ? ` (${esc({sepolia:'Sepolia',holesky:'Holesky',custom:'Custom'}[(dConfig.testnet||{}).network]||'Testnet')})` : ''}</div><div style="font-size:11px"><span style="color:var(--text-muted)">Block</span> <span style="font-family:monospace">${detailMonitoring.l1.blockNumber ?? '-'}</span></div><div style="font-size:11px"><span style="color:var(--text-muted)">Chain</span> <span style="font-family:monospace">${detailMonitoring.l1.chainId ?? '-'}</span></div></div>`;
    if (detailMonitoring.l2) html += `<div style="padding:6px 8px;background:var(--bg);border-radius:6px"><div style="font-weight:600;font-size:11px;margin-bottom:2px">L2</div><div style="font-size:11px"><span style="color:var(--text-muted)">Block</span> <span style="font-family:monospace">${detailMonitoring.l2.blockNumber ?? '-'}</span></div><div style="font-size:11px"><span style="color:var(--text-muted)">Chain</span> <span style="font-family:monospace">${detailMonitoring.l2.chainId ?? '-'}</span></div></div>`;
    html += '</div></div>';
  }
  html += `<div class="card">
    <h3 style="font-size:13px;margin-bottom:6px">Settings</h3>
    <dl class="info-grid" style="font-size:12px">
      <dt>Name</dt><dd>${esc(d.name)}</dd>
      <dt>Chain ID</dt><dd>${detailMonitoring?.l2?.chainId || d.chain_id || '-'}</dd>
      <dt>Docker</dt><dd style="font-size:10px">${d.docker_project || '-'}</dd>
      <dt>Created</dt><dd style="font-size:10px">${new Date(d.created_at).toLocaleDateString()}</dd>
      ${isTestnetDeploy ? `<dt>L1 Network</dt><dd>${esc((dConfig.testnet?.network || '').charAt(0).toUpperCase() + (dConfig.testnet?.network || '').slice(1))}</dd>
      <dt>L1 RPC</dt><dd style="font-size:10px;word-break:break-all">${(() => {
        const url = dConfig.testnet?.l1RpcUrl || '-';
        if (url === '-') return '-';
        try {
          const u = new URL(url);
          const path = u.pathname + u.search;
          const masked = u.origin + (path.length > 8 ? path.slice(0, 4) + '\u2022\u2022\u2022\u2022' + path.slice(-4) : path);
          const uid = 'rpc-' + d.id;
          return '<span id="' + uid + '-masked">' + esc(masked) + '</span><span id="' + uid + '-full" style="display:none">' + esc(url) + '</span> <button onclick="toggleRpcUrl(\'' + uid + '\',this)" style="background:none;border:1px solid var(--border);border-radius:3px;font-size:9px;padding:1px 4px;cursor:pointer;color:var(--text-muted)">Show</button>';
        } catch { return esc(url); }
      })()}</dd>` : ''}
    </dl>
  </div>`;
  // External Access card
  html += `<div class="card">
    <h3 style="font-size:13px;margin-bottom:6px">External Access</h3>`;
  if (d.is_public && d.public_domain) {
    const publicCfg = {
      l2Rpc: d.public_l2_rpc_url || 'http://' + d.public_domain + ':' + (d.l2_port || 1729),
      l2Explorer: d.public_l2_explorer_url || 'http://' + d.public_domain + ':' + (d.tools_l2_explorer_port || 8082),
      l1Explorer: d.public_l1_explorer_url || (d.l1_port ? 'http://' + d.public_domain + ':' + (d.tools_l1_explorer_port || 8083) : null),
      dashboard: d.public_dashboard_url || 'http://' + d.public_domain + ':' + (d.tools_bridge_ui_port || 3000),
    };
    html += '<div style="display:flex;align-items:center;gap:6px;margin-bottom:8px">';
    html += '<span style="background:var(--green-100,#dcfce7);color:var(--green-700,#15803d);padding:2px 8px;border-radius:10px;font-size:10px;font-weight:600">Enabled</span>';
    html += '<span style="font-size:11px;font-family:monospace;color:var(--text-secondary)">' + esc(d.public_domain) + '</span>';
    html += '</div>';
    const copyIcon = '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>';
    const urlRow = (label, url) => url ? '<div style="display:flex;align-items:center;gap:6px;margin-bottom:3px"><span style="font-size:10px;color:var(--text-muted);width:70px">' + label + '</span><code style="font-size:10px;flex:1">' + esc(url) + '</code><button class="pa-copy-btn" data-url="' + esc(url) + '" style="background:none;border:none;cursor:pointer;color:var(--text-muted);padding:2px" title="Copy">' + copyIcon + '</button></div>' : '';
    html += urlRow('Dashboard', publicCfg.dashboard);
    html += urlRow('Bridge', publicCfg.dashboard + '/bridge.html');
    html += urlRow('L2 Explorer', publicCfg.l2Explorer);
    html += urlRow('L2 RPC', publicCfg.l2Rpc);
    if (publicCfg.l1Explorer) html += urlRow('L1 Explorer', publicCfg.l1Explorer);
    html += '<div style="display:flex;gap:6px;margin-top:8px">';
    html += '<button class="btn-secondary" style="padding:3px 10px;font-size:10px" onclick="showPublicAccessModal(\'' + d.id + '\',\'' + esc(d.public_domain) + '\')">Edit</button>';
    html += '<button class="btn-secondary" style="padding:3px 10px;font-size:10px;color:var(--orange-600,#ea580c)" onclick="disablePublicAccess(\'' + d.id + '\',this)">Disable</button>';
    html += '</div>';
  } else {
    html += '<p style="font-size:11px;color:var(--text-muted);margin-bottom:8px">Allow external users to access Dashboard, Bridge, Explorer, and L2 RPC via public domain or IP.</p>';
    html += '<button class="btn-secondary" style="padding:4px 12px;font-size:11px" ' + (isProvisioned ? '' : 'disabled title="Deploy first"') + ' onclick="showPublicAccessModal(\'' + d.id + '\')">Enable Public Access</button>';
  }
  html += '</div>';

  html += `<button class="btn-danger" style="font-size:11px;padding:6px 12px;align-self:flex-start" onclick="deleteDeployment('${d.id}', event)">Remove L2</button>`;
  html += '</div>'; // end right

  html += '</div>'; // close overview-grid
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
    // If destroyed/deleted, show confirmation then go back
    if (data.deleted || action === 'destroy') {
      const indicator = document.getElementById('action-status');
      if (indicator) {
        indicator.textContent = 'All containers and volumes removed.';
        indicator.style.cssText = 'padding:8px 12px;background:var(--green-50,#f0fdf4);border:1px solid var(--green-200,#bbf7d0);border-radius:6px;margin-bottom:12px;font-size:13px;color:var(--green-700,#15803d)';
      }
      setTimeout(() => showView('deployments'), 1500);
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

// toolsAction is defined above (line ~1518) with full button state management

async function deleteDeployment(id, event) {
  const dep = detailDeployment || cachedDeployList?.find(d => d.id === id);
  const name = dep?.name || 'this L2';
  if (!await showConfirm(`Delete "${name}"?\n\nThis will remove the deployment record.`)) return;
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
    // Fetch contract addresses from bridge-ui config.json (retry until available)
    if (detailDeployment?.tools_bridge_ui_port) {
      try {
        const cRes = await fetch(`http://localhost:${detailDeployment.tools_bridge_ui_port}/config.json`);
        if (cRes.ok) detailContracts = await cRes.json();
      } catch (e) { console.error('Failed to fetch bridge UI config:', e); }
    }
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
  const config = d.config ? (typeof d.config === 'string' ? JSON.parse(d.config) : d.config) : {};
  const isTestnet = config.mode === 'testnet';
  const testnet = config.testnet || {};
  const toml = `# Guest Program Registry Configuration\n# Generated by Tokamak for: ${d.name}\n\ndefault_program = "${slug}"\nenabled_programs = ["${slug}"]`;

  let testnetHtml = '';
  if (isTestnet) {
    const netNames = { sepolia: 'Sepolia', holesky: 'Holesky', custom: 'Custom' };
    testnetHtml = `
    <div class="card" style="margin-bottom:24px">
      <h3 style="margin-bottom:12px">Testnet Configuration</h3>
      <dl class="info-grid">
        <dt>L1 Network</dt><dd>${esc(netNames[testnet.network] || testnet.network || '-')}</dd>
        <dt>L1 Chain ID</dt><dd>${esc(String(testnet.l1ChainId || '-'))}</dd>
        <dt>L1 RPC URL</dt><dd style="word-break:break-all"><code>${esc(testnet.l1RpcUrl || d.rpc_url || '-')}</code></dd>
      </dl>
      <h4 style="margin:12px 0 8px;font-size:13px;color:var(--gray-600,#4b5563)">Account Roles</h4>
      <dl class="info-grid">
        <dt>Deployer</dt><dd><code>${testnet.keychainKeyName ? `🔑 ${esc(testnet.keychainKeyName)}` : 'Not configured'}</code><br><span style="font-size:11px;color:var(--gray-400)">Deploys contracts to L1</span></dd>
        <dt>Committer</dt><dd><code>${testnet.committerKeychainKey ? `🔑 ${esc(testnet.committerKeychainKey)}` : testnet.keychainKeyName ? `🔑 ${esc(testnet.keychainKeyName)} <span style="opacity:0.5">(= Deployer)</span>` : '-'}</code><br><span style="font-size:11px;color:var(--gray-400)">Commits L2 batches to L1</span></dd>
        <dt>Proof Coordinator</dt><dd><code>${testnet.proofCoordinatorKeychainKey ? `🔑 ${esc(testnet.proofCoordinatorKeychainKey)}` : testnet.keychainKeyName ? `🔑 ${esc(testnet.keychainKeyName)} <span style="opacity:0.5">(= Deployer)</span>` : '-'}</code><br><span style="font-size:11px;color:var(--gray-400)">Sends ZK proofs to L1</span></dd>
        <dt>Bridge Owner</dt><dd><code>${testnet.bridgeOwnerKeychainKey ? `🔑 ${esc(testnet.bridgeOwnerKeychainKey)}` : testnet.keychainKeyName ? `🔑 ${esc(testnet.keychainKeyName)} <span style="opacity:0.5">(= Deployer)</span>` : '-'}</code><br><span style="font-size:11px;color:var(--gray-400)">Security council: bridge + proposer ownership</span></dd>
      </dl>
      <div style="margin-top:8px;padding:8px 12px;background:var(--blue-50,#eff6ff);border-radius:6px;font-size:12px;color:var(--blue-700,#1d4ed8)">
        L2 contracts are deployed on ${esc(netNames[testnet.network] || 'testnet')} L1. No built-in L1 node is used.
      </div>
    </div>`;
  } else {
    // Local mode: show hardcoded dev account roles
    testnetHtml = `
    <div class="card" style="margin-bottom:24px">
      <h3 style="margin-bottom:12px">Account Roles <span style="font-size:11px;font-weight:400;color:var(--gray-400)">(dev keys — local only)</span></h3>
      <dl class="info-grid">
        <dt>Deployer / Committer</dt><dd><code style="font-size:11px">0x3D1e15a1a55578f7c920884a9943b3B354b4E06F</code><br><span style="font-size:11px;color:var(--gray-400)">Deploys contracts + commits batches</span></dd>
        <dt>Proof Coordinator</dt><dd><code style="font-size:11px">0xE25583099BA105D9ec0A67f5Ae86D90e50036425</code><br><span style="font-size:11px;color:var(--gray-400)">Sends ZK proofs to L1</span></dd>
        <dt>Bridge Owner</dt><dd><code style="font-size:11px">0x4417092b70a3e5f10dc504d0947dd256b965fc62</code><br><span style="font-size:11px;color:var(--gray-400)">Security council: bridge + proposer ownership</span></dd>
      </dl>
      <div style="margin-top:8px;padding:8px 12px;background:var(--amber-50,#fffbeb);border-radius:6px;font-size:12px;color:var(--amber-700,#b45309)">
        These are pre-funded development keys for local Docker deployment. On testnet/mainnet, use separate funded accounts.
      </div>
    </div>`;
  }

  document.getElementById('tab-config').innerHTML = `
    <div class="card" style="margin-bottom:24px">
      <h3 style="margin-bottom:12px">App Configuration</h3>
      <dl class="info-grid">
        <dt>Guest Program</dt><dd>${esc(slug)}</dd>
        <dt>Program Name</dt><dd>${esc(d.program_name || '')}</dd>
        <dt>Deploy Mode</dt><dd><span class="mode-badge ${config.mode || 'local'}">${esc(config.mode || 'local')}</span></dd>
      </dl>
    </div>
    ${testnetHtml}
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
        <div style="margin-bottom:12px"><p style="font-weight:500;color:var(--gray-700);margin-bottom:4px">3. Endpoints</p><div style="color:var(--gray-500)"><p>L1 RPC: <code style="background:var(--gray-200);padding:2px 6px;border-radius:4px;font-size:12px">${isTestnet ? esc(testnet.l1RpcUrl || 'https://your-l1-rpc') : 'http://localhost:8545'}</code></p><p>L2 RPC: <code style="background:var(--gray-200);padding:2px 6px;border-radius:4px;font-size:12px">http://localhost:1729</code></p></div></div>
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

function toggleRpcUrl(uid, btn) {
  const m = document.getElementById(uid + '-masked');
  const f = document.getElementById(uid + '-full');
  const show = f.style.display === 'none';
  f.style.display = show ? 'inline' : 'none';
  m.style.display = show ? 'none' : 'inline';
  btn.textContent = show ? 'Hide' : 'Show';
}

// ============================================================
// External Access (Public Domain/IP)
// ============================================================

// Delegated click handler for copy buttons (avoids inline onclick with URLs)
document.addEventListener('click', (e) => {
  const btn = e.target.closest('.pa-copy-btn');
  if (!btn) return;
  const url = btn.dataset.url;
  if (url) {
    navigator.clipboard.writeText(url).then(() => {
      btn.textContent = '✓';
      setTimeout(() => { btn.innerHTML = '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>'; }, 1000);
    });
  }
});

function showPublicAccessModal(deploymentId, currentDomain) {
  // Remove existing modal
  document.getElementById('public-access-modal')?.remove();

  const modal = document.createElement('div');
  modal.id = 'public-access-modal';
  modal.style.cssText = 'position:fixed;top:0;left:0;right:0;bottom:0;background:rgba(0,0,0,0.5);display:flex;align-items:center;justify-content:center;z-index:1000';
  modal.innerHTML = `
    <div style="background:var(--bg-card,#fff);border-radius:10px;padding:20px;max-width:420px;width:90%;box-shadow:0 8px 32px rgba(0,0,0,0.2)">
      <h3 style="font-size:14px;margin-bottom:12px">External Access Settings</h3>
      <div style="margin-bottom:10px">
        <label style="font-size:11px;font-weight:600;display:block;margin-bottom:4px">Public Domain / IP</label>
        <input id="pa-domain" type="text" value="${currentDomain || ''}" placeholder="e.g. l2.example.com or 203.0.113.50"
          style="width:100%;padding:6px 10px;font-size:12px;border:1px solid var(--border);border-radius:6px;box-sizing:border-box">
        <span style="font-size:10px;color:var(--text-muted)">Other URLs will be auto-calculated from this + port numbers</span>
      </div>
      <details style="margin-bottom:12px">
        <summary style="font-size:11px;cursor:pointer;color:var(--text-secondary)">Advanced URL Settings</summary>
        <div style="margin-top:8px;display:flex;flex-direction:column;gap:6px">
          <div><label style="font-size:10px;color:var(--text-muted)">L2 RPC URL</label><input id="pa-l2rpc" type="text" placeholder="Auto" style="width:100%;padding:4px 8px;font-size:11px;border:1px solid var(--border);border-radius:4px;box-sizing:border-box"></div>
          <div><label style="font-size:10px;color:var(--text-muted)">L2 Explorer URL</label><input id="pa-l2explorer" type="text" placeholder="Auto" style="width:100%;padding:4px 8px;font-size:11px;border:1px solid var(--border);border-radius:4px;box-sizing:border-box"></div>
          <div><label style="font-size:10px;color:var(--text-muted)">L1 Explorer URL</label><input id="pa-l1explorer" type="text" placeholder="Auto" style="width:100%;padding:4px 8px;font-size:11px;border:1px solid var(--border);border-radius:4px;box-sizing:border-box"></div>
          <div><label style="font-size:10px;color:var(--text-muted)">Dashboard URL</label><input id="pa-dashboard" type="text" placeholder="Auto" style="width:100%;padding:4px 8px;font-size:11px;border:1px solid var(--border);border-radius:4px;box-sizing:border-box"></div>
        </div>
      </details>
      <div style="display:flex;gap:8px;justify-content:flex-end">
        <button class="btn-secondary" style="padding:5px 14px;font-size:12px" onclick="document.getElementById('public-access-modal').remove()">Cancel</button>
        <button class="btn-primary" style="padding:5px 14px;font-size:12px" onclick="enablePublicAccess('${deploymentId}',this)">Enable</button>
      </div>
    </div>`;
  modal.addEventListener('click', (e) => { if (e.target === modal) modal.remove(); });
  document.body.appendChild(modal);
  document.getElementById('pa-domain').focus();
}

async function enablePublicAccess(deploymentId, btn) {
  const domain = document.getElementById('pa-domain').value.trim();
  if (!domain) { alert('Please enter a domain or IP'); return; }
  btn.disabled = true;
  btn.textContent = 'Enabling...';
  try {
    const body = { publicDomain: domain };
    const l2Rpc = document.getElementById('pa-l2rpc').value.trim();
    const l2Explorer = document.getElementById('pa-l2explorer').value.trim();
    const l1Explorer = document.getElementById('pa-l1explorer').value.trim();
    const dashboard = document.getElementById('pa-dashboard').value.trim();
    if (l2Rpc) body.l2RpcUrl = l2Rpc;
    if (l2Explorer) body.l2ExplorerUrl = l2Explorer;
    if (l1Explorer) body.l1ExplorerUrl = l1Explorer;
    if (dashboard) body.dashboardUrl = dashboard;
    await fetch(`${API}/deployments/${deploymentId}/public-access`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
    });
    document.getElementById('public-access-modal')?.remove();
    await loadDeployments();
    if (currentDeploymentId === deploymentId) showDeploymentDetail(deploymentId);
  } catch (e) {
    alert('Failed: ' + e.message);
    btn.disabled = false;
    btn.textContent = 'Enable';
  }
}

async function disablePublicAccess(deploymentId, btn) {
  if (!confirm('Disable public access? Services will revert to localhost mode.')) return;
  btn.disabled = true;
  btn.textContent = 'Disabling...';
  try {
    await fetch(`${API}/deployments/${deploymentId}/public-access`, { method: 'DELETE' });
    await loadDeployments();
    if (currentDeploymentId === deploymentId) showDeploymentDetail(deploymentId);
  } catch (e) {
    alert('Failed: ' + e.message);
    btn.disabled = false;
    btn.textContent = 'Disable';
  }
}

// ============================================================
// Init
// ============================================================
checkHealth();
setInterval(checkHealth, 15000);
// Show launch button in header for deployments view
document.getElementById('header-launch-btn').style.display = '';
// Handle URL parameters (e.g. ?view=launch, ?detail=id from Messenger)
const urlParams = new URLSearchParams(window.location.search);
const editId = urlParams.get('edit');
const initialView = urlParams.get('view');
const detailId = urlParams.get('detail');
loadDeployments().then(() => {
  if (detailId) {
    showDeploymentDetail(detailId);
  } else if (editId) {
    editConfiguredDeploy(editId);
  } else if (initialView) {
    showView(initialView);
  }
});
