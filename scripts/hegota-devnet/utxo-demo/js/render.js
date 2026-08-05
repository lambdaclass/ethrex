// Render layer — DOM only. All state and protocol logic come from the backend as JSON payloads.
import { gotoStep, scale, postAction } from './api.js';
import { walletState, sendWalletDeposit, liveInfoPromise, pickWalletAndConnect, hasWallet, onWalletChange } from './wallet.js';

function el(tag, cls, html) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (html != null) e.innerHTML = html;
  return e;
}

const short = (a) => (a && a.length > 16 ? a.slice(0, 8) + '…' + a.slice(-4) : a);

function fmtEth(n) {
  if (n === 0) return '0';
  return n.toFixed(6).replace(/0+$/, '').replace(/\.$/, '');
}

function fmtBytes(n) {
  if (n < 1024) return `${Math.round(n * 100) / 100} B`;
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KiB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MiB`;
  return `${(n / 1024 ** 3).toFixed(1)} GiB`;
}

const fmtGas = (g) => Math.round(g).toLocaleString('en-US');

function labeler(chain) {
  const map = new Map(chain.accounts.map((a) => [a.addr, a.label]));
  map.set(chain.vault, 'UTXO_VAULT');
  return (addr) => map.get(addr) || short(addr);
}

// --- Cards -------------------------------------------------------------------

function vaultCard(chain) {
  const c = el('div', 'card');
  c.append(el('h4', null, 'UTXO_VAULT <span class="mono dim">0x…8312</span>'));
  const rows = [
    ['current block', chain.block],
    ['next_utxo_index', chain.nextIndex],
    ['vault balance', fmtEth(chain.vaultBalance) + ' ETH'],
    ['ring slots used', chain.ringUsed == null ? '—' : `${chain.ringUsed} / ${chain.ringSize}`],
    ['batches sealed', chain.batches == null ? '—' : chain.batches],
    ['unspent UTXOs', chain.unspent],
  ];
  const t = el('table', 'kv');
  for (const [k, v] of rows) t.append(el('tr', null, `<td>${k}</td><td class="mono">${v}</td>`));
  c.append(t);
  return c;
}

function accountsCard(chain, highlight = []) {
  const live = chain.live;
  const c = el('div', 'card');
  c.append(el('h4', null, live ? 'Accounts <span class="tag">live balances</span>' : 'Accounts'));
  const t = el('table', 'acct');
  t.append(el('tr', null, live ? '<th>actor</th><th>devnet address</th><th>ETH</th>' : '<th>actor</th><th>address</th><th>ETH</th>'));
  for (const a of chain.accounts) {
    t.append(el('tr', highlight.includes(a.addr) ? 'hl' : null,
      `<td>${a.label}</td><td class="mono dim">${short(a.realAddr || a.addr)}</td><td class="mono">${fmtEth(a.balance)}</td>`));
  }
  c.append(t);
  return c;
}

function txsCard(chain) {
  if (!chain.txs || !chain.txs.length) return null;
  const c = el('div', 'card');
  c.append(el('h4', null, 'On-chain transactions <span class="tag">real, mined</span>'));
  const t = el('table', 'kv');
  for (const tx of chain.txs) {
    t.append(el('tr', null, `<td class="mono dim">${short(tx.hash)}</td><td>${tx.note}</td>`));
  }
  c.append(t);
  return c;
}

function logsCard(chain, filterRecipient = null) {
  const label = labeler(chain);
  const c = el('div', 'card');
  c.append(el('h4', null, 'UtxoCreated logs <span class="tag">append-only history</span>'));
  const logs = filterRecipient ? chain.logs.filter((l) => l.recipient === filterRecipient) : chain.logs;
  if (!logs.length) { c.append(el('div', 'dim pad', 'no UTXOs created yet')); return c; }
  const t = el('table', 'kv');
  for (const l of logs) {
    t.append(el('tr', null,
      `<td class="mono dim">blk ${l.block}</td><td class="mono">#${l.index}</td>` +
      `<td>${label(l.source)} → <b>${label(l.recipient)}</b></td><td class="mono">${fmtEth(l.value)} ETH</td>`));
  }
  c.append(t);
  return c;
}

