//! EIP-8312 UTXO frames: constants, the spend payload, and the spend hash.
//!
//! A UTXO is a one-shot payment object: created by a deposit to the vault system
//! contract, spent whole by a signed, value-conserving frame (EIP-8141 frame
//! mode `UTXO`). A UTXO's opening is kept in history and proven against
//! per-block openings roots; the only permanent state per UTXO is one spent bit.
//!
//! Spec: `EIPS/eip-8312.md` (Draft) at commit
//! `a5da3f608c6dfbf353bea264054d99fc164ab10c`. Divergences from that text are
//! recorded in `docs/eip-8312.md`; the one visible here is the frame-mode number
//! (the spec's `UTXO_MODE = 3` is EIP-7906's POST_TX in this client, so UTXO
//! takes mode 5 — see [`crate::types::FrameMode`]).
//!
//! This module holds only what is decidable from transaction bytes: the wire
//! shape, the static bounds, and the signing hash. Proof verification, spent
//! bits, conservation, and settlement live in the VM.

use bytes::Bytes;
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};
use serde::{Deserialize, Serialize};

use crate::{Address, H256, U256, utils::keccak};

/// Vault system contract address (`address(0x8312)`). Holds every unspent
/// UTXO's value; its code handles deposits only, and every other write to its
/// storage or balance is performed by the protocol directly.
pub const UTXO_VAULT_U64: u64 = 0x8312;

/// Returns the `UTXO_VAULT` address (0x…8312).
pub fn utxo_vault() -> Address {
    Address::from_low_u64_be(UTXO_VAULT_U64)
}

/// Spend-hash domain prefix. `0x81` lies outside the EIP-2718 transaction type
/// space (`<= 0x7f`), so a spend-hash preimage cannot collide with the signing
/// payload of any transaction type.
pub const SPEND_MAGIC: u8 = 0x81;

/// Openings-root ring length: the vault keeps one openings root per block for
/// the last `RING_SIZE` blocks, at `SLOT_RING_BASE + (block_number % RING_SIZE)`.
pub const RING_SIZE: u64 = 8192;

/// Batch commitment interval. `RING_SIZE == BATCH_SIZE` is what guarantees a
/// batch is sealed before any of its ring slots is overwritten, so a ring proof
/// can always be upgraded to a batch proof with no gap. The equality is relied
/// on by the window checks; keep them equal.
pub const BATCH_SIZE: u64 = 8192;

const _: () = assert!(RING_SIZE == BATCH_SIZE);

/// Depth of a batch tree: a batch always has exactly `BATCH_SIZE` leaves (the
/// openings roots of its blocks), so a batch path is exactly `log2(BATCH_SIZE)`
/// siblings. A witness's `batch_siblings` is either empty (ring proof) or
/// exactly this long (batch proof).
pub const BATCH_PATH_LEN: usize = BATCH_SIZE.trailing_zeros() as usize;

/// Maximum openings-tree depth a witness may claim. A block's openings tree
/// cannot be deeper than this in any realistic block, and the bound keeps proof
/// verification cost statically bounded.
pub const MAX_SIBLINGS: usize = 32;

/// `keccak256("UtxoCreated(address,address,uint64,uint256)")` — topic 0 of the
/// vault's creation log. Wallets MUST match this in addition to the vault
/// address; matching a recipient topic alone would let a future log shape under
/// the vault spoof a payment.
pub const UTXO_CREATED_TOPIC: H256 = H256([
    0x3b, 0x19, 0x24, 0x14, 0x65, 0xa4, 0x7b, 0xc1, 0x87, 0xf1, 0xd9, 0xc7, 0xdb, 0x70, 0x83, 0x48,
    0x55, 0xa9, 0x07, 0x18, 0x37, 0x42, 0xa4, 0xb6, 0x3a, 0xa8, 0x24, 0xc5, 0x76, 0x29, 0x6f, 0x5e,
]);

