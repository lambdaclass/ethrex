# EIP-7906 Attack Proof-of-Concepts

Reproductions of real attack classes against the public Hegotá frame-transaction devnet,
each showing the attack succeeding and then being neutralized by an EIP-7906 POST_TX
assertion. Results from actual runs are in [EVIDENCE.md](EVIDENCE.md); the assumptions the
design rests on were verified first and recorded in [pocs/GATE-RESULTS.md](pocs/GATE-RESULTS.md).

## What EIP-7906 defends, and what it does not

A POST_TX frame is an assertion **authored by the transaction's own sender** that runs
read-only after the transaction body and invalidates the whole transaction if it reverts. So
its reach is transaction-intent integrity: *the effects of my transaction match what I
intended*. That is a narrow claim, and stating it narrowly is what makes it defensible.

We classified 81 documented 2025-2026 incidents against "would a POST_TX assertion have
prevented this". **72 were not defendable**, for structural reasons worth being explicit
about:

- **Attacker-signed protocol bugs** — reentrancy, oracle manipulation, arithmetic overflow,
  access-control failures. The attacker signs the losing transaction and would never attach
  a self-reverting assertion to their own exploit; a protocol cannot force a POST_TX frame
  onto a third party's transaction. EIP-7906 is not a protocol-logic patch.
- **Key compromise** — stolen private keys, cloud KMS breaches, insider abuse. A legitimate
  key holder signs a valid transaction; nothing at the assertion layer stops it.
- **Bridge and cross-chain message forgery** — the defect is in verification logic on the
  receiving side, again exploited by an attacker-signed transaction.

What remains is the class where **the victim signs the losing transaction while deceived
about its effects**, plus the class where **the execution environment moves between what the
user was shown and what executes**. Those are what these scenarios demonstrate.

## The scenarios

| id | scenario | what it demonstrates | status |
|---|---|---|---|
| **P0** | [Guard provenance](pocs/poc0_guard_provenance.py) | An assertion the transaction composer cannot strip, substitute, or weaken | **built** |
| **P1** | [Hidden approval + delayed drain](pocs/poc1_approval_drain.py) | A harmless-looking action that secretly grants an unlimited allowance | **built** |
| **P6** | [Implementation swap](pocs/poc6_proxy_swap.py) | A prior-transaction upgrade, and the two assertion forms that silently fail to catch it | **built** |
| **P7** | [Allowance elimination](pocs/poc7_allowance_elimination.py) | Removing the standing-allowance surface entirely | **built** |
| P2 | Multisig control-plane takeover | Real Safe contracts, real owner signatures; a routine-looking transfer that rewrites the control plane | not yet built |
| P3 | Hidden multicall side effect | A transaction presented as a swap that also moves value elsewhere | not yet built |
| P4 | MEV sandwich | A committed `minOut` asserted against the realized fill | not yet built |
| P5 | Oracle time-of-check/time-of-use | A price moved between the quote and execution | not yet built |

P1 and P6 correspond to items 1 and 6 of the published proof-of-concept note; P7 is an
addition; P0 addresses a gap that note does not cover. The four unbuilt scenarios are
specified in `openspec/changes/eip7906-defi-attack-pocs/`.

### P0 — guard provenance

The gap this closes: every published proof-of-concept assumes the POST_TX assertion is
present on the victim's transaction. Under EIP-8141 whoever composes the transaction composes
the frame list, so in exactly the threat models that matter — a phishing frontend,
compromised signing infrastructure — the adversary omits the assertion and the honest wallet
signs a guardless transaction.

`GuardMandatingAccount` closes it by verifying, inside its VERIFY frame, that the final frame
is POST_TX, targets the configured guard, and carries exactly the committed calldata. Six
attempts at the same malicious intent: an honest transaction mines; guard-omitted,
guard-substituted, guard-weakened and non-owner-signed are all refused before the body runs;
and with the correct guard present the assertion itself fires. **No specification change is
required** — it composes `TXPARAM(0x09)`, `FRAMEPARAM`, `FRAMEDATACOPY`, and the fact that a
reverting VERIFY frame invalidates the transaction.

### P1 — hidden unlimited approval

