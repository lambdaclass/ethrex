// Demo scripts — the backend-owned, DOM-free logic for every demo.
// The server replays setup() + steps[0..step] deterministically on every request,
// so any (step, flags) pair fully determines the state. view() returns only
// JSON-serializable data; all DOM rendering lives in the frontend.
import { Chain, mkaddr } from './chain.js';

const FEE = 0.00042;
const MAXCOST = 0.0006;

// ---------------------------------------------------------------- Demo 1
const D1 = { ALICE: mkaddr('alice'), BOB: mkaddr('bob'), CHARLIE: mkaddr('charlie'), DAVE: mkaddr('dave') };

const demo1 = {
  id: 1,
  title: 'Pay someone who has nothing',
  intro: `<b>Bob has never used Ethereum.</b> His address holds exactly 0 ETH — and under the account
    model, simply receiving ETH would create a permanent ~120-byte account leaf for him. Watch Bob receive
    1 ETH and spend it <b>without ever holding an account balance</b>. Press “Next step”.`,
  setup: async (flags, makeChain) => {
    const chain = makeChain();
    // With a connected wallet, the visitor IS Alice: the deposit is signed in
    // their MetaMask, not by a backend key.
    if (flags.wallet && chain.setExternal) chain.setExternal(D1.ALICE, 'Alice (you)', flags.wallet);
    else await chain.addAccount(D1.ALICE, 'Alice', 10);
    await chain.addAccount(D1.BOB, 'Bob', 0);
    await chain.addAccount(D1.CHARLIE, 'Charlie', 0);
    await chain.addAccount(D1.DAVE, 'Dave', 0);
    return { chain, input: null, frame: null, result: null };
  },
  steps: [
    {
      caption: `<b>Creation.</b> Alice calls <span class="mono">UTXO_VAULT</span> with 1 ETH and Bob's 20-byte
        address as calldata (≈36,500 gas). The vault assigns index <b>#0</b> and emits
        <span class="mono">UtxoCreated</span>. The opening is <b>not stored in state</b> — it lives in the log
        and is committed to the block's openings root at the block boundary. State written at creation: <b>0 bytes</b>.`,
      run: async (s) => {
        s.input = await s.chain.createUtxo(D1.ALICE, D1.BOB, 1, 0.0003);
        await s.chain.produceBlock();
      },
    },
    {
      caption: `<b>Discovery.</b> Bob's wallet scans logs emitted by the vault with
        <span class="mono">topics[2] = Bob</span> (his row is highlighted). No out-of-band message from Alice is
        needed: the opening is discoverable in history and provable against the openings root.`,
      run() { /* discovery is a read of history — no state change */ },
    },
    {
      caption: `<b>The spend frame.</b> Bob declares the whole transition as signed data: one input (#0),
        outputs of 0.4 to Charlie and 0.5 to Dave, change back to himself, and
        <span class="mono">payer = 0</span> — self-funded. Bob still holds 0 ETH. Nothing has executed yet;
        this is data any node (or sponsor) can inspect.`,
      run: async (s) => {
        s.frame = {
          actors: [D1.BOB], inputs: [s.input],
          utxoOuts: [
            { recipient: D1.CHARLIE, value: 0.4 },
            { recipient: D1.DAVE, value: 0.5 },
            { recipient: D1.BOB, value: 0 },
          ],
          accountOuts: [], changeIndex: 2, payer: 0, maxCost: MAXCOST, fee: FEE,
        };
      },
    },
    {
      caption: `<b>VERIFY + settlement.</b> The opening proves against block 1's openings root, the spent bit is
        unset, and conservation holds: <span class="mono">inputs (1 ETH) ≥ outputs (0.9) + max_cost</span>.
        Gas lives <i>inside</i> the conservation rule — the UTXO pays for its own spend. The spent bit flips
        journal-externally, three new UTXOs are created, the actual fee comes out of the consumed value, and the
        remainder lands in the change output. <b>Bob's balance never moved from 0 ETH.</b>`,
      run: async (s) => {
        s.result = await s.chain.spendFrame(s.frame);
        if (!s.result.ok) throw new Error('spend failed: ' + s.result.error);
        await s.chain.produceBlock();
      },
    },
  ],
  view(s, step) {
    return {
      highlight: step >= 1 ? [D1.BOB] : [],
      frame: step >= 2 ? s.frame : null,
      checks: step >= 3 ? [
        { ok: true, text: 'opening #0 proves against openings root of block 1' },
        { ok: true, text: 'spent bit unset at approval; flipped journal-externally' },
        { ok: true, text: `conservation: 1 ETH ≥ 0.9 + ${MAXCOST} max_cost` },
        { ok: true, text: `actual fee ${FEE} ETH paid from the UTXO — Bob paid 0` },
      ] : [],
      logFilter: step === 1 ? D1.BOB : null,
      showBitfield: true,
      acctBytes: step >= 0 ? (step >= 3 ? 360 : 120) : 0,
      utxoBytes: step >= 3 ? 0.75 : 0,
      meterNote: 'Account model: a ~120 B leaf per fresh recipient, forever. UTXO model: one spent bit per consumed UTXO.',
    };
  },
};