function bitfieldCard(chain, maxWords = 4) {
  const c = el('div', 'card');
  c.append(el('h4', null, 'Spent bitfield <span class="tag">permanent state · 1 bit per UTXO</span>'));
  const spent = new Set(chain.spent);
  const words = Math.min(Math.max(chain.spentWords, 1), maxWords);
  for (let w = 0; w < words; w++) {
    c.append(el('div', 'dim wordlabel', `word ${w} · indices ${w * 256}–${w * 256 + 255}`));
    const grid = el('div', 'bitgrid');
    for (let b = 0; b < 256; b++) {
      const idx = w * 256 + b;
      const bit = el('div', 'bit' + (spent.has(idx) ? ' set' : idx < chain.nextIndex ? ' live' : ''));
      bit.title = `index ${idx}${spent.has(idx) ? ' — spent' : idx < chain.nextIndex ? ' — unspent' : ''}`;
      grid.append(bit);
    }
    c.append(grid);
  }
  return c;
}

function stateMeter(acctBytes, utxoBytes, note = '') {
  const c = el('div', 'card meter');
  c.append(el('h4', null, 'Permanent state written'));
  const total = Math.max(acctBytes + utxoBytes, 1);
  const wrap = el('div', 'barwrap');
  const bar1 = el('div', 'bar acct', `<span>account model · ${fmtBytes(acctBytes)}</span>`);
  bar1.style.width = Math.max((acctBytes / total) * 100, acctBytes > 0 ? 14 : 0) + '%';
  const bar2 = el('div', 'bar utxo', `<span>UTXO model · ${fmtBytes(utxoBytes)}</span>`);
  bar2.style.width = Math.max((utxoBytes / total) * 100, utxoBytes > 0 ? 14 : 0) + '%';
  wrap.append(bar1, bar2);
  c.append(wrap);
  if (note) c.append(el('div', 'dim pad', note));
  return c;
}

function frameCard(frame, chain, checks = []) {
  const label = labeler(chain);
  const c = el('div', 'card frame');
  c.append(el('h4', null, 'UTXO frame <span class="tag">declared, inspectable data</span>'));
  const t = el('table', 'kv mono');
  t.append(el('tr', null, `<td>actors</td><td>${frame.actors.map(label).join(', ')}</td>`));
  t.append(el('tr', null, `<td>inputs</td><td>${frame.inputs.map((i) => '#' + i).join(', ')}</td>`));
  const outs = [...frame.utxoOuts, ...frame.accountOuts];
  outs.forEach((o, j) => {
    const kind = j < frame.utxoOuts.length ? 'utxo_out' : 'account_out';
    const mark = j === frame.changeIndex ? ' <span class="tag tag-change">change</span>' : '';
    t.append(el('tr', null, `<td>${kind}[${j}]</td><td>${label(o.recipient)} · ${j === frame.changeIndex ? '<i>set at settlement</i>' : fmtEth(o.value) + ' ETH'}${mark}</td>`));
  });
  t.append(el('tr', null, `<td>payer</td><td>${frame.payer === 0 ? '0 — self-funded (UTXO_VAULT pays)' : label(frame.payer)}</td>`));
  t.append(el('tr', null, `<td>max_cost</td><td>${fmtEth(frame.maxCost)} ETH reserved inside conservation</td>`));
  c.append(t);
  if (checks.length) {
    const u = el('ul', 'checks');
    for (const ch of checks) u.append(el('li', ch.ok ? 'ok' : 'bad', `${ch.ok ? '✓' : '✗'} ${ch.text}`));
    c.append(u);
  }
  return c;
}