Nothing moves when the victim signs, which is why the pattern works: the loss happens later,
in a transaction the victim never sees. The assertion forbids any approval on the victim's
behalf outside an allowlist, so the transaction is invalidated before an allowance exists and
the attacker's later sweep has nothing to spend. The negative control shows the same guard
permitting a deliberate, allowlisted approval — it discriminates rather than blocking
everything.

### P6 — implementation swap, and two assertion traps

The implementation is replaced in an **earlier** transaction. Three guard variants run
against the identical victim intent:

1. **differential** (`slot_before == slot_after`) — **passes, attack succeeds.** `TXDIFF`'s
   before/after is scoped to the guarded transaction, so a slot this transaction never wrote
   reads the same value both ways.
2. **code hash of the proxy** — **passes, attack succeeds.** An upgrade does not change the
   proxy's bytecode; only the storage slot holding the implementation address moves.
3. **absolute** (`slot_after == value committed at signing time`) — **reverts.**

Variants 1 and 2 are deliberate negative results, kept because they are the evidence that
the intuitive assertion forms fail silently here. The consequence for wallets: detecting a
prior-transaction change requires committing to expected environment values at signing time.

### P7 — standing-allowance elimination

The largest EVM cluster in the incident record. **This one does not revert the attack** —
nothing a victim signs can constrain a transaction the attacker signs. Instead the victim's
interaction becomes a single frame transaction bundling approve-exactly-N, use, and an
assertion that no allowance survives, so the attacker's identical later drain finds nothing.
Surface removal rather than attack blocking; the distinction is stated in the scenario and in
its evidence record.

## Running

Requires `solc` (0.8.31 used here) and a Python environment with `eth-account`, `eth-keys`
and `eth-utils`. Accounts are provisioned from the devnet faucet automatically.

```bash
cd scripts/hegota-devnet/pocs
python run_all.py            # every scenario, then regenerate EVIDENCE.md
python run_all.py P6         # one scenario by id
python poc6_proxy_swap.py    # or directly
```

Each scenario asserts its own outcome and exits non-zero if either phase misbehaves: a
scenario that cannot establish the attack has not demonstrated a defense, and neither is
reported as a success. The suite therefore doubles as a regression test for the EIP-7906
implementation. Every run deploys fresh contracts and funds throwaway accounts, so runs never
share mutable state. The faucet rate-limits per address, so a full back-to-back run may pause
for a top-up.

Everything is additive to the devnet: no chain-config change, no binary swap, no re-genesis,
and no existing predeploy is touched.

## Contract layout, and why some of it is Yul

| component | language | why |
|---|---|---|
| `contracts/targets/*.sol` | Solidity | Use no custom opcodes at all |
| `contracts/guards/*.sol` | Solidity | Assertion logic, reading the diff through the shim |
| `contracts/TxIntrospection.yul` | Yul | Solidity **cannot** emit the EIP-7906 opcodes |
| `contracts/accounts/GuardMandatingAccount.yul` | Yul | Forced — see below |

`verbatim_*`, the only way to emit a non-standard opcode, is available in pure Yul but **not**
inside a Solidity `assembly { }` block (verified on solc 0.8.31 against both the legacy and
`--via-ir` pipelines). So the raw opcodes are confined to one ~40-line shim that guards reach
by `STATICCALL`; this is sound because the opcode gate applies to the enclosing transaction
frame and holds throughout the POST_TX call subtree. It also makes guard logic testable
offline against a mocked introspection source, which an all-Yul guard could never be.

`GuardMandatingAccount` cannot use that escape hatch: `APPROVE` cannot be delegated out of a
static VERIFY frame, and the frame must make no external calls to stay admissible under the
ERC-7562 validation observer. It stays Yul by necessity, and its header says so.

## Gas budgets

Storage writes carry EIP-8037 state gas on top of execution cost — roughly 98k per cold
`SSTORE`. A two-`SSTORE` call needed 232k against a naive 200k budget during bring-up. The
budgets in `pocs/common.py` are deliberately generous and the reason is recorded at each
call site; do not tune them down.

## Fidelity limits

The targets are minimal contracts exhibiting the vulnerable *shape* of an incident class, not
reproductions of any real protocol's code. Loss figures cited in scenario docstrings describe
the class the scenario models; they are not claims about these mock contracts. P2, when built,
is the deliberate exception and will use the real Safe contracts, because mock fidelity would
be the weakest link in that scenario's claim.
