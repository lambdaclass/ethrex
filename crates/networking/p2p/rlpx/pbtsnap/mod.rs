//! `pbtsnap/1` — state sync for the EIP-8297 binary tree.
//!
//! An ethrex-experimental RLPx capability. There is no upstream specification
//! for post-binary-tree state sync; the normative draft this implements lives
//! at `docs/eip-draft-pbtsnap.md`.
//!
//! ## Why not `snap/1`
//!
//! Three reasons, each sufficient on its own: the binary tree has no
//! per-account storage tries for `GetStorageRanges` to address, it commits with
//! BLAKE3 over node encodings a keccak/RLP proof verifier cannot read, and a
//! post-activation header's `state_root` names no MPT — so a `snap/1` server
//! answering such a request would prove against a root the header does not
//! commit to. Serving `snap/1` state ranges on a binary-tree-scheduled chain is
//! refused elsewhere in the client, and that refusal is load-bearing.
//!
//! ## Module structure
//!
//! - `messages`: the message structs
//! - `codec`: `RLPxMessage` impls and the RLP form of `PbtLeaf`
//!
//! Deliberately parallel to `rlpx/snap`, so the two read the same way.

mod codec;
mod messages;

pub use codec::codes;
pub use messages::{GetPbtLeafRange, PbtLeaf, PbtLeafRange};
