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
let launchMode = 'ai-deploy';
let selectedStackType = 'ethrex';
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
const pageTitles = { deployments: 'My L2s', launch: 'Launch L2', hosts: 'Remote Hosts', detail: 'L2 Details', 'guest-builds': 'Guest Builds' };

document.querySelectorAll('.nav-link').forEach(btn => {
  btn.addEventListener('click', () => showView(btn.dataset.view));
});

function resetLaunchForm() {
  const db = document.getElementById('launch-deploy-btn'); if (db) db.textContent = 'Deploy L2';
  document.getElementById('launch-name').value = '';
  document.getElementById('launch-chain-id').value = '';
  if (getL1ChainIdEl()) getL1ChainIdEl().value = '';
  if (getL1ChainIdArea()) getL1ChainIdArea().style.display = 'none';
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
  if (name === 'guest-builds') loadGuestBuilds();
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
    fText.innerHTML = 'Engine offline &nbsp;<button onclick="location.reload()" style="background:var(--accent);color:#fff;border:none;border-radius:4px;padding:2px 8px;font-size:10px;cursor:pointer">Reconnect</button>';
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

const THANOS_LOCAL_STEPS = [
  { phase: 'checking_docker', label: 'Checking Docker' },
  { phase: 'pulling', label: 'Pulling Docker Images' },
  { phase: 'l1_starting', label: 'Starting L1 geth' },
  { phase: 'deploying_contracts', label: 'Deploying Contracts' },
  { phase: 'l2_starting', label: 'Starting op-geth (L2)' },
  { phase: 'starting_op_node', label: 'Starting op-node' },
  { phase: 'starting_batcher', label: 'Starting op-batcher' },
  { phase: 'starting_proposer', label: 'Starting op-proposer' },
  { phase: 'starting_tools', label: 'Starting Tools (Explorer, Bridge)' },
  { phase: 'running', label: 'Running' },
];

const THANOS_TESTNET_STEPS = [
  { phase: 'checking_docker', label: 'Checking Docker' },
  { phase: 'pulling', label: 'Pulling Docker Images' },
  { phase: 'deploying_contracts', label: 'Deploying L1 Contracts' },
  { phase: 'l2_starting', label: 'Starting op-geth (L2)' },
  { phase: 'starting_op_node', label: 'Starting op-node' },
  { phase: 'starting_batcher', label: 'Starting op-batcher' },
  { phase: 'starting_proposer', label: 'Starting op-proposer' },
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

// L1 Chain ID DOM helpers (lazy lookup to avoid stale refs)
function getL1ChainIdEl() { return document.getElementById('launch-l1-chain-id'); }
function getL1ChainIdArea() { return document.getElementById('l1-chain-id-area'); }

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

  const filtered = programs.filter(p => {
    return !p.stack || p.stack === selectedStackType;
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
        ${p.stack === 'thanos' ? '<span class="badge-category" style="background:#fef3c7;color:#92400e">Optimism</span>' : '<span class="badge-category" style="background:#ede9fe;color:#5b21b6">Tokamak-appchain</span>'}
        ${p.category ? `<span class="badge-category">${esc(p.category)}</span>` : ''}
        ${p.is_official ? '<span class="badge-official">Official</span>' : ''}
        ${p.use_count ? `<span class="badge-deploys">${p.use_count} deployments</span>` : ''}
      </div>
      <div class="program-card-desc">${esc(p.description || 'No description')}</div>
      <button class="btn-select" onclick="selectProgram('${p.id}')">Select</button>
    </div>
  `).join('');
}



async function selectProgram(id) {
  selectedProgram = programs.find(p => p.id === id);
  if (!selectedProgram) return;
  document.getElementById('launch-name').value = `${selectedProgram.name} L2`;
  // Fetch unique L1 + L2 chain IDs from the server
  const chainIdInput = document.getElementById('launch-chain-id');
  let chainId, l1ChainId;
  try {
    const res = await fetch(`${API}/deployments/next-chain-id`);
    if (res.ok) {
      const data = await res.json();
      if (data && Number.isFinite(data.chainId)) chainId = data.chainId;
      if (data && Number.isFinite(data.l1ChainId)) l1ChainId = data.l1ChainId;
    }
  } catch (_) { /* server unreachable */ }
  if (Number.isFinite(chainId)) {
    chainIdInput.value = chainId;
    chainIdInput.placeholder = '';
  } else {
    chainIdInput.value = '';
    chainIdInput.placeholder = 'Auto-assigned on deploy';
    showLaunchError('Could not fetch Chain IDs. Please ensure the server is running, then re-select the app.');
  }
  // Set L1 chain ID for local mode (testnet sets its own via onL1NodeChange)
  if (getL1ChainIdEl()) {
    if (Number.isFinite(l1ChainId) && !isTestnetL1()) {
      getL1ChainIdEl().value = l1ChainId;
    } else if (!isTestnetL1()) {
      getL1ChainIdEl().value = '';
    }
  }
  if (getL1ChainIdArea() && !isTestnetL1()) getL1ChainIdArea().style.display = 'block';
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
    const isThanos = selectedStackType === 'thanos';

    let configHtml = '<h4>App Configuration</h4>';
    if (isThanos) {
      configHtml += '<p>Stack: Thanos (Optimism)<br>Proof: Fault Proofs<br>Images: Pre-built (pull only, no build)</p>';
    } else if (pid === 'zk-dex') {
      configHtml += '<p>ZK Circuits: SP1 (DEX order matching + settlement)<br>Verification: SP1 Verifier Contract<br>Genesis: Custom L2 genesis with DEX pre-deploys</p>';
    } else if (pid === 'evm-l2') {
      configHtml += '<p>Circuits: Standard EVM execution<br>Verification: Default Verifier Contract<br>Genesis: Standard L2 genesis</p>';
    } else {
      configHtml += `<p>Custom app: ${esc(pid)}<br>Verification: Default Verifier Contract</p>`;
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
          ${isThanos ? '<div style="margin-top:6px;font-size:11px;color:#16a34a">✓ Pre-built Docker images will be pulled</div>' : '<div id="docker-image-status" style="margin-top:6px;font-size:11px;color:var(--text-muted)">Checking Docker image...</div>'}
        </div>
      </div>
    `;

    // Thanos: force L1 to geth, hide L1 node selector
    if (isThanos) {
      const l1Select = document.getElementById('launch-l1-image');
      if (l1Select) l1Select.value = 'geth';
      const l1NodeArea = document.getElementById('l1-node-area');
      if (l1NodeArea) l1NodeArea.style.display = 'none';
    }

    setLaunchMode(launchMode);
    if (!isThanos) checkDockerImage(pid);
  }
}

function setStackType(stack) {
  selectedStackType = stack;
  document.querySelectorAll('.mode-card[data-stack]').forEach(b => {
    b.classList.toggle('active', b.dataset.stack === stack);
  });
  renderPrograms();
}

function setLaunchMode(mode) {
  launchMode = mode;
  document.querySelectorAll('.mode-card[data-mode]').forEach(b => {
    b.classList.toggle('active', b.dataset.mode === mode);
  });
  const remoteArea = document.getElementById('remote-host-area');
  if (remoteArea) remoteArea.style.display = mode === 'remote' ? 'block' : 'none';
  document.getElementById('docker-status-area').style.display = mode === 'local' ? 'block' : 'none';
  document.getElementById('ai-deploy-area').style.display = mode === 'ai-deploy' ? 'block' : 'none';
  document.getElementById('l1-node-area').style.display = mode === 'local' ? 'block' : 'none';
  document.getElementById('deploy-dir-area').style.display = (mode === 'ai-deploy') ? 'none' : 'block';

  const btn = document.getElementById('launch-deploy-btn');
  btn.textContent = mode === 'ai-deploy' ? 'Generate AI Prompt' : 'Deploy L2';

  if (mode === 'local') checkDocker();
  if (mode === 'remote') { loadHostsForLaunch(); loadRemoteKeychainKeys(); }
  if (mode === 'ai-deploy') { loadAIPresets(); onAIDeployTargetChange(); onAIL1ModeChange(); }
}

function onRemoteL1SourceChange() {
  const src = document.getElementById('remote-l1-source').value;
  document.getElementById('remote-testnet-fields').style.display = src === 'testnet' ? 'block' : 'none';
}

function onRemoteL1NetworkChange() {
  // Could add custom chain ID field here if needed
}

async function loadRemoteKeychainKeys() {
  try {
    const res = await fetch(`${API}/deployments/keychain/accounts`);
    if (!res.ok) return;
    const { keys } = await res.json();
    const sel = document.getElementById('remote-testnet-keychain-key');
    if (!sel) return;
    const current = sel.value;
    sel.innerHTML = '<option value="">Select key...</option>';
    (keys || []).forEach(k => {
      const opt = document.createElement('option');
      opt.value = k.name; opt.textContent = k.name;
      if (k.name === current) opt.selected = true;
      sel.appendChild(opt);
    });
  } catch {}
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
    // For testnet, set L1 chain ID from network preset
    if (info.chainId) {
      getL1ChainIdEl().value = info.chainId;
    } else {
      getL1ChainIdEl().value = '';
      getL1ChainIdEl().placeholder = 'Enter L1 Chain ID';
    }
    if (getL1ChainIdArea()) getL1ChainIdArea().style.display = 'block';
  } else {
    // Local L1 — chain ID is auto-generated by server (set in selectProgram)
    // Just show the L1 chain ID area
    if (getL1ChainIdArea()) getL1ChainIdArea().style.display = '';
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
    const body = {
      programSlug: selectedProgram.program_id || selectedProgram.id,
      name,
      stackType: selectedStackType,
      chainId: parseInt(document.getElementById('launch-chain-id').value) || undefined,
      config: {
        mode: launchMode === 'ai-deploy' ? 'ai-deploy' : launchMode,
        l1Image: launchMode === 'local' ? (document.getElementById('launch-l1-image')?.value || 'ethrex') : undefined,
        deployDir: (document.getElementById('launch-deploy-dir')?.value || '').trim() || undefined,
        l1ChainId: parseInt(getL1ChainIdEl()?.value) || parseInt(document.getElementById('ai-l1-chain-id')?.value) || undefined,
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

      // Remote + Testnet configuration
      const remoteL1Source = document.getElementById('remote-l1-source')?.value;
      if (remoteL1Source === 'testnet') {
        const remoteRpc = (document.getElementById('remote-testnet-rpc')?.value || '').trim();
        const remoteKey = (document.getElementById('remote-testnet-keychain-key')?.value || '').trim();
        const remoteNetwork = document.getElementById('remote-l1-network')?.value || 'sepolia';
        if (!remoteRpc) { showLaunchError('L1 RPC URL is required for remote testnet deployment'); btn.disabled = false; btn.textContent = 'Deploy L2'; return; }
        if (!remoteKey) { showLaunchError('Select a deployer key from Keychain'); btn.disabled = false; btn.textContent = 'Deploy L2'; return; }
        const remoteNetInfo = TESTNET_NETWORKS[remoteNetwork] || {};
        body.config.mode = 'testnet';
        body.config.testnet = {
          l1RpcUrl: remoteRpc,
          keychainKeyName: remoteKey,
          l1ChainId: remoteNetInfo.chainId,
          network: remoteNetwork === 'custom-l1' ? 'custom' : remoteNetwork,
        };
        body.rpcUrl = remoteRpc;
      }
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

    if (launchMode === 'ai-deploy') {
      await generateAndShowAIPrompt(launchDeploymentId);
      btn.disabled = false;
      btn.textContent = document.getElementById('ai-cloud')?.value === 'local-docker' ? 'Deploy L2' : 'Generate AI Prompt';
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
    btn.textContent = launchMode === 'ai-deploy' ? 'Generate AI Prompt' : 'Deploy L2';
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
    if (config.l1ChainId) rows.push(['L1 Chain ID', config.l1ChainId]);
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
  let steps;
  if (selectedStackType === 'thanos') {
    steps = isTestnetL1() ? THANOS_TESTNET_STEPS : THANOS_LOCAL_STEPS;
  } else {
    steps = launchMode === 'remote' ? REMOTE_STEPS : (isTestnetL1() ? TESTNET_STEPS : LOCAL_STEPS);
  }
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
        // Restore extra data (imageFound, contract addresses) from stored events
        if (ev.data) {
          try {
            const extra = typeof ev.data === 'string' ? JSON.parse(ev.data) : ev.data;
            if (extra.imageFound) buildingImageFound = extra.imageFound;
            if (extra.bridgeAddress) deployedContracts['CommonBridge'] = extra.bridgeAddress;
            if (extra.proposerAddress) deployedContracts['OnChainProposer'] = extra.proposerAddress;
            if (extra.timelockAddress) deployedContracts['Timelock'] = extra.timelockAddress;
            if (extra.sp1VerifierAddress) deployedContracts['SP1Verifier'] = extra.sp1VerifierAddress;
            if (extra.guestProgramRegistryAddress) deployedContracts['GuestProgramRegistry'] = extra.guestProgramRegistryAddress;
          } catch {}
        }
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

    // Phase 1: Render immediately with DB data (remote running → show as "checking")
    list.forEach(d => {
      const dc = d.config ? (typeof d.config === 'string' ? JSON.parse(d.config) : d.config) : {};
      const isRemote = dc.cloud === 'aws' || !!d.host_id || dc.mode === 'ai-deploy';
      if (isRemote && d.phase === 'running') { d.phase = 'checking'; d.status = 'checking'; }
    });

    container.innerHTML = `
      <table class="data-table">
        <thead>
          <tr>
            <th style="width:40px;padding-left:20px"></th>
            <th>Name</th>
            <th>Status</th>
            <th>Network</th>
            <th>Phase</th>
            <th style="text-align:right">Actions</th>
          </tr>
        </thead>
        <tbody>
          ${list.map(d => renderDeploymentRow(d)).join('')}
        </tbody>
      </table>`;

    // Phase 2: Background status reconciliation (non-blocking)
    Promise.all(list.map(async (d) => {
      const dc = d.config ? (typeof d.config === 'string' ? JSON.parse(d.config) : d.config) : {};
      const isRemote = dc.cloud === 'aws' || !!d.host_id || dc.mode === 'ai-deploy';
      if ((!d.docker_project && !isRemote) || isDeploying(d.phase)) return;
      let changed = false;
      try {
        const ctrl = new AbortController();
        const timer = setTimeout(() => ctrl.abort(), 15000);
        const statusRes = await fetch(`${API}/deployments/${d.id}/status`, { signal: ctrl.signal });
        clearTimeout(timer);
        const statusData = await statusRes.json();
        const containers = statusData.containers || [];
        if (statusData.phase === 'unreachable') {
          d.phase = 'unreachable'; d.status = 'unreachable'; changed = true;
        } else {
          const coreServices = ['tokamak-app-l1', 'tokamak-app-l2', 'tokamak-app-prover'];
          const coreContainers = containers.filter(c => coreServices.includes(c.Service));
          const anyRunning = coreContainers.some(c => (c.State || c.state) === 'running');
          if (anyRunning) {
            d.phase = 'running'; d.status = 'active'; changed = true;
          } else {
            d.phase = 'stopped'; d.status = 'configured'; changed = true;
          }
        }
      } catch {
        if (isRemote && d.phase !== 'unreachable') {
          d.phase = 'unreachable'; d.status = 'unreachable'; changed = true;
        }
      }
      // Update just this row if status changed
      if (changed) {
        const row = document.querySelector(`tr.deploy-row[data-id="${d.id}"]`);
        if (row) row.outerHTML = renderDeploymentRow(d);
      }
    }));
  } catch {
    document.getElementById('deployments-list').innerHTML = '<p class="empty-state">Failed to load deployments</p>';
  }
}

function renderDeploymentRow(d) {
  const isExpanded = expandedDeploymentId === d.id;
  const hasError = !!d.error_message;
  const statusClass = hasError ? 'error' : d.phase === 'running' ? 'running'
    : (d.phase === 'unreachable' || d.phase === 'checking') ? 'building'
    : d.phase === 'configured' ? 'configured'
    : ['building','pulling','l1_starting','deploying_contracts','verifying_contracts','l2_starting','starting_prover','starting_tools','checking_docker'].includes(d.phase) ? 'building' : 'stopped';
  const rowConfig = d.config ? (typeof d.config === 'string' ? JSON.parse(d.config) : d.config) : {};
  const isTestnet = rowConfig.mode === 'testnet';
  const l1ChainId = d.l1_chain_id || rowConfig.testnet?.l1ChainId || rowConfig.l1ChainId || (!isTestnet ? '9' : '');
  const l2ChainId = d.chain_id || rowConfig.chainId || '';
  const chainIds = [l1ChainId ? `L1:${l1ChainId}` : '', l2ChainId ? `L2:${l2ChainId}` : ''].filter(Boolean).join(' · ');
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
      <td style="font-size:12px;font-family:monospace;line-height:1.9">
        ${chainIds ? `<div><span style="color:#6b7280;font-size:10px;font-weight:600;font-family:sans-serif;margin-right:6px">Chain</span><span style="color:#374151">${chainIds}</span></div>` : ''}
        <div><span style="color:#6b7280;font-size:10px;font-weight:600;font-family:sans-serif;margin-right:6px">Ports</span><span style="color:#374151">${ports}</span></div>
      </td>
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
    running: 'Running', stopped: 'Stopped', error: 'Error', unreachable: 'Unreachable', checking: 'Checking...',
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
    configured: 'Not deployed', 'ai-deploy': 'Deploying (AI)', checking_docker: 'Checking Docker', building: 'Building',
    pulling: 'Pulling Images', l1_starting: 'Starting L1', deploying_contracts: 'Deploying',
    verifying_contracts: 'Verifying', l2_starting: 'Starting L2', starting_prover: 'Starting Prover',
    starting_op_node: 'Starting op-node', starting_batcher: 'Starting op-batcher', starting_proposer: 'Starting op-proposer',
    starting_tools: 'Starting Tools',
    running: 'Running', stopped: 'Stopped', error: 'Error', unreachable: 'Unreachable', checking: 'Checking...',
  };
  const animating = ['ai-deploy','checking','checking_docker','building','pulling','l1_starting','deploying_contracts','verifying_contracts','l2_starting','starting_prover','starting_op_node','starting_batcher','starting_proposer','starting_tools'];
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
let detailHost = null;
let detailServerCheck = null;
let detailTab = 'overview';

async function showDeploymentDetail(id) {
  currentDeploymentId = id;
  showView('detail');
  detailDeployment = null;
  detailStatus = null;
  detailMonitoring = null;
  detailContracts = null;
  detailHost = null;
  detailServerCheck = null;
  detailTab = 'overview';
  if (detailPollInterval) { clearInterval(detailPollInterval); detailPollInterval = null; }
  if (logEventSource) { logEventSource.close(); logEventSource = null; }

  // Reset tab buttons to overview active
  document.querySelectorAll('.tab-btn').forEach(btn => btn.classList.toggle('active', btn.dataset.tab === 'overview'));

  try {
    const res = await fetch(`${API}/deployments/${id}`);
    const data = await res.json();
    detailDeployment = data.deployment || data;

    // AI Deploy phase — show chat UI directly instead of dashboard
    if (detailDeployment.phase === 'ai-deploy') {
      renderAIDeployOverview(detailDeployment);
      return;
    }

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
  const dc = d.config ? (typeof d.config === 'string' ? JSON.parse(d.config) : d.config) : {};
  const isRemoteDetail = dc.cloud === 'aws' || !!d.host_id || dc.mode === 'ai-deploy';
  const initialPhase = (isRemoteDetail && d.phase === 'running') ? 'checking' : d.phase;
  document.getElementById('detail-phase').innerHTML = renderPhaseBadge(initialPhase);

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

// ---------------------------------------------------------------------------
// AI Deploy Overview — shown in My L2 detail when phase === 'ai-deploy'
// ---------------------------------------------------------------------------
function renderAIDeployOverview(d) {
  const config = typeof d.config === 'string' ? JSON.parse(d.config || '{}') : (d.config || {});
  const savedPrompt = config.prompt || '';
  if (!savedPrompt) return; // No prompt saved — can't restore

  // Switch to launch view directly (skip loadPrograms/resetLaunchForm — not needed)
  document.querySelectorAll('.view').forEach(v => v.classList.remove('active'));
  const launchView = document.getElementById('view-launch');
  if (launchView) launchView.classList.add('active');
  document.querySelectorAll('.launch-step').forEach(s => s.style.display = 'none');
  const step3 = document.getElementById('launch-step3');
  if (step3) step3.style.display = 'block';

  // Store deployment ID for completion/cancel
  window._aiDeployDetailId = d.id;

  // Call showAIPromptResult with saved prompt and full context from config
  showAIPromptResult(savedPrompt, {
    cloud: config.cloud || 'aws',
    l1Mode: config.l1Mode || 'local',
    l1Network: config.l1Network || '',
    l1RpcUrl: config.l1RpcUrl || '',
    l1ChainId: config.l1ChainId || '',
    l2Name: d.name,
    l2ChainId: d.chain_id,
    programName: config.programName || d.program_slug || 'zk-dex',
    includeProver: config.includeProver !== false,
    cliInfo: { auth: { account: config.awsAccount || '' } },
    // Pass AWS-specific fields so configLines matches the original
    _awsRegion: config.region || '',
    _awsInstanceType: config.vmType || '',
    _awsStorageGB: config.storageGB || '',
    _awsKeyPair: config.keyPairName || '',
  });
}

async function monitorAIDeployment(deploymentId) {
  const statusEl = document.getElementById('ai-deploy-monitor-status');
  const resultEl = document.getElementById('ai-deploy-monitor-result');
  if (statusEl) statusEl.textContent = 'AWS CLI로 확인 중...';
  try {
    const res = await fetch(`${API}/deployments/ai-deploy/monitor`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ deploymentId }),
    });
    const data = await res.json();
    const lines = [];
    const state = data.ec2?.State || 'not_found';
    if (state === 'not_found') {
      lines.push('⚪ EC2 인스턴스 미발견 — 아직 생성되지 않았습니다');
    } else {
      const icon = state === 'running' ? '🟢' : state === 'stopped' ? '🟡' : '🔴';
      lines.push(`${icon} EC2: ${state} (${data.ec2.Type || ''}, ${data.ec2.IP || 'no IP'})`);
      if (data.ec2.Id) lines.push(`Instance ID: ${data.ec2.Id}`);
    }
    if (data.containers && data.containers.length > 0) {
      lines.push('');
      lines.push('📦 컨테이너:');
      data.containers.forEach(c => {
        const icon = c.status?.startsWith('Up') ? '✅' : c.status?.includes('Exited (0)') ? '☑️' : '❌';
        lines.push(`  ${icon} ${c.name} — ${c.status}`);
      });
    } else if (state === 'running' && !data.containers) {
      lines.push('');
      lines.push('📦 컨테이너: SSH 연결 불가');
    }
    if (data.services && Object.keys(data.services).length > 0) {
      lines.push('');
      lines.push('🌐 서비스:');
      for (const [name, svc] of Object.entries(data.services)) {
        lines.push(svc.ok ? `  ✅ ${name}${svc.block !== undefined ? ` (block #${svc.block})` : ''}` : `  ❌ ${name}`);
      }
    }
    // Show in chat messages if available
    const statusMsg = `🖥️ 배포 상태 (${data.vmName || 'unknown'})\n\n${lines.join('\n')}`;
    aiChatMessages.push({ role: 'assistant', content: statusMsg });
    if (document.getElementById('ai-chat-messages')) renderChatMessages();
    // Also show in status bar
    const ec2Status = state === 'running' ? '🟢 Running' : state === 'not_found' ? '⚪ 미발견' : `🟡 ${state}`;
    if (statusEl) statusEl.textContent = `${ec2Status}${data.ec2?.IP ? ' · ' + data.ec2.IP : ''}`;
  } catch (e) {
    if (statusEl) statusEl.textContent = `❌ ${e.message}`;
  }
}

async function saveAndCompleteDeployment(deploymentId, ip) {
  try {
    // First, fetch contract addresses from the latest monitor data
    let contracts = null;
    try {
      const dep = await fetch(`${API}/deployments/${deploymentId}`).then(r => r.json());
      const config = dep.deployment?.config ? JSON.parse(dep.deployment.config) : {};
      const monRes = await fetch(`${API}/deployments/ai-deploy/monitor`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ deploymentId }),
      });
      const monData = await monRes.json();
      if (monData.contracts) contracts = monData.contracts;
    } catch { /* best-effort */ }

    // Save IP, RPC URL, and contract addresses to deployment
    const updateData = {
      phase: 'running',
      status: 'running',
      rpc_url: `http://${ip}:1729`,
      l2_port: 1729,
      l1_port: 8545,
    };
    if (contracts?.bridge) updateData.bridge_address = contracts.bridge;
    if (contracts?.proposer) updateData.proposer_address = contracts.proposer;
    if (contracts?.timelock) updateData.timelock_address = contracts.timelock;
    if (contracts?.sp1Verifier) updateData.sp1_verifier_address = contracts.sp1Verifier;
    if (contracts?.guestProgramRegistry) updateData.guest_program_registry_address = contracts.guestProgramRegistry;

    await fetch(`${API}/deployments/${deploymentId}`, {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(updateData),
    });
    // Clear chat state
    window._aiDeployDetailId = null;
    aiChatRawPrompt = '';
    aiChatMessages = [];
    // Navigate to detail view
    showView('deployments');
    setTimeout(() => showDeploymentDetail(deploymentId), 300);
  } catch (e) {
    alert(`저장 실패: ${e.message}`);
  }
}

async function confirmAIDeployComplete(deploymentId) {
  if (!confirm('배포가 완료되었나요? 완료 확인 후 상태가 "running"으로 변경됩니다.')) return;
  try {
    const res = await fetch(`${API}/deployments/${deploymentId}`, {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ phase: 'running', status: 'running' }),
    });
    if (!res.ok) throw new Error('Failed to update');
    showDeploymentDetail(deploymentId);
  } catch (e) {
    alert(`오류: ${e.message}`);
  }
}

