//! The request-side API. Every method pins one verified head for its whole run, proves what
//! it lacks at that head, and folds what it learned back into the canonical state.
//!
//! The canonical [`State`] sits behind one mutex reached only through synchronous closures;
//! no guard crosses an await. Only the [`crate::follow::Follower`] advances or reanchors it.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use ethrex_common::types::{
    BlockHeader, ChainConfig, GenericTransaction, Transaction, calculate_base_fee_per_blob_gas,
    calculate_base_fee_per_gas,
};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_vm::ExecutionResult;
use tokio::sync::Notify;
use tracing::debug;

use crate::chain_proofs::ProofMode;
use crate::exec::{EstimateError, Executed, Misses, Runner};
use crate::fetch::{self, FetchError, Fetched};
use crate::light_client::Heads;
use crate::peers::{Census, NetError, Peers};
use crate::state::{Account, Key, Merge, Miss, State, Store};
use crate::txbook::{Submitted, SubmittedTx, VerifiedReceipt};
use crate::verify::{BlockNumHash, Proven, VerifiedHeader, VerifyError};
use crate::working::WorkingSet;

/// Most fetch-and-rerun rounds one request gets. A cold request with deep dependencies
/// (an ENS lookup through a resolver) can take tens.
pub const MISS_ROUNDS: usize = 64;

/// Which block a request reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockSel {
    /// The anchor.
    Latest,
    /// The anchor; pending transactions only affect nonces.
    Pending,
    /// The latest finalized header.
    Finalized,
    /// Same as finalized for this node.
    Safe,
    /// A held block by number.
    Number(u64),
    /// A held block by hash.
    Hash(H256),
}

/// How the node is doing, for status endpoints.
#[derive(Clone, Debug)]
pub struct Status {
    /// The anchor.
    pub anchor: BlockNumHash,
    /// Timestamp of the anchor block.
    pub anchor_timestamp: u64,
    /// The finalized execution header, once known.
    pub finalized: Option<BlockNumHash>,
    /// The light client's heads, once seen.
    pub heads: Option<Heads>,
    /// How chain proofs are used.
    pub proof_mode: ProofMode,
    /// Verified chain proofs.
    pub proofs_verified: usize,
    /// Execution blocks proven since the checkpoint.
    pub proven_payloads: usize,
    /// Whether the anchor's execution is proven since the checkpoint.
    pub anchor_proven: bool,
    /// Held accounts.
    pub held_accounts: usize,
    /// Held storage slots.
    pub held_slots: usize,
    /// Pinned keys.
    pub pins: usize,
    /// Learned keys.
    pub learned: usize,
    /// Pending submitted transactions.
    pub pending_txs: usize,
    /// Peer counts.
    pub peers: Census,
}

