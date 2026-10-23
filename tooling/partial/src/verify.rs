//! The trust boundary. Peer bytes become domain values here and nowhere else.
//!
//! The only trusted input is a [`TrustedHash`] from the head source (the light client).
//! Every other argument is untrusted wire data. Each returned type has private fields and
//! no public constructor, so holding one is proof the check ran.
//!
//! Each query type owns both directions of one single-key snap exchange: it builds the
//! request and verifies the reply against the root it was built for.

use std::collections::BTreeMap;
use std::sync::Arc;

use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_common::types::{
    AccountState, BlockBody, BlockHeader, Code, Receipt, compute_receipts_root_and_logs_bloom,
    compute_transactions_root, compute_withdrawals_root,
};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::decode::RLPDecode;
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::{EMPTY_TRIE_HASH, Node, Trie, TrieError};

use crate::state::{Account, Key};
use crate::txbook::VerifiedReceipt;

/// An execution block hash the head source vouched for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrustedHash(H256);

impl TrustedHash {
    /// Marks `hash` as attested by the head source. Only the head-source adapter and the
    /// offline test fixture call this.
    pub const fn attested(hash: H256) -> Self {
        Self(hash)
    }

    /// The attested block hash.
    pub const fn hash(&self) -> H256 {
        self.0
    }
}

/// A header whose hash is a [`TrustedHash`] or links to one through `parent_hash`.
#[derive(Clone, Debug)]
pub struct VerifiedHeader {
    header: Arc<BlockHeader>,
    hash: H256,
}

impl PartialEq for VerifiedHeader {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
    }
}

impl Eq for VerifiedHeader {}

impl VerifiedHeader {
    /// The verified block hash.
    pub const fn hash(&self) -> H256 {
        self.hash
    }

    /// The block number.
    pub fn number(&self) -> u64 {
        self.header.number
    }

    /// The full header.
    pub fn header(&self) -> &BlockHeader {
        &self.header
    }

    /// The header behind a shared pointer, for callers that keep it.
    pub fn header_arc(&self) -> Arc<BlockHeader> {
        Arc::clone(&self.header)
    }

    /// Whether `self` is the direct parent of `child`.
    pub fn is_parent_of(&self, child: &Self) -> bool {
        child.header.parent_hash == self.hash && child.number() == self.number().wrapping_add(1)
    }
}

/// Checks that `header` hashes to the head source's `trusted` hash.
pub fn header(trusted: TrustedHash, header: BlockHeader) -> Result<VerifiedHeader, VerifyError> {
    let hash = header.compute_block_hash(&NativeCrypto);
    if hash != trusted.hash() {
        return Err(VerifyError::HeaderHash {
            expected: trusted.hash(),
            got: hash,
        });
    }
    Ok(VerifiedHeader {
        header: Arc::new(header),
        hash,
    })
}

/// Checks that `header` is the parent of the already verified `child`.
pub fn parent(child: &VerifiedHeader, header: BlockHeader) -> Result<VerifiedHeader, VerifyError> {
    let expected = child.header().parent_hash;
    let hash = header.compute_block_hash(&NativeCrypto);
    if hash != expected {
        return Err(VerifyError::HeaderHash {
            expected,
            got: hash,
        });
    }
    Ok(VerifiedHeader {
        header: Arc::new(header),
        hash,
    })
}

/// Verifies ancestors of `child` given newest first, as a falling `GetBlockHeaders`
/// returns them. Returns them ascending, oldest first.
pub fn ancestors(
    child: &VerifiedHeader,
    headers: Vec<BlockHeader>,
) -> Result<Vec<VerifiedHeader>, VerifyError> {
    let mut verified: Vec<VerifiedHeader> = Vec::with_capacity(headers.len());
    for header in headers {
        let next = parent(verified.last().unwrap_or(child), header)?;
        verified.push(next);
    }
    verified.reverse();
    Ok(verified)
}

/// Verifies the headers strictly between `anchor` and `target`, given newest first.
/// Returns the chain ascending from the anchor's child through `target`.
///
/// A break means a reorg or a lying peer; either way the caller reanchors.
pub fn chain(
    anchor: &VerifiedHeader,
    target: VerifiedHeader,
    between: Vec<BlockHeader>,
) -> Result<Vec<VerifiedHeader>, VerifyError> {
    let mut chain = ancestors(&target, between)?;
    chain.push(target);
    let Some(first) = chain.first() else {
        return Err(VerifyError::ChainBreak {
            anchor: anchor.hash(),
            number: anchor.number(),
        });
    };
    if !anchor.is_parent_of(first) {
        return Err(VerifyError::ChainBreak {
            anchor: anchor.hash(),
            number: first.number(),
        });
    }
    Ok(chain)
}