// ---------------------------------------------------------------- Demo 2
const D2 = {
  SENDERS: [mkaddr('sender1'), mkaddr('sender2'), mkaddr('sender3')],
  S: [mkaddr('stealth1'), mkaddr('stealth2'), mkaddr('stealth3')],
  S4: mkaddr('stealth4'),
};
const D2_VALUES = [0.3, 0.5, 0.2];
const D2_FEE = 0.00048;
const D2_MAXCOST = 0.0007;

const demo2 = {
  id: 2,
  title: 'Stealth payments that consolidate',
  intro: `<b>Rin uses ERC-5564 stealth addresses:</b> every payment arrives at a freshly derived address that
    only she can link to herself. Under the account model, spending from one would mean funding it with gas
    first — linking it on-chain forever. With native UTXOs she receives and consolidates privately.
    Press “Next step”.`,
  setup: async (_flags, makeChain) => {
    const chain = makeChain();
    for (const [i, a] of D2.SENDERS.entries()) await chain.addAccount(a, `Sender ${i + 1}`, 2);
    for (const [i, a] of D2.S.entries()) await chain.addAccount(a, `stealth S${i + 1}`, 0);
    await chain.addAccount(D2.S4, 'stealth S4', 0);
    return { chain, inputs: [], frame: null };
  },
  steps: [
    {
      caption: `<b>Three payments, three fresh addresses.</b> Each sender derives a new stealth address for Rin
        and creates a UTXO to it: 0.3, 0.5 and 0.2 ETH. Three unrelated recipients, zero account state, and none
        of them holds any ETH for gas.`,
      run: async (s) => {
        s.inputs = await Promise.all(
          D2.S.map((addr, i) => s.chain.createUtxo(D2.SENDERS[i], addr, D2_VALUES[i], 0.0003)),
        );
        await s.chain.produceBlock();
      },
    },
    {
      caption: `<b>Discovery by view tag.</b> Stealth recipients can't pre-filter logs by address, so Rin scans
        the vault's logs by view tag and recovers all three openings. To everyone else they are three random
        addresses with no visible connection — recipient unlinkability across payments.`,
      run() { /* read-only scan */ },
    },
    {
      caption: `<b>One consolidation frame.</b> Rin builds a single spend with
        <span class="mono">actors = [S1, S2, S3]</span>. Each input's proven recipient must be an actor, and
        every actor signs the same spend hash — interactive, but <b>one frame, one fee</b>. Ten stealth
        payments would merge the same way.`,
      run(s) {
        s.frame = {
          actors: [...D2.S], inputs: s.inputs,
          utxoOuts: [{ recipient: D2.S4, value: 0 }],
          accountOuts: [], changeIndex: 0, payer: 0, maxCost: D2_MAXCOST, fee: D2_FEE,
        };
      },
    },
    {
      caption: `<b>Settlement.</b> All three inputs are consumed, one output UTXO (0.99952 ETH) is created to a
        <i>new</i> stealth address S4. None of the stealth addresses ever appeared as
        <span class="mono">tx.sender</span>, none ever held a balance, and the only permanent state is
        <b>three spent bits — 0.75 bytes</b>. (Sender privacy still needs a separate mechanism such as mixing.)`,
      run: async (s) => {
        const r = await s.chain.spendFrame(s.frame);
        if (!r.ok) throw new Error('consolidation spend failed: ' + r.error);
        await s.chain.produceBlock();
      },
    },
  ],
  view(s, step) {
    return {
      highlight: step >= 1 ? [...D2.S] : [],
      frame: step >= 2 ? s.frame : null,
      checks: step >= 3 ? [
        { ok: true, text: 'each input’s proven recipient ∈ actors [S1, S2, S3]' },
        { ok: true, text: '3 signatures over the same spend hash' },
        { ok: true, text: `conservation: 1 ETH ≥ 0 + ${D2_MAXCOST} max_cost — one fee for three payments` },
        { ok: true, text: 'no stealth address appears as tx.sender or holds a balance' },
      ] : [],
      logFilter: null,
      showBitfield: true,
      acctBytes: step >= 3 ? 360 : 0,
      utxoBytes: step >= 3 ? 0.75 : 0,
      meterNote: 'Account-model equivalent: funding 3 stealth addresses with gas creates 3 permanent leaves — and links them. UTXOs: 3 spent bits, no linkage.',
    };
  },
};