/// Regular-gas components of `utxo_frame_gas`, per the EIP's schedule. The
/// state-gas components depend on the live EIP-8037 per-byte cost, so the VM adds
/// them; admission uses [`Spend::admission_gas`], which sums both with the
/// canonical state values (policy may be conservative, consensus may not).
pub const GAS_UTXO_FRAME: u64 = 13_000;
pub const GAS_UTXO_INPUT: u64 = 16_048;
pub const GAS_UTXO_SIBLING: u64 = 42;
pub const GAS_UTXO_OUT: u64 = 2_012;
pub const GAS_UTXO_ACCOUNT_OUT: u64 = 9_000;
/// Canonical EIP-8037 values at the pinned per-state-byte cost. levm asserts at
/// compile time that its derived values agree with these.
pub const GAS_UTXO_SPENT_STATE: u64 = 383;
pub const GAS_NEW_ACCOUNT_STATE: u64 = 183_600;

impl Spend {
    /// `utxo_frame_gas` for admission purposes: every component of the EIP's
    /// schedule, both gas dimensions summed. Computable from the frame alone —
    /// no state reads and no signature checks — which is what lets a node reject
    /// an over-budget transaction before doing any expensive work.
    pub fn admission_gas(&self) -> u64 {
        let mut siblings: u64 = 0;
        for input in &self.inputs {
            siblings = siblings
                .saturating_add(u64::try_from(input.siblings.len()).unwrap_or(u64::MAX))
                .saturating_add(u64::try_from(input.batch_siblings.len()).unwrap_or(u64::MAX));
        }
        let inputs = u64::try_from(self.inputs.len()).unwrap_or(u64::MAX);
        let utxo_outs = u64::try_from(self.utxo_outs.len()).unwrap_or(u64::MAX);
        let account_outs = u64::try_from(self.account_outs.len()).unwrap_or(u64::MAX);

        GAS_UTXO_FRAME
            .saturating_add(GAS_UTXO_INPUT.saturating_mul(inputs))
            .saturating_add(GAS_UTXO_SIBLING.saturating_mul(siblings))
            .saturating_add(GAS_UTXO_SPENT_STATE.saturating_mul(inputs))
            .saturating_add(GAS_UTXO_OUT.saturating_mul(utxo_outs))
            .saturating_add(
                GAS_UTXO_ACCOUNT_OUT
                    .saturating_add(GAS_NEW_ACCOUNT_STATE)
                    .saturating_mul(account_outs),
            )
    }
}

/// EIP-8312 mempool admission budget for transactions carrying UTXO frames,
/// replacing the general frame-transaction verify bound for them. Policy, not
/// consensus: operator-tunable, and admission acceptance confers no consensus
/// meaning.
///
/// Counts the actor-signature validation cost plus the combined `utxo_frame_gas`
/// of the transaction's UTXO frames, with both EIP-8037 gas dimensions summed
/// (conservative, which is acceptable for a policy bound). A sponsored
/// transaction's validation prefix stays under the ordinary frame-transaction
/// bound instead — the two lanes are disjoint.
///
/// Note the ceiling this implies, faithful to the EIP but worth stating: each
/// account output carries a full `GAS_NEW_ACCOUNT_STATE` reserve, so a spend with
/// two or more fresh-account outputs exceeds the default and is unrelayable
/// despite being consensus-valid. Raised upstream as author feedback.
pub const MAX_UTXO_VERIFY_GAS: u64 = 400_000;

/// One output of a spend: `[recipient, value]`. The entry designated by
/// `change_index` is signed with `value == 0` and receives the remainder at
/// settlement.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendOutput {
    pub recipient: Address,
    pub value: U256,
}

/// One input of a spend:
/// `[index, creation_block, source, recipient, value, position, siblings, batch_siblings]`.
///
/// `index` and `creation_block` are **signed**; the remaining fields are the
/// **witness**, proven rather than trusted. Because the witness is outside the
/// spend hash, anyone may refresh it (for example upgrade a ring proof to a
/// batch proof) without invalidating a signature — a substituted witness either
/// proves the same opening or fails, since the signed `index` pins it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendInput {
    pub index: u64,
    pub creation_block: u64,
    pub source: Address,
    pub recipient: Address,
    pub value: U256,
    pub position: u64,
    pub siblings: Vec<H256>,
    pub batch_siblings: Vec<H256>,
}