/// A verified header and its block access list.
#[derive(Clone, Debug)]
pub struct VerifiedBlock {
    header: VerifiedHeader,
    bal: BlockAccessList,
}

impl VerifiedBlock {
    /// The block's header.
    pub const fn header(&self) -> &VerifiedHeader {
        &self.header
    }

    /// The block access list, in canonical order.
    pub const fn bal(&self) -> &BlockAccessList {
        &self.bal
    }

    pub(crate) fn into_parts(self) -> (VerifiedHeader, BlockAccessList) {
        (self.header, self.bal)
    }
}

/// Checks one block access list against `header.block_access_list_hash`.
///
/// `bal` is one entry of a BAL reply. `None` means the peer does not have it, reported
/// as [`VerifyError::BalUnavailable`], which blames nobody.
pub fn bal(
    header: VerifiedHeader,
    bal: Option<BlockAccessList>,
) -> Result<VerifiedBlock, VerifyError> {
    let block = header.hash();
    let expected = header
        .header()
        .block_access_list_hash
        .ok_or(VerifyError::NoBalHash { block })?;
    let bal = bal.ok_or(VerifyError::BalUnavailable { block })?;
    // `compute_hash` sorts before encoding, so an unsorted list could still match the
    // commitment. Canonical order is part of what the header commits to.
    if let Err(reason) = bal.validate_ordering() {
        return Err(VerifyError::BalOrdering { block, reason });
    }
    let got = bal.compute_hash(&NativeCrypto);
    if got != expected {
        return Err(VerifyError::BalHash { expected, got });
    }
    Ok(VerifiedBlock { header, bal })
}

/// What a proof established, before it is bound to a block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Fact {
    /// The account at `address`, or proven absence.
    Account {
        address: Address,
        account: Option<Account>,
    },
    /// One slot of a present account, proven at the same root as the account.
    Slot {
        address: Address,
        account: Account,
        slot: U256,
        value: U256,
    },
    /// Code whose keccak matched the requested hash.
    Code { hash: H256, code: Code },
}

/// Number and hash of the block a fact was proven at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockNumHash {
    /// Block number.
    pub number: u64,
    /// Block hash.
    pub hash: H256,
}

impl From<&VerifiedHeader> for BlockNumHash {
    fn from(header: &VerifiedHeader) -> Self {
        Self {
            number: header.number(),
            hash: header.hash(),
        }
    }
}

/// A state fact proven against a verified header's state root.
///
/// Code is content-addressed and valid at every block, so a code fact has no block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proven {
    at: Option<BlockNumHash>,
    fact: Fact,
}

impl Proven {
    /// The block whose state root the fact was proven against. `None` for code.
    pub const fn at(&self) -> Option<BlockNumHash> {
        self.at
    }

    /// The store key the fact covers. `None` for code.
    pub const fn key(&self) -> Option<Key> {
        match &self.fact {
            Fact::Account { address, .. } => Some(Key::Account(*address)),
            Fact::Slot { address, slot, .. } => Some(Key::Slot(*address, *slot)),
            Fact::Code { .. } => None,
        }
    }

    /// The proven account: `Some(None)` for proven absence, `None` for code facts.
    pub const fn account(&self) -> Option<Option<&Account>> {
        match &self.fact {
            Fact::Account { account, .. } => Some(account.as_ref()),
            Fact::Slot { account, .. } => Some(Some(account)),
            Fact::Code { .. } => None,
        }
    }

    /// The proven slot value, for slot facts.
    pub const fn slot_value(&self) -> Option<U256> {
        match &self.fact {
            Fact::Slot { value, .. } => Some(*value),
            Fact::Account { .. } | Fact::Code { .. } => None,
        }
    }

    pub(crate) const fn fact(&self) -> &Fact {
        &self.fact
    }
}

/// The wire shape of a single-key `GetAccountRange`: origin and limit are both `key`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AccountRangeRequest {
    /// State root to query.
    pub root: H256,
    /// `keccak256(address)`.
    pub key: H256,
}

/// The wire shape of a single-key `GetStorageRanges` for one account.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StorageRangeRequest {
    /// State root to query (the request names the state root, not the storage root).
    pub root: H256,
    /// `keccak256(address)`.
    pub account: H256,
    /// `keccak256(slot)`.
    pub slot: H256,
}