// ---------------------------------------------------------------- Demo 3
const D3 = { COMPANY: mkaddr('company'), N: 12, EMP: Array.from({ length: 12 }, (_, i) => mkaddr('emp' + i)) };
const D3_DEPOSIT_GAS = 21000 + D3.N * 15200;
const D3_SPEND_GAS = 62000, D3_SPEND_STATE = 383;
const D3_XFER_GAS = 21000, D3_XFER_STATE = 183600, D3_ACCT_BYTES = 120;

const demo3 = {
  id: 3,
  title: 'Batch payroll',
  intro: `<b>Payroll day:</b> 12 employees, 0.5 ETH each, all first-time recipients. Same workload, two models.
    The chain panel runs the UTXO way; the table keeps the account-model score. Press “Next step”.`,
  setup: async (_flags, makeChain) => {
    const chain = makeChain();
    await chain.addAccount(D3.COMPANY, 'Company', 50);
    for (const [i, e] of D3.EMP.entries()) await chain.addAccount(e, `emp ${i + 1}`, 0);
    return { chain, created: 0, spent: 0 };
  },
  steps: [
    {
      caption: `<b>UTXO creation — one transaction.</b> The company calls the vault twelve times:
        ≈${D3_DEPOSIT_GAS.toLocaleString('en-US')} regular gas, <b>0 state gas, 0 bytes of permanent state</b>.
        Each employee will discover their UTXO in the logs. The account model would already have written
        1,440 B at this point — just for the recipients to exist.`,
      run: async (s) => {
        s.inputs = await Promise.all(D3.EMP.map((e) => s.chain.createUtxo(D3.COMPANY, e, 0.5, 0.0003)));
        s.created = D3.N;
        await s.chain.produceBlock();
      },
    },
    {
      caption: `<b>Employees spend — each self-funded.</b> Nobody had to pre-fund twelve fresh accounts with gas.
        Twelve spends × (62,000 regular + 383 state gas) leave <b>twelve spent bits: 3 bytes</b>. That is the
        complete UTXO cycle's permanent footprint.`,
      run: async (s) => {
        const results = await Promise.all(D3.EMP.map((e, i) => s.chain.spendFrame({
          actors: [e], inputs: [s.inputs[i]],
          utxoOuts: [{ recipient: e, value: 0 }],
          accountOuts: [], changeIndex: 0, payer: 0, maxCost: 0.0006, fee: 0.00042,
        })));
        const bad = results.find((r) => !r.ok);
        if (bad) throw new Error('payroll spend failed: ' + bad.error);
        s.spent = D3.N;
        await s.chain.produceBlock();
      },
    },
    {
      caption: `<b>The score.</b> Payment for payment: the UTXO cycle costs about <b>half the gas</b> and writes
        <b>1/480th of the state</b>. Per 300M-gas block: ~1,600 fresh-account transfers (state gas saturates
        first), adding ≈190 KiB of permanent state — vs ~3,000 complete UTXO cycles adding under 1 KiB.`,
      run() { /* summary only */ },
    },
  ],
  view(s, step) {
    const utxoRegular = (s.created ? D3_DEPOSIT_GAS : 0) + s.spent * D3_SPEND_GAS;
    const utxoState = s.spent * D3_SPEND_STATE;
    const utxoBytes = s.spent * 0.25;
    const acctRegular = s.created ? D3.N * D3_XFER_GAS : 0;
    const acctState = s.created ? D3.N * D3_XFER_STATE : 0;
    const acctBytes = s.created ? D3.N * D3_ACCT_BYTES : 0;
    return {
      highlight: [],
      frame: null,
      checks: [],
      logFilter: null,
      showBitfield: true,
      bitfieldWords: 1,
      gasRows: [
        ['UTXO: create' + (s.spent ? ' + spend' : ''), utxoRegular, utxoState, utxoBytes, 'row-utxo'],
        ['Account: 12 fresh transfers', acctRegular, acctState, acctBytes, 'row-acct'],
      ],
      verdict: step >= 2 ? { acctBytes, utxoBytes, acctState, utxoState, ratio: Math.round(acctBytes / utxoBytes) } : null,
    };
  },
};

