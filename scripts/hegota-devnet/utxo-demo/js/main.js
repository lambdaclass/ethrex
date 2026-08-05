import { mountDemo, mountScale } from './render.js';
import { walletState, pickWalletAndConnect, hasWallet, shortAddr, liveInfoPromise, initWalletListener, onWalletChange } from './wallet.js';
import { faucet, balanceOf } from './api.js';

const PAGES = [
  { id: 'welcome', title: 'Start here: what this demo shows, and why it matters.' },
  { id: 'explained', title: 'The two design rules behind native UTXOs.' },
  { id: 'how', title: 'Create → commit → spend → settle.' },
  { id: 'limits', title: 'Honest limitations.' },
  { id: 'overview', title: 'Five standalone live demos.' },
  { id: 'setup', title: 'One-time setup: connect and fund your wallet to play Alice on-chain.' },
  { id: 'demo1', title: 'Demo 1 of 5 — self-funded spend, signed by your wallet.', demo: 1 },
  { id: 'demo2', title: 'Demo 2 of 5 — stealth consolidation.', demo: 2 },
  { id: 'demo3', title: 'Demo 3 of 5 — payroll at scale.', demo: 3 },
  { id: 'demo4', title: 'Demo 4 of 5 — sponsored spend.', demo: 4 },
  { id: 'demo5', title: 'Demo 5 of 5 — the scale explorer.', demo: 5 },
];

const ICONS = {
  home: '<svg viewBox="0 0 24 24" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>',
  book: '<svg viewBox="0 0 24 24" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"/><path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"/></svg>',
  layers: '<svg viewBox="0 0 24 24" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m12 2 8.5 4.7L12 11.4 3.5 6.7z"/><path d="m3.5 12 8.5 4.7 8.5-4.7"/><path d="m3.5 17.3 8.5 4.7 8.5-4.7"/></svg>',
  alert: '<svg viewBox="0 0 24 24" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m21.7 18-8-14a2 2 0 0 0-3.4 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.7-3"/><path d="M12 9v4"/><path d="M12 17h.01"/></svg>',
  grid: '<svg viewBox="0 0 24 24" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="7" height="7" x="3" y="3" rx="1"/><rect width="7" height="7" x="14" y="3" rx="1"/><rect width="7" height="7" x="14" y="14" rx="1"/><rect width="7" height="7" x="3" y="14" rx="1"/></svg>',
  user: '<svg viewBox="0 0 24 24" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/></svg>',
  eye: '<svg viewBox="0 0 24 24" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7"/><circle cx="12" cy="12" r="3"/></svg>',
  users: '<svg viewBox="0 0 24 24" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>',
  shield: '<svg viewBox="0 0 24 24" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 13c0 5-3.5 7.5-7.7 9a.6.6 0 0 1-.6 0C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.2-2.7a1.2 1.2 0 0 1 1.6 0C14.5 3.8 17 5 19 5a1 1 0 0 1 1 1z"/></svg>',
  chart: '<svg viewBox="0 0 24 24" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 3v16a2 2 0 0 0 2 2h16"/><path d="m7 14 4-4 4 4 5-6"/></svg>',
  key: '<svg viewBox="0 0 24 24" fill="none" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="7.5" cy="15.5" r="5.5"/><path d="m21 2-9.6 9.6"/><path d="m15.5 7.5 3 3L22 7l-3-3"/></svg>',
};

const mounted = new Set();