async function cancelAIDeployment(deploymentId) {
  if (!confirm('이 배포를 취소하시겠습니까?')) return;
  try {
    const res = await fetch(`${API}/deployments/${deploymentId}`, {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ phase: 'configured', status: 'configured' }),
    });
    if (!res.ok) throw new Error('Failed to update');
    showView('deployments');
  } catch (e) {
    alert(`오류: ${e.message}`);
  }
}

function renderOverviewTab() {
  const d = detailDeployment;
  if (!d) return;

  const dCfg = d.config ? (typeof d.config === 'string' ? JSON.parse(d.config) : d.config) : {};
  const isAIDeployOverview = dCfg.mode === 'ai-deploy';
  const isProvisioned = !!d.docker_project || isAIDeployOverview;
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

  // Remote host info
  const isRemote = !!detailHost;
  const remoteHostIp = detailHost ? detailHost.hostname : null;

  // Helper: render service row
  function svcRow(label, svcName, endpoint, isTools) {
    const state = svcState(svcName);
    const running = state === 'running';
    const dot = `<span style="width:7px;height:7px;border-radius:50%;background:${running ? 'var(--green-500)' : 'var(--text-muted)'};flex-shrink:0"></span>`;
    const stateText = `<span style="font-size:11px;color:${running ? 'var(--green-600)' : 'var(--text-muted)'}">${state}</span>`;
    const openIcon = `<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="vertical-align:-1px;cursor:pointer"><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></svg>`;
    // Use public_domain or remote host IP instead of localhost for remote deployments
    const urlHost = isRemote ? (d.public_domain || remoteHostIp) : 'localhost';
    const fullUrl = endpoint ? `http://${urlHost}${endpoint}` : null;
    const epLabel = isRemote && fullUrl ? fullUrl : endpoint;
    const ep = endpoint
      ? (running
        ? `<span style="font-size:11px;font-family:monospace;color:var(--blue-600)">${esc(epLabel)}</span> <a href="${escAttr(fullUrl)}" target="_blank" title="Open in browser" style="color:var(--blue-600)">${openIcon}</a>`
        : `<span style="font-size:11px;font-family:monospace;color:var(--text-muted)">${esc(epLabel)}</span>`)
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

  // Detect testnet mode (reuse dCfg from above)
  const dConfig = dCfg;
  const isTestnetDeploy = dConfig.mode === 'testnet';

  // For AI Deploy, use default ports if DB ports are null
  const effL1Port = d.l1_port || (isAIDeployOverview ? 8545 : null);
  const effL2Port = d.l2_port || (isAIDeployOverview ? 1729 : null);
  const effExplorerL1Port = d.tools_l1_explorer_port || (isAIDeployOverview ? 8083 : null);
  const effExplorerL2Port = d.tools_l2_explorer_port || (isAIDeployOverview ? 8082 : null);
  const effDashboardPort = d.tools_bridge_ui_port || (isAIDeployOverview ? 3000 : null);

  // Core services
  html += '<div style="font-size:10px;font-weight:600;text-transform:uppercase;letter-spacing:0.05em;color:var(--text-muted);margin-bottom:4px">Core</div>';
  if (!isTestnetDeploy) {
    html += svcRow('L1 Node', 'tokamak-app-l1', effL1Port ? `:${effL1Port}` : null);
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
  html += svcRow('L2 Node', 'tokamak-app-l2', effL2Port ? `:${effL2Port}` : null);
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
    html += svcRow('L1 Explorer', 'frontend-l1', effExplorerL1Port ? `:${effExplorerL1Port}` : null, true);
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
  html += svcRow('L2 Explorer', 'frontend-l2', effExplorerL2Port ? `:${effExplorerL2Port}` : null, true);
  html += svcRow('Dashboard', 'bridge-ui', effDashboardPort ? `:${effDashboardPort}` : null, true);

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
    html += '<button class="btn-secondary pa-edit-btn" style="padding:3px 10px;font-size:10px" data-id="' + d.id + '" data-domain="' + esc(d.public_domain) + '">Edit</button>';
    html += '<button class="btn-secondary pa-disable-btn" style="padding:3px 10px;font-size:10px;color:var(--orange-600,#ea580c)" data-id="' + d.id + '">Disable</button>';
    html += '</div>';
  } else {
    html += '<p style="font-size:11px;color:var(--text-muted);margin-bottom:8px">Allow external users to access Dashboard, Bridge, Explorer, and L2 RPC via public domain or IP.</p>';
    html += '<button class="btn-secondary pa-edit-btn" style="padding:4px 12px;font-size:11px" ' + (isProvisioned ? '' : 'disabled title="Deploy first"') + ' data-id="' + d.id + '">Enable Public Access</button>';
  }
  html += '</div>';

  html += `<button class="btn-danger" style="font-size:11px;padding:6px 12px;align-self:flex-start" onclick="deleteDeployment('${d.id}', event)">Remove L2</button>`;
  html += '</div>'; // end right

  html += '</div>'; // close overview-grid

  // Server Info card — full width below grid (remote/AI-deploy only)
  if (isRemote && detailHost) {
    const cpIcon = '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>';
    const cmdRow = (label, cmd) => `<div style="display:flex;align-items:center;gap:8px;margin-bottom:4px">
      <span style="font-size:11px;color:var(--text-muted);width:100px;flex-shrink:0">${label}</span>
      <code style="font-size:11px;flex:1;min-width:0;overflow-x:auto;white-space:nowrap;background:var(--bg);padding:4px 8px;border-radius:4px">${esc(cmd)}</code>
      <button data-copy="${esc(cmd).replace(/"/g,'&quot;')}" onclick="copyCmd(this)" style="background:none;border:none;cursor:pointer;color:var(--text-muted);padding:2px;flex-shrink:0" title="Copy">${cpIcon}</button>
    </div>`;
    const hasIP = detailHost.hostname && detailHost.hostname !== '(pending)';
    const sshKeyPath = detailHost.keyPairName ? `~/.ssh/${detailHost.keyPairName}.pem` : '~/.ssh/key';
    const deployDir = detailHost.deployDir || `/opt/tokamak/${d.id}`;
    const cloudLabel = detailHost.cloud ? detailHost.cloud.toUpperCase() : 'Remote';
    html += `<div class="card" style="margin-top:14px">
      <h3 style="font-size:13px;margin-bottom:8px">Server Info</h3>
      <div style="display:flex;align-items:center;gap:6px;margin-bottom:10px;flex-wrap:wrap">
        <span style="background:var(--blue-100,#dbeafe);color:var(--blue-700,#1d4ed8);padding:2px 8px;border-radius:10px;font-size:10px;font-weight:600">${esc(cloudLabel)}</span>
        ${hasIP ? '<span style="font-size:12px;font-family:monospace;color:var(--text-secondary);font-weight:600">' + esc(detailHost.hostname) + '</span>' : '<span style="font-size:11px;color:var(--orange-500,#f97316)">IP not assigned yet</span>'}
        ${detailHost.vmName ? '<span style="font-size:10px;color:var(--text-muted)">(' + esc(detailHost.vmName) + ')</span>' : ''}
        ${detailHost.region ? '<span style="font-size:10px;color:var(--text-muted)">' + esc(detailHost.region) + '</span>' : ''}
        ${detailHost.vmType ? '<span style="font-size:10px;color:var(--text-muted)">' + esc(detailHost.vmType) + '</span>' : ''}
      </div>`;
    if (hasIP) {
      const composeProject = d.docker_project || detailHost.vmName || d.id;
      const sshCmd = `ssh -i ${sshKeyPath} ${detailHost.username}@${detailHost.hostname}`;
      const dockerCmd = `cd ${deployDir} && docker compose -p ${composeProject} ps`;
      const logsCmd = `cd ${deployDir} && docker compose -p ${composeProject} logs -f --tail 50`;
      html += cmdRow('SSH', sshCmd);
      html += cmdRow('Deploy Dir', deployDir);
      html += cmdRow('Docker Status', dockerCmd);
      html += cmdRow('View Logs', logsCmd);
    } else {
      if (detailHost.keyPairName) html += `<div style="font-size:11px;color:var(--text-muted);margin-bottom:4px">Key Pair: <code style="font-size:10px">${esc(detailHost.keyPairName)}</code></div>`;
      html += `<div style="font-size:11px;color:var(--text-muted);margin-bottom:4px">Deploy Dir: <code style="font-size:10px">${esc(deployDir)}</code></div>`;
    }
    html += `<div style="margin-top:10px;padding-top:8px;border-top:1px solid var(--border-light);display:flex;align-items:center;gap:8px;flex-wrap:wrap">
      <button class="btn-secondary" style="padding:4px 10px;font-size:11px" onclick="checkAIDeployStatus('${d.id}',this)">Check Server Status</button>`;
    if (detailServerCheck) {
      const color = detailServerCheck.ok ? 'var(--green-600,#16a34a)' : 'var(--red-500,#ef4444)';
      html += `<span style="font-size:11px;color:${color}">${esc(detailServerCheck.msg)}</span>`;
    }
    html += '</div>';
    html += '</div>';
  }
  dynamicEl.innerHTML = html;
}

async function checkAIDeployStatus(deployId, btn) {
  btn.disabled = true;
  btn.textContent = 'Checking...';
  try {
    const r = await fetch(`${API}/deployments/ai-deploy/monitor`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ deploymentId: deployId }),
    });
    const data = await r.json();
    let msg = '';
    if (data.ec2) {
      msg += `EC2: ${data.ec2.State || 'unknown'}`;
      if (data.ec2.IP) msg += ` (${data.ec2.IP})`;
    }
    if (data.containers && data.containers.length > 0) {
      const running = data.containers.filter(c => c.status && c.status.startsWith('Up'));
      msg += ` | Containers: ${running.length}/${data.containers.length} running`;
    }
    if (data.services) {
      const okSvcs = Object.entries(data.services).filter(([,v]) => v.ok).map(([k]) => k);
      if (okSvcs.length > 0) msg += ` | OK: ${okSvcs.join(', ')}`;
    }
    detailServerCheck = { msg: msg || 'No data', ok: true, data };
  } catch (e) {
    detailServerCheck = { msg: `Error: ${e.message}`, ok: false };
  }
  // Refresh deployment data (monitor may have saved ec2IP to config)
  try {
    const dRes = await fetch(`${API}/deployments/${deployId}`);
    if (dRes.ok) { const dd = await dRes.json(); detailDeployment = dd.deployment || dd; }
  } catch {}
  await fetchDetailStatus();
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
  if (!currentDeploymentId) return;
  const ddConfig = detailDeployment?.config ? (typeof detailDeployment.config === 'string' ? JSON.parse(detailDeployment.config) : detailDeployment.config) : {};
  const isAIDeployMode = ddConfig.mode === 'ai-deploy';
  if (!detailDeployment?.docker_project && !isAIDeployMode) return;
  try {
    const [sRes, mRes] = await Promise.all([
      fetch(`${API}/deployments/${currentDeploymentId}/status`),
      fetch(`${API}/deployments/${currentDeploymentId}/monitoring`),
    ]);
    if (sRes.ok) {
      detailStatus = await sRes.json();
      detailHost = detailStatus.host || null;
    }
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
  const logsCfg = detailDeployment?.config ? (typeof detailDeployment.config === 'string' ? JSON.parse(detailDeployment.config) : detailDeployment.config) : {};
  const logsIsAIDeploy = logsCfg.mode === 'ai-deploy';
  if (!detailDeployment?.docker_project && !logsIsAIDeploy) {
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
  const toml = `# App Registry Configuration\n# Generated by Tokamak App Store for: ${d.name}\n\ndefault_program = "${slug}"\nenabled_programs = ["${slug}"]`;

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

  // AI Deploy cloud config section
  let cloudHtml = '';
  if (config.mode === 'ai-deploy') {
    const ip = config.ec2IP || '(not assigned)';
    const vmName = config.vmName || '-';
    const region = config.region || '-';
    const vmType = config.vmType || '-';
    const cloud = (config.cloud || 'aws').toUpperCase();
    const keyPair = config.keyPairName || '-';
    const storage = config.storageGB || '-';
    const l1Mode = config.l1Mode || 'local';
    const includeProver = config.includeProver !== false ? 'Yes' : 'No';
    cloudHtml = `
    <div class="card" style="margin-bottom:24px">
      <h3 style="margin-bottom:12px">Cloud Deployment</h3>
      <dl class="info-grid">
        <dt>Provider</dt><dd><span style="background:var(--blue-100,#dbeafe);color:var(--blue-700,#1d4ed8);padding:1px 6px;border-radius:8px;font-size:11px;font-weight:600">${esc(cloud)}</span></dd>
        <dt>Server IP</dt><dd><code style="font-size:12px">${esc(ip)}</code></dd>
        <dt>VM Name</dt><dd><code style="font-size:11px">${esc(vmName)}</code></dd>
        <dt>Region</dt><dd>${esc(region)}</dd>
        <dt>Instance Type</dt><dd>${esc(vmType)}</dd>
        <dt>Storage</dt><dd>${esc(String(storage))} GB</dd>
        <dt>Key Pair</dt><dd><code style="font-size:11px">${esc(keyPair)}</code></dd>
        <dt>L1 Mode</dt><dd>${esc(l1Mode)}</dd>
        <dt>Prover</dt><dd>${esc(includeProver)}</dd>
        ${config.ec2InstanceId ? '<dt>Instance ID</dt><dd><code style="font-size:11px">' + esc(config.ec2InstanceId) + '</code></dd>' : ''}
      </dl>
    </div>`;
  }

  document.getElementById('tab-config').innerHTML = `
    <div class="card" style="margin-bottom:24px">
      <h3 style="margin-bottom:12px">App Configuration</h3>
      <dl class="info-grid">
        <dt>App</dt><dd>${esc(slug)}</dd>
        <dt>Program Name</dt><dd>${esc(d.program_name || '')}</dd>
        <dt>Deploy Mode</dt><dd><span class="mode-badge ${config.mode || 'local'}">${esc(config.mode || 'local')}</span></dd>
      </dl>
    </div>
    ${cloudHtml}
    ${testnetHtml}
    <div class="card" style="margin-bottom:24px">
      <h3 style="margin-bottom:12px">Configuration Files</h3>
      <p style="font-size:14px;color:var(--gray-500);margin-bottom:16px">Download configuration files to run an ethrex L2 node with this app.</p>
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
        <div style="margin-bottom:12px"><p style="font-weight:500;color:var(--gray-700);margin-bottom:4px">2. Run with app</p><pre class="config-pre">make -C crates/l2 init-guest-program PROGRAM=${esc(slug)}</pre></div>
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
// Copy text to clipboard, show "Copied!" feedback on the button
function copyCmd(btn) {
  const text = btn.getAttribute('data-copy');
  if (!text) return;
  const original = btn.innerHTML;
  navigator.clipboard.writeText(text).then(() => {
    btn.textContent = 'Copied!';
    setTimeout(() => { btn.innerHTML = original; }, 1500);
  }).catch(() => {
    btn.textContent = 'Failed';
    setTimeout(() => { btn.innerHTML = original; }, 1500);
  });
}

function esc(str) {
  const div = document.createElement('div');
  div.textContent = str || '';
  return div.innerHTML;
}

// Escape for use inside HTML attributes (also escapes quotes)
function escAttr(str) {
  return esc(str).replace(/"/g, '&quot;').replace(/'/g, '&#39;');
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

// Delegated click handlers for public access buttons (avoids inline onclick with user data)
document.addEventListener('click', (e) => {
  const editBtn = e.target.closest('.pa-edit-btn');
  if (editBtn) { showPublicAccessModal(editBtn.dataset.id, editBtn.dataset.domain); return; }
  const disableBtn = e.target.closest('.pa-disable-btn');
  if (disableBtn) { disablePublicAccess(disableBtn.dataset.id, disableBtn); return; }
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
    const resp = await fetch(`${API}/deployments/${deploymentId}/public-access`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
    });
    if (!resp.ok) { const err = await resp.json(); throw new Error(err.error || 'Server error'); }
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
    const resp = await fetch(`${API}/deployments/${deploymentId}/public-access`, { method: 'DELETE' });
    if (!resp.ok) { const err = await resp.json(); throw new Error(err.error || 'Server error'); }
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
// ---------------------------------------------------------------------------
// AI Deploy helpers
// ---------------------------------------------------------------------------

async function loadAIPresets() {
  const statusEl = document.getElementById('ai-key-status');
  const mainEl = document.getElementById('ai-deploy-main');
  if (!statusEl) return;

  const optMessenger = document.getElementById('ai-opt-messenger');
  const manualRadio = document.getElementById('ai-manual-radio');
  const keySetup = document.getElementById('ai-key-setup');

  // 1. Check Manager's current AI config
  let ai = null;
  try {
    const res = await fetch(`${API}/deployments/ai-deploy/ai-config`);
    if (res.ok) ai = await res.json();
  } catch {}

  // 2. Check Messenger Keychain
  let messengerHasKey = false;
  try {
    const mRes = await fetch(`${API}/deployments/ai-deploy/messenger-ai-config`);
    if (mRes.ok) {
      const messenger = await mRes.json();
      if (messenger.available) {
        messengerHasKey = true;
        const provLabel = { claude: 'Claude', gpt: 'ChatGPT', gemini: 'Gemini' }[messenger.provider] || messenger.provider;
        const detail = document.getElementById('ai-messenger-detail');
        if (detail) detail.textContent = `${provLabel}${messenger.model ? ' · ' + messenger.model : ''} (${messenger.maskedKey})`;
      }
    }
  } catch {}

  // 3. Show UI based on state
  if (messengerHasKey) {
    // Show both options: messenger / manual
    if (optMessenger) optMessenger.style.display = 'block';
    if (manualRadio) manualRadio.style.display = 'block';
    const selected = document.querySelector('input[name="ai-source"]:checked');
    if (!selected) {
      const defaultSource = (ai && ai.configured) ? 'manual' : 'messenger';
      const radio = document.querySelector(`input[name="ai-source"][value="${defaultSource}"]`);
      if (radio) { radio.checked = true; onAISourceChange(defaultSource, true); }
    }
  } else {
    // No messenger key → hide messenger option, show key input directly
    if (optMessenger) optMessenger.style.display = 'none';
    if (manualRadio) manualRadio.style.display = 'none';
    if (keySetup) keySetup.style.display = '';
  }

  // 4. Update status
  if (ai && ai.configured) {
    const providerLabel = { claude: 'Claude', gpt: 'ChatGPT', gemini: 'Gemini' }[ai.provider] || ai.provider;
    statusEl.innerHTML = `<span style="color:#22c55e">&#10003;</span> <b>${escapeHtml(providerLabel)}</b> 연결됨${ai.model ? ' (' + escapeHtml(ai.model) + ')' : ''}`;
    mainEl.style.display = 'block';
    const providerEl = document.getElementById('ai-provider');
    if (providerEl && ai.provider) providerEl.value = ai.provider;
  } else {
    statusEl.innerHTML = '<span style="color:var(--text-muted,#888)">API 키를 입력하세요</span>';
    mainEl.style.display = 'none';
  }
}