/// The RLP payload of a UTXO frame's `data`:
/// `[actors, inputs, utxo_outs, account_outs, change_index, payer,
///   max_fee_per_gas, max_priority_fee_per_gas, max_gas_limit]`.
///
/// `payer` is empty for a self-funded spend (the vault fronts the maximum cost
/// and becomes the transaction's payer) or a 20-byte sponsor address.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spend {
    pub actors: Vec<Address>,
    pub inputs: Vec<SpendInput>,
    pub utxo_outs: Vec<SpendOutput>,
    pub account_outs: Vec<SpendOutput>,
    pub change_index: u64,
    /// Empty = self-funded; 20 bytes = sponsor address. Kept as raw bytes
    /// because the empty and 20-byte forms are distinct: a 20-byte zero address
    /// is *not* self-funded. (The spec's pseudocode conflates them; flagged to
    /// the authors as a consensus-split ambiguity.)
    pub payer: Bytes,
    pub max_fee_per_gas: U256,
    pub max_priority_fee_per_gas: U256,
    pub max_gas_limit: u64,
}

impl RLPEncode for SpendOutput {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.recipient)
            .encode_field(&self.value)
            .finish();
    }
}

impl RLPDecode for SpendOutput {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (recipient, decoder) = decoder.decode_field("recipient")?;
        let (value, decoder) = decoder.decode_field("value")?;
        let rest = decoder.finish()?;
        Ok((SpendOutput { recipient, value }, rest))
    }
}

impl RLPEncode for SpendInput {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.index)
            .encode_field(&self.creation_block)
            .encode_field(&self.source)
            .encode_field(&self.recipient)
            .encode_field(&self.value)
            .encode_field(&self.position)
            .encode_field(&self.siblings)
            .encode_field(&self.batch_siblings)
            .finish();
    }
}

impl RLPDecode for SpendInput {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (index, decoder) = decoder.decode_field("index")?;
        let (creation_block, decoder) = decoder.decode_field("creation_block")?;
        let (source, decoder) = decoder.decode_field("source")?;
        let (recipient, decoder) = decoder.decode_field("recipient")?;
        let (value, decoder) = decoder.decode_field("value")?;
        let (position, decoder) = decoder.decode_field("position")?;
        let (siblings, decoder) = decoder.decode_field("siblings")?;
        let (batch_siblings, decoder) = decoder.decode_field("batch_siblings")?;
        let rest = decoder.finish()?;
        Ok((
            SpendInput {
                index,
                creation_block,
                source,
                recipient,
                value,
                position,
                siblings,
                batch_siblings,
            },
            rest,
        ))
    }
}

impl RLPEncode for Spend {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.actors)
            .encode_field(&self.inputs)
            .encode_field(&self.utxo_outs)
            .encode_field(&self.account_outs)
            .encode_field(&self.change_index)
            .encode_field(&self.payer)
            .encode_field(&self.max_fee_per_gas)
            .encode_field(&self.max_priority_fee_per_gas)
            .encode_field(&self.max_gas_limit)
            .finish();
    }
}

impl RLPDecode for Spend {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (actors, decoder) = decoder.decode_field("actors")?;
        let (inputs, decoder) = decoder.decode_field("inputs")?;
        let (utxo_outs, decoder) = decoder.decode_field("utxo_outs")?;
        let (account_outs, decoder) = decoder.decode_field("account_outs")?;
        let (change_index, decoder) = decoder.decode_field("change_index")?;
        let (payer, decoder) = decoder.decode_field("payer")?;
        let (max_fee_per_gas, decoder) = decoder.decode_field("max_fee_per_gas")?;
        let (max_priority_fee_per_gas, decoder) =
            decoder.decode_field("max_priority_fee_per_gas")?;
        let (max_gas_limit, decoder) = decoder.decode_field("max_gas_limit")?;
        let rest = decoder.finish()?;
        Ok((
            Spend {
                actors,
                inputs,
                utxo_outs,
                account_outs,
                change_index,
                payer,
                max_fee_per_gas,
                max_priority_fee_per_gas,
                max_gas_limit,
            },
            rest,
        ))
    }
}

/// Static validity of a decoded spend: every rule checkable without state.
///
/// Stringly-typed to match the surrounding `validate_static_constraints`
/// convention (the caller prefixes the frame index).
impl Spend {
    /// Decode a UTXO frame's `data`. Rejects trailing bytes after the payload
    /// and inside every nested list (the `Decoder::finish` calls above), so a
    /// frame's data is exactly one spend and nothing more.
    pub fn decode_frame_data(data: &Bytes) -> Result<Self, String> {
        Self::decode(data).map_err(|e| format!("invalid spend payload: {e}"))
    }

