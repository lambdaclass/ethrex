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
about. (The classification is an input to this work rather than part of it: the per-incident
table is not shipped here, so treat the counts as our stated finding rather than something
reproducible from this repository. The structural argument below does not depend on the exact
numbers.)

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
| **P3** | [Hidden side effect](pocs/poc3_hidden_side_effect.py) | A transaction that also moves value to an address the user never saw | **built** |
| **P4** | [Sandwiched swap](pocs/poc4_sandwich.py) | A committed minimum output asserted against the realized fill | **built** |
| **P5** | [Oracle time-of-check/time-of-use](pocs/poc5_oracle_toctou.py) | A price moved between the quote and execution | **built** |
| **P2** | [Multisig control-plane takeover](pocs/poc2_control_plane.py) | **Real Safe** contracts; a routine-looking transfer that rewrites the control plane | **built** |

P1 through P6 correspond to items 1 through 6 of the published proof-of-concept note; P7 is an
addition; P0 addresses a gap that note does not cover.

### P2 — multisig control-plane takeover, against a real Safe

Reproduces the mechanism behind the largest documented loss in the incident record, against
the **real Safe contracts** rather than a mock: real v1.3.0 source, a real three-owner set at
threshold 2, real EIP-712 signatures, real `execTransaction`, and the real
`delegatecall`-overwrites-slot-0 mechanism. Phase A shows two owners approving what is
presented as a routine transfer and the Safe's singleton pointer being replaced — the account
then answers `hijacked() == true`. Phase B submits the identical owner-signed payload through
a guard-mandating executor and it cannot be included.

The shape matters: a real Safe separates signing from submission, so the frame transaction's
sender is the **executor**, whose policy is frozen independently of whatever composed the
payload. The owners' signatures are valid in both phases. What changes is that the transaction
violates the executor's invariant, which is why this demonstrates a defense operating on
transaction **effects** rather than on authorization. The differential assertion is correct
here — unlike P5 and P6 — because the control-plane write happens inside the guarded
transaction.

**The Safe Transaction Guard contrast.** Safe already ships `setGuard`, so the obvious question
is why an application-level guard did not prevent this class. A real permissive Safe Guard is
installed on a third Safe and the identical attack still succeeds. A Safe Guard is a
*precondition*: `checkTransaction` runs before execution and sees only the proposed target,
value, calldata and operation, so it must anticipate the dangerous shape in advance. A POST_TX
assertion is a *postcondition* over actual effects and needs no such foresight. The honest
caveat, stated in the scenario too: a Safe Guard that specifically rejected `DelegateCall` or
allowlisted its targets **would** have blocked this attack. The demonstrated difference is one
of reach and of what has to be known ahead of time, not that Safe Guards are ineffective.

**Deployment notes.** The Safe contracts are deployed with plain `CREATE` at non-canonical
addresses; the Safe Singleton Factory exists for cross-chain address determinism, which is
irrelevant on one devnet. They are compiled with the pinned solc rather than the 0.7.6 used
for the official v1.3.0 release, so bytecode is not byte-identical to canonical even though
the source is the real thing. `CompatibilityFallbackHandler` is omitted: it does not compile
under solc 0.8.x, and nothing this scenario touches lives there.

**The singleton only just fits.** EIP-8037 charges state gas on the code deposit, so the
default build (12,056 bytes) needs more than EIP-7825's per-transaction ceiling of 2**24 gas
and **cannot be deployed**. The size-optimized build (`--via-ir --optimize-runs 1`, 10,353
bytes) lands at 16,278,183 gas — roughly 3% under the cap. That headroom is the whole margin:
contracts moderately larger than a Safe singleton cannot be deployed on a Hegotá-configured
chain in a single transaction. See [NOTES-FOR-7906-AUTHOR.md](NOTES-FOR-7906-AUTHOR.md).

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

### P3 — a transaction that does one extra thing

The victim is shown a transfer; the transaction also moves tokens to an address they never
saw. The assertion targets the exfiltration address's balance slot, and the differential form
is correct here — unlike P5 and P6 — because the adversarial write happens *inside* the
guarded transaction.

The general form, deny-by-default over every slot written, would not require knowing the
exfiltration address in advance and is what you would reach for first. It is currently
unusable: block building and simulation disagree on the transaction's storage-change set (5
versus 2), so such a guard passes simulation and reverts during block building. It then
*appears* to defend, because the malicious transaction does not land — and only the negative
control reveals that it rejects the honest transaction too. That is how the divergence was
found; see [pocs/GATE-RESULTS.md](pocs/GATE-RESULTS.md).

### P4 — sandwiched swap