// ---------------------------------------------------------------- Demo 4
const D4 = { BOB: mkaddr('bob'), CAROL: mkaddr('carol'), SPONSOR: mkaddr('sponsor'), FUNDER: mkaddr('funder') };
const D4_REPAY = 0.002;
const D4_REQUIRED = 0.0005;

const demo4 = {
  id: 4,
  title: 'Trustless sponsorship',
  intro: `<b>Bob holds a 1 ETH UTXO and 0 ETH.</b> A sponsor fronts his gas and is repaid by a signed output
    in the frame itself. The sponsor never trusts Bob: it approves only because the repayment is part of the
    signed, declared transition. Press “Next step” — and at the end, flip the attack toggle to remove the repayment.`,
  setup: async (flags = {}, makeChain) => {
    const attack = !!flags.attack;
    const chain = makeChain();
    await chain.addAccount(D4.FUNDER, 'Funder', 3);
    await chain.addAccount(D4.BOB, 'Bob', 0);
    await chain.addAccount(D4.CAROL, 'Carol', 0);
    await chain.addSponsor(D4.SPONSOR, 'Sponsor', 5);
    const input = await chain.createUtxo(D4.FUNDER, D4.BOB, 1, 0.0003);
    await chain.produceBlock();
    return { chain, input, frame: null, approved: null, result: null, attack };
  },
  steps: [
    {
      caption: `<b>The frame names its payer.</b> Bob crafts a sponsored spend:
        <span class="mono">payer = Sponsor</span>, 0.7 ETH to Carol as a new UTXO, and an
        <span class="mono">account_out</span> of ${D4_REPAY} ETH repaying the sponsor. The signed payer field
        binds the spend — nobody can strip the sponsor frame and re-run it as self-funded. With a named payer,
        the <span class="mono">max_cost</span> term drops out of conservation and is replaced by this repayment.`,
      run(s) {
        const outs = s.attack ? [] : [{ recipient: D4.SPONSOR, value: D4_REPAY }];
        s.frame = {
          actors: [D4.BOB], inputs: [s.input],
          utxoOuts: [{ recipient: D4.CAROL, value: 0.7 }, { recipient: D4.BOB, value: 0 }],
          accountOuts: outs, changeIndex: 1, payer: D4.SPONSOR, maxCost: 0, fee: FEE,
        };
      },
    },
    {
      caption: (s) => s.chain.isLive
        ? `<b>The sponsor co-signs the envelope.</b> On this PoC devnet the sponsor is a funded EOA: it signs the
           frame transaction and its default code approves payment (the SelfVerify prefix, scope 3). A production
           sponsor would be a contract running the repayment check below before approving — that contract-side
           rejection is what the simulated mode shows on the attack path.`
        : `<b>The sponsor inspects — it does not simulate.</b> Its approval logic is one expression over
           declared data: <span class="mono">exists out in outputs where out.recipient == me ∧ out.value ≥
           ${D4_REQUIRED}</span>. If VERIFY fails, the whole transaction fails and the sponsor is never charged.`,
      run(s) {
        const outs = [...s.frame.utxoOuts, ...s.frame.accountOuts];
        s.approved = outs.some((o) => o.recipient === D4.SPONSOR && o.value >= D4_REQUIRED);
        if (!s.approved && !s.chain.isLive) s.result = { ok: false, error: 'sponsor VERIFY failed: no output repays the payer ≥ its cost' };
      },
    },
    {
      caption: (s) => s.attack
        ? (s.chain.isLive
          ? `<b>What the check prevents.</b> Without the repayment output the simple EOA sponsor signed anyway —
             and was <b>never repaid</b>: the spend succeeded, the sponsor lost its fee. That loss is exactly the
             trust a checking sponsor contract removes (see the simulated mode for the rejecting sponsor).`
          : `<b>Attack rejected.</b> Without the repayment output the sponsor's check fails, VERIFY fails, and the
             transaction is invalid — <b>the sponsor is never charged and the spent bit never flips</b>. The entire
             trust requirement is gone: no relayers, no meta-transactions, no bespoke paymaster code.`)
        : `<b>Settlement.</b> The sponsor pays the actual fee, receives the signed ${D4_REPAY} ETH
           repayment, and keeps the spread as compensation for fronting gas. Bob's spend completes — he never
           held ETH. A sponsor contract can serve any number of concurrent spends: it approves per transaction,
           without signing envelopes or consuming nonces.`,
      run: async (s) => {
        if (s.approved || s.chain.isLive) {
          s.result = await s.chain.spendFrame(s.frame);
          if (!s.result.ok && !s.attack) throw new Error('sponsored spend failed: ' + s.result.error);
          if (s.result.ok) await s.chain.produceBlock();
        }
      },
    },
  ],
  view(s, step) {
    const checks = [];
    if (step >= 1) {
      checks.push(s.approved
        ? { ok: true, text: `repayment found: ${D4_REPAY} ETH ≥ required ${D4_REQUIRED} — APPROVE` }
        : { ok: false, text: `no repayment output to sponsor ≥ ${D4_REQUIRED} — VERIFY fails, tx invalid` });
      if (s.approved) checks.push({ ok: true, text: `conservation: inputs 1 ETH ≥ outputs ${0.7 + (s.attack ? 0 : D4_REPAY)}` });
    }
    if (step >= 2 && s.result && s.result.ok) {
      checks.push({ ok: true, text: `sponsor paid ${FEE}, received ${D4_REPAY} → spread ${(D4_REPAY - FEE).toFixed(5)} ETH` });
      checks.push({ ok: true, text: 'spent bit set journal-externally; Bob’s balance still 0' });
    }
    return {
      highlight: [D4.SPONSOR, D4.BOB],
      frame: step >= 0 ? s.frame : null,
      checks,
      logFilter: null,
      showBitfield: step >= 2,
      attack: s.attack,
      approved: s.approved,
      result: s.result,
    };
  },
};

