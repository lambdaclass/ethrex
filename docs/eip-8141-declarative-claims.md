# EIP-8141: declarative claims, segregated witnesses and HASCLAIM

> **Spec target:** the declarative-claims proposal for EIP-8141, a rewrite of the EIP that
> replaces the transaction's `signatures` list with declarative `claims` whose witnesses travel
> beside the transaction (pull request 7 against the EIPs fork it was drafted in, head
> `787a3761`). Where the proposal leaves a value to be determined or is silent, ethrex decides
> here; every such decision is listed under [Decisions beyond the proposal](#decisions-beyond-the-proposal).

Implementation notes for the declarative-claims proposal in ethrex. It builds on the frame
transactions described in [EIP-8141](./eip-8141.md) and is part of `Fork::Hegota`: it activates
with it. This branch started from an implementation of [EIP-8504](./eip-8504.md) and reuses its
block field, its Engine API field and its pool storage for a different design.

## Overview

A frame transaction says what it needs proven and carries the proofs beside it.

- **Declare.** A transaction has a list of **claims**. A claim `[verifier, key, msg, gas_limit,
  witness_len]` states that the verifier at `verifier` attests that `key` authorizes `msg`. An
  empty `key` is the sender, and an empty `msg` is the transaction's own signature hash. The
  claims are part of the transaction: the transaction hash and the signature hash cover them.
- **Satisfy.** Each claim is satisfied by a **witness**: a signature, or any other proof its
  verifier accepts. Witnesses are not part of the transaction. They travel beside it: in the
  **pooled representation** while the transaction is pending, and in the block's
  `claim_witnesses` once it is included. Every node verifies them before any frame executes: a
  block's before the block executes, against the block's pre-state.
- **Read.** Account code tests for a claim with one instruction, `HASCLAIM(verifier, key, msg)`,
  and approves when it finds the one it requires. It never sees a witness.

Three verifiers are part of the protocol and are evaluated natively: secp256k1, P-256 and
leanSPHINCS. Any other verifier is an active precompile or a deployed contract, which runs in a
**pure context**: a static call that can read nothing but its input and its own code.

## How to use it

1. **Schedule Hegotá.** Claims are part of it, so the chain config needs nothing else:

   ```json
   "config": { "hegotaTime": 0 }
   ```

2. **Create accounts controlled by leanSPHINCS keys.** `leansphincs_sender` derives keys from a
   seed and prints the genesis entries of their accounts, each running validation code for its
   key. Merge the output into the genesis `alloc`.

   ```sh
   cargo run --release --manifest-path tooling/Cargo.toml -p leansphincs_sender -- \
     alloc --accounts 4 > alloc.json
   ```

   ```json
   {
     "0xbd6fea88015ad67cc46f39e2bbeb693df935ae50": {
       "balance": "0x3635c9adc5dea00000",
       "code": "0x5f7feb7cfb04946e4e36765467a24939f9ff4cc1d3a28d47d6f24e7df2dd989adfc37300000000000000000000000000000000000000adb4603e575f5ffd5b60035f5faa",
       "nonce": "0x0",
       "storage": {}
     }
   }
   ```

   The code is `require(HASCLAIM(LEANSPHINCS_VERIFIER, public_key, 0)); APPROVE(3)`: 68 bytes.

3. **Run the node.** No flag is needed, for a single node or to exchange these transactions
   with peers:

   ```sh
   cargo run --release --bin ethrex -- --network genesis.json --dev --metrics
   ```

4. **Send transactions.** The same tool signs `[VERIFY, SENDER]` transactions, each with one
   claim, and submits each one with its signature through `eth_sendRawTransaction`:

   ```sh
   cargo run --release --manifest-path tooling/Cargo.toml -p leansphincs_sender -- \
     send --accounts 4 --rpc-url http://localhost:8545 --rate 2 --count 8
   ```

   ```text
   Sending from 4 accounts on chain 9 at 2 tx/s
   sent 0xd58ef90d46e1e71179e40bfbf2ab8b0723153fd824f407cbd54049d435cda661 from 0xbd6fea88015ad67cc46f39e2bbeb693df935ae50 nonce 0 (7918 bytes)
   sent 0x97fdc1b6768ee4ff6258de1c2815fb125642f2ca91be7bf8fbbd44544ce1e75a from 0x008292b820008d9d7218710fa962c6d9610dd970 nonce 0 (7918 bytes)
   ...
   Done in 3.6s: 8 sent, 0 sent again, 4 seen included, 0 rejected
   ```

5. **Read the result.**

   The receipt of a transaction has type `0x6`, its payer and one entry per frame:

   ```sh
   curl -s -X POST -H 'content-type: application/json' http://localhost:8545 --data \
     '{"jsonrpc":"2.0","id":1,"method":"eth_getTransactionReceipt","params":["0xd58ef90d46e1e71179e40bfbf2ab8b0723153fd824f407cbd54049d435cda661"]}'
   ```

   ```json
   {
     "type": "0x6",
     "status": "0x1",
     "blockNumber": "0xa",
     "gasUsed": "0xedce",
     "payer": "0xbd6fea88015ad67cc46f39e2bbeb693df935ae50",
     "frameReceipts": [
       { "status": "0x1", "gasUsed": "0x84", "executionGasUsed": "0x84", "stateGasUsed": "0x0", "logs": [] },
       { "status": "0x1", "gasUsed": "0xbb8", "executionGasUsed": "0xbb8", "stateGasUsed": "0x0", "logs": [] }
     ]
   }
   ```

   The block that included it has two of these transactions. `eth_getBlockByNumber("0xa", true)`
   returns each transaction with its claims, the block's witnesses and the header's commitment
   to them:

   ```json
   {
     "claimWitnessesHash": "0x22d69cd88ea0ea372572c7dce3c96c5b4ee7ffaa2492b98c74806e3a810303db",
     "claimWitnesses": "0x00f93c7eb91e3c…",
     "transactions": [
       {
         "type": "0x6",
         "hash": "0xd58ef90d46e1e71179e40bfbf2ab8b0723153fd824f407cbd54049d435cda661",
         "claims": [
           {
             "verifier": "0x00000000000000000000000000000000000000ad",
             "key": "0xeb7cfb04946e4e36765467a24939f9ff4cc1d3a28d47d6f24e7df2dd989adfc3",
             "msg": "0x",
             "gasLimit": "0x0",
             "witnessLen": "0x0"
           }
         ]
       }
     ]
   }
   ```

   `claimWitnesses` is 15,490 bytes there: the version byte, a 3-byte list header and two
   7,740-byte signatures with a 3-byte header each. A block without claims has
   `"claimWitnesses": "0x"` and commits to `keccak256("")`,
   `0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470`.

   The node's metrics count what it verified:

   ```sh
   curl -s http://localhost:9090/metrics | grep '^eip8141_claims' | grep -v bucket
   ```

   ```text
   eip8141_claims_admitted_total 8
   eip8141_claims_admission_witness_verification_seconds_sum 0.001425542
   eip8141_claims_admission_witness_verification_seconds_count 8
   eip8141_claims_blocks_total 15
   eip8141_claims_witnesses_total 8
   eip8141_claims_witness_bytes_total 61920
   eip8141_claims_pool_transactions 0
   ```

To submit a transaction without the tool, send the type byte followed by the pooled
representation, with one witness for each claim, in the order of the claims:

```text
0x06 || rlp([tx_body, 0, [witness_0, ...], []])                                   without blobs
0x06 || rlp([tx_body, 0, [witness_0, ...], [blobs, commitments, cell_proofs]])    with blobs
```

An account without code needs no validation code at all: the default code approves when the
transaction carries a claim `[SECP256K1_VERIFIER, "", "", 0, 0]`, whose witness is the account's
secp256k1 signature of the signature hash, `v || r || s` with `v` the recovery id (0 or 1).

## Architecture

```text
eth_sendRawTransaction ──┐                       ┌── NewPooledTransactionHashes ──► peers
                         ▼                       │
 pooled representation ──► admission ──► mempool ┤
 (tx_body, witnesses,     static rules   tx pool └── payload builder
  blob sidecar)           count, lengths    + claim_witnesses_pool        │
                          prefix, budget                                  ▼
                          protocol claims                 block body: transactions,
                          custom claims (state)           claim_witnesses
                          prefix simulation               header: claim_witnesses_hash
                                                                          │
   peers ──► GetPooledTransactions / PooledTransactions                   ▼
                                                    import: verify_claim_witnesses
                                                    (length, hash, count, gas, each witness
                                                     against the pre-state) then execute
```

A frame transaction has two forms. The **pooled representation**,
`rlp([tx_body, wrapper_version, witnesses, blob_sidecar])`, is what a user submits, what the pool
keeps and what peers exchange. In a **block** the transaction is the bare `tx_body` and its
witnesses sit in the block's `claim_witnesses`, where a dependency shared by several
transactions has a single witness. The transaction hash is the same in both.

Verification lives in three layers, so that a node and a stateless guest run the same rules:

| Layer | Crate | What it does |
|---|---|---|
| One witness against one dependency | `ethrex-levm` (`claims.rs`) | the three native verifiers; the pure context (`VM::run_pure_verifier`) |
| A block's `claim_witnesses` | `ethrex-vm` (`claims.rs`) | decode, header commitment, one witness per dependency, each verified against the pre-state |
| Where it is called | `ethrex-blockchain`, `ethrex-guest-program` | both import paths, witness generation, the pool, the builder, stateless execution |

## Transaction

```text
tx_body = [chain_id, nonce_keys, nonce_seq, sender, claims, frames, fees, blob_versioned_hashes]
claims  = [[verifier, key, msg, gas_limit, witness_len], ...]

tx_hash  = keccak256(0x06 || rlp(tx_body))
sig_hash = keccak256(0x06 || 0x00 || rlp(tx_body))
```

`nonce_keys` and `nonce_seq` are [EIP-8250](./eip-8250.md)'s replacement for the proposal's
`nonce`. The transaction has no `signatures` list, and frame mode 4 is reserved.

Static constraints on a claim, checked by `validate_static_constraints`:

- at most `MAX_CLAIMS` (64) claims;
- `key` is empty, or 20 or 32 bytes and not all zeros; for `SECP256K1_VERIFIER` and
  `P256_VERIFIER` the resolved key is an address, so its upper 12 bytes are zero;
- `msg` is empty, or 32 bytes and not all zeros;
- for a protocol verifier `gas_limit` and `witness_len` are zero, since the verifier fixes both;
- for any other verifier `gas_limit` is not zero and `witness_len` is at most
  `MAX_CLAIM_WITNESS_BYTES` (1 MiB);
- the claims' gas adds up to less than 2**64.

Each claim resolves to a **dependency**, `(verifier, key, msg, gas_limit, witness_len)` with
the key and message resolved and the gas and length the verifier's when it is a protocol one.
`tx_dependencies` is the transaction's dependencies sorted and deduplicated, and
`block_dependencies` the same over every frame transaction of a block. The order is the order of
the tuple: `verifier`, `key` and `msg` as byte strings, then the two integers.

## Verifiers

| Verifier | Address | `key` | Witness | Gas |
|---|---|---|---|---|
| `SECP256K1_VERIFIER` | `0x…ab` | signer address | `v \|\| r \|\| s`, 65 bytes | 2,800 |
| `P256_VERIFIER` | `0x…ac` | `keccak256(qx \|\| qy)[12:]` | `r \|\| s \|\| qx \|\| qy`, 128 bytes | 6,700 |
| `LEANSPHINCS_VERIFIER` | `0x…ad` | public key, 32 bytes | signature, 7,740 bytes | 13,200 |

The three are **claim-only** addresses. They are not precompiles: `is_precompile` is false for
them at every fork, calling one from EVM code is calling an account without code (cold access,
empty return data, success), and verifying a claim adds nothing to the block access list. Each is
a native function over `key || msg || witness` (`secp256k1_verifier`, `p256_verifier`,
`leansphincs_verifier`). The two ECDSA verifiers accept one encoding of each signature: `v` is 0
or 1, `0 < r < n` and `0 < s <= n // 2`.

### Pure context

A claim whose verifier is not one of the three is verified by calling the verifier with
`key || msg || witness` and the claim's `gas_limit`. The claim holds if and only if the call
succeeds, whatever it returns.

- The call is static, with no value and no state gas.
- The opcode table halts on everything that reads beyond the input and the verifier's own code:
  `ADDRESS` through `CALLER`, `GASPRICE` through `EXTCODECOPY`, `EXTCODEHASH`, `BLOCKHASH`
  through `SLOTNUM`, `SLOAD`, `TLOAD`, `APPROVE`, `TXPARAM` through `HASCLAIM`, `CALL`,
  `CALLCODE` and `DELEGATECALL`, and the instructions later EIPs add (`TXTRACE`,
  `EVENTDATACOPY`, `SETCODEFROM`). `CALLVALUE` stays, since it is zero.
- `STATICCALL` is allowed only to an active precompile; to anything else it halts.
- A verifier that is itself an active precompile is dispatched directly. An account without
  code, or whose code is an EIP-7702 delegation indicator, is not a verifier.
- The call has its own access sets, with only the precompiles warm. The verifier's entry access
  comes out of the claim's `gas_limit`: 100 for a precompile, the cold account access for a
  contract. Nothing it touches stays warm.
- The transaction is charged the whole `gas_limit`, whatever the verifier uses.

A block's custom claims are verified against the state before the block, so a verifier has to
be deployed in an earlier block than the first one that relies on it.

## Block

- The body's fourth item is `claim_witnesses`, one byte string. It is empty when the block has
  no dependencies. Otherwise it is `0x00 || rlp([witness_0, witness_1, ...])` with one witness
  for each entry of `block_dependencies(block)`, in that order.
- The header's `claim_witnesses_hash` is `keccak256(claim_witnesses)`. It sits between
  `slot_number` and `burned_fees`.
- Both are present from Hegotá, and absent before.

`verify_claim_witnesses` (`ethrex-vm`) runs on every import path before execution and rejects a
block unless:

1. `claim_witnesses` is at most `MAX_BLOCK_WITNESS_BYTES` (2 MiB) long;
2. the header's `claim_witnesses_hash` is its Keccak-256 hash;
3. it decodes to exactly one witness per dependency, and each witness has its dependency's
   `witness_len` and is accepted by its verifier.

Before the verifiers of rule 3 run, the gas of the block's claims is compared with the block's
gas limit, which bounds the work of the check by one block's gas.

The form of the byte string for a given version byte is decided in one function,
`decode_claim_witnesses`, which rejects any version other than `0x00`.

The Engine API carries the field as `claimWitnesses`, one hex string, in the execution payload
of `engine_newPayloadV5` and `engine_getPayloadV6` and in the payload bodies responses. The node
derives the header's commitment from it. An absent field is the empty byte string from Hegotá
on; before that, an absent or empty field means none. `eth_getBlockByNumber` and
`eth_getBlockByHash` return `claimWitnesses` with full transactions and `claimWitnessesHash` in
every form.

## Gas

All of it is intrinsic, fixed by the transaction body:

```text
claim_verification_cost = Σ 2,800 | 6,700 | 13,200 | claim.gas_limit
protocol_witness_cost   = Σ WITNESS_BYTE_GAS (4) * witness length      protocol verifiers
custom_witness_cost     = Σ 16 * claim.witness_len                     any other verifier
claim_metadata_cost     = calldata cost of each claim's verifier, key and msg
```

The first two are charged on both sides of the `max_gas` comparison, so the calldata floor
cannot absorb them, and a protocol witness does not count toward the floor's tokens. A custom
witness does: it costs 16 gas per byte in the intrinsic cost and 64 in the floor, by its declared
length.

| Transaction with one claim | Added to its gas |
|---|---|
| secp256k1, default key and message | 2,800 + 4 × 65 + 92 = 3,152 |
| leanSPHINCS, 32-byte key | 13,200 + 4 × 7,740 + 92 + 512 = 44,764 |
| custom, 32-byte key and message, `gas_limit` 50,000, `witness_len` 300 | 50,000 + 16 × 300 + 1,344 = 56,144 |

The last term of each row is `claim_metadata_cost` for keys, messages and a custom verifier
address without zero bytes; a protocol verifier's address has 19 of them.

A dependency two transactions share is paid by both, while the block carries one witness.

## EVM

- **`HASCLAIM` (`0xb4`)**, 3 gas. <a id="hasclaim"></a> Stack: `verifier` on top, then `key`,
  then `msg`. Pushes 1 when the transaction has a claim with that verifier, that resolved key and
  that `msg`, and 0 otherwise. A zero `msg` matches a claim with an empty `msg`; a non-zero one
  matches a claim with exactly that `msg`, so the signature hash spelled out matches nothing. A
  `verifier` with any of its upper 12 bytes set matches nothing. Outside a frame transaction it
  halts.
- **`TXPARAM`**: the proposal's table (`0x00` to `0x04`, `0x07` to `0x0A`, `0x0C`) plus EIP-8250's
  `0x0D` to `0x10`. `0x0B`, which counted signatures, halts. `0xb5`, which copied signature
  bytes, is not an instruction.
- **Default code** of an account without code, in a `VERIFY` frame: revert if the frame allows no
  scope, revert if `frame.data` is not empty, revert unless
  `HASCLAIM(SECP256K1_VERIFIER, resolved_target, 0)`, then `APPROVE(allowed_scope)`.

## Mempool

Admission follows the proposal's acceptance algorithm:

1. The pooled representation decodes with `wrapper_version` 0.
2. The transaction satisfies the static constraints.
3. It comes with exactly one witness per claim, each of the length its claim fixes, and its
   own witnesses fit a block's `MAX_BLOCK_WITNESS_BYTES`.
4. `claim_verification_cost` alone is within `MAX_VERIFY_GAS`, and the validation prefix is a
   recognized shape whose execution limits plus `claim_verification_cost` are within it.
5. Every protocol verifier's claim is verified, before any state is read.
6. Every other claim is verified in the pure context against the head state.
7. The validation prefix is simulated, as for any frame transaction.

A transaction with claims that arrives without witnesses is rejected, and a second copy of a
pooled transaction is ignored whatever witnesses it brings. The pool keeps the witnesses with the
transaction and drops them with it. Witnesses are verified once: the revalidation after each
block does not repeat it.

The builder takes each transaction's witnesses from the pool, verifies its custom claims again
against the pre-state of the block it builds, skips a transaction whose witnesses would take the
block's `claim_witnesses` over `MAX_BLOCK_WITNESS_BYTES`, and writes one witness per dependency.

## Networking

No protocol version and no message is added. A frame transaction is announced by hash in
`NewPooledTransactionHashes`, with type `0x06` and the size of its pooled representation, and
fetched with `GetPooledTransactions`. `PooledTransactions` carries the pooled representation;
the bare `tx_body` does not decode there. ethrex never sends a frame transaction in a
`Transactions` message. This holds on every `eth` version ethrex speaks, eth/72 included, where
a frame transaction's blobs are never elided.

Block bodies carry `claim_witnesses`, since it is part of the body's RLP.

## Stateless execution

`execute_blocks` in `ethrex-guest-program` runs the same `verify_claim_witnesses` on each block
before it executes it, against the state the execution witness provides. The witness a node
collects on import includes the code of every custom verifier, because the check reads it through
the same logger as execution.

The leanSPHINCS verifier is linked only into native builds (the `leansphincs` feature). A build
without it, such as a zkVM guest, refuses a block that has a `LEANSPHINCS_VERIFIER` claim with
`UnverifiedClaimWitnesses`, and verifies every other claim.

## Metrics

All under the `eip8141_claims_` prefix:

| Metric | Meaning |
|---|---|
| `eip8141_claims_blocks_total`, `eip8141_claims_witnesses_total`, `eip8141_claims_witness_bytes_total` | Blocks checked, and the witnesses in them |
| `eip8141_claims_block_witnesses`, `eip8141_claims_block_witness_bytes` | The most recent block |
| `eip8141_claims_block_witness_verification_seconds` | Time checking one block's witnesses |
| `eip8141_claims_pool_transactions`, `eip8141_claims_pool_witness_bytes` | Frame transactions with claims in the pool |
| `eip8141_claims_admission_witness_verification_seconds` | Time verifying one transaction's protocol claims on admission |
| `eip8141_claims_admitted_total`, `eip8141_claims_rejected_total{reason}` | Admissions, and rejections by the check that failed |

## Decisions beyond the proposal

Each entry gives what the proposal says or leaves open, and then what ethrex does and why.

### Transaction

1. **Verifier addresses.** The proposal leaves the three addresses to be determined, "analogous
   to `ENTRY_POINT`", outside the precompile range and outside the range EIP-7587 reserves.
   ethrex uses `0x…ab`, `0x…ac` and `0x…ad`, the three addresses after `ENTRY_POINT` (`0x…aa`).
   They meet both conditions and nothing else uses them.
2. **Nonce.** The proposal's `tx_body` has one `nonce`. ethrex implements EIP-8250, so the body
   has `nonce_keys` and `nonce_seq` in its place, and both hashes cover them.
3. **Frame modes.** The proposal accepts modes 0 to 2. ethrex also keeps EIP-7906's `ASSERT`
   (3). Mode 4 is reserved again and a transaction that uses it is invalid.
4. **Integer widths.** The proposal bounds `chain_id` at 2**256 and a frame's gas at 2**64.
   ethrex keeps the widths it had: `chain_id` is a `u64` and a frame's gas limits are at most
   2**63 - 1. A claim's `gas_limit` and `witness_len` are `u64`, as the proposal bounds them.
5. **EIP-7825 cap.** The proposal lists it among the static constraints. ethrex keeps
   enforcing it where it did, in the VM and on admission, now with the claims' costs inside
   `frame_tx_intrinsic_gas`.
6. **JSON form of a claim.** The proposal defines none. ethrex serializes a claim as
   `{"verifier", "key", "msg", "gasLimit", "witnessLen"}`, and a transaction's `claims` before
   its `frames`.

### Verification

7. **Where claims are verified.** The proposal requires every claim to hold before any frame
   executes. ethrex verifies a block's claims in one step before the block executes, and a
   pending transaction's on admission. The VM does not verify claims itself: `HASCLAIM` reads
   `tx.claims`. Any other caller that executes a frame transaction (a simulation, a test that
   drives the VM directly) therefore treats its claims as satisfied.
8. **A claim with a malformed `key` or `msg` in a block.** Such a transaction is invalid by the
   static constraints, so its block is invalid. ethrex reports it from the claim check
   (`MalformedClaim`), since that claim has no dependency to verify.
9. **The verifier's entry charge.** The proposal says the verifier's "warm or cold" access is
   drawn from `claim.gas_limit`. The pure context starts with only the precompiles warm, so a
   precompile verifier pays the warm cost (100) and a contract verifier always pays the cold one
   (2,600). A claim whose `gas_limit` does not cover it is not satisfied.
10. **Instructions of later EIPs in the pure context.** The proposal halts on any instruction a
    later EIP introduces unless that EIP permits it. ethrex builds the pure table from Hegotá's
    and halts on the three it has from other EIPs (`TXTRACE`, `EVENTDATACOPY`, `SETCODEFROM`),
    none of which says anything about the pure context.
11. **The pure call's context.** The proposal makes `ADDRESS`, `CALLER` and `ORIGIN` unreadable
    and so does not say what they are. ethrex calls the verifier from `ENTRY_POINT`, at the
    verifier's own address, under a placeholder transaction.
12. **Block access list.** The proposal says a protocol verifier adds nothing to the EIP-7928
    block access list. ethrex records nothing for a custom verifier either: its claim is
    verified outside every transaction, against the pre-state, and it can read no state but its
    own code.
13. **Signature encodings.** As the proposal's functions: one accepted encoding per ECDSA
    signature. ethrex checks the ranges before it calls the curve library, so a high `s` is
    rejected for P-256 although `P256VERIFY` accepts it.

### Block

14. **Position of the new fields.** The proposal does not place them. `claim_witnesses_hash` is
    the header field after `slot_number`, and `claim_witnesses` is the body's fourth item, after
    the withdrawals. Both are where the branch this one started from had the fields they replace.
15. **A block without dependencies.** The proposal says its `claim_witnesses` is the empty byte
    string. ethrex takes that as the only valid form: `0x00 || rlp([])`, an unknown version byte,
    bytes after the list and a list item that is not a byte string are all invalid.
16. **The header commitment is checked with the witnesses.** Rule 2 is checked by
    `verify_claim_witnesses`, not only by the body validation, because one of ethrex's two
    import paths does not run the body validation.
17. **Order of the checks and their errors.** The proposal lists three rules. ethrex checks, in
    order: the length and the form (`InvalidClaimWitnesses`), the commitment
    (`ClaimWitnessesHashMismatch`), the claims' form (`MalformedClaim`), the number of witnesses
    (`ClaimWitnessesCountMismatch`), the claims' gas (entry 18), then each witness in order
    (`ClaimNotSatisfied(index)`). The witnesses are verified in sequence, not in parallel.
18. **The work of the claim check.** The proposal verifies every dependency before any
    transaction executes, while the rules that bound a block's gas apply as its transactions
    execute. Before it runs a verifier, ethrex adds up the gas of the claims of the block's
    frame transactions and rejects the block when the total is over its gas limit
    (`ClaimGasExceedsGasLimit`, reported as a gas allowance error). A valid block never fails
    this, since a claim's gas is part of its transaction's intrinsic gas. Without it, a block
    that execution would reject could make a node run a verifier for up to 2**64 gas.
19. **Engine API.** The proposal passes `claim_witnesses` as an opaque byte string through
    `engine_newPayload` and `engine_getPayload`, and names no method version or JSON key. ethrex
    adds `claimWitnesses`, a hex string, to the payload of the methods Hegotá already uses
    (`engine_newPayloadV5`, `engine_getPayloadV6`) and to the payload bodies. Absent means empty
    from Hegotá on. Before Hegotá a non-empty value makes the block invalid.
20. **Block JSON.** Not in the proposal. `eth_getBlockBy*` returns `claimWitnessesHash` always
    and `claimWitnesses` when it returns full transactions.
21. **Genesis.** A genesis at Hegotá commits to `keccak256("")` and has empty claim witnesses.
22. **Version byte.** One function, `decode_claim_witnesses`, decides what the byte string means
    for each version byte and accepts only `0x00`.

### Mempool and block building

23. **The pooled representation is the only way in.** `PooledTransactions` and
    `eth_sendRawTransaction` take a frame transaction only in its pooled representation, also
    when it has no claims (with `witnesses` empty). A bare `tx_body` sent to
    `eth_sendRawTransaction` is rejected with a message that says so.
24. **Blob sidecar.** The proposal has one `wrapper_version` for the whole wrapper. With version
    0 the sidecar is always `[blobs, commitments, cell_proofs]`, the EIP-7594 form; a blob
    sidecar with per-blob proofs has no encoding.
25. **A second copy with other witnesses.** A witness is not covered by the transaction hash, so
    two valid pooled forms of one transaction can differ. ethrex keeps the first and ignores the
    rest.
26. **A transaction that cannot fit any block.** The proposal bounds one witness at 1 MiB and
    a block's witnesses at 2 MiB. ethrex rejects, on admission and before it verifies anything,
    a transaction whose own witnesses exceed `MAX_BLOCK_WITNESS_BYTES`, since no block could
    carry it. Claims that resolve to one dependency count as one witness. The EIP-7825 cap
    rejects such a transaction too, but only once its gas is computed.
27. **Size of a pooled transaction.** The proposal gives no limit for the pooled
    representation. ethrex caps the transaction body at 128 KiB, like any other transaction,
    and does not count the witnesses toward that cap: entry 26 and the gas they cost bound them.
28. **`claim_verification_cost` against `MAX_VERIFY_GAS`.** As the proposal: it is added to the
    validation prefix's execution limits. ethrex also rejects, earlier and with its own error, a
    transaction whose claims alone exceed the limit.
29. **Revalidation.** The proposal re-simulates the affected prefixes after each block. ethrex
    does not verify witnesses again there: a protocol verifier's result depends on no state, and
    custom claims are verified again when the transaction is built into a block.
30. **Building.** The proposal says nothing about it. The builder verifies a transaction's
    custom claims against the pre-state of the block it builds and drops the transaction from
    the pool when one fails. It skips, and keeps in the pool, a transaction whose witnesses do
    not fit the block. When two transactions share a dependency it keeps the first one's witness.
31. **No limit flag.** `MAX_CLAIMS` is a consensus constant, so the option that bounded the
    signatures of a transaction in the pool is removed.
32. **Peers.** A peer that sends a transaction with an invalid witness is not penalized, as for
    any other transaction the pool rejects. The proposal does not ask for it.

### Networking and RPC

33. **No new protocol version.** As the proposal. The `eth/73` version and its two messages,
    which the branch this one started from added, are removed.
34. **eth/72.** The proposal does not consider it. There a blob transaction travels without its
    blobs, which are sampled by cell. That encoding and its cell mask are defined for type 3
    only, so a frame transaction with blobs travels whole.
35. **A frame transaction in a `Transactions` message.** ethrex does not send one. One received
    is admitted only if it has no claims, since it brings no witnesses.

### EVM

36. **`TXPARAM`.** The proposal's table, plus the four parameters of EIP-8250 (`0x0D` to
    `0x10`), with `0x01` returning `nonce_seq`.
