// EIP-8312 demo server — zero dependencies, node:http only.
// Serves the static site and the JSON API that owns all demo state transitions.
//
//   GET  /api/demos                     -> { live, demos: [{ id, title, totalSteps }] }
//   POST /api/demo/:id/goto             -> { step, flags }  => full client payload
//   POST /api/demo/5/scale              -> { count }        => scale model numbers
//
// Modes:
//   simulated (default) — in-process engine, deterministic replay, instant.
//   live (EIP8312_LIVE=1) — every operation is a real transaction on an ethrex
//   EIP-8312 PoC devnet (EIP8312_RPC), signed by EIP8312_MASTER_KEY-funded actors.
//
// Run:  node server/server.js   (PORT env, default 8000)
import http from 'node:http';
import { readFile } from 'node:fs/promises';
import { existsSync, readFileSync } from 'node:fs';
import { extname, join, normalize, resolve , dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { demos, runDemo, computeScale } from '../shared/scripts.js';
import { LiveChain } from './livechain.js';

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


const ROOT = fileURLToPath(new URL('..', import.meta.url));

const LIVE = process.env.EIP8312_LIVE === '1';
const RPC = process.env.EIP8312_RPC || 'http://localhost:8545';
const KEYS_FILE = process.env.EIP8312_KEYS_FILE || join(repoRoot(ROOT), 'fixtures', 'keys', 'private_keys_l1.txt');

function defaultMasterKey() {
  if (process.env.EIP8312_MASTER_KEY) return process.env.EIP8312_MASTER_KEY;
  if (existsSync(KEYS_FILE)) {
    // line 2: the first richly-funded genesis account on the lambdaclass devnets
    const line = readFileSync(KEYS_FILE, 'utf8').split('\n')[1]?.trim();
    if (line) return line;
  }
  return null;
}
const MASTER_KEY = LIVE ? defaultMasterKey() : null;

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.md': 'text/markdown; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.ico': 'image/x-icon',
};

function send(res, status, body, type = 'application/json; charset=utf-8') {
  const data = Buffer.isBuffer(body) || typeof body === 'string' ? body : JSON.stringify(body);
  res.writeHead(status, { 'Content-Type': type });
  res.end(data);
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let buf = '';
    req.on('data', (c) => {
      buf += c;
      if (buf.length > 64 * 1024) reject(new Error('body too large'));
    });
    req.on('end', () => {
      if (!buf) return resolve({});
      try { resolve(JSON.parse(buf)); } catch { reject(new Error('invalid JSON body')); }
    });
    req.on('error', reject);
  });
}

async function serveStatic(res, path) {
  let rel = normalize(decodeURIComponent(path)).replace(/^([/\\])+/, '');
  if (rel.startsWith('..')) return send(res, 403, 'forbidden', 'text/plain');
  if (rel === '' || rel.endsWith('/')) rel = join(rel, 'index.html');
  try {
    const data = await readFile(join(ROOT, rel));
    send(res, 200, data, MIME[extname(rel)] || 'application/octet-stream');
  } catch {
    send(res, 404, 'not found', 'text/plain');
  }
}

// --- Live sessions ------------------------------------------------------------
// Live demos cannot be replayed (the chain is real), so they advance
// forward-only: goto(-1) starts a fresh scenario, goto(step+1) advances.

const sessions = new Map(); // demoId -> { step, flags, flagsKey, state, pending }

const isAddress = (a) => typeof a === 'string' && /^0x[0-9a-fA-F]{40}$/.test(a);

async function liveGoto(id, step, flags) {
  const d = demos[id];
  if (!d) { const e = new Error(`unknown demo ${id}`); e.status = 404; throw e; }
  if (!Number.isInteger(step) || step < -1 || step > d.steps.length - 1) {
    const e = new Error(`step must be -1..${d.steps.length - 1}`); e.status = 400; throw e;
  }
  if (flags.wallet != null && !isAddress(flags.wallet)) {
    const e = new Error('flags.wallet must be a 20-byte hex address'); e.status = 400; throw e;
  }
  let session = sessions.get(id);
  const fresh = !session || step === -1 || session.flagsKey !== JSON.stringify(flags);
  if (fresh) {
    if (step !== -1) { const e = new Error('live demos start at step -1 (Reset)'); e.status = 409; throw e; }
    session = { step: -1, flags, flagsKey: JSON.stringify(flags), state: null, pending: null };
    session.state = await d.setup(flags, () => new LiveChain({ rpc: RPC, masterKey: MASTER_KEY }));
    sessions.set(id, session);
  } else if (step !== session.step + 1 && step !== session.step) {
    const e = new Error(`live demos advance one step at a time (current: ${session.step}); Reset to start over`);
    e.status = 409; throw e;
  }
  if (step === session.step + 1) {
    try {
      await d.steps[step].run(session.state);
    } catch (err) {
      if (!err.walletAction) throw err;
      // The visitor must confirm this transaction in their own wallet; the
      // step resumes when the frontend posts the resulting tx hash to /action.
      session.pending = { stepIdx: step, action: err.walletAction };
      return {
        id, step: session.step, totalSteps: d.steps.length, flags: session.flags, live: true,
        caption: typeof d.steps[step].caption === 'function' ? d.steps[step].caption(session.state) : d.steps[step].caption,
        view: d.view(session.state, session.step, session.flags),
        chain: await session.state.chain.snapshot(),
        pendingAction: err.walletAction,
      };
    }
    session.step = step;
  }
  const s = session.state;
  const cap = session.step < 0 ? d.intro : d.steps[session.step].caption;
  return {
    id, step: session.step, totalSteps: d.steps.length, flags: session.flags, live: true,
    caption: typeof cap === 'function' ? cap(s) : cap,
    view: d.view(s, session.step, session.flags),
    chain: await s.chain.snapshot(),
  };
}