function onAISourceChange(source, silent) {
  const keySetup = document.getElementById('ai-key-setup');
  const statusEl = document.getElementById('ai-key-status');
  if (source === 'messenger') {
    if (keySetup) keySetup.style.display = 'none';
    // Apply messenger config
    fetch(`${API}/deployments/ai-deploy/use-messenger-ai`, { method: 'POST' })
      .then(r => { if (!r.ok) throw new Error('Failed'); return r.json(); })
      .then(() => { if (!silent) loadAIPresets(); })
      .catch(e => { if (statusEl) statusEl.innerHTML = `<span style="color:#ef4444">${escapeHtml(e.message)}</span>`; });
  } else {
    if (keySetup) keySetup.style.display = '';
  }
}

async function saveAIConfig() {
  const provider = document.getElementById('ai-provider').value;
  const apiKey = document.getElementById('ai-api-key').value.trim();
  const statusEl = document.getElementById('ai-key-status');
  if (!apiKey) { statusEl.innerHTML = '<span style="color:#ef4444">API 키를 입력해주세요</span>'; return; }

  statusEl.innerHTML = '<span style="color:var(--text-muted,#888)">저장 중...</span>';
  try {
    const res = await fetch(`${API}/deployments/ai-deploy/ai-config`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ provider, apiKey }),
    });
    if (!res.ok) throw new Error('Save failed');
    const result = await res.json();
    document.getElementById('ai-api-key').value = '';
    statusEl.innerHTML = result.messengerSynced
      ? '<span style="color:#22c55e">저장 완료 — 메신저에도 동기화됨</span>'
      : '<span style="color:#22c55e">저장 완료</span>';
    setTimeout(() => loadAIPresets(), 1000);
  } catch (e) {
    statusEl.innerHTML = `<span style="color:#ef4444">${escapeHtml(e.message)}</span>`;
  }
}

function onAIDeployTargetChange() {
  const target = document.getElementById('ai-cloud').value;
  const isLocal = target === 'local-docker';
  const isCloud = target && !isLocal;

  // Show/hide cloud-specific fields
  const gcpLabel = document.getElementById('gcp-project-label');
  if (gcpLabel) gcpLabel.style.display = target === 'gcp' ? '' : 'none';
  const vultrLabel = document.getElementById('vultr-apikey-label');
  if (vultrLabel) vultrLabel.style.display = target === 'vultr' ? '' : 'none';
  const vmOptions = document.getElementById('cloud-vm-options');
  if (vmOptions) vmOptions.style.display = target === 'aws' ? '' : 'none';
  if (target === 'aws') onCloudOptionChange();
  const statusEl = document.getElementById('cloud-cli-status');
  if (statusEl && !isCloud) statusEl.style.display = 'none';

  // Update button text: Local Docker uses direct deploy, Cloud uses AI prompt
  const btn = document.getElementById('launch-deploy-btn');
  if (btn) btn.textContent = isLocal ? 'Deploy L2' : 'Generate AI Prompt';

  // Local Docker: check docker instead of cloud CLI
  if (isLocal) {
    checkLocalDockerForAI();
  } else if (isCloud) {
    checkCloudCLI();
  }
  updateSpecRecommendation();
}

async function checkLocalDockerForAI() {
  const statusEl = document.getElementById('cloud-cli-status');
  if (!statusEl) return;
  statusEl.style.display = 'block';
  statusEl.innerHTML = '<div style="padding:10px 14px;background:var(--bg-surface,#1a1a2e);border:1px solid var(--border,#333);border-radius:8px;font-size:12px;color:var(--text-muted,#888)">Checking Docker...</div>';
  try {
    const res = await fetch(`${API}/deployments/docker/status`);
    if (!res.ok) throw new Error('Docker check failed');
    const data = await res.json();
    const ok = data.available;
    const lines = [];
    if (ok) {
      lines.push(`<div style="display:flex;align-items:center;gap:6px"><span style="color:#22c55e">&#10003;</span> <span><b>Docker</b> available</span></div>`);
    } else {
      lines.push(`<div style="display:flex;align-items:center;gap:6px"><span style="color:#ef4444">&#10007;</span> <span>Docker not found — <a href="https://www.docker.com/products/docker-desktop/" target="_blank" style="color:#60a5fa">Install Docker Desktop</a></span></div>`);
    }
    statusEl.innerHTML = `<div style="padding:10px 14px;background:var(--bg-surface,#1a1a2e);border:1px solid ${ok ? '#22c55e' : '#ef4444'};border-radius:8px;font-size:12px;line-height:1.6;color:#e2e8f0">${lines.join('')}</div>`;
  } catch {
    statusEl.innerHTML = `<div style="padding:10px 14px;background:var(--bg-surface,#1a1a2e);border:1px solid var(--border,#333);border-radius:8px;font-size:12px;color:#ef4444">Docker check failed</div>`;
  }
}

