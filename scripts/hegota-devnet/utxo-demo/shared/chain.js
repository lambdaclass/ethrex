// Simulated EIP-8312 chain.
// NOTE: hashes are deterministic pseudo-hashes (FNV-1a based), NOT real keccak256.
// The simulation models the *state transitions* of the EIP, not its cryptography.

export const VAULT = '0x0000000000000000000000000000000000008312';
export const RING_SIZE = 8192;

const MASK64 = 0xffffffffffffffffn;

function fnv(s, seed) {
  let h = seed;
  for (let i = 0; i < s.length; i++) {
    h ^= BigInt(s.charCodeAt(i));
    h = (h * 0x100000001b3n) & MASK64;
  }
  return h;
}

export function pseudoHash(...parts) {
  const s = parts.join('|');
  let out = '';
  for (let k = 0; k < 4; k++) {
    const seed = (0xcbf29ce484222325n ^ ((BigInt(k) * 0x9e3779b97f4a7c15n) & MASK64)) & MASK64;
    out += fnv(s + '#' + k, seed).toString(16).padStart(16, '0');
  }
  return out;
}

export function merkleRoot(leaves) {
  if (leaves.length === 0) return '0'.repeat(64);
  let level = [...leaves];
  while (level.length & (level.length - 1)) level.push('0'.repeat(64));
  while (level.length > 1) {
    const next = [];
    for (let i = 0; i < level.length; i += 2) next.push(pseudoHash(level[i], level[i + 1]));
    level = next;
  }
  return level[0];
}

// leaf = keccak256(index_be8 ++ source ++ recipient ++ value_be32)  (simulated)
export function leafHash(o) {
  return pseudoHash('leaf', o.index, o.source, o.recipient, o.value);
}

export const r6 = (x) => Math.round(x * 1e6) / 1e6;

// Deterministic valid-hex address from a tag, for demo actors.
export const mkaddr = (tag) => '0x' + pseudoHash('addr', tag).slice(0, 40);

export class Chain {
  constructor() {
    this.block = 1;              // block currently being built
    this.accounts = new Map();   // addr -> { label, balance }  (balance in ETH)
    this.vaultBalance = 0;       // ETH locked in UTXO_VAULT
    this.nextIndex = 0;
    this.ring = new Map();       // ring slot -> openings root
    this.batches = new Map();    // batch number -> batch root
    this.spent = new Set();      // spent UTXO indices (the packed bitfield, modeled as a set)
    this.history = new Map();    // index -> opening { index, source, recipient, value, block }  (append-only history)
    this.pending = [];           // openings created in the current block
    this.blockRoots = new Map(); // block -> openings root
    this.logs = [];              // UtxoCreated logs: { block, source, recipient, index, value }
  }

  addAccount(addr, label, balance = 0) {
    this.accounts.set(addr, { label, balance });
    return addr;
  }

  // Simulated mode needs no special sponsor: a plain funded account.
  addSponsor(addr, label, balance = 0) {
    return this.addAccount(addr, label, balance);
  }

  label(addr) {
    if (addr === VAULT) return 'UTXO_VAULT';
    const a = this.accounts.get(addr);
    return a ? a.label : addr.slice(0, 8) + '…';
  }

  // --- Creation: a call to UTXO_VAULT with value and the recipient in calldata ---
  createUtxo(source, recipient, value, fee = 0) {
    const src = this.accounts.get(source);
    if (!src || src.balance < value + fee) throw new Error('insufficient funds');
    src.balance = r6(src.balance - value - fee);
    this.vaultBalance = r6(this.vaultBalance + value);
    const opening = { index: this.nextIndex++, source, recipient, value, block: this.block };
    this.pending.push(opening);
    this.history.set(opening.index, opening);
    this.logs.push({ block: this.block, source, recipient, index: opening.index, value });
    return opening.index;
  }

  // --- Block boundary: the client commits the openings root, like the receipts root ---
  produceBlock() {
    const root = merkleRoot(this.pending.map(leafHash));
    this.blockRoots.set(this.block, root);
    this.ring.set(this.block % RING_SIZE, root);
    if (this.block % RING_SIZE === RING_SIZE - 1) {
      const roots = [];
      for (let b = this.block - RING_SIZE + 1; b <= this.block; b++) roots.push(this.blockRoots.get(b));
      this.batches.set(Math.floor(this.block / RING_SIZE), merkleRoot(roots));
    }
    this.pending = [];
    this.block++;
  }

