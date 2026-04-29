# Snap sync internals

## snap/2 wire protocol (EIP-8189)

### Capability negotiation

Both `snap/1` and `snap/2` are listed in `SUPPORTED_SNAP_CAPABILITIES` (highest version first). The negotiated version is stored as `SnapCapVersion` (an enum with variants `V1` and `V2`) in `RLPxCodec` and propagated to every `Message::decode` / `Message::encode` call.

File: `crates/networking/p2p/rlpx/message.rs` — `SnapCapVersion`, `Message::decode` snap branch (lines ~286–330).

### Message codes

Defined in `crates/networking/p2p/rlpx/snap/codec.rs` (`codes` submodule):

| Constant | Code | Valid in |
|----------|------|---------|
| `GET_TRIE_NODES` | 0x06 | snap/1 only |
| `TRIE_NODES` | 0x07 | snap/1 only |
| `GET_BLOCK_ACCESS_LISTS` | 0x08 | snap/2 only |
| `BLOCK_ACCESS_LISTS` | 0x09 | snap/2 only |

`Message::decode` enforces the version gate: codes 0x08/0x09 under snap/1 return `RLPDecodeError::MalformedData`; codes 0x06/0x07 under snap/2 return the same error.

A compile-time `const_assert` at the top of `message.rs` verifies that `SNAP_CAPABILITY_OFFSET + 0x09 < BASED_CAPABILITY_OFFSET` for each supported eth version, preventing message-id collisions.

### Version negotiation flow

1. `exchange_hello_messages` (in `rlpx/connection/server.rs`) records the highest mutually supported snap capability in `Established::negotiated_snap_capability`.
2. `initialize_connection` maps that to `SnapCapVersion::V1` or `SnapCapVersion::V2` and writes it into the `Arc<RwLock<SnapCapVersion>>` shared with `RLPxCodec`.
3. From that point on, all frames on the connection are decoded/encoded with the negotiated version.

### Server handler

`process_block_access_lists_request` — `crates/networking/p2p/snap/server.rs`.

- Iterates requested hashes in order; calls `store.get_block_access_list(hash)` for each.
- Accumulates encoded byte count; stops adding BALs when `min(request.response_bytes, 2 MiB)` is crossed.
- Position correspondence: emits `None` for every remaining slot once the cap is reached.
- First BAL is always included even if it alone exceeds the cap (soft cap).
- Wired into the connection dispatcher as a handler for `Message::GetBlockAccessLists`.

### Client method

`request_block_access_lists` — `crates/networking/p2p/snap/client.rs`.

- Calls `get_best_peer(vec![Capability::snap(2)])` to select a peer.
- Returns `SnapError::PeerSelection` if no snap/2 peer is available (triggers caller fallback to snap/1).
- Validates response ID; calls `record_failure` on mismatch.
- Returns `(Vec<Option<BlockAccessList>>, peer_id)` so the caller can attribute failures to the right peer.

### BAL replay engine

`crates/networking/p2p/sync/bal_healing/`

- `apply_bal` — `apply.rs`: applies one `BlockAccessList` diff to the state trie; returns the new root.
- `advance_state_via_bals` — `mod.rs`: batch-fetches BALs and applies them in order with per-block state-root verification, retry/fallback logic, and BAL persistence.

Key constants (`crates/networking/p2p/snap/constants.rs`):
- `BAL_RESPONSE_SOFT_CAP_BYTES = 2 MiB`
- `BAL_MAX_RETRIES_PER_BLOCK = 3`
- `BAL_REQUEST_BATCH_SIZE = 64`

### Sync state machine integration

`crates/networking/p2p/sync/snap_sync.rs` — inner staleness loop (~line 395):

```
if snap/2 peer available:
    advance_state_via_bals(...)
else:
    heal_state_trie_wrap(...)   # snap/1 fallback
```

The outer staleness loop (~line 289, pivot selection) is unchanged for both versions.