/// Why a request failed.
#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    /// Peers did not answer.
    #[error(transparent)]
    Net(#[from] NetError),
    /// A key could not be proven.
    #[error(transparent)]
    Unprovable(#[from] VerifyError),
    /// The fetch failed for another reason.
    #[error("{0}")]
    Fetch(String),
    /// The EVM refused the call.
    #[error("execution error: {0}")]
    Exec(String),
    /// The call reverted.
    #[error("execution reverted")]
    Reverted {
        /// Revert data.
        data: Bytes,
    },
    /// The call halted.
    #[error("execution halted: {reason}")]
    Halted {
        /// Why.
        reason: String,
    },
    /// Gas could not be estimated.
    #[error(transparent)]
    Estimate(#[from] EstimateError),
    /// The request kept discovering new keys.
    #[error("request did not converge after {rounds} fetch rounds")]
    TooManyMissRounds {
        /// Rounds run.
        rounds: usize,
    },
    /// The request asks for something this node does not serve.
    #[error("unsupported: {what}")]
    Unsupported {
        /// What.
        what: String,
    },
    /// The raw transaction is not acceptable.
    #[error("invalid transaction: {0}")]
    InvalidTransaction(String),
    /// The node is not anchored yet or is stopping.
    #[error("node not ready")]
    NotReady,
}

impl From<FetchError> for NodeError {
    fn from(err: FetchError) -> Self {
        match err {
            FetchError::Net(net) => Self::Net(net),
            FetchError::Unprovable(verify) => Self::Unprovable(verify),
            FetchError::Unsupported(what) => Self::Unsupported {
                what: what.to_string(),
            },
        }
    }
}

/// Proof bookkeeping the follower publishes for status and `require` mode.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProofSummary {
    /// Verified chain proofs.
    pub verified: usize,
    /// Execution blocks proven since the checkpoint.
    pub proven_payloads: usize,
    /// Whether the current anchor is proven.
    pub anchor_proven: bool,
}

/// What the node and the follower share.
pub struct Shared {
    state: Mutex<State>,
    peers: Arc<dyn Peers>,
    chain_config: ChainConfig,
    gas_cap: u64,
    proof_mode: ProofMode,
    heads: Mutex<Option<Heads>>,
    proofs: Mutex<ProofSummary>,
    /// Wakes the follower to warm unheld keys.
    pub(crate) wake: Notify,
}

impl Shared {
    /// Shared state anchored at `anchor` with nothing held.
    pub fn new(
        anchor: VerifiedHeader,
        working: WorkingSet,
        peers: Arc<dyn Peers>,
        chain_config: ChainConfig,
        gas_cap: u64,
        proof_mode: ProofMode,
    ) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::new(anchor, working)),
            peers,
            chain_config,
            gas_cap,
            proof_mode,
            heads: Mutex::new(None),
            proofs: Mutex::new(ProofSummary::default()),
            wake: Notify::new(),
        })
    }

    /// Reads the state through a synchronous closure.
    pub fn read<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        let guard = match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&guard)
    }

    /// Updates the state through a synchronous closure.
    pub fn update<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut guard = match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&mut guard)
    }

    /// The peers.
    pub fn peers(&self) -> &Arc<dyn Peers> {
        &self.peers
    }

    /// The chain configuration.
    pub const fn chain_config(&self) -> &ChainConfig {
        &self.chain_config
    }

    /// How chain proofs are used.
    pub const fn proof_mode(&self) -> ProofMode {
        self.proof_mode
    }

    pub(crate) fn set_heads(&self, heads: Heads) {
        if let Ok(mut guard) = self.heads.lock() {
            *guard = Some(heads);
        }
    }

    pub(crate) fn set_proofs(&self, summary: ProofSummary) {
        if let Ok(mut guard) = self.proofs.lock() {
            *guard = summary;
        }
    }

    /// The store a request pins: the published one, or a retained one for a held block.
    fn pin(&self, sel: BlockSel) -> Result<Arc<Store>, NodeError> {
        self.read(|state| {
            let store = state.store();
            match sel {
                BlockSel::Latest | BlockSel::Pending => Ok(state.snapshot()),
                BlockSel::Number(number) => {
                    let hash = store
                        .header(number)
                        .map(VerifiedHeader::hash)
                        .ok_or_else(|| unsupported_state(store))?;
                    state
                        .snapshot_at(hash)
                        .ok_or_else(|| unsupported_state(store))
                }
                BlockSel::Hash(hash) => state
                    .snapshot_at(hash)
                    .ok_or_else(|| unsupported_state(store)),
                BlockSel::Finalized | BlockSel::Safe => {
                    let finalized = state.finalized().ok_or(NodeError::NotReady)?;
                    state
                        .snapshot_at(finalized.hash())
                        .ok_or_else(|| unsupported_state(store))
                }
            }
        })
    }

    /// Runs `step` over a pinned store, proving the misses it reports at that store's
    /// anchor between runs, then learns every key read or fetched and folds the facts into
    /// the canonical store if it is still the published one.
    pub(crate) async fn resolve<T>(
        &self,
        mut store: Arc<Store>,
        step: impl Fn(Arc<Store>) -> Result<Executed<T>, Misses>,
    ) -> Result<T, NodeError> {
        let anchor = store.anchor().hash();
        let mut facts: Vec<Proven> = Vec::new();
        let mut read: HashSet<Key> = HashSet::new();
        let outcome = async {
            for round in 0..=MISS_ROUNDS {
                match step(Arc::clone(&store)) {
                    Ok(executed) => {
                        read.extend(executed.read);
                        return Ok(executed.result);
                    }
                    Err(misses) => {
                        if round == MISS_ROUNDS {
                            return Err(NodeError::TooManyMissRounds { rounds: round });
                        }
                        let fetched = fetch::misses(&self.peers, &store, misses.iter()).await?;
                        let inner = Arc::make_mut(&mut store);
                        for item in fetched {
                            match item {
                                Fetched::Proven(proven) => {
                                    if let Some(key) = proven.key() {
                                        read.insert(key);
                                    }
                                    inner
                                        .insert(&proven)
                                        .map_err(|e| NodeError::Fetch(e.to_string()))?;
                                    facts.push(proven);
                                }
                                Fetched::Ancestors(headers) => {
                                    inner
                                        .insert_ancestors(headers)
                                        .map_err(|e| NodeError::Fetch(e.to_string()))?;
                                }
                            }
                        }
                    }
                }
            }
            Err(NodeError::TooManyMissRounds {
                rounds: MISS_ROUNDS,
            })
        }
        .await;
        let merged = self.update(|state| {
            state.learn(read.iter().copied());
            let merged = state.merge(anchor, &facts);
            if state.unheld().next().is_some() {
                self.wake.notify_one();
            }
            merged
        });
        if merged == Merge::Dropped && !facts.is_empty() {
            debug!("facts proven at a moved anchor were dropped");
        }
        outcome
    }
}