/// An untrusted `AccountRange` reply: leaves as `(keccak(address), account)` plus proof nodes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccountReply {
    /// Account leaves in ascending hash order.
    pub accounts: Vec<(H256, AccountState)>,
    /// RLP-encoded proof nodes.
    pub proof: Vec<Vec<u8>>,
}

/// An untrusted `StorageRanges` reply for one account: leaves as `(keccak(slot), value)`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StorageReply {
    /// Slot leaves in ascending hash order.
    pub slots: Vec<(H256, U256)>,
    /// RLP-encoded proof nodes.
    pub proof: Vec<Vec<u8>>,
}

/// One single-key `GetAccountRange` bound to the root it is verified against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccountQuery {
    at: BlockNumHash,
    root: H256,
    address: Address,
    key: H256,
}

impl AccountQuery {
    /// A query for `address` at the state root of `at`.
    pub fn new(at: &VerifiedHeader, address: Address) -> Self {
        Self {
            at: at.into(),
            root: at.header().state_root,
            address,
            key: keccak(address.as_bytes()),
        }
    }

    /// The address queried.
    pub const fn address(&self) -> Address {
        self.address
    }

    /// The request to send: origin and limit are both `keccak256(address)`.
    pub const fn request(&self) -> AccountRangeRequest {
        AccountRangeRequest {
            root: self.root,
            key: self.key,
        }
    }

    /// Verifies `reply` and returns the account or its proven absence.
    pub fn verify(&self, reply: AccountReply) -> Result<ProvenAccount, VerifyError> {
        let leaf = single_key(self.root, self.key, reply.accounts, &reply.proof)?;
        Ok(ProvenAccount { query: *self, leaf })
    }
}

/// An account proven at a state root, or its proven absence, holding the storage root
/// its slots are proven under.
#[derive(Clone, Debug, PartialEq)]
pub struct ProvenAccount {
    query: AccountQuery,
    leaf: Option<AccountState>,
}

impl ProvenAccount {
    /// The account fact.
    pub fn proven(&self) -> Proven {
        Proven {
            at: Some(self.query.at),
            fact: Fact::Account {
                address: self.query.address,
                account: self.leaf.as_ref().map(Account::from),
            },
        }
    }

    /// `slot` of this account. An absent account, or one with an empty storage root,
    /// makes the slot a known zero; otherwise it takes a storage proof.
    pub fn slot(&self, slot: U256) -> SlotProof {
        let Some(leaf) = &self.leaf else {
            return SlotProof::Known(self.proven());
        };
        let query = StorageQuery {
            account: self.query,
            state: Account::from(leaf),
            storage_root: leaf.storage_root,
            slot,
            slot_key: keccak(&slot.to_big_endian()),
        };
        if leaf.storage_root == EMPTY_TRIE_HASH {
            SlotProof::Known(query.fact(U256::zero()))
        } else {
            SlotProof::Wanted(query)
        }
    }
}

/// What proving one slot of a [`ProvenAccount`] takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlotProof {
    /// The account alone settles it: absent, or with empty storage.
    Known(Proven),
    /// A storage proof against the account's storage root.
    Wanted(StorageQuery),
}

/// One single-key `GetStorageRanges` against a proven account's storage root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageQuery {
    account: AccountQuery,
    state: Account,
    storage_root: H256,
    slot: U256,
    slot_key: H256,
}

impl StorageQuery {
    /// The request to send: origin and limit are both `keccak256(slot)`.
    pub const fn request(&self) -> StorageRangeRequest {
        StorageRangeRequest {
            root: self.account.root,
            account: self.account.key,
            slot: self.slot_key,
        }
    }

    /// Verifies `storage` against the account's storage root.
    pub fn verify(&self, storage: StorageReply) -> Result<Proven, VerifyError> {
        if storage.slots.iter().any(|(_, value)| value.is_zero()) {
            return Err(VerifyError::ZeroSlot);
        }
        let value = single_key(
            self.storage_root,
            self.slot_key,
            storage.slots,
            &storage.proof,
        )?
        .unwrap_or_default();
        Ok(self.fact(value))
    }

    const fn fact(&self, value: U256) -> Proven {
        Proven {
            at: Some(self.account.at),
            fact: Fact::Slot {
                address: self.account.address,
                account: self.state,
                slot: self.slot,
                value,
            },
        }
    }
}