The pool is moved against the victim before their swap lands, and the assertion carries the
minimum output they committed to at signing time. Two results worth noting:

- The defended transaction is excluded **and the approved gas payment is rolled back, so the
  victim pays nothing** — measured at 0 wei. Today the choice is between eating a bad fill and
  reverting on slippage while still burning gas. The attacker's own trades still execute, so a
  searcher who moved the pool can be left holding the position.
- That same property leaves a builder bearing uncompensated execution cost, which is the
  anti-denial-of-service question already open in `docs/eip-7906.md` and a plausible incentive
  to deprioritize guarded traffic.

The pool is moved in an earlier transaction rather than by winning a same-block ordering race.
The economics the victim experiences are identical, and it avoids making the result depend on
beating the builder's ordering, which frame-transaction gossip makes unreliable. The strict
same-block variant is not claimed here.

### P5 — oracle time-of-check/time-of-use

The victim's own trade fills at a price that moved after they were shown a quote. Worth
keeping a naming collision straight: "oracle manipulation" as a *hack class* — a flash-loaned
pool skewed to drain a lending market — is attacker-signed and outside EIP-7906's reach
entirely. This is the different thing wearing the same name.

Same three-way comparison as P6: the differential assertion passes and the bad fill lands,
while both an absolute price-band assertion and a net-effect bound on the realized output stop
it. The net-effect bound is the more practical of the two, since it needs no knowledge of the
counterparty's storage layout.

## Reproducing this

### Prerequisites

- **`solc` 0.8.31** on `PATH`. The version is *checked*, not merely documented: the Safe
  singleton deploys with roughly 3% gas headroom, so a compiler emitting slightly larger code
  makes that scenario fail with an opaque out-of-gas. Override with `POC_ALLOW_ANY_SOLC=1` if
  you accept that bytecode and gas will differ from the committed evidence.
- **Python 3.11+** with the pinned dependencies.
- **`git`**, used once to fetch the Safe sources.
- Network access to the public devnet endpoints and faucet. No account or key of your own is
  needed — the harness provisions its own.

```bash
cd scripts/hegota-devnet/pocs
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
```

### Running the scenarios

```bash
.venv/bin/python run_all.py          # all eight, then regenerate EVIDENCE.md
.venv/bin/python run_all.py P6 P7    # selected scenarios by id
.venv/bin/python poc6_proxy_swap.py  # or one directly
```

A full run takes on the order of half an hour: every scenario deploys fresh contracts and
waits for real blocks, and several deliberately wait out a transaction that must *never* be
included. Exit status is meaningful — each scenario asserts both of its phases and exits
non-zero if either misbehaves, so a scenario that cannot establish the attack is never
reported as a defense. The suite therefore doubles as a regression test for the EIP-7906
implementation.

### State the harness creates outside the repository

Both are created automatically on first use. Delete either to start clean.

| path | what it is |
|---|---|
| `~/.poc7906_bank_key` | A funding account, persisted so repeated runs reuse it instead of hitting the faucet's per-address rate limit. It holds devnet-only funds. Deleting it makes the next run create and fund a new one. |
| `~/.cache/poc7906-safe-src` | A shallow clone of `safe-global/safe-smart-account` at `v1.3.0-libs.0`, used by the control-plane scenario. Override the location with `SAFE_SRC=/path`. |

Scenario contracts are always deployed fresh, so runs never share mutable on-chain state and
the suite can be re-run at will. The faucet rate-limits per address, so a full back-to-back run
may pause while the funding account is topped up.

### Offline unit tests for the guards

The assertion guards also have tests that need no chain at all, covering the cases a devnet
round-trip is too slow and too coarse to reach — empty diffs, allowlist misses, slot
derivation, boundary indices:

```bash
cd scripts/hegota-devnet
forge install foundry-rs/forge-std --no-git   # first time only
forge test --use "$(which solc)"
```

`--use` is required because some foundry builds ship a solc registry whose checksum for this
release does not match; pointing forge at the same compiler the scenarios use also guarantees
both are testing the same bytecode.

### Reading the results

`EVIDENCE.md` is generated by `run_all.py` from the JSON records in `pocs/evidence/`, so every
figure in it comes from a real run rather than being written by hand. Re-running regenerates
it; anything that stops reproducing disappears from the document instead of going stale.

Note what a *successful defense* looks like: a POST_TX assertion that fires invalidates the
whole transaction, so it is excluded from the block and has **no receipt and no explorer
page**. "admitted, never mined" together with unchanged victim state is the positive result.

### Devnet safety

Everything is additive: no chain-config change, no binary swap, no re-genesis, and no existing
predeploy is touched. Scenarios only deploy new contracts and fund their own throwaway
accounts.

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