37. **Default code.** As the proposal, plus one check ethrex already had: a frame that allows
    the execution scope reverts unless its target is the sender. The static constraints already
    exclude that case.

### Other

38. **Stateless execution.** The proposal does not cover it. The guest verifies a block's
    claims with the same function as the node, and refuses a block with a `LEANSPHINCS_VERIFIER`
    claim when the verifier is not linked into its build.
39. **Metrics.** Renamed to the `eip8141_claims_` prefix; those of the removed gossip are gone.
40. **Test-suite exception names.** The consensus test runner maps
    `BlockException.INVALID_CLAIM_WITNESSES` and `BlockException.INVALID_CLAIM_WITNESSES_HASH`.
    No test suite defines names for this proposal yet, so these two are placeholders.
41. **Reference vectors.** There is no other implementation to take vectors from. The golden
    vector of the encoding and the two hashes comes from a second implementation written for
    the purpose with its own RLP and Keccak-256.
42. **Devnet network file.** `fixtures/networks/pq-el-devnet-0.yaml` is removed: it described a
    network for the design this branch started from.

## Where the proposal is ambiguous or incomplete

- **Custom verifiers and the block access list.** The text excludes protocol verifiers from
  the block access list and says nothing about a custom verifier, whose code a node does read
  from the pre-state. Decision 12 records nothing. A stateless verifier still needs that code
  in its execution witness.