/// Code by hash. Code is content-addressed, so the reply is checked by keccak alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeQuery {
    hash: H256,
}

impl CodeQuery {
    /// A query for the code hashing to `hash`.
    pub const fn new(hash: H256) -> Self {
        Self { hash }
    }

    /// The code hash to request.
    pub const fn request(&self) -> H256 {
        self.hash
    }

    /// Checks `code` against the hash. `None` means the peer did not return it.
    pub fn verify(&self, code: Option<bytes::Bytes>) -> Result<Proven, VerifyError> {
        let code = code.ok_or(VerifyError::CodeUnavailable { hash: self.hash })?;
        let got = keccak(&code);
        if got != self.hash {
            return Err(VerifyError::CodeHash {
                expected: self.hash,
                got,
            });
        }
        Ok(Proven {
            at: None,
            fact: Fact::Code {
                hash: self.hash,
                code: Code::from_bytecode_unchecked(code, self.hash),
            },
        })
    }
}

/// Verifies a single-key reply against `root` by walking the proof nodes as an authenticated
/// trie: every node is addressed by its keccak, so a walk that reaches a leaf at `key`
/// proves presence, a walk that ends short of one proves absence, and a missing node
/// fails. A leaf the walk finds but the reply withholds is a lie.
///
/// An empty reply with no proof is a peer that lacks the root, never absence, unless the
/// root is the empty trie.
fn single_key<V: RLPEncode>(
    root: H256,
    key: H256,
    leaves: Vec<(H256, V)>,
    proof: &[Vec<u8>],
) -> Result<Option<V>, VerifyError> {
    if leaves.is_empty() && proof.is_empty() {
        return if root == EMPTY_TRIE_HASH {
            Ok(None)
        } else {
            Err(VerifyError::RootUnavailable { root })
        };
    }
    let mut nodes = BTreeMap::new();
    for encoded in proof {
        let node = Node::decode(encoded).map_err(|_| VerifyError::MalformedProof { root })?;
        nodes.insert(keccak(encoded), node);
    }
    let trie = Trie::from_nodes(root, &nodes)
        .map_err(|source| VerifyError::RangeProof { root, source })?;
    let found = trie
        .get(key.as_bytes())
        .map_err(|source| VerifyError::RangeProof { root, source })?;
    let Some(value_rlp) = found else {
        return Ok(None);
    };
    let supplied = leaves.into_iter().find(|(hash, _)| *hash == key);
    match supplied {
        Some((_, value)) if value.encode_to_vec() == value_rlp => Ok(Some(value)),
        Some(_) => Err(VerifyError::LeafMismatch { root }),
        None => Err(VerifyError::WithheldLeaf { root }),
    }
}

/// A block's transactions, checked against the header's transaction, ommers, and
/// withdrawals roots.
#[derive(Clone, Debug)]
pub struct VerifiedBody {
    header: VerifiedHeader,
    body: BlockBody,
}

impl VerifiedBody {
    /// The block's header.
    pub const fn header(&self) -> &VerifiedHeader {
        &self.header
    }

    /// The block's body.
    pub const fn body(&self) -> &BlockBody {
        &self.body
    }
}

/// Checks `body` against the roots in `header`.
pub fn body(header: VerifiedHeader, body: BlockBody) -> Result<VerifiedBody, VerifyError> {
    let block = header.hash();
    let h = header.header();
    if compute_transactions_root(&body.transactions, &NativeCrypto) != h.transactions_root {
        return Err(VerifyError::Root {
            block,
            part: BlockPart::Transactions,
        });
    }
    if !body.ommers.is_empty() {
        return Err(VerifyError::Root {
            block,
            part: BlockPart::Ommers,
        });
    }
    let withdrawals_match = match (h.withdrawals_root, &body.withdrawals) {
        (Some(root), Some(withdrawals)) => {
            compute_withdrawals_root(withdrawals, &NativeCrypto) == root
        }
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    };
    if !withdrawals_match {
        return Err(VerifyError::Root {
            block,
            part: BlockPart::Withdrawals,
        });
    }
    Ok(VerifiedBody { header, body })
}

/// A block's receipts, checked against the header's receipts root and logs bloom.
#[derive(Clone, Debug)]
pub struct VerifiedReceipts {
    header: VerifiedHeader,
    receipts: Vec<Receipt>,
}

impl VerifiedReceipts {
    /// The block's header.
    pub const fn header(&self) -> &VerifiedHeader {
        &self.header
    }

