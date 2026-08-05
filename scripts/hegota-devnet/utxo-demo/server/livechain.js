// LiveChain — a Chain-compatible adapter that runs every demo operation for real
// against an ethrex EIP-8312 PoC devnet, via JSON-RPC (reads) and the Python
// txforge helper (transaction construction, signing, submission).
//
// The demo scripts address actors by stable "script addresses"; LiveChain maps
// each to a freshly generated devnet key. Funded script accounts are topped up
// from the master key; zero-balance actors (Bob, stealth addresses) are never
// funded — that is the point of the demos.
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const VAULT = '0x0000000000000000000000000000000000008312';
const TXFORGE = fileURLToPath(new URL('../devnet/txforge.py', import.meta.url));
const PYTHON = fileURLToPath(new URL('../.venv/bin/python', import.meta.url));

const wei = (eth) => BigInt(Math.round(eth * 1e18));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export class LiveChain {
  constructor({ rpc, masterKey }) {
    this.rpc = rpc;
    this.masterKey = masterKey;
    this.isLive = true;
    this.accounts = new Map();  // scriptAddr -> { label, realAddr, key }
    this.byReal = new Map();    // realAddr.toLowerCase() -> scriptAddr
    this.openings = new Map();  // index -> { index, source, recipient, valueWei, block } (real addrs)
    this.spent = new Set();
    this.logs = [];             // session-scoped, script-address keyed for labeling
    this.txs = [];              // { hash, note }
    this.block = 0;
    this.nonces = new Map();        // realAddr.toLowerCase() -> next nonce
    this.nonceChains = new Map();   // realAddr.toLowerCase() -> Promise (allocation queue)
    this.masterAddr = null;         // resolved lazily
    this.nextCreateReturns = null;  // set when resuming a wallet-signed deposit
  }

  // --- plumbing ---------------------------------------------------------------

  forge(cmd) {
    return new Promise((resolve, reject) => {
      const p = spawn(PYTHON, [TXFORGE], { stdio: ['pipe', 'pipe', 'pipe'] });
      let out = '', err = '';
      p.stdout.on('data', (d) => (out += d));
      p.stderr.on('data', (d) => (err += d));
      p.on('close', (code) => {
        try {
          const body = JSON.parse(out);
          if (body.error) return reject(new Error(body.error));
          if (code !== 0) return reject(new Error(err || `txforge exited ${code}`));
          resolve(body);
        } catch (e) {
          reject(new Error(`txforge: ${e.message} ${err}`.trim()));
        }
      });
      p.stdin.end(JSON.stringify({ rpc: this.rpc, ...cmd }));
    });
  }

  async rpcCall(method, params) {
    const r = await fetch(this.rpc, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
    });
    const body = await r.json();
    if (body.error) throw new Error(`${method}: ${body.error.message}`);
    return body.result;
  }

  async blockNumber() {
    return parseInt(await this.rpcCall('eth_blockNumber', []), 16);
  }

  // Per-sender nonce allocator: assigns unique nonces to parallel transactions
  // from the same sender without waiting for receipts.
  async nextNonce(addr) {
    const key = addr.toLowerCase();
    const prev = this.nonceChains.get(key) ?? Promise.resolve();
    const p = prev.then(async () => {
      if (this.nonces.has(key)) {
        const n = this.nonces.get(key);
        this.nonces.set(key, n + 1);
        return n;
      }
      const n = parseInt(await this.rpcCall('eth_getTransactionCount', [addr, 'pending']), 16);
      this.nonces.set(key, n + 1);
      return n;
    });
    this.nonceChains.set(key, p.catch(() => {}));
    return p;
  }

  async masterAddress() {
    if (!this.masterAddr) this.masterAddr = (await this.forge({ op: 'addressOf', key: this.masterKey })).address;
    return this.masterAddr;
  }

  // --- Chain interface ----------------------------------------------------------

  async addAccount(scriptAddr, label, balanceEth = 0) {
    const { keys } = await this.forge({ op: 'genkey', n: 1 });
    const { key, address } = keys[0];
    this.accounts.set(scriptAddr, { label, realAddr: address, key });
    this.byReal.set(address.toLowerCase(), scriptAddr);
    if (balanceEth > 0) {
      const from = await this.masterAddress();
      const nonce = await this.nextNonce(from);
      const r = await this.forge({ op: 'transfer', key: this.masterKey, to: address, valueWei: wei(balanceEth).toString(), nonce });
      if (r.status !== '0x1') throw new Error(`funding ${label} reverted: ${r.txHash}`);
      this.txs.push({ hash: r.txHash, note: `fund ${label} with ${balanceEth} ETH` });
    }
    this.block = await this.blockNumber();
    return scriptAddr;
  }

  // The sponsor is a funded EOA that signs the spend envelope; its default code
  // approves payment. Same as addAccount on this PoC.
  addSponsor(scriptAddr, label, balanceEth = 0) {
    return this.addAccount(scriptAddr, label, balanceEth);
  }

  // Register an actor backed by the visitor's own wallet (MetaMask): no key is
  // held server-side, and transactions from it are confirmed in the wallet.
  setExternal(scriptAddr, label, realAddr) {
    this.accounts.set(scriptAddr, { label, realAddr, key: null, external: true });
    this.byReal.set(realAddr.toLowerCase(), scriptAddr);
  }

  label(scriptAddr) {
    if (scriptAddr === VAULT) return 'UTXO_VAULT';
    return this.accounts.get(scriptAddr)?.label ?? scriptAddr.slice(0, 8) + '…';
  }

  realOf(scriptAddr) {
    return this.accounts.get(scriptAddr)?.realAddr ?? scriptAddr;
  }

  async createUtxo(sourceScript, recipientScript, valueEth, _feeIgnored) {
    const src = this.accounts.get(sourceScript);
    const dst = this.accounts.get(recipientScript);
    // Resumed after the visitor confirmed the deposit in their wallet.
    if (this.nextCreateReturns != null) {
      const i = this.nextCreateReturns;
      this.nextCreateReturns = null;
      return i;
    }
    if (src.external) {
      const err = new Error('wallet confirmation required');
      err.walletAction = {
        type: 'wallet-deposit', sourceScript, recipientScript, valueEth,
        to: VAULT, data: dst.realAddr, valueWei: wei(valueEth).toString(),
        note: `Create UTXO → ${dst.label} (${valueEth} ETH)`,
      };
      throw err;
    }
    const nonce = await this.nextNonce(src.realAddr);
    const r = await this.forge({
      op: 'deposit', key: src.key, recipient: dst.realAddr, valueWei: wei(valueEth).toString(), nonce,
    });
    if (r.status !== '0x1') throw new Error(`deposit reverted: ${r.txHash}`);
    this.recordOpening(r.index, valueEth, r.block, sourceScript, recipientScript, src.realAddr, dst.realAddr);
    this.txs.push({ hash: r.txHash, note: `create UTXO #${r.index} → ${dst.label} (${valueEth} ETH)` });
    this.block = Math.max(this.block, r.block);
    return r.index;
  }

  recordOpening(index, valueEth, block, sourceScript, recipientScript, sourceReal, recipientReal) {
    this.openings.set(index, { index, source: sourceReal, recipient: recipientReal, valueWei: wei(valueEth), block });
    this.logs.push({ block, source: sourceScript, recipient: recipientScript, index, value: valueEth });
  }

  // Complete a wallet-signed deposit: the visitor sent the tx themselves; we
  // read the opening back from the receipt (index and value from the log, not
  // from what was requested).
  async completeWalletDeposit(action, txHash) {
    const r = await this.forge({ op: 'waitReceipt', txHash });
    if (r.status !== '0x1') throw new Error(`deposit reverted on-chain: ${txHash}`);
    const created = r.created.find((c) => c.recipient.toLowerCase() === this.accounts.get(action.recipientScript).realAddr.toLowerCase());
    if (!created) throw new Error('no UtxoCreated log for the expected recipient in that tx');
    const valueEth = Number(BigInt(created.valueWei)) / 1e18;
    const src = this.accounts.get(action.sourceScript);
    this.recordOpening(created.index, valueEth, r.block, action.sourceScript, action.recipientScript, src.realAddr, created.recipient);
    this.txs.push({ hash: txHash, note: `create UTXO #${created.index} → ${this.accounts.get(action.recipientScript).label} (${valueEth} ETH) · signed in your wallet` });
    this.block = Math.max(this.block, r.block);
    this.nextCreateReturns = created.index;
    return created.index;
  }

  async produceBlock() {
    const target = this.block + 1;
    let current = await this.blockNumber();
    while (current < target) {
      await sleep(1500);
      current = await this.blockNumber();
    }
    this.block = current;
  }

  async spendFrame(f) {
    const inputs = f.inputs.map((i) => {
      const o = this.openings.get(i);
      if (!o) return { ok: false, error: `unknown input #${i}` };
      return { index: o.index, creationBlock: o.block, source: o.source, recipient: o.recipient, valueWei: o.valueWei.toString() };
    });
    if (inputs.some((i) => i.ok === false)) return inputs.find((i) => i.ok === false);
    const cmd = {
      op: f.payer === 0 ? 'spend' : 'sponsoredSpend',
      actorKeys: f.actors.map((a) => this.accounts.get(a).key),
      inputs,
      utxoOuts: f.utxoOuts.map((o) => ({ recipient: this.realOf(o.recipient), valueWei: wei(o.value).toString() })),
      accountOuts: f.accountOuts.map((o) => ({ recipient: this.realOf(o.recipient), valueWei: wei(o.value).toString() })),
      changeIndex: f.changeIndex,
    };
    if (f.payer !== 0) cmd.sponsorKey = this.accounts.get(f.payer).key;
    let r;
    try {
      r = await this.forge(cmd);
    } catch (e) {
      return { ok: false, error: e.message };
    }
    if (r.status !== '0x1') return { ok: false, error: `spend reverted on-chain (gasUsed ${r.gasUsed}) — ${r.txHash}` };
    for (const i of f.inputs) this.spent.add(i);
    let change = 0;
    const changeReal = f.changeIndex < f.utxoOuts.length ? this.realOf(f.utxoOuts[f.changeIndex].recipient).toLowerCase() : null;
    for (const c of r.created) {
      const value = Number(BigInt(c.valueWei)) / 1e18;
      const recipientScript = this.byReal.get(c.recipient.toLowerCase()) ?? c.recipient;
      this.openings.set(c.index, { index: c.index, source: VAULT, recipient: c.recipient, valueWei: BigInt(c.valueWei), block: r.block });
      this.logs.push({ block: r.block, source: f.actors.length === 1 ? f.actors[0] : VAULT, recipient: recipientScript, index: c.index, value });
      if (changeReal && c.recipient.toLowerCase() === changeReal) change = value;
    }
    this.txs.push({ hash: r.txHash, note: `spend: ${f.inputs.length} input(s) → ${r.created.length} output(s), gas ${r.gasUsed}` });
    this.block = Math.max(this.block, r.block);
    return { ok: true, change, newIndices: r.created.map((c) => c.index), txHash: r.txHash, gasUsed: r.gasUsed };
  }

  spentWordCount() {
    return Math.ceil(Math.max(1, ...this.openings.keys(), 0) / 256) || 1;
  }

  async snapshot() {
    const [block, vaultBal, nextIdx] = await Promise.all([
      this.blockNumber(),
      this.rpcCall('eth_getBalance', [VAULT, 'latest']),
      this.rpcCall('eth_getStorageAt', [VAULT, '0x0', 'latest']),
    ]);
    const accounts = [];
    for (const [script, a] of this.accounts) {
      const bal = await this.rpcCall('eth_getBalance', [a.realAddr, 'latest']);
      accounts.push({ addr: script, realAddr: a.realAddr, label: a.label, balance: Number(BigInt(bal)) / 1e18 });
    }
    return {
      live: true,
      block,
      vault: VAULT,
      vaultBalance: Number(BigInt(vaultBal)) / 1e18,
      nextIndex: parseInt(nextIdx, 16),
      ringUsed: null,   // global ring scan is too expensive; shown as —
      ringSize: 8192,
      batches: null,
      unspent: this.openings.size - this.spent.size, // session-scoped
      accounts,
      logs: this.logs.slice(-12),
      spent: [...this.spent],
      spentWords: Math.ceil(parseInt(nextIdx, 16) / 256),
      txs: this.txs.slice(-10),
    };
  }
}