- **A verifier whose code changes.** A custom claim is verified against the block's pre-state
  and its transaction executes later in the block. The text does not say whether anything binds
  the two. Nothing does here: a claim that held in the pre-state stays satisfied for the block.
- **`MAX_CLAIM_WITNESS_BYTES` cannot bind.** The calldata floor charges a custom witness 64 gas
  per byte and the EIP-7825 cap of 16,777,216 gas applies to the floor, so the custom witnesses
  of one transaction total under 262,144 bytes. The 1 MiB constant is never the limit that
  binds, and no valid transaction comes near `MAX_BLOCK_WITNESS_BYTES` on its own. The
  encoding still lets a transaction declare 64 MiB of witnesses, which a node has to reject
  before it does any work on them; see decision 26.
- **`claim_witnesses` and the block size limit.** The text bounds `claim_witnesses` at 2 MiB
  and does not say whether it counts toward EIP-7934's limit on a block's RLP size. Here it
  does: it is part of the body, so the builder counts it and a block over the limit is invalid.
- **"Warm or cold" entry access.** With only the precompiles warm, a contract verifier is never
  warm. See decision 9.
- **`wrapper_version` and blobs.** The pooled wrapper has one version for the witnesses and the
  blob sidecar, and the text points to EIP-7594 for both the version and the sidecar. See
  decision 24.