  // --- The UTXO frame: VERIFY (read-only), APPROVAL (set spent bits), SETTLEMENT ---
  // frame = { actors, inputs: [index], utxoOuts: [{recipient, value}],
  //           accountOuts: [{recipient, value}], changeIndex, payer (0 or address),
  //           maxCost, fee }
  // Returns { ok, error?, change?, newIndices? }. On VERIFY failure nothing mutates.
  spendFrame(f) {
    const inputs = f.inputs.map((i) => this.history.get(i));

    // VERIFY — reads only protocol state: openings roots, spent bits, signatures.
    if (f.inputs.length < 1) return { ok: false, error: 'a spend must consume at least one input' };
    for (const o of inputs) {
      if (!o) return { ok: false, error: 'input does not prove against any openings root' };
      if (this.spent.has(o.index)) return { ok: false, error: `input #${o.index}: spent bit already set — double spend rejected` };
      if (o.block >= this.block) return { ok: false, error: `input #${o.index}: openings root not committed yet — spendable from the next block` };
      if (!f.actors.includes(o.recipient)) return { ok: false, error: `input #${o.index}: proven recipient is not one of the actors` };
    }
    const outs = [...f.utxoOuts, ...f.accountOuts];
    if (f.changeIndex >= outs.length) return { ok: false, error: 'change_index out of range' };
    const spentValue = r6(inputs.reduce((s, o) => s + o.value, 0));
    const signedOut = r6(outs.reduce((s, o) => s + o.value, 0));
    if (f.payer === 0) {
      if (spentValue < signedOut + f.maxCost)
        return { ok: false, error: `conservation violated: inputs ${spentValue} < outputs ${signedOut} + max_cost ${f.maxCost}` };
    } else {
      if (spentValue < signedOut)
        return { ok: false, error: `conservation violated: inputs ${spentValue} < outputs ${signedOut}` };
      const payer = this.accounts.get(f.payer);
      if (!payer || payer.balance < f.fee) return { ok: false, error: 'sponsor cannot cover the fee' };
    }

    // APPROVAL — check-and-set spent bits, journal-external: final even if a later frame reverts.
    for (const o of inputs) this.spent.add(o.index);

    // SETTLEMENT — cannot fail; VERIFY already proved it solvent.
    let fee = 0;
    if (f.payer === 0) {
      fee = f.fee; // paid out of the consumed UTXO value, from the vault
    } else {
      const payer = this.accounts.get(f.payer);
      payer.balance = r6(payer.balance - f.fee);
    }
    const change = r6(spentValue - signedOut - fee);
    // Consuming inputs and creating utxo_outs only reshuffles obligations inside the
    // vault; ETH physically leaves it only for account_outs and the (self-funded) fee.
    this.vaultBalance = r6(this.vaultBalance - fee);
    const source = f.actors.length === 1 ? f.actors[0] : VAULT;
    const newIndices = [];
    outs.forEach((out, j) => {
      const value = j === f.changeIndex ? change : out.value;
      if (j < f.utxoOuts.length) {
        const opening = { index: this.nextIndex++, source, recipient: out.recipient, value, block: this.block };
        this.pending.push(opening);
        this.history.set(opening.index, opening);
        this.logs.push({ block: this.block, source, recipient: out.recipient, index: opening.index, value });
        newIndices.push(opening.index);
      } else {
        let acct = this.accounts.get(out.recipient);
        if (!acct) { acct = { label: out.recipient.slice(0, 8) + '…', balance: 0 }; this.accounts.set(out.recipient, acct); }
        acct.balance = r6(acct.balance + value);
        this.vaultBalance = r6(this.vaultBalance - value); // state.balance[UTXO_VAULT] -= value
      }
    });
    return { ok: true, change, newIndices, spent: f.inputs.slice() };
  }

  spentWordCount() { return Math.ceil(this.nextIndex / 256); }

  // JSON-serializable view of everything a client needs to render the chain.
  snapshot() {
    return {
      block: this.block,
      vault: VAULT,
      vaultBalance: this.vaultBalance,
      nextIndex: this.nextIndex,
      ringUsed: this.ring.size,
      ringSize: RING_SIZE,
      batches: this.batches.size,
      unspent: this.nextIndex - this.spent.size,
      accounts: [...this.accounts].map(([addr, a]) => ({ addr, label: a.label, balance: a.balance })),
      logs: this.logs.slice(-12),
      spent: [...this.spent],
      spentWords: this.spentWordCount(),
    };
  }
}
