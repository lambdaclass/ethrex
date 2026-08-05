// End-to-end tests: boot the real HTTP server and drive every demo through its
// JSON API, asserting protocol behavior and invariants on the returned payloads.
// Run: node --test test/
import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from '../server/server.js';
import { Chain, mkaddr } from '../shared/chain.js';

let server, base;

before(async () => {
  server = createServer();
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  base = `http://127.0.0.1:${server.address().port}`;
});

after(() => server.close());

const goto = (id, step, flags = {}) =>
  fetch(`${base}/api/demo/${id}/goto`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ step, flags }),
  }).then(async (r) => ({ status: r.status, body: await r.json() }));

// Vault solvency invariant, checkable from any snapshot:
// vault balance == sum of values of logs whose index is not spent.
function assertSolvent(chain, tag) {
  const spent = new Set(chain.spent);
  const unspentValue = chain.logs
    .filter((l) => !spent.has(l.index))
    .reduce((s, l) => s + l.value, 0);
  assert.ok(Math.abs(chain.vaultBalance - unspentValue) < 1e-6,
    `${tag}: vault ${chain.vaultBalance} != unspent claims ${unspentValue}`);
}

const balanceOf = (chain, label) => chain.accounts.find((a) => a.label === label)?.balance;

test('API: lists demos', async () => {
  const r = await fetch(`${base}/api/demos`);
  const body = await r.json();
  assert.equal(r.status, 200);
  assert.equal(body.live, false);
  assert.deepEqual(body.demos.map((d) => d.id), [1, 2, 3, 4]);
  assert.ok(body.demos.every((d) => d.totalSteps >= 3));
});

test('demo 1 e2e: zero-ETH recipient receives and spends, self-funded', async () => {
  const intro = await goto(1, -1);
  assert.equal(intro.status, 200);
  assert.equal(intro.body.step, -1);
  assert.match(intro.body.caption, /Bob/);

  // walk every step, checking solvency after each transition
  for (let s = 0; s <= 3; s++) {
    const { status, body } = await goto(1, s);
    assert.equal(status, 200);
    assert.equal(body.step, s);
    assertSolvent(body.chain, `demo1 step ${s}`);
  }

  const { body } = await goto(1, 3);
  assert.equal(balanceOf(body.chain, 'Bob'), 0, 'Bob never held ETH');
  assert.ok(body.chain.spent.includes(0), 'input #0 spent');
  assert.ok(body.chain.unspent >= 3, 'three new UTXOs (Charlie, Dave, change)');
  assert.ok(body.view.checks.length === 4 && body.view.checks.every((c) => c.ok));
  assert.equal(body.view.utxoBytes, 0.75, 'one spent bit per UTXO');
  // Charlie's 0.4 output exists in history
  const charlie = body.chain.logs.find((l) => l.value === 0.4);
  assert.ok(charlie && !body.chain.spent.includes(charlie.index));
});

test('demo 2 e2e: stealth consolidation in one multi-actor frame', async () => {
  const { body } = await goto(2, 3);
  assert.equal(body.view.frame.actors.length, 3, 'three stealth actors sign one frame');
  assert.equal(body.chain.spent.length, 3, 'all three inputs consumed');
  assertSolvent(body.chain, 'demo2 final');
  // consolidated output: 1 - 0.00048 fee = 0.99952
  const out = body.chain.logs.find((l) => Math.abs(l.value - 0.99952) < 1e-9);
  assert.ok(out, 'single consolidated output to S4');
  for (const name of ['stealth S1', 'stealth S2', 'stealth S3'])
    assert.equal(balanceOf(body.chain, name), 0, `${name} never held a balance`);
});

test('demo 3 e2e: payroll gas and state accounting matches the EIP', async () => {
  const create = await goto(3, 0);
  assert.equal(create.body.chain.nextIndex, 12, '12 UTXOs in one tx');
  assert.equal(create.body.chain.spent.length, 0);
  assertSolvent(create.body.chain, 'demo3 create');

  const spend = await goto(3, 1);
  assert.equal(spend.body.chain.spent.length, 12, 'all 12 spent');
  assertSolvent(spend.body.chain, 'demo3 spend');
  const [utxoRow, acctRow] = spend.body.view.gasRows;
  assert.equal(utxoRow[2], 12 * 383, 'UTXO state gas = 12 × 383');
  assert.equal(utxoRow[3], 3, '12 spent bits = 3 bytes');
  assert.equal(acctRow[2], 12 * 183600, 'account state gas = 12 × 183,600');
  assert.equal(acctRow[3], 1440, 'account model leaves 1,440 B');

  const score = await goto(3, 2);
  assert.equal(score.body.view.verdict.ratio, 480, '1/480th of the state');
});

