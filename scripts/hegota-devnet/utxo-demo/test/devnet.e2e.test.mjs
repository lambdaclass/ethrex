// Live devnet e2e: drives demo 1 (and the sponsored path of demo 4) through the
// backend against a real ethrex EIP-8312 PoC devnet. Skipped unless
// EIP8312_LIVE_TEST=1 and the RPC is reachable.
//
//   EIP8312_LIVE=1 EIP8312_RPC=http://localhost:8545 EIP8312_LIVE_TEST=1 node --test test/devnet.e2e.test.mjs
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { readFileSync , existsSync } from 'node:fs';
import { join , dirname } from 'node:path';
import { createServer } from '../server/server.js';

// Walk up from this file until `fixtures/keys` is found, so the demo can be moved
// within the repo without breaking the key path.
function repoRoot(from) {
  let dir = from;
  for (let i = 0; i < 8; i++) {
    if (existsSync(join(dir, 'fixtures', 'keys', 'private_keys_l1.txt'))) return dir;
    const up = dirname(dir);
    if (up === dir) break;
    dir = up;
  }
  throw new Error('could not locate the repo root (fixtures/keys/private_keys_l1.txt) from ' + from);
}


const RUN = process.env.EIP8312_LIVE_TEST === '1';
const RPC = process.env.EIP8312_RPC || 'http://localhost:8545';
const ROOT = new URL('..', import.meta.url).pathname;
const MASTER = RUN ? readFileSync(join(repoRoot(ROOT), 'fixtures', 'keys', 'private_keys_l1.txt'), 'utf8').split('\n')[1].trim() : null;

function forge(cmd) {
  return new Promise((resolve, reject) => {
    const p = spawn(join(ROOT, '.venv/bin/python'), [join(ROOT, 'devnet/txforge.py')], { stdio: ['pipe', 'pipe', 'pipe'] });
    let out = '', err = '';
    p.stdout.on('data', (d) => (out += d));
    p.stderr.on('data', (d) => (err += d));
    p.on('close', () => {
      try { const b = JSON.parse(out); b.error ? reject(new Error(b.error)) : resolve(b); }
      catch (e) { reject(new Error(e.message + err)); }
    });
    p.stdin.end(JSON.stringify({ rpc: RPC, ...cmd }));
  });
}

let server, base;

async function rpc(method, params) {
  const r = await fetch(RPC, {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
  });
  const b = await r.json();
  if (b.error) throw new Error(b.error.message);
  return b.result;
}

async function reachable() {
  try { await rpc('eth_blockNumber', []); return true; } catch { return false; }
}

before(async () => {
  if (!RUN || !(await reachable())) return;
  server = createServer();
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  base = `http://127.0.0.1:${server.address().port}`;
});

after(() => server?.close());

const goto = (id, step, flags = {}) =>
  fetch(`${base}/api/demo/${id}/goto`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ step, flags }),
  }).then(async (r) => ({ status: r.status, body: await r.json() }));

test('live demo 1: zero-ETH recipient spends on the real devnet', { skip: !RUN, timeout: 300_000 }, async () => {
  const intro = await goto(1, -1);
  assert.equal(intro.status, 200, JSON.stringify(intro.body));
  assert.equal(intro.body.live, true);

  const s0 = await goto(1, 0);
  assert.equal(s0.status, 200, JSON.stringify(s0.body));
  const bobRow = s0.body.chain.accounts.find((a) => a.label === 'Bob');
  assert.equal(bobRow.balance, 0, 'Bob unfunded');
  assert.ok(s0.body.chain.txs.length >= 2, 'funding + deposit txs recorded');
  const utxoCount = s0.body.chain.unspent;

  await goto(1, 1);
  await goto(1, 2);
  const s3 = await goto(1, 3);
  assert.equal(s3.status, 200, JSON.stringify(s3.body));
  assert.equal(s3.body.chain.spent.length, 1, 'one spent bit set');
  assert.ok(s3.body.chain.unspent >= utxoCount + 2, 'three outputs created, one input consumed');
  assert.equal(s3.body.chain.accounts.find((a) => a.label === 'Bob').balance, 0, 'Bob still 0 ETH after spending');

  // verify the spent bit on-chain, not just in the session
  const idx = s3.body.chain.spent[0];
  const slot = (1n << 129n) + BigInt(idx >> 8);
  const word = BigInt(await rpc('eth_getStorageAt', ['0x0000000000000000000000000000000000008312', '0x' + slot.toString(16), 'latest']));
  assert.ok(word & (1n << BigInt(idx & 0xff)), `spent bit for #${idx} set on-chain`);
});