function currentPage() {
  const hash = location.hash.replace(/^#\/?/, '') || 'welcome';
  return PAGES.find((p) => p.id === hash) ? hash : 'welcome';
}

function mountIfNeeded(page) {
  const def = PAGES.find((p) => p.id === page);
  if (!def?.demo || mounted.has(def.demo)) return;
  mounted.add(def.demo);
  const root = document.getElementById(`${page}-app`);
  if (def.demo === 5) mountScale(root);
  else mountDemo(root, def.demo);
}

function show(page) {
  document.querySelectorAll('.page').forEach((s) => s.classList.toggle('visible', s.id === `page-${page}`));
  document.querySelectorAll('.nav-item').forEach((a) => a.classList.toggle('active', a.dataset.page === page));
  const idx = PAGES.findIndex((p) => p.id === page);
  document.getElementById('page-pos').textContent = `Page ${idx + 1}/${PAGES.length}`;
  document.getElementById('page-hint').textContent = PAGES[idx].title;
  document.getElementById('nav-prev').disabled = idx === 0;
  document.getElementById('nav-next').disabled = idx === PAGES.length - 1;
  if (page === 'setup') renderSetup();
  mountIfNeeded(page);
  window.scrollTo({ top: 0 });
}

// --- wallet button + setup page ------------------------------------------------

function walletBox() {
  const box = document.getElementById('wallet-box');
  const acc = walletState().account;
  box.innerHTML = '';
  if (acc) {
    const chip = document.createElement('div');
    chip.className = 'wallet-chip';
    const name = walletState().providerInfo?.name;
    chip.textContent = name ? `${shortAddr(acc)} · ${name}` : shortAddr(acc);
    chip.title = acc;
    box.appendChild(chip);
  } else {
    const btn = document.createElement('button');
    btn.className = 'btn primary wallet-btn';
    btn.textContent = hasWallet() ? 'Connect Wallet' : 'No wallet found';
    btn.disabled = !hasWallet();
    btn.onclick = () => pickWalletAndConnect();
    box.appendChild(btn);
  }
}

async function renderSetup() {
  const body = document.getElementById('setup-body');
  const info = await liveInfoPromise;
  body.innerHTML = '';
  if (!info.live) {
    body.innerHTML = `<div class="callout">This server is running in <b>simulated</b> mode — no wallet needed.
      Restart with <span class="mono">EIP8312_LIVE=1</span> against the devnet to play with your own wallet.</div>`;
    return;
  }
  const acc = walletState().account;
  if (!acc) {
    const b = document.createElement('div');
    b.className = 'banner-warn';
    b.innerHTML = '<span>⚠ Connect your wallet to set up your account.</span>';
    const btn = document.createElement('button');
    btn.className = 'btn primary';
    btn.textContent = hasWallet() ? 'Connect Wallet' : 'No wallet found';
    btn.disabled = !hasWallet();
    btn.onclick = async () => { await pickWalletAndConnect(); renderSetup(); };
    b.appendChild(btn);
    body.appendChild(b);
    return;
  }
  const bal = await balanceOf(acc).then((r) => Number(BigInt(r.balanceWei)) / 1e18).catch(() => null);
  const card = document.createElement('div');
  card.className = 'card setup-card';
  card.innerHTML = `<h4>Your account</h4>
    <table class="kv">
      <tr><td>address</td><td class="mono">${acc}</td></tr>
      <tr><td>network</td><td class="mono">ethrex EIP-8312 devnet · chain 3151908</td></tr>
      <tr><td>balance</td><td class="mono">${bal == null ? '—' : bal.toFixed(4) + ' ETH'}</td></tr>
    </table>`;
  body.appendChild(card);
  const status = document.createElement('div');
  if (bal != null && bal < 0.5) {
    status.className = 'banner-warn';
    status.innerHTML = '<span>⚠ You need devnet ETH to play Alice (she deposits 1 ETH to the vault).</span>';
    const btn = document.createElement('button');
    btn.className = 'btn primary';
    btn.textContent = 'Get 1 ETH from the faucet';
    btn.onclick = async () => {
      btn.disabled = true;
      btn.textContent = '⏳ sending…';
      try { await faucet(acc); } catch (e) { alert(e.message); }
      renderSetup();
    };
    status.appendChild(btn);
  } else {
    status.className = 'banner-ok';
    status.innerHTML = '<span>✓ Account ready — head to <a href="#/demo1">Demo 1</a> and sign the vault deposit yourself.</span>';
  }
  body.appendChild(status);
}

function go(delta) {
  const idx = PAGES.findIndex((p) => p.id === currentPage());
  const next = Math.min(Math.max(idx + delta, 0), PAGES.length - 1);
  location.hash = `/${PAGES[next].id}`;
}

window.addEventListener('DOMContentLoaded', async () => {
  document.querySelectorAll('.ic').forEach((s) => { s.innerHTML = ICONS[s.dataset.ic] || ''; });
  document.getElementById('nav-prev').onclick = () => go(-1);
  document.getElementById('nav-next').onclick = () => go(1);
  document.getElementById('start-tour').onclick = () => { location.hash = '/explained'; };
  window.addEventListener('hashchange', () => show(currentPage()));
  initWalletListener();
  onWalletChange(() => { walletBox(); if (currentPage() === 'setup') renderSetup(); });
  walletBox();
  show(currentPage());

  try {
    const info = await fetch('/api/demos').then((r) => r.json());
    const badge = document.getElementById('mode-badge');
    const pill = document.createElement('span');
    pill.className = info.live ? 'live-pill' : 'sim-pill';
    pill.textContent = info.live ? `LIVE DEVNET · ${info.rpc}` : 'SIMULATED';
    badge.appendChild(pill);
  } catch { /* leave unbadged */ }
});