function gasTable(rows) {
  const t = el('table', 'acct gastable');
  t.append(el('tr', null, '<th></th><th>regular gas</th><th>state gas</th><th>permanent state</th>'));
  for (const [label, rg, sg, b, cls] of rows)
    t.append(el('tr', cls || null,
      `<td>${label}</td><td class="mono">${fmtGas(rg)}</td><td class="mono">${fmtGas(sg)}</td><td class="mono">${fmtBytes(b)}</td>`));
  return t;
}

// --- Per-demo stage renderers ------------------------------------------------

const renderers = {
  1: (stage, p) => renderStandard(stage, p),
  2: (stage, p) => renderStandard(stage, p),
  3: (stage, p) => {
    const grid = el('div', 'stage-grid');
    grid.append(vaultCard(p.chain), logsCard(p.chain), bitfieldCard(p.chain, p.view.bitfieldWords || 1));
    stage.append(grid);
    const card = el('div', 'card');
    card.append(el('h4', null, 'Same payroll, two models'));
    card.append(gasTable(p.view.gasRows));
    if (p.view.verdict) {
      const v = p.view.verdict;
      card.append(el('div', 'pad verdict',
        `Permanent state: <b>${fmtBytes(v.acctBytes)}</b> vs <b>${fmtBytes(v.utxoBytes)}</b> — a ×${v.ratio} difference.
         State gas: ${fmtGas(v.acctState)} vs ${fmtGas(v.utxoState)}.`));
    }
    stage.append(card);
  },
  4: (stage, p, api) => {
    const grid = el('div', 'stage-grid');
    grid.append(vaultCard(p.chain), accountsCard(p.chain, p.view.highlight));
    stage.append(grid);
    if (p.view.frame) stage.append(frameCard(p.view.frame, p.chain, p.view.checks));
    const toggle = el('button', 'btn' + (p.view.attack ? ' danger' : ''),
      p.view.attack ? '⚠ Attack ON: repayment removed — click to restore' : 'Remove repayment output (attack)');
    toggle.onclick = () => api.setFlags({ attack: !p.view.attack });
    const wrap = el('div', 'pad');
    wrap.append(toggle);
    if (p.step >= 1 && p.view.attack) wrap.append(el('span', 'dim', ' — re-running the same steps without the repayment.'));
    stage.append(wrap);
    if (p.view.showBitfield) stage.append(bitfieldCard(p.chain, 1));
  },
};

function renderStandard(stage, p) {
  const grid = el('div', 'stage-grid');
  grid.append(vaultCard(p.chain), accountsCard(p.chain, p.view.highlight));
  stage.append(grid);
  if (p.view.frame) stage.append(frameCard(p.view.frame, p.chain, p.view.checks));
  const grid2 = el('div', 'stage-grid');
  grid2.append(logsCard(p.chain, p.view.logFilter), bitfieldCard(p.chain));
  stage.append(grid2);
  const txs = txsCard(p.chain);
  if (txs) stage.append(txs);
  stage.append(stateMeter(p.view.acctBytes, p.view.utxoBytes, p.view.meterNote));
}

// --- Stepper widget (demos 1–4) ----------------------------------------------

function walletGate(stage, onConnect) {
  stage.innerHTML = '';
  const b = el('div', 'banner-warn',
    `<span>⚠ Connect your wallet using the button in the sidebar to run this demo — <b>you</b> play Alice and sign the vault deposit yourself.</span>`);
  const btn = el('button', 'btn primary', hasWallet() ? 'Connect Wallet' : 'No wallet found');
  btn.disabled = !hasWallet();
  btn.onclick = onConnect;
  b.append(btn);
  stage.append(b);
}