test('live demo 4: sponsored spend repays the sponsor on-chain', { skip: !RUN, timeout: 300_000 }, async () => {
  await goto(4, -1);
  const s0 = await goto(4, 0);
  assert.equal(s0.status, 200, JSON.stringify(s0.body));
  const sponsorBefore = s0.body.chain.accounts.find((a) => a.label === 'Sponsor').balance;

  await goto(4, 1);
  const s2 = await goto(4, 2);
  assert.equal(s2.status, 200, JSON.stringify(s2.body));
  const sponsorAfter = s2.body.chain.accounts.find((a) => a.label === 'Sponsor').balance;
  assert.ok(sponsorAfter > sponsorBefore - 0.001, `sponsor repaid (before ${sponsorBefore}, after ${sponsorAfter})`);
  assert.equal(s2.body.chain.accounts.find((a) => a.label === 'Bob').balance, 0);
});

test('live sessions reject backward steps', { skip: !RUN, timeout: 120_000 }, async () => {
  await goto(2, -1);
  await goto(2, 0);
  const back = await goto(2, 0); // same step = repaint, allowed
  assert.equal(back.status, 200);
  const bad = await goto(2, 2); // skipping a step is not
  assert.equal(bad.status, 409);
});

test('live demo 1 with a wallet: pendingAction, visitor-signed deposit, resume', { skip: !RUN, timeout: 300_000 }, async () => {
  const wallet = (await forge({ op: 'addressOf', key: MASTER })).address;

  const intro = await goto(1, -1, { wallet });
  assert.equal(intro.status, 200, JSON.stringify(intro.body));
  const aliceRow = intro.body.chain.accounts.find((a) => a.label.startsWith('Alice'));
  assert.equal(aliceRow.realAddr.toLowerCase(), wallet.toLowerCase(), 'Alice is the visitor wallet');
  assert.ok(!intro.body.chain.txs.some((t) => t.note.includes('fund Alice')), 'wallet Alice is not backend-funded');

  const step0 = await goto(1, 0, { wallet });
  assert.equal(step0.status, 200, JSON.stringify(step0.body));
  const action = step0.body.pendingAction;
  assert.ok(action, 'step 0 must pause for wallet confirmation');
  assert.equal(action.to, '0x0000000000000000000000000000000000008312');
  assert.equal(step0.body.step, -1, 'step not advanced before confirmation');

  // play MetaMask: the visitor sends the plain vault call themselves
  const dep = await forge({ op: 'deposit', key: MASTER, recipient: action.data, valueWei: action.valueWei });
  assert.equal(dep.status, '0x1');

  const resumed = await fetch(`${base}/api/demo/1/action`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ txHash: dep.txHash }),
  }).then(async (r) => ({ status: r.status, body: await r.json() }));
  assert.equal(resumed.status, 200, JSON.stringify(resumed.body));
  assert.equal(resumed.body.step, 0, 'step completed after the wallet tx');
  assert.ok(resumed.body.chain.unspent >= 1);
  assert.ok(resumed.body.chain.txs.some((t) => t.hash === dep.txHash), 'wallet tx recorded');

  // the rest of the demo runs unchanged (Bob's self-funded spend)
  await goto(1, 1, { wallet });
  await goto(1, 2, { wallet });
  const fin = await goto(1, 3, { wallet });
  assert.equal(fin.status, 200, JSON.stringify(fin.body));
  assert.equal(fin.body.chain.accounts.find((a) => a.label === 'Bob').balance, 0);
});

test('action endpoint rejects bad input', { skip: !RUN, timeout: 60_000 }, async () => {
  const r = await fetch(`${base}/api/demo/1/action`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ txHash: '0x1234' }),
  });
  assert.equal(r.status, 400);
});