async function checkCloudCLI() {
  const cloud = document.getElementById('ai-cloud').value;
  const statusEl = document.getElementById('cloud-cli-status');
  if (!cloud || cloud === 'local-docker') { if (statusEl) statusEl.style.display = 'none'; return; }

  // Vultr also checks CLI like GCP/AWS — fall through to normal check
  statusEl.style.display = 'block';
  statusEl.innerHTML = '<div style="padding:10px 14px;background:var(--bg-surface,#1a1a2e);border:1px solid var(--border,#333);border-radius:8px;font-size:12px;color:var(--text-muted,#888)">Checking CLI...</div>';

  try {
    const awsRegion = document.getElementById('aws-region')?.value || 'ap-northeast-2';
    const res = await fetch(`${API}/deployments/ai-deploy/check-cli?cloud=${cloud}&region=${awsRegion}`);
    if (!res.ok) throw new Error('Check failed');
    const r = await res.json();

    const cliName = r.cli.name;
    const lines = [];

    // CLI install status
    if (r.cli.installed) {
      lines.push(`<div style="display:flex;align-items:center;gap:6px;margin-bottom:4px"><span style="color:#22c55e">&#10003;</span> <span><b>${cliName}</b> installed (v${r.cli.version})</span></div>`);
    } else {
      const installCmd = cloud === 'gcp'
        ? 'brew install --cask google-cloud-sdk'
        : cloud === 'vultr' ? 'brew install vultr'
        : 'brew install awscli';
      lines.push(`<div style="display:flex;align-items:center;gap:6px;margin-bottom:4px"><span style="color:#ef4444">&#10007;</span> <span><b>${cliName}</b> not found</span></div>`);
      lines.push(`<div style="display:flex;align-items:center;gap:8px;margin-left:20px;font-size:11px;color:var(--text-muted,#888)">Install: <code style="background:var(--bg-surface,#161622);padding:2px 6px;border-radius:4px">${installCmd}</code> <button onclick="checkCloudCLI()" style="padding:2px 10px;font-size:11px;border:1px solid var(--border,#444);border-radius:4px;background:var(--bg-surface,#1a1a2e);color:var(--text-muted,#ccc);cursor:pointer">Re-check</button></div>`);
    }

    // Auth status
    if (r.cli.installed) {
      if (r.auth.authenticated) {
        let detail = r.auth.account;
        if (r.auth.project) detail += ` (project: ${r.auth.project})`;
        lines.push(`<div style="display:flex;align-items:center;gap:6px;margin-bottom:4px"><span style="color:#22c55e">&#10003;</span> <span>Authenticated: ${detail}</span></div>`);
      } else if (cloud === 'vultr') {
        lines.push(`<div style="display:flex;align-items:center;gap:6px;margin-bottom:4px"><span style="color:#ef4444">&#10007;</span> <span>API Key 미설정 — 위에서 입력하세요</span></div>`);
      } else {
        const authCmd = cloud === 'gcp' ? 'gcloud auth login' : 'aws configure';
        lines.push(`<div style="display:flex;align-items:center;gap:6px;margin-bottom:4px"><span style="color:#ef4444">&#10007;</span> <span>Not authenticated</span></div>`);
        lines.push(`<div style="display:flex;align-items:center;gap:8px;margin-left:20px;font-size:11px;color:var(--text-muted,#888)">Run: <code style="background:var(--bg-surface,#161622);padding:2px 6px;border-radius:4px">${authCmd}</code> <button onclick="checkCloudCLI()" style="padding:2px 10px;font-size:11px;border:1px solid var(--border,#444);border-radius:4px;background:var(--bg-surface,#1a1a2e);color:var(--text-muted,#ccc);cursor:pointer">Re-check</button></div>`);
      }
    }

    // AWS SSH Key Pairs
    if (cloud === 'aws' && r.cli.installed && r.auth.authenticated) {
      if (r.keyPairs && r.keyPairs.length > 0) {
        const opts = r.keyPairs.map(kp => `<option value="${escapeHtml(kp.name)}">${escapeHtml(kp.name)}</option>`).join('');
        lines.push(`<div style="display:flex;align-items:center;gap:6px;margin-bottom:4px"><span style="color:#22c55e">&#10003;</span> <span>SSH Key Pairs (${r.keyPairs.length}개)</span></div>`);
        lines.push(`<div style="display:flex;align-items:center;gap:8px;margin-left:20px;font-size:11px;margin-bottom:4px"><select id="aws-key-pair-select" style="padding:3px 8px;font-size:11px;border:1px solid var(--border,#444);border-radius:4px;background:var(--bg-surface,#161622);color:#e2e8f0">${opts}</select> <span style="color:#888">또는</span> <button onclick="showCreateKeyPairUI()" style="padding:2px 10px;font-size:11px;border:1px solid #3b82f6;border-radius:4px;background:transparent;color:#60a5fa;cursor:pointer">+ 새로 만들기</button></div>`);
      } else {
        lines.push(`<div style="display:flex;align-items:center;gap:6px;margin-bottom:4px"><span style="color:#f59e0b">&#9888;</span> <span>SSH Key Pair 없음</span></div>`);
        lines.push(`<div style="display:flex;align-items:center;gap:8px;margin-left:20px;font-size:11px;margin-bottom:4px"><button onclick="showCreateKeyPairUI()" style="padding:3px 12px;font-size:11px;border:1px solid #3b82f6;border-radius:4px;background:#3b82f6;color:white;cursor:pointer">+ Key Pair 만들기</button></div>`);
      }
      lines.push(`<div id="aws-create-keypair-ui" style="display:none;margin-left:20px;margin-top:4px"></div>`);
    }

    // Auto-fill GCP project ID if detected
    if (cloud === 'gcp' && r.auth.project) {
      const projInput = document.getElementById('gcp-project-id');
      if (projInput && !projInput.value) projInput.value = r.auth.project;
    }

    // Hide Vultr API key field once authenticated
    if (cloud === 'vultr' && r.auth.authenticated) {
      const vLabel = document.getElementById('vultr-apikey-label');
      if (vLabel) vLabel.style.display = 'none';
    }

    const allGood = r.cli.installed && r.auth.authenticated;
    const borderColor = allGood ? '#22c55e' : '#ef4444';
    statusEl.innerHTML = `<div style="padding:10px 14px;background:var(--bg-surface,#1a1a2e);border:1px solid ${borderColor};border-radius:8px;font-size:12px;line-height:1.6;color:#e2e8f0">${lines.join('')}</div>`;
  } catch (e) {
    statusEl.innerHTML = `<div style="padding:10px 14px;background:var(--bg-surface,#1a1a2e);border:1px solid var(--border,#333);border-radius:8px;font-size:12px;color:#ef4444">CLI check failed: ${escapeHtml(e.message)}</div>`;
  }
  updateSpecRecommendation();
}

async function saveVultrApiKey() {
  const input = document.getElementById('vultr-api-key-input');
  const statusEl = document.getElementById('vultr-apikey-status');
  const key = input?.value?.trim();
  if (!key) { if (statusEl) statusEl.innerHTML = '<span style="color:#ef4444">API Key를 입력하세요</span>'; return; }
  if (statusEl) statusEl.innerHTML = '<span style="color:#888">저장 중...</span>';
  try {
    const res = await fetch(`${API}/deployments/ai-deploy/vultr-api-key`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ apiKey: key }),
    });
    if (!res.ok) throw new Error('Save failed');
    if (statusEl) statusEl.innerHTML = '<span style="color:#22c55e">&#10003; API Key 저장됨</span>';
    input.value = '';
    checkCloudCLI();
  } catch (e) {
    if (statusEl) statusEl.innerHTML = `<span style="color:#ef4444">${escapeHtml(e.message)}</span>`;
  }
}

const AWS_INSTANCE_PRICING = {
  't3.medium': { hr: 0.052, vcpu: 2, ram: 4, note: 'L2만 (Prover 불가)' },
  't3.large': { hr: 0.104, vcpu: 2, ram: 8, note: 'L1+L2 가능, Prover 제한적' },
  't3.xlarge': { hr: 0.208, vcpu: 4, ram: 16, note: 'L1+L2+Prover 가능' },
  't3.2xlarge': { hr: 0.416, vcpu: 8, ram: 32, note: 'Prover 여유' },
};

function calcAWSCost(instanceType, storageGB) {
  const info = AWS_INSTANCE_PRICING[instanceType] || AWS_INSTANCE_PRICING['t3.xlarge'];
  const instanceMo = info.hr * 24 * 30;
  const storageMo = storageGB * 0.096;
  const ipMo = 3.60;
  return { info, instanceMo, storageMo, ipMo, monthCost: instanceMo + storageMo + ipMo, dayCost: (info.hr * 24) + (storageMo / 30) + (ipMo / 30) };
}

function onCloudOptionChange() {
  const el = document.getElementById('aws-cost-estimate');
  if (!el) return;
  const instanceType = document.getElementById('aws-instance-type')?.value || 't3.xlarge';
  const storageGB = parseInt(document.getElementById('aws-storage-gb')?.value) || 30;
  const includeProver = document.getElementById('ai-include-prover')?.checked;
  const { info, instanceMo, storageMo, ipMo, dayCost, monthCost } = calcAWSCost(instanceType, storageGB);

  // Prover compatibility check
  let proverWarning = '';
  if (includeProver && info.ram < 16) {
    proverWarning = `<div style="color:#f59e0b;margin-top:4px">⚠️ SP1 Prover에는 최소 16GB RAM이 필요합니다. ${instanceType}은 ${info.ram}GB입니다.</div>`;
  }

  el.innerHTML = `
    <div style="display:flex;justify-content:space-between;align-items:center">
      <span><b>월 예상 비용</b> (Seoul, 24/7 기준)</span>
      <span style="color:#22c55e;font-weight:700;font-size:15px">~$${monthCost.toFixed(0)}/월</span>
    </div>
    <div style="font-size:12px;color:#a0aec0;margin-top:6px;display:flex;gap:14px">
      <span>인스턴스 ~$${instanceMo.toFixed(0)}</span>
      <span>스토리지 ${storageGB}GB ~$${storageMo.toFixed(1)}</span>
      <span>Public IP ~$${ipMo.toFixed(1)}</span>
    </div>
    <div style="font-size:12px;color:#60a5fa;margin-top:5px">
      💡 테스트용: ~$${dayCost.toFixed(1)}/일 — terminate하면 즉시 과금 중지
    </div>
    <div style="font-size:12px;color:#a0aec0;margin-top:3px">${info.note}</div>
    ${proverWarning ? proverWarning.replace('margin-top:4px', 'margin-top:6px;font-size:13px;font-weight:500') : ''}`;
  updateSpecRecommendation();
}

async function checkDeploymentStatus() {
  const statusEl = document.getElementById('ai-monitor-status');
  if (statusEl) statusEl.innerHTML = '<span style="color:#888">AWS CLI로 확인 중...</span>';

  // Get vmName and keyPair from deployment context
  const vmName = aiChatRawPrompt.match(/Value=([^\s}'"]+)/)?.[1] || '';
  const keyPairName = document.getElementById('aws-key-pair-select')?.value || aiChatRawPrompt.match(/--key-name\s+(\S+)/)?.[1] || '';
  const region = document.getElementById('aws-region')?.value || 'ap-northeast-2';

  if (!vmName) { if (statusEl) statusEl.innerHTML = '<span style="color:#f59e0b">배포 프롬프트에서 VM 이름을 찾을 수 없습니다</span>'; return; }

  try {
    const res = await fetch(`${API}/deployments/ai-deploy/monitor`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ vmName, region, keyPairName }),
    });
    const data = await res.json();

    const lines = [];
    // EC2 status
    if (data.ec2?.State === 'not_found') {
      lines.push('🔴 EC2 인스턴스 미발견 — 아직 생성되지 않았습니다');
    } else if (data.ec2) {
      const stateIcon = data.ec2.State === 'running' ? '🟢' : data.ec2.State === 'stopped' ? '🟡' : '🔴';
      lines.push(`${stateIcon} EC2: ${data.ec2.State} (${data.ec2.Type}, ${data.ec2.IP || 'no IP'})`);
      if (data.ec2.Id) lines.push(`   Instance ID: ${data.ec2.Id}`);
    }

    // Containers
    if (data.containers && data.containers.length > 0) {
      lines.push('\n📦 컨테이너:');
      data.containers.forEach(c => {
        const icon = c.status.startsWith('Up') ? '✅' : c.status.includes('Exited (0)') ? '☑️' : '❌';
        lines.push(`   ${icon} ${c.name} — ${c.status}`);
      });
    } else if (data.ec2?.State === 'running' && data.containers === null) {
      lines.push('\n📦 컨테이너: SSH 연결 불가 (아직 Docker 설치 전이거나 SSH 키 확인 필요)');
    }

    // Services
    if (Object.keys(data.services).length > 0) {
      lines.push('\n🌐 서비스:');
      for (const [name, svc] of Object.entries(data.services)) {
        if (svc.ok) {
          lines.push(`   ✅ ${name}${svc.block !== undefined ? ` (block #${svc.block})` : ''}`);
        } else {
          lines.push(`   ❌ ${name} — 응답 없음`);
        }
      }
    }

    // Contracts
    if (data.contracts) {
      const c = data.contracts;
      const hasAny = c.bridge || c.proposer || c.timelock;
      if (hasAny) {
        lines.push('\n📋 L1 컨트랙트:');
        if (c.timelock) lines.push(`   Timelock: ${c.timelock}`);
        if (c.proposer) lines.push(`   OnChainProposer: ${c.proposer}`);
        if (c.bridge) lines.push(`   CommonBridge: ${c.bridge}`);
        if (c.sp1Verifier) lines.push(`   SP1Verifier: ${c.sp1Verifier}`);
        if (c.guestProgramRegistry) lines.push(`   GuestProgramRegistry: ${c.guestProgramRegistry}`);
      }
    }

    const actualVmName = data.vmName || vmName;
    const statusMsg = `🖥️ 배포 상태 모니터링\nVM: ${actualVmName} (${region})\n\n${lines.join('\n')}`;
    aiChatMessages.push({ role: 'assistant', content: statusMsg });

    // Check if all services are healthy AND contracts are deployed
    const allServicesOk = data.ec2?.State === 'running'
      && data.containers && data.containers.length > 0
      && data.containers.every(c => c.status?.startsWith('Up') || c.status?.includes('Exited (0)'))
      && Object.keys(data.services).length > 0
      && Object.values(data.services).every(s => s.ok)
      && data.contracts?.timelock; // Timelock contract must be deployed

    if (allServicesOk && data.ec2?.IP) {
      const ip = data.ec2.IP;
      const deployId = window._aiDeployDetailId || launchDeploymentId;
      aiChatMessages.push({ role: 'assistant', content: `✅ 모든 서비스가 정상 동작합니다!\n\n| 서비스 | URL |\n|--------|-----|\n| L2 RPC | http://${ip}:1729 |\n| L2 Explorer | http://${ip}:8082 |\n| L1 Explorer | http://${ip}:8083 |\n| Dashboard | http://${ip}:3000 |\n\n아래 버튼을 누르면 배포 정보를 저장하고 대시보드 화면으로 이동합니다.` });
      // Add save button after render
      setTimeout(() => {
        const container = document.getElementById('ai-chat-messages');
        if (!container) return;
        const btnDiv = document.createElement('div');
        btnDiv.style.cssText = 'align-self:flex-start;margin-top:-4px';
        btnDiv.innerHTML = `<button onclick="saveAndCompleteDeployment('${deployId}', '${ip}')" style="padding:8px 20px;font-size:13px;border:none;border-radius:8px;background:#22c55e;color:white;cursor:pointer;font-weight:600">✅ 배포 완료 — 정보 저장</button>`;
        container.appendChild(btnDiv);
        container.scrollTop = container.scrollHeight;
      }, 100);
    }

    renderChatMessages();

    // Summary in status bar
    const state = data.ec2?.State || 'not_found';
    const ec2Status = state === 'running' ? '🟢 Running' : state === 'not_found' ? '⚪ 아직 생성 안 됨' : state === 'stopped' ? '🟡 Stopped' : `🔴 ${state}`;
    const ip = data.ec2?.IP || '';
    if (statusEl) statusEl.innerHTML = `${ec2Status}${ip ? ' · ' + ip : ''}`;
  } catch (e) {
    if (statusEl) statusEl.innerHTML = `<span style="color:#ef4444">${escapeHtml(e.message)}</span>`;
  }
}

function showCreateKeyPairUI() {
  const container = document.getElementById('aws-create-keypair-ui');
  if (!container) return;
  container.style.display = 'block';
  container.innerHTML = `
    <div style="display:flex;align-items:center;gap:6px;font-size:11px">
      <input id="aws-new-keypair-name" type="text" placeholder="tokamak-key" value="tokamak-key"
        style="padding:3px 8px;font-size:11px;border:1px solid var(--border,#444);border-radius:4px;background:var(--bg-surface,#161622);color:#e2e8f0;width:140px">
      <button onclick="createAWSKeyPair()" style="padding:3px 12px;font-size:11px;border:1px solid #22c55e;border-radius:4px;background:#22c55e;color:white;cursor:pointer">생성</button>
      <button onclick="document.getElementById('aws-create-keypair-ui').style.display='none'" style="padding:3px 8px;font-size:11px;border:1px solid var(--border,#444);border-radius:4px;background:transparent;color:#888;cursor:pointer">취소</button>
      <span id="aws-keypair-status" style="font-size:11px"></span>
    </div>`;
}

async function createAWSKeyPair() {
  const nameEl = document.getElementById('aws-new-keypair-name');
  const statusEl = document.getElementById('aws-keypair-status');
  const keyName = nameEl?.value?.trim();
  if (!keyName) { if (statusEl) statusEl.innerHTML = '<span style="color:#ef4444">이름을 입력하세요</span>'; return; }
  if (statusEl) statusEl.innerHTML = '<span style="color:#888">생성 중...</span>';
  try {
    const res = await fetch(`${API}/deployments/ai-deploy/create-key-pair`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ keyName, region: document.getElementById('aws-region')?.value || 'ap-northeast-2' }),
    });
    const data = await res.json();
    if (!res.ok) throw new Error(data.error || 'Failed');
    if (statusEl) statusEl.innerHTML = `<span style="color:#22c55e">&#10003; 생성됨 (${data.pemPath})</span>`;
    // Refresh CLI check to update key pair list
    setTimeout(() => checkCloudCLI(), 500);
  } catch (e) {
    if (statusEl) statusEl.innerHTML = `<span style="color:#ef4444">${escapeHtml(e.message)}</span>`;
  }
}