fn unsupported_state(store: &Store) -> NodeError {
    NodeError::Unsupported {
        what: format!(
            "state at blocks other than the verified head {} and the ones retained before it",
            store.anchor().number()
        ),
    }
}

/// The node's request-side API.
#[derive(Clone)]
pub struct Node {
    shared: Arc<Shared>,
    client_version: String,
}

impl Node {
    /// A node over `shared`.
    pub fn new(shared: Arc<Shared>, client_version: String) -> Self {
        Self {
            shared,
            client_version,
        }
    }

    /// The shared state.
    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    /// The client version string.
    pub fn client_version(&self) -> &str {
        &self.client_version
    }

    /// The chain id.
    pub fn chain_id(&self) -> u64 {
        self.shared.chain_config.chain_id
    }

    /// The chain configuration.
    pub fn chain_config(&self) -> ChainConfig {
        self.shared.chain_config
    }

    /// The anchor header.
    pub fn head(&self) -> VerifiedHeader {
        self.shared.read(|state| state.anchor().clone())
    }

    /// A header by selector: the anchor, a held ancestor, or the finalized header.
    pub fn header(&self, sel: BlockSel) -> Result<VerifiedHeader, NodeError> {
        self.shared.read(|state| {
            let store = state.store();
            match sel {
                BlockSel::Latest | BlockSel::Pending => Ok(store.anchor().clone()),
                BlockSel::Number(number) => {
                    store
                        .header(number)
                        .cloned()
                        .ok_or_else(|| NodeError::Unsupported {
                            what: format!("header {number} is not held"),
                        })
                }
                BlockSel::Hash(hash) => store
                    .header_by_hash(hash)
                    .cloned()
                    .or_else(|| state.finalized().filter(|f| f.hash() == hash).cloned())
                    .ok_or_else(|| NodeError::Unsupported {
                        what: format!("header {hash:#x} is not held"),
                    }),
                BlockSel::Finalized | BlockSel::Safe => {
                    state.finalized().cloned().ok_or(NodeError::NotReady)
                }
            }
        })
    }

    /// `Ok(None)` is a proven-absent account.
    pub async fn account(
        &self,
        address: Address,
        sel: BlockSel,
    ) -> Result<Option<Account>, NodeError> {
        let store = self.shared.pin(sel)?;
        self.shared
            .resolve(store, move |store| match store.account(address) {
                Ok(account) => Ok(Executed {
                    result: account.copied(),
                    read: HashSet::from([Key::Account(address)]),
                }),
                Err(miss) => Err(single(miss)),
            })
            .await
    }

    /// A storage slot.
    pub async fn storage(
        &self,
        address: Address,
        slot: U256,
        sel: BlockSel,
    ) -> Result<U256, NodeError> {
        let store = self.shared.pin(sel)?;
        self.shared
            .resolve(store, move |store| match store.slot(address, slot) {
                Ok(value) => Ok(Executed {
                    result: value,
                    read: HashSet::from([Key::Slot(address, slot)]),
                }),
                // One storage proof carries its account proof.
                Err(Miss::Account(_)) => Err(single(Miss::Storage(address, slot))),
                Err(miss) => Err(single(miss)),
            })
            .await
    }