    /// The receipts in transaction order.
    pub fn receipts(&self) -> &[Receipt] {
        &self.receipts
    }

    /// The receipt view for the transaction at `index`.
    ///
    /// `None` if `body` is of a different block or `index` is out of range.
    pub fn receipt(&self, body: &VerifiedBody, index: usize) -> Option<VerifiedReceipt> {
        if body.header.hash() != self.header.hash() {
            return None;
        }
        let tx = body.body().transactions.get(index)?;
        let receipt = self.receipts.get(index)?;
        let previous = self.receipts.get(..index)?;
        let prior_gas = previous.last().map_or(0, |r| r.cumulative_gas_used);
        let first_log_index = previous
            .iter()
            .map(|r| u64::try_from(r.logs.len()).unwrap_or(u64::MAX))
            .fold(0u64, u64::saturating_add);
        Some(VerifiedReceipt::new(
            &self.header,
            u64::try_from(index).ok()?,
            tx,
            receipt,
            receipt.cumulative_gas_used.saturating_sub(prior_gas),
            first_log_index,
        ))
    }
}

/// Checks receipts against `header`. Blooms are recomputed because eth/69+ carries none.
pub fn receipts(
    header: VerifiedHeader,
    receipts: Vec<Receipt>,
) -> Result<VerifiedReceipts, VerifyError> {
    let block = header.hash();
    let (root, bloom) = compute_receipts_root_and_logs_bloom(&receipts, &NativeCrypto);
    if root != header.header().receipts_root {
        return Err(VerifyError::Root {
            block,
            part: BlockPart::Receipts,
        });
    }
    if bloom != header.header().logs_bloom {
        return Err(VerifyError::Root {
            block,
            part: BlockPart::LogsBloom,
        });
    }
    Ok(VerifiedReceipts { header, receipts })
}

/// The header commitment a body or receipt list failed to match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockPart {
    /// `transactions_root`.
    Transactions,
    /// `ommers_hash` (post-merge bodies carry none).
    Ommers,
    /// `withdrawals_root`, including presence.
    Withdrawals,
    /// `receipts_root`.
    Receipts,
    /// `logs_bloom`.
    LogsBloom,
}

/// A peer reply failed verification. The sender lied or the reply was garbled, except for
/// the `*Unavailable` variants, which mean the peer lacks the data and blame nobody.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    /// A header does not hash to the trusted or linked hash.
    #[error("header hash {got:#x} does not match expected {expected:#x}")]
    HeaderHash {
        /// Hash the header had to match.
        expected: H256,
        /// Hash the header actually has.
        got: H256,
    },
    /// The verified chain does not descend from the anchor.
    #[error("header {number} does not link to anchor {anchor:#x}")]
    ChainBreak {
        /// Hash of the anchor the chain had to extend.
        anchor: H256,
        /// Number of the oldest header in the chain.
        number: u64,
    },
    /// The peer answered with neither leaves nor proof: it does not serve this root.
    #[error("peer does not serve state root {root:#x}")]
    RootUnavailable {
        /// Root the request targeted.
        root: H256,
    },
    /// A proof node is not RLP a trie node decodes from.
    #[error("proof against {root:#x} carries a malformed node")]
    MalformedProof {
        /// Root the reply was checked against.
        root: H256,
    },
    /// The proof holds a leaf at the requested key that the reply left out.
    #[error("reply against {root:#x} withholds the leaf its own proof carries")]
    WithheldLeaf {
        /// Root the reply was checked against.
        root: H256,
    },
    /// The reply's leaf value differs from the one the proof authenticates.
    #[error("reply against {root:#x} carries a leaf value its proof does not")]
    LeafMismatch {
        /// Root the reply was checked against.
        root: H256,
    },
    /// A storage reply carried a zero value, which is never a trie leaf.
    #[error("storage reply carries a zero slot value")]
    ZeroSlot,
    /// The proof does not authenticate the key against the root.
    #[error("proof against {root:#x} is invalid: {source}")]
    RangeProof {
        /// Root the reply was checked against.
        root: H256,
        /// Why the proof failed.
        #[source]
        source: TrieError,
    },
    /// The peer did not return the requested code.
    #[error("peer does not have code {hash:#x}")]
    CodeUnavailable {
        /// Requested code hash.
        hash: H256,
    },
    /// Code does not hash to the requested hash.
    #[error("code hashes to {got:#x}, expected {expected:#x}")]
    CodeHash {
        /// Requested code hash.
        expected: H256,
        /// Hash of the returned code.
        got: H256,
    },
    /// The header predates block access lists.
    #[error("header {block:#x} has no block access list hash")]
    NoBalHash {
        /// Block hash.
        block: H256,
    },
    /// The peer does not have the block access list.
    #[error("peer has no block access list for {block:#x}")]
    BalUnavailable {
        /// Block hash.
        block: H256,
    },
    /// The block access list does not hash to the header's commitment.
    #[error("block access list hashes to {got:#x}, expected {expected:#x}")]
    BalHash {
        /// `block_access_list_hash` from the header.
        expected: H256,
        /// Hash of the returned list.
        got: H256,
    },
    /// The block access list is not in canonical order.
    #[error("block access list for {block:#x} is not canonically ordered: {reason}")]
    BalOrdering {
        /// Block hash.
        block: H256,
        /// What is out of order.
        reason: String,
    },
    /// A body or receipt list does not match a header commitment.
    #[error("{part:?} mismatch in block {block:#x}")]
    Root {
        /// Block hash.
        block: H256,
        /// The commitment that failed.
        part: BlockPart,
    },
}