async function onAIL1ModeChange() {
  const mode = document.getElementById('ai-l1-mode').value;
  const rpcArea = document.getElementById('ai-l1-rpc-area');
  const walletArea = document.getElementById('ai-wallet-area');
  const chainIdInput = document.getElementById('ai-l1-chain-id');
  const isTestnet = mode !== 'local';
  if (rpcArea) rpcArea.style.display = isTestnet ? 'block' : 'none';
  if (walletArea) {
    walletArea.style.display = isTestnet ? 'block' : 'none';
    if (isTestnet) loadAIKeychainKeys();
  }
  if (chainIdInput) {
    if (mode === 'local') {
      // Fetch unique L1 chain ID from server (avoids conflicts with existing deployments)
      try {
        const res = await fetch(`${API}/deployments/next-chain-id`);
        if (res.ok) {
          const data = await res.json();
          chainIdInput.value = data.l1ChainId || '9';
        } else { chainIdInput.value = '9'; }
      } catch { chainIdInput.value = '9'; }
    }
    else if (mode === 'sepolia') { chainIdInput.value = '11155111'; }
    else if (mode === 'mainnet') { chainIdInput.value = '1'; }
  }
  updateSpecRecommendation();
}

const VM_SPECS = {
  gcp: {
    withProver:    { type: 'e2-standard-4',  cpu: 4, ram: '16GB', disk: '100GB SSD', price: '~$106/월' },
    withoutProver: { type: 'e2-standard-2',  cpu: 2, ram: '8GB',  disk: '50GB SSD',  price: '~$53/월' },
  },
  aws: {
    withProver:    { type: 't3.xlarge',   cpu: 4, ram: '16GB', disk: '100GB gp3', price: '~$152/월' },
    withoutProver: { type: 't3.large',    cpu: 2, ram: '8GB',  disk: '50GB gp3',  price: '~$76/월' },
  },
  vultr: {
    withProver:    { type: 'vc2-6c-16gb', cpu: 6, ram: '16GB', disk: '320GB SSD', price: '$96/월' },
    withoutProver: { type: 'vc2-4c-8gb',  cpu: 4, ram: '8GB',  disk: '160GB SSD', price: '$48/월' },
  },
};

function onProverToggle() {
  updateSpecRecommendation();
  // Auto-select appropriate instance type
  const includeProver = document.getElementById('ai-include-prover')?.checked;
  const instanceSelect = document.getElementById('aws-instance-type');
  if (instanceSelect) {
    if (includeProver && (instanceSelect.value === 't3.medium' || instanceSelect.value === 't3.large')) {
      instanceSelect.value = 't3.xlarge';
    } else if (!includeProver && instanceSelect.value === 't3.xlarge') {
      instanceSelect.value = 't3.large';
    }
  }
  onCloudOptionChange();
}

// ---------------------------------------------------------------------------
// AI Deploy — Wallet Configuration (Keychain)
// ---------------------------------------------------------------------------
async function loadAIKeychainKeys() {
  try {
    const res = await fetch(`${API}/deployments/keychain/accounts`);
    if (!res.ok) return;
    const { accounts } = await res.json();
    const keys = accounts || [];
    const keyOptions = keys.map(k => `<option value="${escapeHtml(k)}">${escapeHtml(k)}</option>`).join('');

    const deployer = document.getElementById('ai-deployer-key');
    const prevDeployer = deployer?.value;
    if (deployer) {
      deployer.innerHTML = '<option value="">Select key...</option>' + keyOptions;
      if (prevDeployer && keys.includes(prevDeployer)) deployer.value = prevDeployer;
    }
    const roleIds = ['ai-committer-key', 'ai-proof-coordinator-key', 'ai-bridge-owner-key'];
    for (const id of roleIds) {
      const sel = document.getElementById(id);
      if (!sel) continue;
      const prev = sel.value;
      sel.innerHTML = '<option value="">Same as Deployer</option>' + keyOptions;
      if (prev && keys.includes(prev)) sel.value = prev;
    }
    onAIKeyChange();
  } catch (e) {
    console.error('Failed to load AI keychain keys:', e);
  }
}

async function onAIKeyChange() {
  const deployerKey = document.getElementById('ai-deployer-key')?.value;
  if (!deployerKey) {
    ['ai-deployer-addr', 'ai-committer-addr', 'ai-proof-coordinator-addr', 'ai-bridge-owner-addr'].forEach(id => {
      const el = document.getElementById(id);
      if (el) el.textContent = '';
    });
    return;
  }
  const rpcUrl = document.getElementById('ai-l1-rpc-url')?.value?.trim() || '';
  try {
    const res = await fetch(`${API}/deployments/testnet/resolve-keys`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        rpcUrl: rpcUrl || 'https://rpc.sepolia.org',
        deployerKey,
        committerKey: document.getElementById('ai-committer-key')?.value || '',
        proofCoordinatorKey: document.getElementById('ai-proof-coordinator-key')?.value || '',
        bridgeOwnerKey: document.getElementById('ai-bridge-owner-key')?.value || '',
      }),
    });
    if (!res.ok) {
      const err = await res.json();
      const statusEl = document.getElementById('ai-wallet-status');
      if (statusEl) statusEl.innerHTML = `<span style="color:#ef4444">${escapeHtml(err.error || 'Key resolve failed')}</span>`;
      return;
    }
    const data = await res.json();
    const roles = data.roles || {};
    const setAddr = (id, role) => {
      const el = document.getElementById(id);
      if (el && role?.address) {
        el.textContent = role.address;
        if (role.balance !== undefined) el.textContent += ` (${role.balance} ETH)`;
      } else if (el) {
        el.textContent = role?.address || '';
      }
    };
    setAddr('ai-deployer-addr', roles.deployer);
    setAddr('ai-committer-addr', roles.committer);
    setAddr('ai-proof-coordinator-addr', roles.proofCoordinator);
    setAddr('ai-bridge-owner-addr', roles.bridgeOwner);

    const statusEl = document.getElementById('ai-wallet-status');
    if (statusEl) {
      if (data.deployerSufficient === false) {
        statusEl.innerHTML = `<span style="color:#f59e0b">⚠ Deployer 잔액 부족 — 예상 배포 비용: ${data.estimatedDeployCostEth || '?'} ETH</span>`;
      } else if (data.deployerSufficient) {
        statusEl.innerHTML = `<span style="color:#22c55e">✓ 잔액 확인 완료 (Gas: ${data.gasPriceGwei || '?'} gwei)</span>`;
      } else {
        statusEl.innerHTML = '';
      }
    }
  } catch (e) {
    console.error('AI key resolve error:', e);
  }
}

async function registerAIKeychainKey() {
  try {
    const res = await fetch(`${API}/open-url`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ url: 'keychain-register' }),
    });
    const data = await res.json();
    if (data.ok && data.keyName) {
      await loadAIKeychainKeys();
      const sel = document.getElementById('ai-deployer-key');
      if (sel) sel.value = data.keyName;
      await onAIKeyChange();
    }
  } catch (e) {
    console.error('Register key failed:', e);
  }
}

function updateSpecRecommendation() {
  const el = document.getElementById('ai-spec-recommendation');
  if (!el) return;
  const cloud = document.getElementById('ai-cloud')?.value;
  if (!cloud) { el.innerHTML = '<span style="color:#888">배포 대상을 선택하세요</span>'; return; }

  const includeProver = document.getElementById('ai-include-prover')?.checked;
  const l1Mode = document.getElementById('ai-l1-mode')?.value || 'local';
  const components = ['L2 Node', 'Tools (Explorer, Dashboard, Bridge)'];
  if (l1Mode === 'local') components.unshift('L1 Node');
  if (includeProver) components.push('SP1 Prover');

  if (cloud === 'local-docker') {
    el.innerHTML = `
      <div style="margin-bottom:4px"><b>📦 배포 구성:</b> ${components.join(' + ')}</div>
      <div style="margin-bottom:4px"><b>💻 로컬 Docker 배포</b> — 이 머신에서 Docker Compose로 실행</div>
      ${includeProver ? '<div style="color:#f59e0b;font-size:10px">⚠ SP1 Prover 포함 시 최소 16GB RAM 권장</div>' : ''}
    `;
    return;
  }

  const cloudLabel = cloud === 'gcp' ? 'GCP' : cloud === 'vultr' ? 'Vultr' : 'AWS';

  // Read user-selected values (AWS)
  const instanceType = document.getElementById('aws-instance-type')?.value || '';
  const storageGB = parseInt(document.getElementById('aws-storage-gb')?.value) || 30;
  if (cloud === 'aws' && AWS_INSTANCE_PRICING[instanceType]) {
    const { info: pricing, instanceMo, storageMo, ipMo, monthCost: totalMo } = calcAWSCost(instanceType, storageGB);

    let proverNote = '';
    if (includeProver && pricing.ram < 16) {
      proverNote = `<div style="margin-top:4px;color:#f59e0b">⚠ SP1 Prover에는 최소 16GB RAM 필요 — ${instanceType}은 ${pricing.ram}GB입니다. t3.xlarge 이상을 권장합니다.</div>`;
    } else if (includeProver) {
      proverNote = `<div style="margin-top:4px;color:#a0aec0">✓ SP1 Prover 실행 가능 (${pricing.ram}GB RAM)</div>`;
    }

    el.innerHTML = `
      <div style="margin-bottom:4px"><b>📦 배포 구성:</b> ${components.join(' + ')}</div>
      <div style="margin-bottom:4px"><b>💻 선택 사양 (${cloudLabel}):</b> ${instanceType} — ${pricing.vcpu} vCPU, ${pricing.ram}GB RAM, ${storageGB}GB gp3</div>
      <div><b>💰 예상 비용:</b> ~$${totalMo.toFixed(0)}/월 (인스턴스 $${instanceMo.toFixed(0)} + 스토리지 $${storageMo.toFixed(0)} + Public IP $${ipMo.toFixed(0)})</div>
    <div style="font-size:11px;color:#888;margin-top:2px">* Public IP: EC2에 자동 할당 ($0.005/hr, AWS 필수 과금)</div>
      ${proverNote}
    `;
  } else {
    const specs = VM_SPECS[cloud];
    if (!specs) { el.innerHTML = ''; return; }
    const s = includeProver ? specs.withProver : specs.withoutProver;
    el.innerHTML = `
      <div style="margin-bottom:4px"><b>📦 배포 구성:</b> ${components.join(' + ')}</div>
      <div style="margin-bottom:4px"><b>💻 권장 사양 (${cloudLabel}):</b> ${s.type} — ${s.cpu} vCPU, ${s.ram} RAM, ${s.disk}</div>
      <div><b>💰 예상 비용:</b> ${s.price}</div>
      ${includeProver ? '<div style="margin-top:4px;color:#f59e0b">⚠ SP1 Prover는 증명 생성 시 메모리를 많이 사용합니다. 16GB 이상 권장.</div>' : ''}
    `;
  }
}

async function generateAndShowAIPrompt(deploymentId) {
  const cloud = document.getElementById('ai-cloud').value;
  if (!cloud) {
    showLaunchError('배포 대상을 선택해주세요');
    return;
  }

  // Collect cloud-specific info
  let gcpProjectId = '';
  if (cloud === 'gcp') {
    gcpProjectId = (document.getElementById('gcp-project-id')?.value || '').trim();
    if (!gcpProjectId) {
      showLaunchError('GCP Project ID를 입력해주세요');
      return;
    }
  }

  // Collect L1 mode info
  const l1Mode = document.getElementById('ai-l1-mode')?.value || 'local';
  const l1RpcUrl = document.getElementById('ai-l1-rpc-url')?.value?.trim() || '';
  if (l1Mode !== 'local' && !l1RpcUrl) {
    showLaunchError('L1 RPC URL을 입력해주세요');
    return;
  }
  const l1ChainId = parseInt(document.getElementById('ai-l1-chain-id')?.value) || (l1Mode === 'sepolia' ? 11155111 : l1Mode === 'mainnet' ? 1 : 9);
  const l1Network = l1Mode === 'sepolia' ? 'Sepolia' : l1Mode === 'mainnet' ? 'Mainnet' : 'Local';
  const includeProver = document.getElementById('ai-include-prover')?.checked ?? true;

  // Collect wallet config for testnet/mainnet
  let walletConfig = null;
  if (l1Mode !== 'local') {
    const deployerKey = document.getElementById('ai-deployer-key')?.value;
    if (!deployerKey) {
      showLaunchError('Deployer 키를 선택해주세요');
      return;
    }
    walletConfig = {
      deployerKey,
      committerKey: document.getElementById('ai-committer-key')?.value || '',
      proofCoordinatorKey: document.getElementById('ai-proof-coordinator-key')?.value || '',
      bridgeOwnerKey: document.getElementById('ai-bridge-owner-key')?.value || '',
      deployerAddr: document.getElementById('ai-deployer-addr')?.textContent?.split(' ')[0] || '',
    };
  }

  // Get CLI/auth status for context (skip for local-docker)
  let cliInfo = {};
  if (cloud !== 'local-docker') {
    try {
      const cliRes = await fetch(`${API}/deployments/ai-deploy/check-cli?cloud=${cloud}`);
      if (cliRes.ok) cliInfo = await cliRes.json();
    } catch {}
  }

  // Collect VM options (AWS-specific)
  let awsRegion = '', awsInstanceType = '', awsStorageGB = 30, awsKeyPair = '';
  if (cloud === 'aws') {
    awsRegion = document.getElementById('aws-region')?.value || 'ap-northeast-2';
    awsInstanceType = document.getElementById('aws-instance-type')?.value || 't3.xlarge';
    awsStorageGB = parseInt(document.getElementById('aws-storage-gb')?.value) || 30;
    awsKeyPair = document.getElementById('aws-key-pair-select')?.value || '';
  }

  const promptCloud = cloud === 'local-docker' ? 'local' : cloud;
  try {
    const res = await fetch(`${API}/deployments/${deploymentId}/ai-prompt`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ cloud: promptCloud, l1Mode: l1Mode === 'local' ? 'local' : 'testnet', l1RpcUrl, l1ChainId, l1Network, includeProver, walletConfig, region: awsRegion, vmType: awsInstanceType, storageGB: awsStorageGB, keyPairName: awsKeyPair, awsAccount: cliInfo?.auth?.account || '', programName: (document.querySelector('.program-card.selected .program-name')?.textContent || selectedProgram?.name || selectedProgram?.id || 'evm-l2') }),
    });
    if (!res.ok) { const err = await res.json(); throw new Error(err.error || 'Failed to generate prompt'); }
    const { prompt } = await res.json();
    const l2Name = document.getElementById('launch-name')?.value?.trim() || 'My L2';
    const l2ChainId = document.getElementById('launch-chain-id')?.value || 'auto';
    const programEl = document.querySelector('.program-card.selected .program-name');
    const programName = programEl?.textContent || selectedProgram?.name || selectedProgram?.id || 'evm-l2';
    showAIPromptResult(prompt, { cloud, gcpProjectId, cliInfo, l1Mode, l1Network, l1RpcUrl, l1ChainId, l2Name, l2ChainId, programName, includeProver, walletConfig });
  } catch (e) {
    showLaunchError(e.message);
  }
}