async function liveAction(id, txHash) {
  if (typeof txHash !== 'string' || !/^0x[0-9a-fA-F]{64}$/.test(txHash)) {
    const e = new Error('txHash must be a 32-byte hex hash'); e.status = 400; throw e;
  }
  const d = demos[id];
  const session = sessions.get(id);
  if (!d || !session || !session.pending) { const e = new Error('no pending wallet action for this demo'); e.status = 409; throw e; }
  const { stepIdx, action } = session.pending;
  session.pending = null;
  await session.state.chain.completeWalletDeposit(action, txHash);
  await d.steps[stepIdx].run(session.state); // resumes: createUtxo returns the recorded index
  session.step = stepIdx;
  const s = session.state;
  const cap = d.steps[stepIdx].caption;
  return {
    id, step: session.step, totalSteps: d.steps.length, flags: session.flags, live: true,
    caption: typeof cap === 'function' ? cap(s) : cap,
    view: d.view(s, session.step, session.flags),
    chain: await s.chain.snapshot(),
  };
}

export function createServer() {
  return http.createServer(async (req, res) => {
    const url = new URL(req.url, 'http://localhost');
    const path = url.pathname;
    try {
      if (path === '/api/demos' && req.method === 'GET') {
        return send(res, 200, {
          live: LIVE,
          rpc: LIVE ? RPC : null,
          demos: Object.values(demos).map((d) => ({ id: d.id, title: d.title, totalSteps: d.steps.length })),
        });
      }
      const gotoMatch = path.match(/^\/api\/demo\/(\d+)\/goto$/);
      if (gotoMatch && req.method === 'POST') {
        const body = await readBody(req);
        const id = Number(gotoMatch[1]);
        if (LIVE) return send(res, 200, await liveGoto(id, body.step, body.flags || {}));
        return send(res, 200, await runDemo(id, body.step, body.flags || {}));
      }
      const actionMatch = path.match(/^\/api\/demo\/(\d+)\/action$/);
      if (actionMatch && req.method === 'POST') {
        if (!LIVE) { const e = new Error('wallet actions only exist in live mode'); e.status = 400; throw e; }
        const body = await readBody(req);
        return send(res, 200, await liveAction(Number(actionMatch[1]), body.txHash));
      }
      if (path === '/api/faucet' && req.method === 'POST') {
        if (!LIVE) { const e = new Error('the faucet only exists in live mode'); e.status = 400; throw e; }
        const body = await readBody(req);
        if (!isAddress(body.address)) return send(res, 400, { error: 'address must be 20-byte hex' });
        const chain = new LiveChain({ rpc: RPC, masterKey: MASTER_KEY });
        const from = await chain.masterAddress();
        const nonce = await chain.nextNonce(from);
        const r = await chain.forge({ op: 'transfer', key: MASTER_KEY, to: body.address, valueWei: '1000000000000000000', nonce });
        return send(res, 200, r);
      }
      if (path === '/api/balance' && req.method === 'POST') {
        const body = await readBody(req);
        if (!isAddress(body.address)) return send(res, 400, { error: 'address must be 20-byte hex' });
        const chain = new LiveChain({ rpc: RPC, masterKey: MASTER_KEY });
        const bal = await chain.rpcCall('eth_getBalance', [body.address, 'latest']);
        return send(res, 200, { balanceWei: BigInt(bal).toString() });
      }
      if (path === '/api/demo/5/scale' && req.method === 'POST') {
        const body = await readBody(req);
        return send(res, 200, computeScale(Number(body.count) || 1));
      }
      if (path.startsWith('/api/')) return send(res, 404, { error: 'unknown endpoint' });
      return serveStatic(res, path);
    } catch (err) {
      return send(res, err.status || 500, { error: err.message });
    }
  });
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  const port = Number(process.env.PORT) || 8000;
  if (LIVE && !MASTER_KEY) {
    console.error('EIP8312_LIVE=1 but no master key: set EIP8312_MASTER_KEY or EIP8312_KEYS_FILE');
    process.exit(1);
  }
  createServer().listen(port, () => {
    console.log(`EIP-8312 demo → http://localhost:${port} (${LIVE ? `LIVE via ${RPC}` : 'simulated'})`);
  });
}