    /// Code of an account; empty for no account.
    pub async fn code(&self, address: Address, sel: BlockSel) -> Result<Bytes, NodeError> {
        let store = self.shared.pin(sel)?;
        self.shared
            .resolve(store, move |store| {
                let account = match store.account(address) {
                    Ok(Some(account)) => *account,
                    Ok(None) => {
                        return Ok(Executed {
                            result: Bytes::new(),
                            read: HashSet::from([Key::Account(address)]),
                        });
                    }
                    Err(miss) => return Err(single(miss)),
                };
                match store.code(account.code_hash) {
                    Ok(code) => Ok(Executed {
                        result: code.code_bytes(),
                        read: HashSet::from([Key::Account(address)]),
                    }),
                    Err(miss) => Err(single(miss)),
                }
            })
            .await
    }

    /// Nonce of an account. `Pending` adds consecutive nonces of submitted transactions.
    pub async fn transaction_count(
        &self,
        address: Address,
        sel: BlockSel,
    ) -> Result<u64, NodeError> {
        let nonce = self.account(address, sel).await?.map_or(0, |a| a.nonce);
        if sel == BlockSel::Pending {
            return Ok(self
                .shared
                .read(|state| state.txs().pending_nonce(address, nonce)));
        }
        Ok(nonce)
    }

    /// Runs `tx` as `eth_call`.
    pub async fn call(
        &self,
        tx: GenericTransaction,
        sel: BlockSel,
    ) -> Result<ExecutionResult, NodeError> {
        let store = self.shared.pin(sel)?;
        let chain_config = self.shared.chain_config;
        let gas_cap = self.shared.gas_cap;
        let result = self
            .shared
            .resolve(store, move |store| {
                Runner::new(store, chain_config, gas_cap).call(&tx)
            })
            .await?;
        match result {
            Ok(ExecutionResult::Revert { output, .. }) => Err(NodeError::Reverted { data: output }),
            Ok(ExecutionResult::Halt { reason, .. }) => Err(NodeError::Halted { reason }),
            Ok(result) => Ok(result),
            Err(err) => Err(NodeError::Exec(err.to_string())),
        }
    }

    /// Estimates the gas `tx` needs.
    pub async fn estimate_gas(
        &self,
        tx: GenericTransaction,
        sel: BlockSel,
    ) -> Result<u64, NodeError> {
        let store = self.shared.pin(sel)?;
        let chain_config = self.shared.chain_config;
        let gas_cap = self.shared.gas_cap;
        let result = self
            .shared
            .resolve(store, move |store| {
                Runner::new(store, chain_config, gas_cap).estimate_gas(&tx)
            })
            .await?;
        Ok(result?)
    }

    /// Records and broadcasts a raw transaction. Returns its hash even if no peer got it:
    /// the follower rebroadcasts pending transactions.
    pub async fn submit(&self, raw: &[u8]) -> Result<H256, NodeError> {
        let tx = Transaction::decode_canonical(raw)
            .map_err(|e| NodeError::InvalidTransaction(e.to_string()))?;
        if matches!(tx, Transaction::EIP4844Transaction(_)) {
            return Err(NodeError::InvalidTransaction(
                "blob transactions are not relayed by this node".into(),
            ));
        }
        match tx.chain_id() {
            Some(id) if id == self.chain_id() => {}
            Some(id) => {
                return Err(NodeError::InvalidTransaction(format!(
                    "chain id {id} is not this chain's {}",
                    self.chain_id()
                )));
            }
            None => {
                return Err(NodeError::InvalidTransaction(
                    "transactions without a chain id are not relayed".into(),
                ));
            }
        }
        let sender = tx
            .sender(&NativeCrypto)
            .map_err(|e| NodeError::InvalidTransaction(e.to_string()))?;
        let hash = tx.hash(&NativeCrypto);
        let since: BlockNumHash = (&self.head()).into();
        let outcome = self
            .shared
            .update(|state| state.txs_mut().submit(tx.clone(), sender, since));
        if outcome != Submitted::Duplicate {
            match self.shared.peers.broadcast(tx).await {
                Ok(count) => debug!(%hash, count, "transaction sent to peers"),
                Err(err) => debug!(%hash, %err, "transaction not sent yet"),
            }
        }
        Ok(hash)
    }

    /// A submitted transaction by hash.
    pub fn transaction(&self, hash: H256) -> Option<SubmittedTx> {
        self.shared.read(|state| state.txs().get(hash))
    }

    /// The verified receipt of a submitted transaction.
    pub fn receipt(&self, hash: H256) -> Option<VerifiedReceipt> {
        self.shared.read(|state| state.txs().receipt(hash).cloned())
    }