export function mountDemo(root, demoId) {
  root.innerHTML = '';
  const cap = el('div', 'demo-caption');
  const stage = el('div', 'demo-stage');
  const controls = el('div', 'demo-controls');
  const prev = el('button', 'btn', '‹ Back');
  const next = el('button', 'btn primary', 'Next step ›');
  const auto = el('button', 'btn', '▶ Auto-play');
  const resetB = el('button', 'btn', '↺ Reset');
  const count = el('span', 'demo-stepcount');
  controls.append(prev, next, auto, resetB, count);
  root.append(cap, stage, controls);

  let payload = null;
  let flags = {};
  let timer = null;
  let busy = false;
  let gated = false;

  const api = {
    setFlags: (f) => { flags = { ...flags, ...f }; go(payload ? payload.step : -1); },
  };

  const effectiveFlags = () => {
    const acc = walletState().account;
    return demoId === 1 && acc ? { ...flags, wallet: acc } : flags;
  };

  function stopAuto() {
    if (timer) { clearInterval(timer); timer = null; auto.textContent = '▶ Auto-play'; }
  }

  async function go(step) {
    stopAuto();
    const info = await liveInfoPromise;
    // Demo 1 in live mode is played by the visitor's own wallet (MetaMask).
    if (info.live && demoId === 1 && !walletState().account) {
      gated = true;
      cap.innerHTML = 'This demo is played with your own wallet on the live devnet.';
      walletGate(stage, async () => { await pickWalletAndConnect(); });
      prev.disabled = next.disabled = true;
      return;
    }
    gated = false;
    busy = true;
    prev.disabled = next.disabled = resetB.disabled = true;
    if (payload?.live) cap.innerHTML = '<span class="dim">⏳ waiting for devnet blocks…</span>';
    try {
      payload = await gotoStep(demoId, step, effectiveFlags());
      paint();
    } catch (err) {
      cap.innerHTML = `<span class="err">backend error: ${err.message}</span>`;
      prev.disabled = next.disabled = resetB.disabled = false;
    }
    busy = false;
  }

  async function confirmWalletAction(action) {
    busy = true;
    try {
      const txHash = await sendWalletDeposit(action);
      cap.innerHTML = '<span class="dim">⏳ deposit sent — waiting for inclusion…</span>';
      payload = await postAction(demoId, txHash);
      paint();
    } catch (err) {
      cap.innerHTML = `<span class="err">wallet action failed: ${err.message}</span>`;
      paint();
    }
    busy = false;
  }

  function paint() {
    cap.innerHTML = payload.caption;
    count.textContent = payload.step < 0 ? `0 / ${payload.totalSteps}` : `${payload.step + 1} / ${payload.totalSteps}`;
    stage.innerHTML = '';
    renderers[demoId](stage, payload, api);
    if (payload.pendingAction) {
      const b = el('div', 'banner-warn',
        `<span>⚠ <b>Your turn.</b> ${payload.pendingAction.note} — confirm the vault deposit in MetaMask. It is a plain call to
         <span class="mono">0x…8312</span>: any wallet can send it.</span>`);
      const btn = el('button', 'btn primary', 'Confirm in MetaMask');
      btn.onclick = () => { btn.disabled = true; confirmWalletAction(payload.pendingAction); };
      b.append(btn);
      stage.prepend(b);
    }
    prev.disabled = payload.step <= -1 || !!payload.live;
    next.disabled = payload.step >= payload.totalSteps - 1 || !!payload.pendingAction;
    resetB.disabled = false;
  }

  prev.onclick = () => go(Math.max((payload?.step ?? 0) - 1, -1));
  next.onclick = () => { if (!busy && payload && payload.step < payload.totalSteps - 1) go(payload.step + 1); };
  resetB.onclick = () => { flags = {}; go(-1); };
  auto.onclick = () => {
    if (timer) { stopAuto(); return; }
    auto.textContent = '⏸ Pause';
    timer = setInterval(() => {
      if (busy) return;
      if (!payload || payload.step >= payload.totalSteps - 1 || payload.pendingAction) { stopAuto(); return; }
      go(payload.step + 1);
    }, 2800);
  };

  // If the visitor connects their wallet while gated on demo 1, start.
  onWalletChange((s) => { if (s.account && gated) go(-1); });

  go(-1);
}