    /// The outputs in canonical order: `utxo_outs` followed by `account_outs`.
    /// `change_index` indexes into this concatenation.
    pub fn outputs(&self) -> impl Iterator<Item = &SpendOutput> {
        self.utxo_outs.iter().chain(self.account_outs.iter())
    }

    /// Total number of outputs.
    pub fn output_count(&self) -> usize {
        self.utxo_outs.len() + self.account_outs.len()
    }

    /// Whether this is a self-funded spend (empty `payer`), in which case the
    /// vault fronts the transaction's maximum cost and becomes its payer.
    ///
    /// Tested on the *length*, never on a numeric zero: a 20-byte zero address
    /// is a (nonsensical, and rejected) sponsor, not a self-funded marker.
    pub fn is_self_funded(&self) -> bool {
        self.payer.is_empty()
    }

    /// The sponsor named by this spend, if any.
    pub fn sponsor(&self) -> Option<Address> {
        (self.payer.len() == 20).then(|| Address::from_slice(&self.payer))
    }

    /// Static bounds and shape rules, per EIP-8312 §Constraints. Rules that need
    /// state (proof verification, spent bits, conservation) or the transaction
    /// envelope (fee-cap comparison, sender checks) are enforced elsewhere.
    pub fn validate_static(&self) -> Result<(), String> {
        // Actors: at least one, pairwise distinct.
        if self.actors.is_empty() {
            return Err("spend has no actors".to_string());
        }
        for (i, actor) in self.actors.iter().enumerate() {
            if self.actors[..i].contains(actor) {
                return Err(format!("spend actor {actor:#x} appears more than once"));
            }
        }

        // Inputs: at least one, strictly increasing indices (which statically
        // excludes spending one UTXO twice within a frame), bounded witnesses.
        if self.inputs.is_empty() {
            return Err("spend has no inputs".to_string());
        }
        for (i, input) in self.inputs.iter().enumerate() {
            if i > 0 && input.index <= self.inputs[i - 1].index {
                return Err(format!(
                    "spend input indices must be strictly increasing (input {i}: {} after {})",
                    input.index,
                    self.inputs[i - 1].index
                ));
            }
            if input.siblings.len() > MAX_SIBLINGS {
                return Err(format!(
                    "spend input {i}: {} siblings exceeds the {MAX_SIBLINGS} limit",
                    input.siblings.len()
                ));
            }
            // `position` selects a leaf in a tree of depth `len(siblings)`.
            if input.siblings.len() < 64 && input.position >= (1u64 << input.siblings.len()) {
                return Err(format!(
                    "spend input {i}: position {} out of range for a depth-{} path",
                    input.position,
                    input.siblings.len()
                ));
            }
            // A batch path is either absent (ring proof) or exactly the batch
            // tree's depth.
            if !input.batch_siblings.is_empty() && input.batch_siblings.len() != BATCH_PATH_LEN {
                return Err(format!(
                    "spend input {i}: batch path must be empty or {BATCH_PATH_LEN} siblings, got {}",
                    input.batch_siblings.len()
                ));
            }
        }

        // Outputs: change index in range; the change entry is signed with value
        // zero and every other output with a non-zero value; no zero recipients.
        let output_count = self.output_count();
        if self.change_index >= output_count as u64 {
            return Err(format!(
                "spend change_index {} out of range for {output_count} outputs",
                self.change_index
            ));
        }
        for (j, out) in self.outputs().enumerate() {
            if out.recipient == Address::zero() {
                return Err(format!(
                    "spend output {j} has the zero address as recipient"
                ));
            }
            let is_change = j as u64 == self.change_index;
            if is_change && !out.value.is_zero() {
                return Err(format!(
                    "spend change output {j} must be signed with value zero, got {}",
                    out.value
                ));
            }
            if !is_change && out.value.is_zero() {
                return Err(format!("spend output {j} must have a non-zero value"));
            }
        }

        // Payer: empty (self-funded) or a 20-byte address that is neither the
        // vault nor the zero address. The vault is excluded because it is the
        // payer the protocol assigns for self-funded spends; the zero address is
        // excluded so that the two encodings of "no sponsor" cannot be confused.
        match self.payer.len() {
            0 => {}
            20 => {
                let sponsor = Address::from_slice(&self.payer);
                if sponsor == utxo_vault() {
                    return Err("spend payer must not be the vault".to_string());
                }
                if sponsor == Address::zero() {
                    return Err(
                        "spend payer must not be the zero address (use an empty payer for a self-funded spend)"
                            .to_string(),
                    );
                }
            }
            other => {
                return Err(format!("spend payer must be 0 or 20 bytes, got {other}"));
            }
        }

        Ok(())
    }