    /// The base fee the next block will charge, from the anchor.
    pub fn next_base_fee(&self) -> u64 {
        next_base_fee(self.head().header(), &self.shared.chain_config)
    }

    /// A priority fee suggestion from recent verified blocks' transactions, or 1 gwei.
    pub async fn max_priority_fee(&self) -> Result<u64, NodeError> {
        const DEFAULT_TIP: u64 = 1_000_000_000;
        const CAP: u64 = 500_000_000_000;
        const BLOCKS: usize = 3;
        let headers: Vec<VerifiedHeader> = self.shared.read(|state| {
            state
                .store()
                .headers()
                .rev()
                .take(BLOCKS)
                .cloned()
                .collect()
        });
        let mut tips: Vec<u64> = Vec::new();
        for header in headers {
            let base_fee = header.header().base_fee_per_gas;
            let Ok(body) = fetch::body(self.shared.peers.as_ref(), header).await else {
                continue;
            };
            let mut block_tips: Vec<u64> = body
                .body()
                .transactions
                .iter()
                .filter_map(|tx| {
                    let effective = tx.effective_gas_price(base_fee)?;
                    let tip = effective.saturating_sub(U256::from(base_fee.unwrap_or_default()));
                    u64::try_from(tip).ok()
                })
                .collect();
            block_tips.sort_unstable();
            tips.extend(block_tips.into_iter().take(3));
        }
        if tips.is_empty() {
            return Ok(DEFAULT_TIP);
        }
        tips.sort_unstable();
        let index = (tips.len().saturating_sub(1)).saturating_mul(60) / 100;
        Ok(tips.get(index).copied().unwrap_or(DEFAULT_TIP).min(CAP))
    }

    /// `eth_gasPrice`: next base fee plus the tip suggestion.
    pub async fn gas_price(&self) -> Result<u64, NodeError> {
        Ok(self
            .next_base_fee()
            .saturating_add(self.max_priority_fee().await?))
    }

    /// `eth_feeHistory` over held headers: base fees, gas-used ratios and, if asked, reward
    /// percentiles from verified bodies. `newest` is the newest block included.
    pub async fn fee_history(
        &self,
        block_count: u64,
        newest: BlockSel,
        reward_percentiles: &[f64],
    ) -> Result<FeeHistory, NodeError> {
        const MAX_BLOCKS: u64 = 1024;
        const MAX_BLOCKS_WITH_REWARDS: u64 = 16;
        let newest = self.header(newest)?;
        let cap = if reward_percentiles.is_empty() {
            MAX_BLOCKS
        } else {
            MAX_BLOCKS_WITH_REWARDS
        };
        let block_count = block_count.clamp(1, cap);
        let headers: Vec<VerifiedHeader> = self.shared.read(|state| {
            let store = state.store();
            let oldest = newest
                .number()
                .saturating_sub(block_count.saturating_sub(1));
            (oldest..=newest.number())
                .filter_map(|n| store.header(n).cloned())
                .collect()
        });
        let Some(first) = headers.first() else {
            return Err(NodeError::Unsupported {
                what: "fee history outside the held headers".into(),
            });
        };
        let config = &self.shared.chain_config;
        let mut history = FeeHistory {
            oldest_block: first.number(),
            base_fee_per_gas: Vec::new(),
            base_fee_per_blob_gas: Vec::new(),
            gas_used_ratio: Vec::new(),
            blob_gas_used_ratio: Vec::new(),
            reward: Vec::new(),
        };
        for header in &headers {
            let h = header.header();
            history
                .base_fee_per_gas
                .push(h.base_fee_per_gas.unwrap_or_default());
            history.base_fee_per_blob_gas.push(blob_base_fee(h, config));
            history.gas_used_ratio.push(ratio(h.gas_used, h.gas_limit));
            let max_blob_gas = config.get_fork_blob_schedule(h.timestamp).map_or(0, |s| {
                u64::from(s.max).saturating_mul(u64::from(ethrex_common::constants::GAS_PER_BLOB))
            });
            history
                .blob_gas_used_ratio
                .push(ratio(h.blob_gas_used.unwrap_or_default(), max_blob_gas));
            if !reward_percentiles.is_empty() {
                let body = fetch::body(self.shared.peers.as_ref(), header.clone()).await?;
                let mut tips: Vec<(u64, u64)> = body
                    .body()
                    .transactions
                    .iter()
                    .filter_map(|tx| {
                        let effective = tx.effective_gas_price(h.base_fee_per_gas)?;
                        let tip = effective
                            .saturating_sub(U256::from(h.base_fee_per_gas.unwrap_or_default()));
                        Some((u64::try_from(tip).ok()?, tx.gas_limit()))
                    })
                    .collect();
                tips.sort_unstable();
                let total_gas: u64 = tips.iter().map(|(_, gas)| *gas).sum();
                let rewards = reward_percentiles
                    .iter()
                    .map(|p| {
                        let threshold = (total_gas as f64) * p / 100.0;
                        let mut acc = 0u64;
                        for (tip, gas) in &tips {
                            acc = acc.saturating_add(*gas);
                            if (acc as f64) >= threshold {
                                return *tip;
                            }
                        }
                        tips.last().map_or(0, |(tip, _)| *tip)
                    })
                    .collect();
                history.reward.push(rewards);
            }
        }
        if let Some(last) = headers.last() {
            history
                .base_fee_per_gas
                .push(next_base_fee(last.header(), config));
            history
                .base_fee_per_blob_gas
                .push(next_blob_base_fee(last.header(), config));
        }
        Ok(history)
    }