test('demo 4 e2e: trustless sponsorship approves repayment, rejects attack', async () => {
  // happy path
  const ok = await goto(4, 2, { attack: false });
  assert.equal(ok.body.view.approved, true);
  assert.ok(ok.body.view.result.ok);
  assert.ok(Math.abs(balanceOf(ok.body.chain, 'Sponsor') - 5.00158) < 1e-9, 'sponsor repaid with spread');
  assert.equal(balanceOf(ok.body.chain, 'Bob'), 0);
  assertSolvent(ok.body.chain, 'demo4 sponsored');

  // attack path: repayment removed -> sponsor never charged, bit never flips
  const bad = await goto(4, 2, { attack: true });
  assert.equal(bad.body.view.approved, false);
  assert.equal(bad.body.view.result.ok, false);
  assert.match(bad.body.view.result.error, /VERIFY/);
  assert.equal(balanceOf(bad.body.chain, 'Sponsor'), 5, 'sponsor never charged');
  assert.equal(bad.body.chain.spent.length, 0, 'spent bit never flipped');
  assertSolvent(bad.body.chain, 'demo4 attack');
});

test('demo 5 e2e: scale model', async () => {
  const r = await fetch(`${base}/api/demo/5/scale`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ count: 1e9 }),
  });
  const body = await r.json();
  assert.equal(body.acctBytes, 1.2e11, '1B payments × 120 B');
  assert.ok(body.utxoBytes > 3e8 && body.utxoBytes < 3.1e8, '1B × 0.3 B + bounded overhead');
  assert.ok(body.ratio > 300 && body.ratio < 500);
});

test('API errors: unknown demo, out-of-range step, bad method', async () => {
  assert.equal((await goto(99, 0)).status, 404);
  assert.equal((await goto(1, 99)).status, 400);
  assert.equal((await goto(1, -5)).status, 400);
  const r = await fetch(`${base}/api/demo/1/goto`);
  assert.equal(r.status, 404, 'GET on goto is not a route');
});

test('engine unit: double spend, same-block spend, conservation', () => {
  const A = mkaddr('a'), B = mkaddr('b'), C = mkaddr('c');
  const ch = new Chain();
  ch.addAccount(A, 'Alice', 10);
  ch.addAccount(B, 'Bob', 0);
  ch.addAccount(C, 'Carol', 0);
  const i = ch.createUtxo(A, B, 1, 0.0003);
  const frame = {
    actors: [B], inputs: [i],
    utxoOuts: [{ recipient: C, value: 0.5 }, { recipient: B, value: 0 }],
    accountOuts: [], changeIndex: 1, payer: 0, maxCost: 0.0006, fee: 0.00042,
  };
  assert.equal(ch.spendFrame(frame).ok, false, 'not spendable in creation block');
  ch.produceBlock();
  assert.equal(ch.spendFrame(frame).ok, true);
  const again = ch.spendFrame(frame);
  assert.equal(again.ok, false);
  assert.match(again.error, /double spend/);

  const j = ch.createUtxo(A, B, 1, 0.0003);
  ch.produceBlock();
  const greedy = ch.spendFrame({ ...frame, inputs: [j], utxoOuts: [{ recipient: C, value: 1.0 }, { recipient: B, value: 0 }] });
  assert.equal(greedy.ok, false, 'conservation enforced');
  assert.ok(!ch.spent.has(j), 'failed VERIFY sets no spent bit');
});

test('static site is served', async () => {
  const html = await fetch(`${base}/`).then((r) => r.text());
  assert.match(html, /EIP-8312/);
  const js = await fetch(`${base}/js/main.js`);
  assert.equal(js.status, 200);
  assert.match(js.headers.get('content-type'), /javascript/);
});