    /// The spend hash actors sign: `keccak256(SPEND_MAGIC || rlp([...]))`, where
    /// per-input witness fields are replaced by the signed pair
    /// `[index, creation_block]` so that refreshing a witness does not
    /// invalidate a signature.
    pub fn spend_hash(&self, chain_id: u64) -> H256 {
        let signed_inputs: Vec<SignedInput> = self
            .inputs
            .iter()
            .map(|input| SignedInput {
                index: input.index,
                creation_block: input.creation_block,
            })
            .collect();

        let mut payload = Vec::new();
        Encoder::new(&mut payload)
            .encode_field(&chain_id)
            .encode_field(&self.actors)
            .encode_field(&signed_inputs)
            .encode_field(&self.utxo_outs)
            .encode_field(&self.account_outs)
            .encode_field(&self.change_index)
            .encode_field(&self.payer)
            .encode_field(&self.max_fee_per_gas)
            .encode_field(&self.max_priority_fee_per_gas)
            .encode_field(&self.max_gas_limit)
            .finish();

        let mut preimage = Vec::with_capacity(1 + payload.len());
        preimage.push(SPEND_MAGIC);
        preimage.extend_from_slice(&payload);
        keccak(&preimage)
    }
}

/// The signed projection of an input: `[index, creation_block]`. Only these two
/// fields enter the spend hash.
struct SignedInput {
    index: u64,
    creation_block: u64,
}

impl RLPEncode for SignedInput {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.index)
            .encode_field(&self.creation_block)
            .finish();
    }
}

// ---------------------------------------------------------------------------
// Vault storage layout
//
// Slot regions are disjoint by construction, given `index < 2**64` and
// `block_number < 2**64`:
//   next-index  : 0
//   ring        : 1 .. 1 + RING_SIZE            (8193 at most)
//   batch roots : 2**128 .. 2**128 + 2**51      (block/8192 < 2**64/2**13)
//   spent bits  : 2**129 .. 2**129 + 2**56      (index>>8 < 2**64/2**8)
// ---------------------------------------------------------------------------

/// Counter of assigned UTXO indices.
pub const SLOT_NEXT_INDEX: u64 = 0;
/// Base of the per-block openings-root ring.
pub const SLOT_RING_BASE: u64 = 1;

/// Base of the batch-root region (`2**128`).
pub fn slot_batch_base() -> U256 {
    U256::one() << 128
}

/// Base of the spent-bit bitfield region (`2**129`).
pub fn slot_spent_base() -> U256 {
    U256::one() << 129
}

/// Vault slot holding block `block_number`'s openings root.
pub fn ring_slot(block_number: u64) -> U256 {
    U256::from(SLOT_RING_BASE) + U256::from(block_number % RING_SIZE)
}

/// Vault slot holding the batch root of the batch containing `block_number`.
pub fn batch_slot_for_block(block_number: u64) -> U256 {
    slot_batch_base() + U256::from(block_number / BATCH_SIZE)
}

/// Vault slot holding the batch root of batch `batch_index`.
pub fn batch_slot(batch_index: u64) -> U256 {
    slot_batch_base() + U256::from(batch_index)
}

/// The `(slot, bit_mask)` pair addressing a UTXO's spent bit: bit
/// `index & 0xFF` of the word at `SLOT_SPENT_BASE + (index >> 8)`. A slot packs
/// 256 flags, which is why a spend is charged 1/256 of a new slot's state gas.
pub fn spent_bit_location(index: u64) -> (U256, U256) {
    let slot = slot_spent_base() + U256::from(index >> 8);
    let mask = U256::one() << (index & 0xFF) as usize;
    (slot, mask)
}

/// Whether `word` (the value of a spent-bit slot) marks `index` as spent.
pub fn is_spent(word: U256, index: u64) -> bool {
    let (_, mask) = spent_bit_location(index);
    !(word & mask).is_zero()
}

