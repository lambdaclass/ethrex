# Binary Trie RPC — `eth_getBinaryProof`

After a node has transitioned to binary mode, `eth_getProof` returns an error and callers must use the new `eth_getBinaryProof` method.

## `eth_getProof` behavior post-switch

Any call to `eth_getProof` on a node with `BackendKind ∈ {Binary, Transition}` returns:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32099,
    "message": "state moved to binary trie at block 23456789, use eth_getBinaryProof",
    "data": {
      "switch_block": 23456789,
      "frozen_mpt_root": "0x..."
    }
  }
}
```

Return code `-32099` is in the JSON-RPC "server error" range (`-32099` to `-32000`) reserved for application-defined errors. Clients can key on this exact code and fall back to `eth_getBinaryProof`.

## `eth_getBinaryProof`

### Request

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "eth_getBinaryProof",
  "params": [
    "0x742d35cc6634c0532925a3b844bc9e7595f0beb7",
    ["0x0000000000000000000000000000000000000000000000000000000000000000"],
    "latest"
  ]
}
```

Parameters (same shape as `eth_getProof`):
1. `address` — 20-byte hex, 0x-prefixed.
2. `storage_keys` — array of 32-byte hex keys.
3. `block` — block number, tag (`"latest"`, `"finalized"`, `"safe"`), or block hash.

### Response

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "address": "0x742d35cc6634c0532925a3b844bc9e7595f0beb7",
    "binary": {
      "stem": "0x7a3c9f...<31 bytes>",
      "basic_data": "0x00000000000000000000000000000000000000000000000000000000000000",
      "code_hash": "0x...<32 bytes>",
      "stem_siblings": ["0x..."],
      "stem_depth": 248,
      "stem_node_hash": null,
      "storage": [
        {
          "key": "0x0000...",
          "sub_index": 64,
          "value": "0x0000...",
          "subtree_siblings": ["0x...", "0x...", "0x...", "0x...", "0x...", "0x...", "0x...", "0x..."]
        }
      ]
    },
    "fallback_mpt": null,
    "overlay_root": "0x...<32 bytes>",
    "frozen_mpt_root": "0x..."
  }
}
```

### Field semantics

- **`binary.stem`** — The 31-byte address stem. Absent iff the account has never been touched post-switch (then see `fallback_mpt`).
- **`binary.basic_data`** — The packed 32-byte `BASIC_DATA` leaf: `version(1) || reserved(4) || code_size(3) || nonce(8) || balance(16)`. Zero-padded if the account exists but has never had `BASIC_DATA` explicitly set (only reachable in corner cases; normally the overlay-stem-integrity invariant ensures it is always set post-switch).
- **`binary.code_hash`** — 32-byte keccak256 of the account's code (as on MPT; binary trie stores the same `code_hash`).
- **`binary.stem_siblings`** — Merkle sibling hashes from the root down to the `StemNode`, one per `InternalNode` level.
- **`binary.stem_depth`** — Depth at which the stem was located (or would have been located for absence proofs).
- **`binary.stem_node_hash`** — For "different-stem" absence proofs: the hash of the `StemNode` that was encountered instead. `null` for presence proofs and for "path terminated at empty child" proofs.
- **`binary.storage[]`** — One entry per requested storage key:
  - **`key`** — The original 32-byte storage key.
  - **`sub_index`** — The last byte of the computed tree key (range 0–255).
  - **`value`** — Storage value at that slot (`0x0000...` for absent slots).
  - **`subtree_siblings`** — 8 hashes proving `leaf → stem subtree root`. Always exactly 8 entries (depth of a 256-leaf binary tree is 8).
- **`fallback_mpt`** — Present only when the stem is absent from the binary overlay and the account is read from MPT. Same shape as the MPT `eth_getProof` response (`accountProof`, `storageProof[].proof`). Verifier should check this against `frozen_mpt_root`.
- **`overlay_root`** — The current binary trie root at the requested block.
- **`frozen_mpt_root`** — The MPT root at the switch block (constant for the DB's lifetime).

### Verification

Given `address`, `storage_keys`, the response, and a trusted `overlay_root` + `frozen_mpt_root`:

1. **If `fallback_mpt` is present**: verify `fallback_mpt` against `frozen_mpt_root` using the standard EIP-1186 MPT proof algorithm. Trust its `balance`, `nonce`, `code_hash`, `storage[].value` fields. Ignore the `binary` block.
2. **If `binary.stem` is present**: reconstruct the stem's merkle hash using the storage proofs (each storage entry gives a leaf-to-subtree-root sibling path; combine with `basic_data`, `code_hash` to form the full 256-leaf subtree), then walk up using `stem_siblings` for `stem_depth` levels. Final hash must equal `overlay_root`.

### Block-number semantics

- **`block >= transition_switch_block`**: Overlay may or may not contain the account. If absent from overlay, `fallback_mpt` is populated with the state as of `frozen_mpt_root` (which is the state at `transition_switch_block - 1`, frozen). This is the correct answer under overlay semantics: untouched accounts have not changed since the switch.
- **`block < transition_switch_block`**: Binary overlay has no data. Response has `binary` fields empty/null and `fallback_mpt` populated with a proof against the requested historical block's state root — but note that non-archive nodes may not retain the state for blocks older than ~128 back, in which case the request fails with `unknown block state`.

### Error cases

| Situation | Behavior |
|---|---|
| Block number > latest | Standard `block not found` error |
| Block number between `latest - 128` and `latest` but state pruned | `state unavailable for block` error |
| Called on a pure-MPT node (no transition yet) | `binary trie not active on this node` error, code `-32098` |
| Address is not 20 bytes | Standard invalid-params error |
| Storage key is not 32 bytes | Standard invalid-params error |

## Reference implementation

The proof-building logic reuses `BinaryTrieProof::get_proof` from `ethrex-binary-trie`. The RPC handler:

1. Resolves `block` to a state root.
2. For post-switch blocks, opens the `TransitionBackend` at that root, produces a `BinaryTrieProof` per storage key + one for the account stem.
3. If the stem is absent, opens the `MptBackend` at `frozen_mpt_root` and produces an EIP-1186 MPT proof.
4. Serializes to the response shape above.

See `plan.md` Phase 8 for implementation specifics and `crates/networking/rpc/eth/account.rs` for the handler skeleton.