// ---------------------------------------------------------------------------
// AI Chat state
// ---------------------------------------------------------------------------
let aiChatMessages = []; // { role, content }
let aiChatSystemPrompt = '';
let aiChatSending = false;
let aiChatLocalDeploy = false; // true when local-docker target
let aiChatDeployStarted = false; // true after deploy button clicked
let aiChatDeployEventSource = null;
let aiChatRawPrompt = ''; // raw generated prompt for copy
let aiChatUserMessage = ''; // user config summary for copy

function copyAIPrompt() {
  if (!aiChatRawPrompt) return;
  const fullPrompt = aiChatUserMessage
    ? `${aiChatUserMessage}\n\n---\n\n${aiChatRawPrompt}`
    : aiChatRawPrompt;
  navigator.clipboard.writeText(fullPrompt).then(() => {
    const btn = document.getElementById('ai-copy-prompt-btn');
    if (btn) { btn.textContent = '✅ 복사됨!'; setTimeout(() => { btn.textContent = '📋 프롬프트 복사'; }, 2000); }
  }).catch(() => {
    // fallback
    const ta = document.createElement('textarea');
    ta.value = fullPrompt;
    document.body.appendChild(ta);
    ta.select();
    document.execCommand('copy');
    document.body.removeChild(ta);
    const btn = document.getElementById('ai-copy-prompt-btn');
    if (btn) { btn.textContent = '✅ 복사됨!'; setTimeout(() => { btn.textContent = '📋 프롬프트 복사'; }, 2000); }
  });
}

function aiChatGoBack() {
  // If came from My L2 detail, go back there
  if (window._aiDeployDetailId) {
    const id = window._aiDeployDetailId;
    window._aiDeployDetailId = null;
    // Switch to detail view directly (skip loadDeployments which reconciles all)
    document.querySelectorAll('.view').forEach(v => v.classList.remove('active'));
    const detailView = document.getElementById('view-detail');
    if (detailView) detailView.classList.add('active');
    showDeploymentDetail(id);
    return;
  }
  // Reset deploy-card and go back to step 2
  const deployCard = document.querySelector('.deploy-card');
  if (deployCard) { deployCard.style.maxWidth = ''; deployCard.style.width = ''; deployCard.innerHTML = ''; }
  const step3 = document.getElementById('launch-step3');
  if (step3) { step3.style.maxWidth = ''; step3.style.padding = ''; }
  launchGoStep(2);
}

function showAIPromptResult(prompt, cloudCtx = {}) {
  aiChatRawPrompt = prompt; // save for copy button
  // Switch to step 3
  document.querySelectorAll('.launch-step').forEach(s => s.style.display = 'none');
  const step3 = document.getElementById('launch-step3');
  step3.style.display = 'block';

  const infoText = document.getElementById('deploy-info-text');
  infoText.textContent = '';

  // Hide deploy-specific UI
  const progressSteps = document.getElementById('deploy-progress-steps');
  if (progressSteps) progressSteps.innerHTML = '';
  const elapsedBar = document.querySelector('.elapsed-bar');
  if (elapsedBar) elapsedBar.style.display = 'none';
  ['build-log-details', 'event-log-details', 'deploy-message'].forEach(id => {
    const el = document.getElementById(id);
    if (el) el.style.display = 'none';
  });
  ['goto-dashboard-btn', 'cancel-deploy-btn', 'resume-deploy-btn'].forEach(id => {
    const el = document.getElementById(id);
    if (el) el.style.display = 'none';
  });

  // Build chat UI inside deploy-card — expand to full width
  const deployCard = document.querySelector('.deploy-card');
  if (!deployCard) return;
  deployCard.style.maxWidth = 'none';
  deployCard.style.width = '100%';
  step3.style.maxWidth = 'none';
  step3.style.padding = '0';

  // Build config summary
  const ctx = cloudCtx;
  const cloudLabel = ctx.cloud === 'local-docker' ? 'Local (Docker)' : ctx.cloud === 'gcp' ? 'Google Cloud (GCP)' : ctx.cloud === 'vultr' ? 'Vultr' : 'AWS';
  const l1Label = ctx.l1Mode === 'local' ? 'Local (VM 내 L1)' : (ctx.l1Network || 'Testnet');
  const proverLabel = ctx.includeProver !== false ? 'SP1 Prover 포함' : 'Prover 미포함';
  const summaryItems = [
    `<b>App:</b> ${escapeHtml(ctx.programName || '')}`,
    `<b>L2:</b> ${escapeHtml(ctx.l2Name || '')} (Chain ID: ${ctx.l2ChainId || 'auto'})`,
    `<b>Cloud:</b> ${cloudLabel}`,
    ctx.gcpProjectId ? `<b>Project:</b> ${escapeHtml(ctx.gcpProjectId)}` : '',
    `<b>L1:</b> ${l1Label}${ctx.l1ChainId ? ' (Chain ID: ' + ctx.l1ChainId + ')' : ''}`,
    ctx.l1RpcUrl ? `<b>L1 RPC:</b> ${escapeHtml(ctx.l1RpcUrl)}` : '',
    `<b>Prover:</b> ${proverLabel}`,
    ctx.cliInfo?.auth?.account ? `<b>Account:</b> ${escapeHtml(ctx.cliInfo.auth.account)}` : '',
  ].filter(Boolean).join(' &middot; ');

  deployCard.innerHTML = `
    <div id="ai-chat-container" style="display:flex;flex-direction:column;height:calc(100vh - 160px);min-height:500px">
      <div id="ai-chat-config-summary" style="padding:8px 14px;background:var(--bg-surface,#161622);border-bottom:1px solid var(--border,#333);font-size:11px;line-height:1.8;color:#a0aec0;flex-shrink:0;display:flex;align-items:center;gap:10px">
        <button onclick="aiChatGoBack()" style="padding:4px 10px;font-size:11px;border:1px solid var(--border,#444);border-radius:4px;background:transparent;color:#a0aec0;cursor:pointer;flex-shrink:0">← Back</button>
        <span style="flex:1">${summaryItems}</span>
      </div>
      <div id="ai-chat-messages" style="flex:1;overflow-y:auto;padding:12px;display:flex;flex-direction:column;gap:10px">
      </div>
      <div id="ai-monitor-bar" style="border-top:1px solid var(--border,#333);padding:6px 12px;display:flex;gap:8px;align-items:center;font-size:11px;color:#a0aec0;flex-shrink:0">
        <button onclick="checkDeploymentStatus()" style="padding:4px 12px;font-size:11px;border:1px solid #22c55e;border-radius:4px;background:#22c55e;color:white;cursor:pointer;white-space:nowrap">🖥️ 배포 상태 확인</button>
        <span id="ai-monitor-status" style="flex:1;font-size:11px"></span>
      </div>
      <div style="border-top:1px solid var(--border,#333);padding:10px 12px;display:flex;gap:8px;align-items:flex-end">
        <textarea id="ai-chat-input" placeholder="질문을 입력하세요..." rows="2"
          style="flex:1;resize:none;padding:8px 12px;border:1px solid var(--border,#444);border-radius:8px;background:var(--bg-surface,#161622);color:#e0e0e0;caret-color:#e0e0e0;font-size:13px;line-height:1.5;font-family:inherit"
          onkeydown="if(event.key==='Enter'&&!event.shiftKey){event.preventDefault();sendAIChatMessage()}"></textarea>
        <button onclick="sendAIChatMessage()" class="btn-primary" id="ai-chat-send-btn" style="padding:8px 16px;font-size:13px;white-space:nowrap">Send</button>
      </div>
    </div>`;

  // Build system prompt with cloud context
  const cloud = cloudCtx.cloud || 'gcp';
  const projectId = cloudCtx.gcpProjectId || '';
  const cliInfo = cloudCtx.cliInfo || {};
  const account = cliInfo.auth?.account || '';
  const project = cliInfo.auth?.project || projectId;

  const includeProver = cloudCtx.includeProver !== false;
  const specKey = includeProver ? 'withProver' : 'withoutProver';
  const recSpec = VM_SPECS[cloud]?.[specKey] || {};

  let cloudInfo = '';
  const proverNote = includeProver
    ? `\n\n## SP1 Prover 포함 배포
- SP1 Prover는 ZK 증명 생성 시 대량의 메모리를 사용합니다
- 최소 16GB RAM 필요, 32GB 권장 (증명 생성 속도에 영향)
- Prover가 포함되므로 높은 사양의 VM을 선택해야 합니다`
    : `\n\n## Prover 미포함 배포
- SP1 Prover가 포함되지 않은 경량 배포입니다
- L2 노드 + Tools만 배포하므로 상대적으로 낮은 사양으로 운영 가능
- Prover는 별도 서버에서 나중에 연결할 수 있습니다`;

  if (cloud === 'local-docker' || cloud === 'local') {
    cloudInfo = `
## Local Docker Deployment
- 이 머신에서 Docker Compose로 직접 실행
- Docker Desktop이 설치되어 있고 실행 중이어야 합니다
- 별도 VM이나 클라우드 비용 없음
${proverNote}`;
  } else if (cloud === 'gcp') {
    cloudInfo = `
## User's GCP Environment
- GCP Project ID: ${project}
- Authenticated account: ${account}
- gcloud CLI: v${cliInfo.cli?.version || 'unknown'}

## 권장 VM 사양 (${includeProver ? 'Prover 포함' : 'Prover 미포함'})
- **권장**: ${recSpec.type} (${recSpec.cpu} vCPU, ${recSpec.ram}) — ${recSpec.price}

## GCP VM 전체 비교표 (asia-northeast3, 월 예상)
| VM Type | vCPU | RAM | 가격 | ${includeProver ? 'Prover 포함' : 'Prover 미포함'} |
|---------|------|-----|------|------|
| e2-medium | 2 | 4GB | ~$27/월 | ${includeProver ? '❌ 메모리 부족' : '⚠️ 최소 사양'} |
| e2-standard-2 | 2 | 8GB | ~$53/월 | ${includeProver ? '❌ 메모리 부족' : '✅ 권장 (테스트)'} |
| e2-standard-4 | 4 | 16GB | ~$106/월 | ${includeProver ? '✅ 최소 권장' : '✅ 권장 (프로덕션)'} |
| e2-standard-8 | 8 | 32GB | ~$212/월 | ${includeProver ? '✅ 권장 (안정적)' : '과대 사양'} |
- 신규 계정: 90일 무료 체험 ($300 크레딧)
- SSD 디스크 50GB: ~$8.50/월 추가${proverNote}`;
  } else if (cloud === 'vultr') {
    cloudInfo = `
## Vultr Environment
- vultr CLI: v${cliInfo.cli?.version || 'unknown'}

## 권장 VM 사양 (${includeProver ? 'Prover 포함' : 'Prover 미포함'})
- **권장**: ${recSpec.type} (${recSpec.cpu} vCPU, ${recSpec.ram}, ${recSpec.disk}) — ${recSpec.price}

## Vultr VM 전체 비교표 (월 가격)
| Plan | vCPU | RAM | Disk | 가격 | ${includeProver ? 'Prover 포함' : 'Prover 미포함'} |
|------|------|-----|------|------|------|
| vc2-2c-4gb | 2 | 4GB | 80GB | $24/월 | ${includeProver ? '❌ 메모리 부족' : '⚠️ 최소 사양'} |
| vc2-4c-8gb | 4 | 8GB | 160GB | $48/월 | ${includeProver ? '❌ 메모리 부족' : '✅ 권장 (테스트)'} |
| vc2-6c-16gb | 6 | 16GB | 320GB | $96/월 | ${includeProver ? '✅ 최소 권장' : '✅ 권장 (프로덕션)'} |
| vc2-8c-32gb | 8 | 32GB | 640GB | $192/월 | ${includeProver ? '✅ 권장 (안정적)' : '과대 사양'} |
- Regions: Seoul (icn), Tokyo (nrt), Singapore (sgp), US (ewr, lax)
- OS: Ubuntu 22.04 LTS 권장

## Vultr 배포 절차:
1. Vultr 콘솔 접속 (https://my.vultr.com/)
2. Deploy New Server → Cloud Compute → 플랜 및 리전 선택
3. OS: Ubuntu 22.04 선택
4. SSH key 등록 (또는 비밀번호)
5. 서버 생성 후 SSH 접속: ssh root@<server-ip>
6. Docker 설치 → compose 배포${proverNote}`;
  } else {
    cloudInfo = `
## 권장 VM 사양 (${includeProver ? 'Prover 포함' : 'Prover 미포함'})
- **권장**: ${recSpec.type} (${recSpec.cpu} vCPU, ${recSpec.ram}) — ${recSpec.price}

## AWS VM 전체 비교표 (ap-northeast-2, 월 예상)
| Instance | vCPU | RAM | 가격 | ${includeProver ? 'Prover 포함' : 'Prover 미포함'} |
|----------|------|-----|------|------|
| t3.medium | 2 | 4GB | ~$38/월 | ${includeProver ? '❌ 메모리 부족' : '⚠️ 최소 사양'} |
| t3.large | 2 | 8GB | ~$76/월 | ${includeProver ? '❌ 메모리 부족' : '✅ 권장 (테스트)'} |
| t3.xlarge | 4 | 16GB | ~$152/월 | ${includeProver ? '✅ 최소 권장' : '✅ 권장 (프로덕션)'} |
| t3.2xlarge | 8 | 32GB | ~$304/월 | ${includeProver ? '✅ 권장 (안정적)' : '과대 사양'} |
- EBS (gp3 50GB): ~$4/월 추가${proverNote}`;
  }

  const l1ModeVal = cloudCtx.l1Mode || 'local';
  let secretGuide = '';
  const wc = cloudCtx.walletConfig;
  if (l1ModeVal !== 'local') {
    const isGcp = cloud === 'gcp';
    const walletInfo = wc ? `
## Wallet Configuration (사용자가 이미 설정함)
- Deployer: ${wc.deployerKey} (${wc.deployerAddr || 'address resolving...'})
- Committer: ${wc.committerKey || '= Deployer'}
- Proof Coordinator: ${wc.proofCoordinatorKey || '= Deployer'}
- Bridge Owner: ${wc.bridgeOwnerKey || '= Deployer'}

키는 사용자의 macOS Keychain에 저장되어 있습니다.
서버 배포 시 이 키들을 안전하게 전달해야 합니다.` : '';

    const secretMethod = isGcp ? `
### GCP Secret Manager 사용:
1. Secret Manager API 활성화:
   gcloud services enable secretmanager.googleapis.com
2. 키 등록 (사용자가 로컬에서 실행):
   security find-generic-password -a "KEY_NAME" -s "tokamak-appchain" -w | gcloud secrets create tokamak-deployer-key --data-file=-
3. VM에서 키 가져오기:
   DEPLOYER_PRIVATE_KEY=$(gcloud secrets versions access latest --secret=tokamak-deployer-key)` : cloud === 'vultr' ? `
### Vultr 서버에 안전하게 키 전달:
1. 사용자의 로컬에서 Keychain의 키를 읽어서 서버로 전달
2. SSH로 접속 후 .env 파일에 직접 입력 (절대 shell history에 남기지 않을 것)
3. 방법: ssh root@SERVER_IP "cat > /opt/tokamak/DATA_DIR/.env" <<< "DEPLOYER_PRIVATE_KEY=..."
4. 또는: scp로 로컬에서 만든 .env 파일을 서버로 전송` : `
### AWS Secrets Manager 사용:
1. 키 등록: aws secretsmanager create-secret --name tokamak-deployer-key --secret-string "0xKEY"
2. VM에서: DEPLOYER_PRIVATE_KEY=$(aws secretsmanager get-secret-value --secret-id tokamak-deployer-key --query SecretString --output text)`;

    secretGuide = `
## CRITICAL: Private Key Management (${cloudCtx.l1Network || 'Testnet'})
${walletInfo}

### 보안 원칙:
- NEVER hardcode private keys in compose files or shell history
- 프로덕션에서는 각 역할에 별도 키 사용 권장
${secretMethod}

Important: Deployer 계정에 충분한 ${cloudCtx.l1Network} ETH가 있는지 확인하세요.`;
  }

  const isLocalDocker = cloud === 'local-docker' || cloud === 'local';
  aiChatLocalDeploy = isLocalDocker; // flag for deploy button injection
  aiChatDeployStarted = false;
  aiChatSystemPrompt = `You are a deployment assistant for Tokamak Appchain L2.
Respond in Korean (한국어). Be concise and practical.

## IMPORTANT: Deployment Flow
1. FIRST: Present a deployment plan summary to the user, including:
${isLocalDocker ? `   - Docker Compose로 로컬 배포
   - L1/L2 Chain ID 정보
   - 배포될 컨테이너 목록과 할당된 포트
   - 예상 디스크/메모리 사용량
   - 예상 소요 시간 (이미지 pull 3-5분, 컨트랙트 배포 2-3분, 총 약 10분)` : `   - VM specs and region
   - Estimated monthly cost
   - What will be installed (Docker, L2 node, tools)
   - Required steps overview${cloud === 'gcp' ? '\n   - Whether Compute Engine API needs to be enabled: gcloud services enable compute.googleapis.com' : ''}`}${l1ModeVal !== 'local' ? '\n   - Private key setup (MUST be done first)' : ''}
2. ${isLocalDocker
  ? 'End your plan summary with: "확인해주시면 바로 배포를 시작합니다." The app will show a Deploy button — the user clicks it and the app handles deployment automatically.'
  : `WAIT for user confirmation before proceeding
3. AFTER user confirms: Execute the deployment steps directly, one step at a time
4. Run the exact commands yourself — do not just show them for the user to copy
5. After each step, report progress (e.g. "✅ Step 2/7 완료: Docker 설치 완료"), verify the result, and proceed to the next automatically
6. Always show which step you are on and total steps remaining (e.g. "[Step 3/7] Docker Compose 파일 생성 중...")`}
${secretGuide}
${cloudInfo}

## Deployment Configuration & Instructions
${prompt}`;

  aiChatMessages = [];

  // Send initial request with full config summary
  const includesL1 = l1ModeVal === 'local';
  const components = [];
  if (includesL1) components.push('L1 노드');
  components.push('L2 노드');
  if (includeProver) components.push('SP1 Prover');
  components.push('Tools (Explorer, Dashboard, Bridge)');
  // Collect AWS-specific settings (from UI or restored context)
  const awsRegion = document.getElementById('aws-region')?.value || cloudCtx._awsRegion || 'ap-northeast-2';
  const awsInstanceType = document.getElementById('aws-instance-type')?.value || cloudCtx._awsInstanceType || recSpec.type || 't3.xlarge';
  const awsStorageGB = document.getElementById('aws-storage-gb')?.value || cloudCtx._awsStorageGB || '30';
  const awsKeyPair = document.getElementById('aws-key-pair-select')?.value || cloudCtx._awsKeyPair || '';
  const awsAccount = cliInfo.auth?.account || '';

  const configLines = [
    `앱: ${cloudCtx.programName || 'evm-l2'}`,
    `L2 이름: ${cloudCtx.l2Name || 'My L2'} (Chain ID: ${cloudCtx.l2ChainId || 'auto'})`,
    `배포 대상: ${cloud === 'local-docker' ? 'Local (Docker)' : cloud === 'gcp' ? 'GCP' : cloud === 'vultr' ? 'Vultr' : 'AWS'}${project ? ', 프로젝트: ' + project : ''}`,
    cloud === 'aws' ? `리전: ${awsRegion}` : '',
    cloud === 'aws' ? `인스턴스: ${awsInstanceType}` : '',
    cloud === 'aws' ? `스토리지: ${awsStorageGB}GB gp3` : '',
    awsKeyPair ? `SSH Key Pair: ${awsKeyPair} (~/.ssh/${awsKeyPair}.pem)` : '',
    awsAccount ? `AWS 계정: ${awsAccount}` : '',
    `L1: ${includesL1 ? (cloud === 'local-docker' ? '로컬 Docker L1 노드' : 'VM 내 로컬 L1 노드 포함') : cloudCtx.l1Network} (Chain ID: ${cloudCtx.l1ChainId || ''})`,
    cloudCtx.l1RpcUrl ? `L1 RPC: ${cloudCtx.l1RpcUrl}` : '',
    `배포 구성: ${components.join(' + ')}`,
  ].filter(Boolean).join('\n');

  const userMsg = `다음 구성으로 배포해줘. 먼저 배포 계획을 알려줘.\n\n${configLines}`;
  aiChatUserMessage = userMsg;

  // Show guide message first, then user config
  const tokenEstimate = Math.ceil(prompt.length / 3.5);
  const guideMsg = `📋 배포 프롬프트 생성 완료 (~${tokenEstimate.toLocaleString()} tokens)

📌 배포 실행 방법:
1. 아래 파란 메시지의 📋 아이콘 클릭 → 전체 프롬프트 복사
2. **Claude Code**에 붙여넣기 → AI가 로컬 터미널에서 직접 실행
3. AWS EC2 생성 → Docker 설치 → 배포 완료까지 자동 진행

⚠️ **반드시 본인 로컬 PC의 Claude Code(터미널)에서 실행하세요.**
보안을 위해 SSH가 현재 PC의 IP로만 허용됩니다. Claude.ai(웹)에서 실행하면 외부 서버 IP로 SSH가 열려 보안 위험이 있습니다.

💬 아래 질문을 클릭하거나 직접 입력하세요:`;

  aiChatMessages.push({ role: 'assistant', content: guideMsg });
  aiChatMessages.push({ role: 'user', content: userMsg });
  renderChatMessages();

  // Update step indicator
  const indicator = document.getElementById('step-indicator');
  if (indicator) {
    const stepLabels = ['Select App', 'Configure', 'AI Deploy Guide'];
    indicator.innerHTML = [1, 2, 3].map((n, i) =>
      (i > 0 ? '<div class="step-line done"></div>' : '') +
      '<div class="step-item"><div class="step-circle' + (n === 3 ? ' active' : ' done') + '">' + (n < 3 ? '\u2713' : n) + '</div>' +
      '<span class="step-label' + (n === 3 ? ' active' : ' done') + '">' + stepLabels[i] + '</span></div>'
    ).join('');
  }
}