- **Protocol versions that elide blobs.** The pooled representation carries the whole sidecar.
  The text does not say how it combines with a protocol version in which blob payloads are
  fetched by cell. See decision 34.
- **The work an invalid block can cause.** Rule 3 runs before any transaction executes, and
  the text does not say which gas rules hold by then. Until a transaction's gas is checked
  against the EIP-7825 cap and the block's gas limit, a claim's `gas_limit` is bounded only by
  2**64, so a node that follows the text's order runs a verifier for as long as a block's
  author chooses. See decision 18.
- **`TXPARAM` and the nonce.** The table has `nonce`; with EIP-8250 the transaction has none.
  See decision 36.
- **Constants marked provisional.** `MAX_BLOCK_WITNESS_BYTES` and `WITNESS_BYTE_GAS` are
  provisional in the text, and are implemented with the values it gives.

## Not implemented

- **Aggregation.** `claim_witnesses` has the raw form only. There is no proof of the witnesses,
  no version byte other than `0x00`, and nothing that depends on a later fork.
- **The consensus client.** Nothing outside the execution client changes. A consensus client
  has to carry `claim_witnesses` in the execution payload, bounded by
  `MAX_BLOCK_WITNESS_BYTES`, and pass it through the Engine API.
- **The SSZ payload of stateless validation.** `NewPayloadRequest` has no `claim_witnesses`
  field, so the path that validates a block from that request cannot carry a Hegotá block. The
  guest's block execution, which takes whole blocks, does verify claims.