// --- Scale widget (demo 5) ---------------------------------------------------

export function mountScale(root) {
  root.innerHTML = '';
  const cap = el('div', 'demo-caption',
    `<b>How much permanent state do one-shot payments leave behind?</b> Drag the slider — the numbers come from
     the backend's scale model. The account model writes a ~120 B leaf per fresh recipient at creation and never
     reclaims it. The UTXO model writes <b>nothing</b> at creation and one bit (~0.3 B effective) when spent:
     the permanent cost moves from creation to consumption.`);
  const stage = el('div', 'demo-stage');
  root.append(cap, stage);

  const slider = el('input', 'scale-slider');
  slider.type = 'range'; slider.min = 0; slider.max = 90; slider.value = 0;
  const readout = el('div', 'scale-readout');
  const bars = el('div', 'scale-bars');
  const anchors = el('div', 'scale-anchors');

  const fmtN = (n) => {
    if (n >= 1e9) return (n / 1e9).toLocaleString('en-US', { maximumFractionDigits: 1 }) + ' B';
    if (n >= 1e6) return (n / 1e6).toLocaleString('en-US', { maximumFractionDigits: 1 }) + ' M';
    if (n >= 1e3) return (n / 1e3).toLocaleString('en-US', { maximumFractionDigits: 1 }) + ' k';
    return String(n);
  };

  async function paint() {
    const n = Math.max(1, Math.round(10 ** (slider.value / 10)));
    const r = await scale(n);
    readout.innerHTML = `<span class="mono big">${fmtN(r.count)}</span> one-shot payments to fresh recipients`;
    const la = Math.log10(r.acctBytes), lu = Math.log10(r.utxoBytes), max = Math.max(la, lu);
    bars.innerHTML = '';
    const b1 = el('div', 'scale-bar');
    b1.append(el('div', 'scale-fill acct'), el('span', null, `account model — ${fmtBytes(r.acctBytes)} permanent`));
    b1.querySelector('.scale-fill').style.width = Math.max((la / max) * 100, 2) + '%';
    const b2 = el('div', 'scale-bar');
    b2.append(el('div', 'scale-fill utxo'), el('span', null, `native UTXOs — ${fmtBytes(r.utxoBytes)} permanent`));
    b2.querySelector('.scale-fill').style.width = Math.max((lu / max) * 100, 2) + '%';
    const ratio = Math.round(r.ratio).toLocaleString('en-US');
    bars.append(b1, b2, el('div', 'dim pad', `×${ratio} less permanent state (log-scaled bars). UTXO figure includes the fixed 256 KiB ring and ~10 KiB/year of sealed batches.`));

    anchors.innerHTML = '';
    for (const m of [1e6, 1e9]) {
      const a = await scale(m);
      anchors.append(el('div', 'anchor',
        `<b>${fmtN(m)} payments</b><br>account model: <span class="mono">${fmtBytes(a.acctBytes)}</span><br>` +
        `UTXOs: <span class="mono">${fmtBytes(a.utxoBytes)}</span>`));
    }
  }

  slider.oninput = paint;
  const sliderWrap = el('div', 'card');
  sliderWrap.append(el('h4', null, 'Payments scale'), slider, readout);
  stage.append(sliderWrap, bars, anchors);
  stage.append(el('div', 'card pad dim',
    `For scale: at 1 B payments the account model adds ~112 GiB that nodes must keep hot forever; the UTXO model
     adds ~280 MiB of spent bits, growing a quarter byte per payment. The trade-off (discussed on the ethresear.ch
     thread): existence moves into history, so wallets must keep openings provable — under history expiry, openings
     need a retention story. Bitcoin's alternative keeps the whole unspent set in state instead.`));
  paint();
}