function renderChatMessages() {
  const container = document.getElementById('ai-chat-messages');
  if (!container) return;
  container.innerHTML = '';

  aiChatMessages.forEach(msg => {
    const bubble = document.createElement('div');
    const isUser = msg.role === 'user';
    bubble.style.cssText = `max-width:92%;padding:10px 14px;border-radius:12px;font-size:13px;line-height:1.6;word-wrap:break-word;white-space:pre-wrap;position:relative;${
      isUser
        ? 'align-self:flex-end;background:#2563eb;color:#fff;border-bottom-right-radius:4px'
        : 'align-self:flex-start;background:var(--bg-surface,#1a1a2e);border:1px solid var(--border,#333);color:#e2e8f0;border-bottom-left-radius:4px'
    }`;
    bubble.textContent = msg.content;
    // Copy button for all messages (user + assistant)
    const copyBtn = document.createElement('button');
    const copyColor = isUser ? '#fff' : '#888';
    const copyBorder = isUser ? 'rgba(255,255,255,0.4)' : '#555';
    const copyIconSvg = `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="${copyColor}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>`;
    copyBtn.innerHTML = copyIconSvg;
    copyBtn.title = '복사';
    copyBtn.style.cssText = `position:absolute;top:6px;right:6px;background:${isUser ? 'rgba(255,255,255,0.15)' : 'rgba(255,255,255,0.05)'};border:1px solid ${copyBorder};border-radius:5px;padding:4px 5px;cursor:pointer;color:${copyColor};opacity:0.7;transition:opacity 0.15s;display:flex;align-items:center`;
    copyBtn.onmouseenter = () => { copyBtn.style.opacity = '1'; copyBtn.style.background = isUser ? 'rgba(255,255,255,0.25)' : 'rgba(255,255,255,0.1)'; };
    copyBtn.onmouseleave = () => { copyBtn.style.opacity = '0.7'; copyBtn.style.background = isUser ? 'rgba(255,255,255,0.15)' : 'rgba(255,255,255,0.05)'; };
    copyBtn.onclick = (e) => {
      e.stopPropagation();
      // For user messages: copy combined prompt (user msg + technical prompt)
      const textToCopy = isUser && aiChatRawPrompt ? `${msg.content}\n\n---\n\n${aiChatRawPrompt}` : msg.content;
      navigator.clipboard.writeText(textToCopy).then(() => {
        copyBtn.innerHTML = '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="#4ade80" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>';
        setTimeout(() => { copyBtn.innerHTML = copyIconSvg; }, 1500);
      });
    };
    bubble.appendChild(copyBtn);
    container.appendChild(bubble);

    // Add suggested questions after the first guide message
    if (!isUser && msg.content.includes('배포 프롬프트 생성 완료') && !container.querySelector('.suggested-questions')) {
      const suggestionsDiv = document.createElement('div');
      suggestionsDiv.className = 'suggested-questions';
      suggestionsDiv.style.cssText = 'align-self:flex-start;display:flex;flex-wrap:wrap;gap:6px;margin-top:-4px';
      const questions = [
        '배포 계획 요약',
        '컨테이너 상태 확인 명령어',
        'L2 RPC 응답 확인 방법',
        '로그 확인 방법 (Deployer/L2/Prover)',
        '배포 실패 시 재시작 방법',
        '비용 절약 팁 (중지/terminate)',
      ];
      questions.forEach(q => {
        const btn = document.createElement('button');
        btn.textContent = q;
        btn.style.cssText = 'padding:5px 12px;font-size:11px;border:1px solid #3b82f6;border-radius:16px;background:transparent;color:#60a5fa;cursor:pointer;transition:all 0.15s;white-space:nowrap';
        btn.onmouseenter = () => { btn.style.background = '#3b82f6'; btn.style.color = '#fff'; };
        btn.onmouseleave = () => { btn.style.background = 'transparent'; btn.style.color = '#60a5fa'; };
        btn.onclick = () => {
          const input = document.getElementById('ai-chat-input');
          if (input) { input.value = q; sendAIChatMessage(); }
        };
        suggestionsDiv.appendChild(btn);
      });
      container.appendChild(suggestionsDiv);
    }
  });

  // Scroll to bottom
  container.scrollTop = container.scrollHeight;
}

async function sendAIChatMessage() {
  if (aiChatSending) return;
  const input = document.getElementById('ai-chat-input');
  const text = input.value.trim();
  if (!text) return;

  input.value = '';
  aiChatMessages.push({ role: 'user', content: text });
  renderChatMessages();
  doAIChatRequest();
}

async function doAIChatRequest() {
  if (aiChatSending) return;
  aiChatSending = true;

  const sendBtn = document.getElementById('ai-chat-send-btn');
  if (sendBtn) { sendBtn.disabled = true; sendBtn.textContent = '...'; }

  // Show typing indicator
  const container = document.getElementById('ai-chat-messages');
  const typingEl = document.createElement('div');
  typingEl.id = 'ai-typing';
  typingEl.style.cssText = 'align-self:flex-start;padding:10px 14px;border-radius:12px;background:var(--bg-surface,#1a1a2e);border:1px solid var(--border,#333);color:var(--text-muted,#888);font-size:13px;border-bottom-left-radius:4px';
  typingEl.textContent = 'AI가 응답 중...';
  if (container) { container.appendChild(typingEl); container.scrollTop = container.scrollHeight; }

  try {
    const res = await fetch(`${API}/deployments/ai-deploy/chat`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ messages: aiChatMessages, systemPrompt: aiChatSystemPrompt }),
    });

    const typing = document.getElementById('ai-typing');
    if (typing) typing.remove();

    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      throw new Error(err.error || `Error: ${res.status}`);
    }

    const reply = await res.json();
    aiChatMessages.push({ role: 'assistant', content: reply.content });
    renderChatMessages();

    // For local-docker: show Deploy button after AI presents the plan
    if (aiChatLocalDeploy && !aiChatDeployStarted) {
      const container = document.getElementById('ai-chat-messages');
      if (container) {
        const btnWrap = document.createElement('div');
        btnWrap.id = 'ai-chat-deploy-btn-wrap';
        btnWrap.style.cssText = 'align-self:center;padding:12px 0;display:flex;gap:10px;align-items:center';
        btnWrap.innerHTML = `
          <button onclick="startLocalDeployFromChat()" class="btn-primary" style="padding:10px 28px;font-size:14px;font-weight:600;border-radius:8px">
            Deploy L2
          </button>
          <span style="font-size:11px;color:#888">매니저가 직접 빌드하고 배포합니다</span>`;
        container.appendChild(btnWrap);
        container.scrollTop = container.scrollHeight;
      }
    }
  } catch (e) {
    const typing = document.getElementById('ai-typing');
    if (typing) typing.remove();
    // Show error as system message
    const errBubble = document.createElement('div');
    errBubble.style.cssText = 'align-self:center;padding:8px 14px;border-radius:8px;background:rgba(239,68,68,0.15);color:#ef4444;font-size:12px;text-align:center';
    errBubble.textContent = e.message;
    if (container) { container.appendChild(errBubble); container.scrollTop = container.scrollHeight; }
  } finally {
    aiChatSending = false;
    if (sendBtn) { sendBtn.disabled = false; sendBtn.textContent = 'Send'; }
  }
}

// ---------------------------------------------------------------------------
// Local Docker deploy from AI chat
// ---------------------------------------------------------------------------

const PHASE_LABELS = {
  configured: '설정 완료',
  checking_docker: 'Docker 확인 중',
  building: '이미지 빌드 중',
  pulling: '이미지 다운로드 중',
  l1_starting: 'L1 노드 시작 중',
  deploying_contracts: '컨트랙트 배포 중',
  verifying_contracts: '컨트랙트 검증 중',
  l2_starting: 'L2 노드 시작 중',
  starting_prover: 'Prover 시작 중',
  starting_tools: 'Tools 시작 중',
  running: '배포 완료',
};

async function startLocalDeployFromChat() {
  if (aiChatDeployStarted) return;
  aiChatDeployStarted = true;

  // Remove deploy button
  const btnWrap = document.getElementById('ai-chat-deploy-btn-wrap');
  if (btnWrap) btnWrap.remove();

  const container = document.getElementById('ai-chat-messages');

  // Add deploy started message
  addChatSystemMessage('🚀 배포를 시작합니다...');

  // Create progress area in chat
  const progressEl = document.createElement('div');
  progressEl.id = 'ai-chat-deploy-progress';
  progressEl.style.cssText = 'align-self:stretch;margin:8px 0;padding:14px;border-radius:10px;background:var(--bg-surface,#161622);border:1px solid var(--border,#333);font-size:12px;line-height:1.8;color:#e2e8f0';
  progressEl.innerHTML = '<div id="ai-chat-phase" style="font-weight:600;margin-bottom:6px">[Step 1] Docker 확인 중...</div><div id="ai-chat-deploy-log" style="max-height:200px;overflow-y:auto;font-size:11px;color:#888;font-family:monospace;white-space:pre-wrap"></div>';
  if (container) { container.appendChild(progressEl); container.scrollTop = container.scrollHeight; }

  // Hide chat input during deploy
  const inputArea = document.querySelector('#ai-chat-container > div:last-child');

  try {
    const provRes = await fetch(`${API}/deployments/${launchDeploymentId}/provision`, { method: 'POST' });
    if (!provRes.ok) {
      const err = await provRes.json().catch(() => ({}));
      throw new Error(err.error || 'Failed to start provisioning');
    }

    // Listen to SSE events
    let chatPhaseIdx = 0;
    const phaseOrder = ['checking_docker', 'pulling', 'building', 'l1_starting', 'deploying_contracts', 'l2_starting', 'starting_prover', 'starting_tools', 'running'];

    if (aiChatDeployEventSource) aiChatDeployEventSource.close();
    aiChatDeployEventSource = new EventSource(`${API}/deployments/${launchDeploymentId}/events`);

    aiChatDeployEventSource.onmessage = (e) => {
      try {
        const data = JSON.parse(e.data);
        const phaseEl = document.getElementById('ai-chat-phase');
        const logEl = document.getElementById('ai-chat-deploy-log');

        if (data.event === 'log') {
          if (logEl) {
            logEl.textContent += (data.message || '') + '\n';
            // Keep last 50 lines
            const lines = logEl.textContent.split('\n');
            if (lines.length > 50) logEl.textContent = lines.slice(-50).join('\n');
            logEl.scrollTop = logEl.scrollHeight;
          }
          if (container) container.scrollTop = container.scrollHeight;
          return;
        }

        if (data.phase) {
          const idx = phaseOrder.indexOf(data.phase);
          if (idx >= 0) chatPhaseIdx = idx;
          const label = PHASE_LABELS[data.phase] || data.phase;
          const stepNum = Math.max(1, chatPhaseIdx + 1);
          const total = phaseOrder.length;
          if (phaseEl) phaseEl.textContent = `[Step ${stepNum}/${total}] ${label}`;
        }

        if (data.event === 'error') {
          addChatSystemMessage(`❌ 배포 실패: ${data.message || 'Unknown error'}`);
          aiChatDeployEventSource.close();
          // Tell AI about the error
          aiChatMessages.push({ role: 'user', content: `배포 중 에러 발생: ${data.message}. 어떻게 해결할 수 있어?` });
          doAIChatRequest();
        }

        if (data.event === 'complete' || data.phase === 'running') {
          if (phaseEl) phaseEl.textContent = '✅ 배포 완료!';
          aiChatDeployEventSource.close();

          // Fetch final deployment info and tell AI
          fetch(`${API}/deployments/${launchDeploymentId}`).then(r => r.json()).then(dep => {
            const info = dep.deployment || dep;
            const urls = [];
            if (info.l2_port) urls.push(`L2 RPC: http://localhost:${info.l2_port}`);
            if (info.l1_port) urls.push(`L1 RPC: http://localhost:${info.l1_port}`);
            if (info.tools_l2_explorer_port) urls.push(`L2 Explorer: http://localhost:${info.tools_l2_explorer_port}`);
            if (info.tools_l1_explorer_port) urls.push(`L1 Explorer: http://localhost:${info.tools_l1_explorer_port}`);
            if (info.tools_bridge_ui_port) urls.push(`Dashboard: http://localhost:${info.tools_bridge_ui_port}`);
            const contracts = [];
            if (info.bridge_address) contracts.push(`CommonBridge: ${info.bridge_address}`);
            if (info.proposer_address) contracts.push(`OnChainProposer: ${info.proposer_address}`);

            addChatSystemMessage(`✅ 배포 완료!\n\n${urls.join('\n')}${contracts.length ? '\n\n' + contracts.join('\n') : ''}`);

            aiChatMessages.push({ role: 'user', content: `배포가 완료되었습니다. 접속 정보를 정리해줘.\n\n${urls.join('\n')}\n${contracts.join('\n')}` });
            doAIChatRequest();
          }).catch(() => {
            addChatSystemMessage('✅ 배포 완료!');
          });
        }
      } catch {}
    };
  } catch (e) {
    addChatSystemMessage(`❌ 배포 시작 실패: ${e.message}`);
  }
}