- **Pruning `claim_witnesses` from history.** The proposal allows it. ethrex stores them with
  the body and serves them with it.
- **Direct evaluation of protocol-defined frames.** The proposal allows a node to skip the
  simulation of a validation prefix made only of protocol-defined frames. ethrex always
  simulates.
- **Building a block from transactions given explicitly.** A builder call that names its
  transactions (`testing_buildBlockV1`) takes bare transactions, so it can include a
  transaction with claims only if the pool already has its witnesses.
- **Pool rules the base did not have.** Payer reservations, the per-payer exposure bound, the
  eviction order and replacement increments are as they were on the base. This branch changes
  only what claims and witnesses require.
- **Parallel claim verification.** A block's dependencies are verified one after another.

## Testing

```sh
cargo test --release -p ethrex-test -- eip8141_claims pooled_frame_tx frame_tx_validation
cargo test --release -p ethrex-common --lib -- claim dependencies pooled gas golden
cargo test --release -p ethrex-rpc --lib -- claim_witnesses pooled
cargo test --release --manifest-path tooling/Cargo.toml -p leansphincs_sender
```

The end-to-end tests sign with real secp256k1, P-256 and leanSPHINCS keys. They take a
transaction from submission to a built block and its import on both paths (`add_block` and
`add_block_pipeline`), and reject the block when a witness is missing, extra, tampered, of
another key, of the wrong length or out of order, when the header commits to other bytes, when
`claim_witnesses` is malformed or over the size limit, when a claim is malformed and when the
claims take more gas than the block's limit. The golden vector of the wire format
(`golden_frame_tx_rlp_and_hashes`) was computed with an RLP encoder and a Keccak-256 written
independently of ethrex.