impl VerifyError {
    /// Whether the reply proves the peer sent bad data. A peer lacking data, a header
    /// without a BAL hash, and a chain break (which a reorg also causes) blame nobody.
    pub const fn blames_peer(&self) -> bool {
        match self {
            Self::RootUnavailable { .. }
            | Self::CodeUnavailable { .. }
            | Self::BalUnavailable { .. }
            | Self::NoBalHash { .. }
            | Self::ChainBreak { .. } => false,
            Self::HeaderHash { .. }
            | Self::MalformedProof { .. }
            | Self::WithheldLeaf { .. }
            | Self::LeafMismatch { .. }
            | Self::ZeroSlot
            | Self::RangeProof { .. }
            | Self::CodeHash { .. }
            | Self::BalHash { .. }
            | Self::BalOrdering { .. }
            | Self::Root { .. } => true,
        }
    }
}

/// `keccak256` as an `H256`.
pub(crate) fn keccak(data: &[u8]) -> H256 {
    ethrex_common::utils::keccak(data)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::testing::{World, deploy_code, eth, writer_runtime};
    use ethrex_common::evm::calculate_create_address;

    const NOBODY: Address = Address::repeat_byte(0xde);

    /// Deploys a contract writing slots 1..=3 and returns its address and the head.
    async fn world_with_token() -> (World, Address) {
        let mut world = World::new().await;
        let deployer = world.signer(0).address;
        let initcode = deploy_code(&[(1, 11), (2, 22), (3, 33)], &writer_runtime());
        world.mine(vec![world.deploy(0, 0, initcode)]).await;
        assert!(world.rejected.is_empty(), "{:?}", world.rejected);
        let token = calculate_create_address(deployer, 0);
        assert!(world.account_at_head(token).is_some(), "token deployed");
        (world, token)
    }

    #[tokio::test]
    async fn present_account_matches_world() {
        let world = World::new().await;
        let head = world.verified(world.head());
        let alice = world.signer(0).address;
        let proven = world.prove_account(&head, alice).await.proven();
        let expected = world.account_at_head(alice).unwrap();
        let account = proven.account().unwrap().unwrap();
        assert_eq!(proven.at(), Some(BlockNumHash::from(&head)));
        assert_eq!(proven.key(), Some(Key::Account(alice)));
        assert_eq!(account.balance, expected.balance);
        assert_eq!(account.nonce, expected.nonce);
        assert_eq!(account.code_hash, expected.code_hash);
        assert!(!account.has_storage);
    }

    #[tokio::test]
    async fn absent_account_is_known_absent() {
        let world = World::new().await;
        let head = world.verified(world.head());
        let query = AccountQuery::new(&head, NOBODY);
        let reply = world.account_reply(&query.request()).await;
        assert!(
            reply
                .accounts
                .iter()
                .all(|(hash, _)| *hash > query.request().key),
            "the server returns only the boundary leaf"
        );
        assert_eq!(query.verify(reply).unwrap().proven().account(), Some(None));
    }

    #[tokio::test]
    async fn storage_present_absent_and_empty_root() {
        let (world, token) = world_with_token().await;
        let head = world.verified(world.head());
        let account = world.prove_account(&head, token).await;
        async fn slot(world: &World, account: &ProvenAccount, n: u64) -> Proven {
            match account.slot(U256::from(n)) {
                SlotProof::Wanted(query) => query
                    .verify(world.storage_reply(&query.request()).await)
                    .unwrap(),
                SlotProof::Known(proven) => panic!("slot {n} needs a proof: {proven:?}"),
            }
        }
        assert_eq!(
            slot(&world, &account, 2).await.slot_value(),
            Some(U256::from(22))
        );
        assert_eq!(
            slot(&world, &account, 9).await.slot_value(),
            Some(U256::zero())
        );

        let absent = world.prove_account(&head, NOBODY).await;
        let SlotProof::Known(known) = absent.slot(U256::one()) else {
            panic!("an absent account's slot needs no storage proof");
        };
        assert_eq!((known.account(), known.slot_value()), (Some(None), None));

        let eoa = world.prove_account(&head, world.signer(0).address).await;
        let SlotProof::Known(known) = eoa.slot(U256::one()) else {
            panic!("a slot of empty storage needs no storage proof");
        };
        assert_eq!(known.slot_value(), Some(U256::zero()));
        assert_eq!(
            known.key(),
            Some(Key::Slot(world.signer(0).address, U256::one()))
        );
    }

    #[tokio::test]
    async fn tampered_account_and_slot_are_rejected() {
        let (world, token) = world_with_token().await;
        let head = world.verified(world.head());
        let query = AccountQuery::new(&head, token);
        let mut reply = world.account_reply(&query.request()).await;
        reply.accounts[0].1.balance += U256::one();
        assert!(matches!(
            query.verify(reply),
            Err(VerifyError::LeafMismatch { .. })
        ));

        let account = world.prove_account(&head, token).await;
        let SlotProof::Wanted(query) = account.slot(U256::from(2)) else {
            panic!("slot 2 needs a proof");
        };
        let mut storage = world.storage_reply(&query.request()).await;
        storage.slots[0].1 = U256::from(23);
        assert!(matches!(
            query.verify(storage),
            Err(VerifyError::LeafMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn proof_against_another_root_is_rejected() {
        let mut world = World::new().await;
        let alice = world.signer(0).address;
        let old_head = world.verified(world.head());
        let stale = world
            .account_reply(&AccountQuery::new(&old_head, alice).request())
            .await;
        world.mine(vec![world.transfer(0, 0, NOBODY, eth(1))]).await;
        let query = AccountQuery::new(&world.verified(world.head()), alice);
        assert!(matches!(
            query.verify(stale),
            Err(VerifyError::RangeProof { .. })
        ));
    }

    #[tokio::test]
    async fn dropped_proof_node_is_rejected() {
        let world = World::new().await;
        let head = world.verified(world.head());
        for address in [world.signer(0).address, NOBODY] {
            let query = AccountQuery::new(&head, address);
            let mut reply = world.account_reply(&query.request()).await;
            // The server concatenates two edge proofs; a node may appear twice.
            reply.proof.sort();
            reply.proof.dedup();
            assert!(!reply.proof.is_empty());
            let original = query.verify(reply.clone()).unwrap();
            let mut failed = 0;
            for dropped in 0..reply.proof.len() {
                let mut forged = reply.clone();
                forged.proof.remove(dropped);
                match query.verify(forged) {
                    Err(error) => {
                        assert!(
                            error.blames_peer(),
                            "dropping node {dropped} gave {error:?}"
                        );
                        failed += 1;
                    }
                    // A node only the other edge path needs changes nothing.
                    Ok(same) => assert_eq!(same, original),
                }
            }
            assert!(failed > 0, "no node on the key's path for {address:#x}");
        }
    }

    #[tokio::test]
    async fn withheld_leaf_cannot_fake_absence() {
        let world = World::new().await;
        let head = world.verified(world.head());
        let query = AccountQuery::new(&head, world.signer(0).address);
        let key = query.request().key;
        let past =
            ethrex_common::utils::u256_to_h256(U256::from_big_endian(key.as_bytes()) + U256::one());
        let hiding = world
            .account_reply(&AccountRangeRequest {
                key: past,
                ..query.request()
            })
            .await;
        let error = query.verify(hiding).unwrap_err();
        assert!(
            matches!(error, VerifyError::WithheldLeaf { .. }),
            "{error:?}"
        );
        assert!(error.blames_peer());
    }

    #[tokio::test]
    async fn empty_reply_is_unavailable_not_absence() {
        let (world, token) = world_with_token().await;
        let head = world.verified(world.head());
        let root = head.header().state_root;
        let query = AccountQuery::new(&head, NOBODY);
        let error = query.verify(AccountReply::default()).unwrap_err();
        assert!(matches!(error, VerifyError::RootUnavailable { root: r } if r == root));
        assert!(!error.blames_peer());

        let account = world.prove_account(&head, token).await;
        let SlotProof::Wanted(query) = account.slot(U256::one()) else {
            panic!("slot 1 needs a proof");
        };
        assert!(matches!(
            query.verify(StorageReply::default()),
            Err(VerifyError::RootUnavailable { .. })
        ));
    }

    #[tokio::test]
    async fn code_is_checked_by_hash() {
        let (world, token) = world_with_token().await;
        let code_hash = world.account_at_head(token).unwrap().code_hash;
        let code = world.code(code_hash).unwrap();
        let query = CodeQuery::new(code_hash);
        assert_eq!(query.verify(Some(code)).unwrap().at(), None);
        let forged = bytes::Bytes::from_static(&[0x60, 0x00]);
        assert!(matches!(
            query.verify(Some(forged)),
            Err(VerifyError::CodeHash { expected, .. }) if expected == code_hash
        ));
        assert!(matches!(
            query.verify(None),
            Err(VerifyError::CodeUnavailable { hash }) if hash == code_hash
        ));
    }

    #[tokio::test]
    async fn chain_links_gap_headers_ascending() {
        let mut world = World::new().await;
        let anchor = world.verified(world.head());
        let mut hashes = Vec::new();
        for nonce in 0..3 {
            hashes.push(
                world
                    .mine(vec![world.transfer(0, nonce, NOBODY, eth(1))])
                    .await,
            );
        }
        let between = world.headers_between(anchor.hash(), hashes[2]);
        assert_eq!(between.len(), 2);

        let linked = chain(&anchor, world.verified(hashes[2]), between.clone()).unwrap();
        assert_eq!(
            linked.iter().map(VerifiedHeader::hash).collect::<Vec<_>>(),
            hashes
        );

        let missing_middle = between[1..].to_vec();
        assert!(matches!(
            chain(&anchor, world.verified(hashes[2]), missing_middle),
            Err(VerifyError::HeaderHash { .. })
        ));
        assert!(matches!(
            chain(&anchor, world.verified(hashes[2]), Vec::new()),
            Err(VerifyError::ChainBreak { .. })
        ));
    }

    #[tokio::test]
    async fn body_receipts_and_bal_are_checked_against_the_header() {
        let mut world = World::new().await;
        let hash = world.mine(vec![world.transfer(0, 0, NOBODY, eth(1))]).await;
        let header = world.verified(hash);

        let mut body = world.body(hash).await;
        assert_eq!(body.transactions.len(), 1);
        let good = super::body(header.clone(), body.clone()).unwrap();
        assert_eq!(good.body().transactions.len(), 1);
        body.transactions.clear();
        assert!(matches!(
            super::body(header.clone(), body),
            Err(VerifyError::Root {
                part: BlockPart::Transactions,
                ..
            })
        ));

        let mut receipts = world.receipts(hash).await;
        assert_eq!(receipts.len(), 1);
        let verified = super::receipts(header.clone(), receipts.clone()).unwrap();
        let view = verified.receipt(&good, 0).unwrap();
        assert_eq!(view.gas_used(), receipts[0].cumulative_gas_used);
        assert_eq!(view.first_log_index(), 0);
        receipts[0].cumulative_gas_used += 1;
        assert!(matches!(
            super::receipts(header.clone(), receipts),
            Err(VerifyError::Root {
                part: BlockPart::Receipts,
                ..
            })
        ));

        let bal = world.bal(hash).expect("amsterdam blocks carry a BAL");
        assert!(super::bal(header.clone(), Some(bal.clone())).is_ok());
        let unavailable = super::bal(header.clone(), None).unwrap_err();
        assert!(matches!(unavailable, VerifyError::BalUnavailable { .. }));
        assert!(!unavailable.blames_peer());
        let mut accounts = bal.accounts().to_vec();
        if let Some(change) = accounts
            .iter_mut()
            .find_map(|a| a.balance_changes.first_mut())
        {
            change.post_balance += U256::one();
        }
        let forged = BlockAccessList::from_accounts(accounts);
        assert!(matches!(
            super::bal(header, Some(forged)),
            Err(VerifyError::BalHash { .. })
        ));
    }
}