function addChatSystemMessage(text) {
  const container = document.getElementById('ai-chat-messages');
  if (!container) return;
  const el = document.createElement('div');
  el.style.cssText = 'align-self:center;padding:8px 16px;border-radius:8px;background:rgba(59,130,246,0.1);color:#60a5fa;font-size:12px;text-align:center;white-space:pre-wrap;line-height:1.6;max-width:90%';
  el.textContent = text;
  container.appendChild(el);
  container.scrollTop = container.scrollHeight;
}

function escapeHtml(str) {
  if (!str) return '';
  return String(str).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function copyAIPrompt() {
  const content = document.getElementById('ai-prompt-content');
  if (!content) return;
  const text = content.textContent;
  navigator.clipboard.writeText(text).then(() => {
    const btn = document.getElementById('copy-prompt-btn');
    if (btn) { btn.textContent = 'Copied!'; setTimeout(() => { btn.textContent = 'Copy to Clipboard'; }, 2000); }
  });
}

// ---------------------------------------------------------------------------
// Init
// ---------------------------------------------------------------------------

// ============================================================
// Guest Builds
// ============================================================
let guestBuildPollInterval = null;
let guestBuildDetailOpen = false; // prevent auto-refresh from overwriting detail view
let guestBuildHasActive = false; // only poll when there are active builds
let guestBuildEventSource = null; // track active SSE connection

async function loadGuestBuilds() {
  const container = document.getElementById('guest-builds-list');
  if (!container) return;
  guestBuildDetailOpen = false;
  if (guestBuildEventSource) { guestBuildEventSource.close(); guestBuildEventSource = null; }

  // Smart polling: only auto-refresh when there are active builds
  if (guestBuildPollInterval) clearInterval(guestBuildPollInterval);
  guestBuildPollInterval = setInterval(() => {
    if (guestBuildDetailOpen) return;
    if (!guestBuildHasActive) { clearInterval(guestBuildPollInterval); return; }
    const activeView = document.querySelector('.view.active');
    if (activeView && activeView.id === 'view-guest-builds') {
      loadGuestBuilds_render();
    } else {
      clearInterval(guestBuildPollInterval);
    }
  }, 3000);

  await loadGuestBuilds_render();
}

async function loadGuestBuilds_render() {
  const container = document.getElementById('guest-builds-list');
  if (!container) return;

  try {
    const res = await fetch(`${API}/guest-builds`);
    const data = await res.json();
    const builds = data.builds || [];
    guestBuildHasActive = builds.some(b => b.status === 'building');

    if (builds.length === 0) {
      container.innerHTML = `
        <div class="empty-state" style="padding:48px 0;">
          <p style="font-size:14px;font-weight:600;">No builds yet</p>
          <p style="font-size:12px;margin-top:4px;">Build requests from ChainForge Studio will appear here.</p>
        </div>`;
      return;
    }

    builds.sort((a, b) => {
      if (a.status === 'building' && b.status !== 'building') return -1;
      if (b.status === 'building' && a.status !== 'building') return 1;
      return b.startedAt - a.startedAt;
    });

    const buildStatusClass = (s) => s === 'completed' ? 'running' : s === 'error' ? 'error' : 'deploying';
    const buildStatusLabel = (s) => s === 'completed' ? 'Completed' : s === 'error' ? 'Failed' : 'Building';
    const buildPhaseLabel = (s) => s === 'completed' ? 'Done' : s === 'error' ? 'Error' : 'In Progress';

    container.innerHTML = `
      <table class="data-table">
        <thead>
          <tr>
            <th style="width:40px;padding-left:20px"></th>
            <th>Program</th>
            <th>Status</th>
            <th>Artifacts</th>
            <th>Phase</th>
            <th style="text-align:right">Actions</th>
          </tr>
        </thead>
        <tbody>
          ${builds.map(b => {
            const time = new Date(b.startedAt).toLocaleTimeString();
            const duration = b.result?.buildDuration ? `${(b.result.buildDuration / 1000).toFixed(1)}s` : b.status === 'building' ? `${Math.floor((Date.now() - b.startedAt) / 1000)}s` : '-';
            const sClass = buildStatusClass(b.status);
            const initial = (b.programName || '?').charAt(0).toUpperCase();

            return `
            <tr class="deploy-row" onclick="showBuildDetail('${b.buildId}')" style="cursor:pointer">
              <td style="padding-left:20px"></td>
              <td>
                <div class="name-cell">
                  <div class="icon-box">${initial}</div>
                  <div>
                    <div class="name-text">${b.programName}</div>
                    <div style="font-size:11px;color:var(--text-muted)">${time} · ${duration}${b.sourceHash ? ` · <span style="font-family:monospace">${b.sourceHash}</span>` : ''}</div>
                  </div>
                </div>
              </td>
              <td>
                <div class="status-cell">
                  <span class="status-dot ${sClass}"></span>
                  <span>${buildStatusLabel(b.status)}</span>
                </div>
              </td>
              <td style="font-size:11px;font-family:monospace;color:var(--text-muted)">
                ${b.result?.elfHash ? `<div>ELF: ${b.result.elfHash.slice(0, 14)}...</div>` : '<div>-</div>'}
                ${b.result?.vk ? `<div>VK: ${b.result.vk.slice(0, 14)}...</div>` : ''}
              </td>
              <td>${renderBuildPhaseBadge(b.status)}</td>
              <td>
                <div class="actions-cell">
                  ${b.status === 'completed' ? `<a href="${API}/guest-builds/${b.buildId}/elf" download class="icon-btn" title="Download ELF" onclick="event.stopPropagation()"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg></a>` : ''}
                  <button class="icon-btn" title="View Details" onclick="event.stopPropagation(); showBuildDetail('${b.buildId}')"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></svg></button>
                  <button class="icon-btn" title="Delete" onclick="event.stopPropagation(); deleteGuestBuild('${b.buildId}')" style="color:var(--danger,#ef4444)"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg></button>
                </div>
              </td>
            </tr>`;
          }).join('')}
        </tbody>
      </table>`;
  } catch (err) {
    container.innerHTML = `<p style="color:var(--danger);">Failed to load builds: ${err.message}</p>`;
  }
}

async function deleteGuestBuild(buildId) {
  try {
    const resp = await fetch(`${API}/guest-builds/${buildId}`, { method: 'DELETE' });
    if (!resp.ok) {
      const data = await resp.json().catch(() => ({}));
      console.error('[guest-builds] delete failed:', data);
    }
    loadGuestBuilds();
  } catch (err) {
    console.error('Failed to delete:', err.message);
  }
}

function renderBuildPhaseBadge(status) {
  if (status === 'completed') return '<span style="display:inline-block;padding:2px 10px;border-radius:12px;font-size:11px;font-weight:600;background:#dcfce7;color:#16a34a">Done</span>';
  if (status === 'error') return '<span style="display:inline-block;padding:2px 10px;border-radius:12px;font-size:11px;font-weight:600;background:#fef2f2;color:#dc2626">Error</span>';
  return '<span style="display:inline-block;padding:2px 10px;border-radius:12px;font-size:11px;font-weight:600;background:#fef9c3;color:#ca8a04">Building</span>';
}

async function showBuildDetail(buildId) {
  guestBuildDetailOpen = true; // prevent auto-refresh from overwriting
  const container = document.getElementById('guest-builds-list');
  if (!container) return;

  container.innerHTML = '<p style="color:#52525b;">Loading build details...</p>';

  try {
    const res = await fetch(`${API}/guest-builds/${buildId}`);
    const b = await res.json();
    if (b.error) throw new Error(b.error);

    const statusColor = b.status === 'completed' ? '#22c55e' : b.status === 'error' ? '#ef4444' : '#f59e0b';
    const statusLabel = b.status === 'completed' ? 'Completed' : b.status === 'error' ? 'Failed' : 'Building...';
    const time = b.startedAt ? new Date(b.startedAt).toLocaleString() : '-';
    const duration = b.result?.buildDuration ? `${(b.result.buildDuration / 1000).toFixed(1)}s` : '-';
    const logs = b.logs || [];

    container.innerHTML = `
      <div>
        <button onclick="loadGuestBuilds()" style="padding:6px 14px;background:#27272a;border:1px solid #3f3f46;border-radius:8px;color:#a1a1aa;font-size:12px;font-weight:600;cursor:pointer;margin-bottom:16px;">← Back to list</button>

        <div style="padding:20px;border:1px solid #27272a;border-radius:12px;background:#18181b;margin-bottom:16px;">
          <div style="display:flex;align-items:center;gap:12px;margin-bottom:16px;">
            <span style="font-weight:700;font-size:18px;">${b.programName}</span>
            <span style="font-size:12px;color:${statusColor};font-weight:600;padding:3px 8px;background:${statusColor}15;border-radius:6px;">${statusLabel}</span>
          </div>

          <div style="display:grid;grid-template-columns:repeat(2,1fr);gap:16px;font-size:12px;">
            <div>
              <div style="color:#52525b;font-size:10px;font-weight:600;text-transform:uppercase;margin-bottom:4px;">Build ID</div>
              <div style="color:#a1a1aa;font-family:monospace;font-size:11px;word-break:break-all;">${buildId}</div>
            </div>
            <div>
              <div style="color:#52525b;font-size:10px;font-weight:600;text-transform:uppercase;margin-bottom:4px;">Started</div>
              <div style="color:#a1a1aa;">${time}</div>
            </div>
            <div>
              <div style="color:#52525b;font-size:10px;font-weight:600;text-transform:uppercase;margin-bottom:4px;">Duration</div>
              <div style="color:#a1a1aa;">${duration}</div>
            </div>
            <div>
              <div style="color:#52525b;font-size:10px;font-weight:600;text-transform:uppercase;margin-bottom:4px;">Source Hash</div>
              <div style="color:#a1a1aa;font-family:monospace;">${b.sourceHash || '-'}</div>
            </div>
          </div>

          ${b.result && b.status === 'completed' ? `
          <div style="margin-top:16px;padding-top:16px;border-top:1px solid #27272a;">
            <div style="font-size:10px;font-weight:600;text-transform:uppercase;color:#22c55e;margin-bottom:8px;">Artifacts</div>
            <div style="display:grid;grid-template-columns:repeat(2,1fr);gap:12px;font-size:12px;">
              <div>
                <div style="color:#52525b;font-size:10px;margin-bottom:2px;">ELF Hash</div>
                <div style="color:#a1a1aa;font-family:monospace;font-size:10px;word-break:break-all;">${b.result.elfHash || '-'}</div>
              </div>
              <div>
                <div style="color:#52525b;font-size:10px;margin-bottom:2px;">Verification Key</div>
                <div style="color:#a1a1aa;font-family:monospace;font-size:10px;word-break:break-all;">${b.result.vk || '-'}</div>
              </div>
              <div>
                <div style="color:#52525b;font-size:10px;margin-bottom:2px;">ELF Size</div>
                <div style="color:#a1a1aa;">${b.result.elfSize ? (b.result.elfSize / 1024).toFixed(1) + ' KB' : '-'}</div>
              </div>
              <div>
                <div style="color:#52525b;font-size:10px;margin-bottom:2px;">VK Extraction</div>
                <div style="color:#a1a1aa;">${b.result.vkExtracted ? '✅ SP1 SDK (real)' : '⚠️ Deterministic fallback'}</div>
              </div>
            </div>
            <div style="margin-top:12px;">
              <a href="${API}/guest-builds/${buildId}/elf" download style="padding:6px 14px;background:#22c55e20;border:1px solid #22c55e40;border-radius:8px;color:#22c55e;font-size:11px;font-weight:600;text-decoration:none;">Download ELF</a>
            </div>
          </div>` : ''}

          ${b.result?.error ? `
          <div style="margin-top:16px;padding:12px;background:#ef444415;border:1px solid #ef444430;border-radius:8px;">
            <div style="font-size:10px;font-weight:600;color:#ef4444;margin-bottom:4px;">Error</div>
            <div style="font-size:12px;color:#fca5a5;">${b.result.error}</div>
          </div>` : ''}
        </div>

        <div style="padding:16px;border:1px solid #27272a;border-radius:12px;background:#18181b;">
          <div style="display:flex;align-items:center;justify-content:space-between;margin-bottom:12px;">
            <div style="font-size:12px;font-weight:700;color:#a1a1aa;">Build Logs</div>
            <span style="font-size:10px;color:#52525b;">${logs.length} lines</span>
          </div>
          <div style="background:#000;border-radius:8px;padding:12px;max-height:280px;overflow-y:auto;font-family:monospace;font-size:10px;line-height:1.6;">
            ${logs.length === 0
              ? '<span style="color:#52525b;">No logs available</span>'
              : logs.map(line => {
                  const color = line.startsWith('[error]') ? '#ef4444'
                    : line.startsWith('[build]') ? '#22c55e'
                    : line.startsWith('[vk]') ? '#a78bfa'
                    : line.startsWith('[cache]') ? '#eab308'
                    : '#71717a';
                  return `<div style="color:${color};white-space:pre-wrap;word-break:break-all;">${esc(line)}</div>`;
                }).join('')
            }
          </div>
        </div>

        ${b.status === 'building' ? `
        <div style="margin-top:12px;text-align:center;">
          <button onclick="showBuildDetail('${buildId}')" style="padding:6px 14px;background:#f59e0b20;border:1px solid #f59e0b40;border-radius:8px;color:#f59e0b;font-size:11px;font-weight:600;cursor:pointer;">Refresh Logs</button>
        </div>` : ''}
      </div>`;

    // Auto-refresh detail view via SSE when building
    if (b.status === 'building') {
      if (guestBuildEventSource) { guestBuildEventSource.close(); guestBuildEventSource = null; }
      const evtSource = new EventSource(`${API}/guest-builds/${buildId}/logs`);
      guestBuildEventSource = evtSource;
      evtSource.onmessage = (e) => {
        try {
          const evt = JSON.parse(e.data);
          if (evt.type === 'completed' || evt.type === 'error') {
            evtSource.close();
            if (guestBuildDetailOpen) showBuildDetail(buildId);
          } else if (evt.type === 'log') {
            const logContainer = container.querySelector('[style*="background:#000"]');
            if (logContainer) {
              const div = document.createElement('div');
              div.style.cssText = 'color:#71717a;white-space:pre-wrap;word-break:break-all;';
              div.textContent = evt.message;
              logContainer.appendChild(div);
              logContainer.scrollTop = logContainer.scrollHeight;
            }
          }
        } catch (_) {}
      };
      evtSource.onerror = () => evtSource.close();
    }
  } catch (err) {
    container.innerHTML = `
      <button onclick="loadGuestBuilds()" style="padding:6px 14px;background:#27272a;border:1px solid #3f3f46;border-radius:8px;color:#a1a1aa;font-size:12px;font-weight:600;cursor:pointer;margin-bottom:16px;">← Back</button>
      <p style="color:#ef4444;font-size:13px;">Failed to load build: ${err.message}</p>`;
  }
}

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