    /// The node's status.
    pub async fn status(&self) -> Status {
        let peers = self.shared.peers.census().await;
        let heads = self.shared.heads.lock().ok().and_then(|g| *g);
        let proofs = self.shared.proofs.lock().map(|g| *g).unwrap_or_default();
        self.shared.read(|state| {
            let store = state.store();
            Status {
                anchor: store.anchor().into(),
                anchor_timestamp: store.anchor().header().timestamp,
                finalized: state.finalized().map(BlockNumHash::from),
                heads,
                proof_mode: self.shared.proof_mode,
                proofs_verified: proofs.verified,
                proven_payloads: proofs.proven_payloads,
                anchor_proven: proofs.anchor_proven,
                held_accounts: store.account_count(),
                held_slots: store.slot_count(),
                pins: state.working().pin_count(),
                learned: state.working().learned().count(),
                pending_txs: state.txs().pending().len(),
                peers,
            }
        })
    }
}

/// `eth_feeHistory` data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeeHistory {
    /// First block covered.
    pub oldest_block: u64,
    /// Base fees, one per block plus the next block's.
    pub base_fee_per_gas: Vec<u64>,
    /// Blob base fees, one per block plus the next block's.
    pub base_fee_per_blob_gas: Vec<U256>,
    /// Gas used over gas limit, per block.
    pub gas_used_ratio: Vec<f64>,
    /// Blob gas used over the maximum, per block.
    pub blob_gas_used_ratio: Vec<f64>,
    /// Reward percentiles per block, if requested.
    pub reward: Vec<Vec<u64>>,
}

fn single(miss: Miss) -> Misses {
    Misses::one(miss)
}

fn ratio(used: u64, limit: u64) -> f64 {
    if limit == 0 {
        0.0
    } else {
        (used as f64) / (limit as f64)
    }
}

/// The base fee of the block after `header`.
pub fn next_base_fee(header: &BlockHeader, _config: &ChainConfig) -> u64 {
    let Some(parent_base_fee) = header.base_fee_per_gas else {
        return 0;
    };
    calculate_base_fee_per_gas(
        header.gas_limit,
        header.gas_limit,
        header.gas_used,
        parent_base_fee,
        ethrex_common::types::ELASTICITY_MULTIPLIER,
    )
    .unwrap_or(parent_base_fee)
}

fn blob_base_fee(header: &BlockHeader, config: &ChainConfig) -> U256 {
    let Some(schedule) = config.get_fork_blob_schedule(header.timestamp) else {
        return U256::zero();
    };
    calculate_base_fee_per_blob_gas(
        header.excess_blob_gas.unwrap_or_default(),
        schedule.base_fee_update_fraction,
    )
}

fn next_blob_base_fee(header: &BlockHeader, config: &ChainConfig) -> U256 {
    let Some(schedule) = config.get_fork_blob_schedule(header.timestamp) else {
        return U256::zero();
    };
    let fork = config.fork(header.timestamp);
    let excess = ethrex_common::types::calc_excess_blob_gas(header, schedule, fork);
    calculate_base_fee_per_blob_gas(excess, schedule.base_fee_update_fraction)
}