## Files

| Component | File | Key items |
|---|---|---|
| Claims, dependencies, pooled form | `crates/common/types/transaction.rs` | `Claim`, `ProtocolVerifier`, `Dependency`, `FrameTransaction::{dependencies, claim_dependencies, compute_sig_hash}`, `block_dependencies`, `PooledFrameTransaction`, `encode_claim_witnesses`, `decode_claim_witnesses` |
| Header and body | `crates/common/types/block.rs` | `claim_witnesses_hash`, `claim_witnesses`, `compute_claim_witnesses_hash` |
| Native verifiers | `crates/vm/levm/src/claims.rs` | `secp256k1_verifier`, `p256_verifier`, `leansphincs_verifier`, `verify_dependency` |
| Pure context | `crates/vm/levm/src/vm.rs`, `crates/vm/levm/src/opcodes.rs` | `VM::run_pure_verifier`, `build_opcode_table_pure` |
| `HASCLAIM`, default code | `crates/vm/levm/src/opcode_handlers/frame_tx.rs` | `OpHasClaimHandler`, `has_claim`, `OpPureStaticCallHandler`, `execute_default_code` |
| Block check | `crates/vm/claims.rs`, `crates/blockchain/claim_witnesses.rs` | `verify_claim_witnesses`, `validate_claim_witnesses` |
| Admission | `crates/blockchain/blockchain.rs` | `add_frame_transaction_to_pool`, `validate_transaction_inner` |
| Pool | `crates/blockchain/mempool.rs` | `claim_witnesses_pool`, `get_claim_witnesses`, `get_pooled_frame_transaction` |
| Builder | `crates/blockchain/payload.rs` | `new_claim_witnesses`, `finalize_payload` |
| Engine API | `crates/networking/rpc/types/payload.rs` | `ExecutionPayload::claim_witnesses`, `normalize_claim_witnesses` |
| Submission and block JSON | `crates/networking/rpc/types/transaction.rs`, `crates/networking/rpc/types/block.rs` | `SendRawTransactionRequest::Frame`, `FullBlockBody::claim_witnesses` |
| Network | `crates/networking/p2p/rlpx/eth/transactions.rs`, `crates/networking/p2p/rlpx/eth/eth72/transactions.rs` | announced size, `PooledTransactions::handle` |
| Stateless execution | `crates/guest-program/src/common/execution.rs` | `verify_claim_witnesses` |
| Metrics | `crates/blockchain/metrics/claims.rs` | `METRICS_CLAIMS` |
| Sender tool | `tooling/leansphincs_sender/` | `alloc`, `send` |
| Tests | `test/tests/levm/eip8141_claims_tests.rs`, `test/tests/blockchain/eip8141_claims_tests.rs`, `test/tests/p2p/rlpx/pooled_frame_tx_tests.rs`, `test/tests/common/frame_tx_validation_tests.rs` | |

## References

- The declarative-claims proposal for EIP-8141, "Declarative claims, segregated witnesses, and
  HASCLAIM"
- [EIP-8141](./eip-8141.md), frame transactions as this branch implements the rest of them
- [EIP-8504](./eip-8504.md), the design this branch started from
- [EIP-8288](https://eips.ethereum.org/EIPS/eip-8288), the aggregation the block's
  `claim_witnesses` version byte leaves room for
- leanEthereum/leanVM at `92a97c919b2f2d6fe09853090b6b23f781ecec07`, the `lean_sphincs` crate used
  to verify and, in tests and tooling, to sign
- [EIP-7594](https://eips.ethereum.org/EIPS/eip-7594), whose cell-proof sidecar a frame
  transaction with blobs carries