/// Whether a block is the last of its batch, i.e. the block at whose end the
/// batch root is sealed.
pub fn seals_batch(block_number: u64) -> bool {
    block_number % BATCH_SIZE == BATCH_SIZE - 1
}

// ---------------------------------------------------------------------------
// Openings tree
//
// ONE definition, shared by root construction (block end), proof verification
// (frame execution), and mempool policy. EIP-8272's forged-roots hole came from
// commitment logic living in more than one place; do not duplicate these.
// ---------------------------------------------------------------------------

/// A UTXO's openings-tree leaf:
/// `keccak256(index_be8 ++ source ++ recipient ++ value_be32)` — 80 bytes of
/// preimage. Leaves are keccak of 80 bytes while interior nodes are keccak of
/// 64, so a leaf can never be reinterpreted as an interior node (and vice
/// versa) without a preimage: that domain separation is what makes the
/// all-zeros empty-tree sentinel unforgeable, since no leaf can hash to zero.
pub fn opening_leaf(index: u64, source: Address, recipient: Address, value: U256) -> H256 {
    let mut preimage = [0u8; 80];
    preimage[..8].copy_from_slice(&index.to_be_bytes());
    preimage[8..28].copy_from_slice(source.as_bytes());
    preimage[28..48].copy_from_slice(recipient.as_bytes());
    preimage[48..80].copy_from_slice(&value.to_big_endian());
    keccak(preimage)
}

/// Merkle root of a block's openings, per EIP-8312.
///
/// Empty input is the all-zeros sentinel. Otherwise the leaf list is padded with
/// all-zeros leaves until its length is a power of two, then folded bottom-up
/// with `parent = keccak256(left ++ right)`.
///
/// The power-of-two padding is load-bearing and is NOT the same as padding each
/// odd level with one zero: for five leaves the former pairs `e` against
/// `keccak(0‖0)` at the second level while the latter pairs it against a raw
/// zero word, producing a different root. Pairing is positional — never sorted
/// or commutative — so that the position-bit [`fold`] verifier accepts a proof
/// for every leaf.
pub fn merkle_root(leaves: &[H256]) -> H256 {
    if leaves.is_empty() {
        return H256::zero();
    }
    let mut level: Vec<H256> = leaves.to_vec();
    while !level.len().is_power_of_two() {
        level.push(H256::zero());
    }
    while level.len() > 1 {
        level = level
            .chunks_exact(2)
            .map(|pair| hash_pair(pair[0], pair[1]))
            .collect();
    }
    level[0]
}

/// `keccak256(left ++ right)` — one interior node of the openings tree.
pub fn hash_pair(left: H256, right: H256) -> H256 {
    let mut preimage = [0u8; 64];
    preimage[..32].copy_from_slice(left.as_bytes());
    preimage[32..].copy_from_slice(right.as_bytes());
    keccak(preimage)
}

/// Recompute a root from `node` and its `siblings`, taking the side to hash on
/// from the low bit of `position` at each level (bit set = `node` is the right
/// child). Used for both the in-block openings path and the batch path.
pub fn fold(node: H256, position: u64, siblings: &[H256]) -> H256 {
    let mut node = node;
    let mut position = position;
    for sibling in siblings {
        node = if position & 1 == 1 {
            hash_pair(*sibling, node)
        } else {
            hash_pair(node, *sibling)
        };
        position >>= 1;
    }
    node
}

/// The sibling path proving `leaves[position]` under [`merkle_root`], or `None`
/// if `position` is out of range. Provided so that root construction and witness
/// construction cannot drift apart; nodes are ordered leaf-to-root, matching
/// [`fold`].
pub fn merkle_proof(leaves: &[H256], position: usize) -> Option<Vec<H256>> {
    if position >= leaves.len() {
        return None;
    }
    let mut level: Vec<H256> = leaves.to_vec();
    while !level.len().is_power_of_two() {
        level.push(H256::zero());
    }
    let mut idx = position;
    let mut proof = Vec::new();
    while level.len() > 1 {
        proof.push(level[idx ^ 1]);
        level = level
            .chunks_exact(2)
            .map(|pair| hash_pair(pair[0], pair[1]))
            .collect();
        idx /= 2;
    }
    Some(proof)
}