// ---------------------------------------------------------------- Demo 5 (scale model, no chain)
export const SCALE = {
  ACCT_BYTES: 120,             // ~120 B per fresh account leaf
  UTXO_BYTES: 0.3,             // ~0.3 B effective per spent UTXO
  RING_BYTES: 256 * 1024,      // fixed ring buffer
  BATCH_BYTES_PER_YEAR: 10 * 1024,
};

export function computeScale(n) {
  const count = Math.max(1, Math.min(1e12, Math.round(n)));
  const acctBytes = count * SCALE.ACCT_BYTES;
  const utxoBytes = count * SCALE.UTXO_BYTES + SCALE.RING_BYTES + SCALE.BATCH_BYTES_PER_YEAR;
  return { count, acctBytes, utxoBytes, ratio: acctBytes / utxoBytes };
}

// ---------------------------------------------------------------- registry
export const demos = { 1: demo1, 2: demo2, 3: demo3, 4: demo4 };

// Replay a demo deterministically and return the full client payload.
// In simulated mode the replay is exact; the server drives live sessions
// forward-only against a devnet (see server/livechain.js).
export async function runDemo(id, step, flags = {}, makeChain = null) {
  const d = demos[id];
  if (!d) { const e = new Error(`unknown demo ${id}`); e.status = 404; throw e; }
  if (!Number.isInteger(step) || step < -1 || step > d.steps.length - 1) {
    const e = new Error(`step must be -1..${d.steps.length - 1}`); e.status = 400; throw e;
  }
  const state = await d.setup(flags, makeChain ?? (() => new Chain()));
  for (let i = 0; i <= step; i++) await d.steps[i].run(state);
  const cap = step < 0 ? d.intro : d.steps[step].caption;
  return {
    id, step, totalSteps: d.steps.length, flags,
    caption: typeof cap === 'function' ? cap(state) : cap,
    view: d.view(state, step, flags),
    chain: await state.chain.snapshot(),
  };
}
