//! FOCIL enforcement for frame transactions: Profile 2 of the FOCIL
//! frame-transaction EIP, the second branch of the EIP-7805 omission check.
//!
//! [`inclusion_list_validator`](crate::inclusion_list_validator) keeps the
//! EIP-7805 nonce-and-balance check verbatim as Profile 1. This module adds the
//! check for EIP-8141 frame transactions, whose validity is decided by code the
//! transaction itself names. An omitted frame transaction's validation prefix
//! is replayed at the two states the evaluator already holds, the one the block
//! executed from (`S_start`, the parent's post-state) and the one it ended at
//! (`S_end`, after the last transaction and before withdrawals), and the
//! omission is unjustified if the transaction is eligible at either:
//!
//! ```text
//! omission unjustified  <=>  eligible at S_start  or  eligible at S_end
//! ```
//!
//! Three stateless pieces are computed from the inclusion list alone, so every
//! evaluator reaches the same result from nothing: candidacy
//! ([`profile2_candidate`]), which decides from the transaction bytes whether a
//! frame transaction fits a shape the replay can judge; its budget cost
//! ([`verify_budget_cost`]); and the per-list budget fill
//! ([`budget_fill`]), which bounds the metered replay an attester performs on
//! anyone's behalf. The stateful omission check
//! ([`Blockchain::check_profile2_omissions`]) then replays each admitted,
//! absent candidate through the VM with the Profile 2 policy attached (the
//! validation surface and the per-list code budget), and records rather than
//! guesses whenever a verdict cannot be computed.
//!
//! ## One list, not sixteen
//!
//! The EIP defines the budget fill and the code budget per inclusion list, in
//! list order, before deduplication across lists. The Engine API delivers a
//! single flat `inclusionListTransactions` array with no list boundaries, so
//! the execution layer cannot recover which committee member listed what. This
//! implementation treats the delivered array as the one list: one gas budget
//! and one code budget for everything it carries, in the order delivered.

use std::collections::HashSet;
use std::sync::Arc;

use ethrex_common::{
    Address, H256, U256,
    types::{
        AccountState, BlockHeader, ChainConfig, Code, CodeMetadata, Fork, FrameMode,
        FrameTransaction, GWEI_TO_WEI, PrefixShape, Transaction, ValidationPrefix, Withdrawal,
    },
};
use ethrex_crypto::{Crypto, NativeCrypto};
use ethrex_vm::{CodeBudget, Evm, EvmError, Profile2Replay, VmDatabase, validate_frame_signatures};
use rustc_hash::FxHashMap;
use tracing::warn;

use crate::Blockchain;
use crate::constants::{AMSTERDAM_MAX_CODE_SIZE, MAX_CODE_SIZE};
use crate::error::MempoolError;
use crate::vm::StoreVmDatabase;

/// `MAX_VERIFY_GAS_PER_IL`: the declared validation gas one inclusion list may
/// have replayed on every attester's behalf. A consensus constant: it is never
/// read from node configuration, and in particular never from the mempool's
/// operator-tunable `MAX_VERIFY_GAS`, which answers a different question.
pub const MAX_VERIFY_GAS_PER_IL: u64 = 1 << 20;
/// `MAX_VERIFY_GAS_PER_TX`: the most one transaction may declare across its
/// signatures, its protocol verifier frames and its validation prefix. Equal to
/// the list budget, so one transaction may consume a whole list's budget.
pub const MAX_VERIFY_GAS_PER_TX: u64 = MAX_VERIFY_GAS_PER_IL;
/// `MAX_VALIDATION_CODE_BODIES`: distinct code bodies one list's replays may load.
pub const MAX_VALIDATION_CODE_BODIES: u64 = 16;

/// `MAX_VALIDATION_CODE_BYTES = MAX_VALIDATION_CODE_BODIES * MAX_CODE_SIZE`,
/// with `MAX_CODE_SIZE` the one the active fork defines (EIP-7907 raises it at
/// Amsterdam).
pub fn max_validation_code_bytes(config: &ChainConfig, block_timestamp: u64) -> u64 {
    let max_code_size = if config.is_amsterdam_activated(block_timestamp) {
        AMSTERDAM_MAX_CODE_SIZE
    } else {
        MAX_CODE_SIZE
    };
    MAX_VALIDATION_CODE_BODIES.saturating_mul(u64::from(max_code_size))
}

/// A listed frame transaction that passed every Profile 2 candidacy condition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile2Candidate {
    /// The validation prefix, shape-matched with the protocol verifier frames
    /// disregarded; `recent_root_index` names the leading recent-root frame.
    pub prefix: ValidationPrefix,
    /// `payer`, resolved from the prefix shape before any frame executes:
    /// `sender` for the `self_verify` shapes, the `pay` frame's resolved target
    /// otherwise (a null target resolves to `sender`).
    pub payer: Address,
    /// `verify_budget_cost(tx)`, the static price the budget fill charges.
    pub verify_budget_cost: u64,
}

/// Why a listed frame transaction is not a Profile 2 candidate. Each variant is
/// one candidacy condition of the EIP; a transaction failing any of them has
/// its omission excused without touching state.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NotProfile2Candidate {
    #[error("statically invalid frame transaction: {0}")]
    StaticallyInvalid(String),
    #[error("carries blob versioned hashes")]
    CarriesBlobs,
    #[error("frame {frame_index}: protocol verifier frame out of position or duplicated")]
    ProtocolVerifierMisplaced { frame_index: usize },
    #[error("validation prefix matches no admitted shape")]
    UnrecognisedShape,
    #[error("frame {frame_index}: verify frame targets an account other than the sender")]
    VerifyTargetNotSender { frame_index: usize },
    #[error("frame {frame_index}: validation prefix frame has ATOMIC_BATCH_FLAG set")]
    AtomicBatchInPrefix { frame_index: usize },
    #[error("frame {frame_index}: VERIFY-mode body frame")]
    VerifyBodyFrame { frame_index: usize },
    #[error("frame {frame_index}: mode not defined by EIP-8141")]
    UndefinedMode { frame_index: usize },
    #[error("verify_budget_cost {cost} exceeds MAX_VERIFY_GAS_PER_TX {limit}")]
    BudgetCostExceeded { cost: u64, limit: u64 },
}

/// The declared execution gas of the validation prefix and of the protocol
/// verifier frames, decided from decoding and shape alone. `None` when the
/// transaction is unpriceable: statically invalid, or matching no admitted
/// shape.
///
/// Shape matching filters the protocol verifier frames out of the prefix;
/// replay executes them, so their declared gas is added back here. An
/// implementation that prices the filtered list undercounts by the expiry
/// frame and the recent-root frame.
pub fn prefix_and_verifier_frame_cost(tx: &FrameTransaction) -> Option<u64> {
    if tx.validate_static_constraints().is_err() {
        return None;
    }
    let prefix = tx.validation_prefix().ok()?;
    let mut cost = 0u64;
    for &index in &prefix.frame_indices {
        cost = cost.saturating_add(tx.frames.get(index)?.gas_limit);
    }
    for (index, frame) in tx.frames.iter().enumerate() {
        if frame.is_expiry_verifier() || Some(index) == prefix.recent_root_index {
            cost = cost.saturating_add(frame.gas_limit);
        }
    }
    Some(cost)
}

/// `verify_budget_cost(tx)`: the intrinsic signature verification cost plus
/// [`prefix_and_verifier_frame_cost`]. `None` when the latter is.
pub fn verify_budget_cost(tx: &FrameTransaction) -> Option<u64> {
    Some(
        tx.signature_verification_cost()
            .saturating_add(prefix_and_verifier_frame_cost(tx)?),
    )
}

/// Profile 2 candidacy, conditions 1 through 8 of the EIP, decided from the
/// transaction bytes alone with no state access.
pub fn profile2_candidate(
    tx: &FrameTransaction,
) -> Result<Profile2Candidate, NotProfile2Candidate> {
    // 1. A statically valid EIP-8141 frame transaction.
    tx.validate_static_constraints()
        .map_err(NotProfile2Candidate::StaticallyInvalid)?;
    // 2. No blobs: blob gas has its own budget and no omission check over it.
    if !tx.blob_versioned_hashes.is_empty() {
        return Err(NotProfile2Candidate::CarriesBlobs);
    }
    // 7. Every frame has a mode EIP-8141 defines. Static validation already
    // rejects reserved modes; kept explicit so a later relaxation there cannot
    // admit a frame kind the replay never observes.
    if let Some((frame_index, _)) = tx
        .frames
        .iter()
        .enumerate()
        .find(|(_, frame)| frame.execution_mode().is_none())
    {
        return Err(NotProfile2Candidate::UndefinedMode { frame_index });
    }
    // 4. Protocol verifier frames in the positions EIP-8141 and EIP-8272
    // require: an expiry frame first, a recent-root frame immediately after it
    // or first in its absence, at most one of each. Static validation bounds
    // the expiry frame to one; `recent_root_verifier_index` names the one
    // leading position, so a second recent-root frame is misplaced by definition.
    let recent_root_index = tx.recent_root_verifier_index();
    for (frame_index, frame) in tx.frames.iter().enumerate() {
        if frame.is_expiry_verifier() && frame_index != 0 {
            return Err(NotProfile2Candidate::ProtocolVerifierMisplaced { frame_index });
        }
        if frame.is_recent_root_verifier() && Some(frame_index) != recent_root_index {
            return Err(NotProfile2Candidate::ProtocolVerifierMisplaced { frame_index });
        }
    }
    // 3. Modes and flags of the prefix match one of the four shapes (with the
    // protocol verifier frames disregarded); the targets are checked below.
    let prefix = tx
        .validation_prefix()
        .map_err(|_| NotProfile2Candidate::UnrecognisedShape)?;
    let is_sponsored = matches!(
        prefix.shape,
        PrefixShape::OnlyVerifyPay | PrefixShape::DeployOnlyVerifyPay
    );
    for &frame_index in &prefix.frame_indices {
        let frame = tx
            .frames
            .get(frame_index)
            .ok_or(NotProfile2Candidate::UnrecognisedShape)?;
        // 5. No validation prefix frame carries ATOMIC_BATCH_FLAG.
        if frame.is_atomic_batch() {
            return Err(NotProfile2Candidate::AtomicBatchInPrefix { frame_index });
        }
        // 3, targets: `self_verify` and `only_verify` target the sender, explicitly
        // or through a null target; the deploy frame targets a factory and the
        // `pay` frame a sponsor, so neither is constrained.
        let is_pay_frame = is_sponsored && prefix.pay_index == Some(frame_index);
        let is_deploy_frame = prefix.deploy_index == Some(frame_index);
        if !is_pay_frame
            && !is_deploy_frame
            && frame.target.is_some_and(|target| target != tx.sender)
        {
            return Err(NotProfile2Candidate::VerifyTargetNotSender { frame_index });
        }
    }
    // 6. No body frame has mode VERIFY: the replay never observes a body frame,
    // and a reverting VERIFY frame there would invalidate a transaction that
    // replayed cleanly.
    let prefix_end = prefix.frame_indices.last().copied().unwrap_or_default();
    if let Some((frame_index, _)) = tx
        .frames
        .iter()
        .enumerate()
        .skip(prefix_end.saturating_add(1))
        .find(|(_, frame)| frame.execution_mode() == Some(FrameMode::Verify))
    {
        return Err(NotProfile2Candidate::VerifyBodyFrame { frame_index });
    }
    // 8. The static budget cost fits the per-transaction cap.
    let cost = verify_budget_cost(tx).ok_or(NotProfile2Candidate::UnrecognisedShape)?;
    if cost > MAX_VERIFY_GAS_PER_TX {
        return Err(NotProfile2Candidate::BudgetCostExceeded {
            cost,
            limit: MAX_VERIFY_GAS_PER_TX,
        });
    }

    let payer = if is_sponsored {
        prefix
            .pay_index
            .and_then(|index| tx.frames.get(index))
            .and_then(|frame| frame.target)
            .unwrap_or(tx.sender)
    } else {
        tx.sender
    };

    Ok(Profile2Candidate {
        prefix,
        payer,
        verify_budget_cost: cost,
    })
}

/// The outcome of the budget fill over one inclusion list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BudgetFill {
    /// Hashes of the transactions with at least one admitted occurrence.
    pub admitted: HashSet<H256>,
    /// What is left of `MAX_VERIFY_GAS_PER_IL` after every occurrence.
    pub remaining_gas: u64,
}

/// The EIP's budget fill over one inclusion list, in list order, before any
/// deduplication. Frame transactions without blobs are metered; everything
/// else passes through unmetered (Profile 1 candidates are not processed here).
///
/// The debit is in two stages so that a structurally valid transaction with a
/// bad signature pays for the signature check it caused and nothing more: the
/// signature half is charged before the signatures are verified, the prefix
/// half only once they pass. A candidate that fails after its cost is deducted
/// keeps the debit. An unpriceable occurrence, or one that does not fit the
/// per-transaction cap or what remains of the list budget, is ignored without
/// a debit.
pub fn budget_fill(il: &[Transaction], fork: Fork, crypto: &dyn Crypto) -> BudgetFill {
    let mut remaining = MAX_VERIFY_GAS_PER_IL;
    let mut admitted = HashSet::new();
    for occurrence in il {
        let Transaction::FrameTransaction(frame_tx) = occurrence else {
            continue;
        };
        if !frame_tx.blob_versioned_hashes.is_empty() {
            continue;
        }
        let Some(prefix_cost) = prefix_and_verifier_frame_cost(frame_tx) else {
            continue;
        };
        let signature_cost = frame_tx.signature_verification_cost();
        let cost = signature_cost.saturating_add(prefix_cost);
        if cost > MAX_VERIFY_GAS_PER_TX || cost > remaining {
            continue;
        }
        remaining = remaining.saturating_sub(signature_cost);
        if !validate_frame_signatures(
            &frame_tx.signatures,
            frame_tx.compute_sig_hash(),
            frame_tx.sender,
            fork,
            crypto,
        ) {
            continue;
        }
        remaining = remaining.saturating_sub(prefix_cost);
        if profile2_candidate(frame_tx).is_ok() {
            admitted.insert(occurrence.hash(crypto));
        }
    }
    BudgetFill {
        admitted,
        remaining_gas: remaining,
    }
}

/// The judged block `B`, as the omission check needs it.
#[derive(Debug, Clone, Copy)]
pub struct JudgedBlock<'a> {
    /// `B`'s header: the replay context at both states, and the root of `S_end`
    /// (before the withdrawal credits are discounted).
    pub header: &'a BlockHeader,
    /// `P`'s header: the root of `S_start`.
    pub parent_header: &'a BlockHeader,
    /// `B`'s withdrawals. `S_end` precedes them, and the committed post-state
    /// root does not, so their credits are discounted from `S_end` balances.
    pub withdrawals: &'a [Withdrawal],
    /// Hashes of the transactions present in `B`, anywhere in the block.
    pub block_tx_hashes: &'a HashSet<H256>,
}

/// The Profile 2 omission check's result over one inclusion list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile2Verdict {
    /// The first listed frame transaction whose omission is unjustified, if any.
    pub unjustified: Option<H256>,
    /// Transactions for which a verdict could not be computed at a state, and
    /// no computed replay found eligible, with the reason. Recorded so the
    /// failure is visible, never treated as ineligibility and never grounds for
    /// reporting the payload unsatisfied.
    pub undecided: Vec<(H256, String)>,
}

/// `S_end` is the state after `B`'s last transaction, before the withdrawals,
/// while the committed post-state root already carries their credits. This
/// view reads the committed state and subtracts each recipient's block credits
/// again. Withdrawals only ever add balance, so the subtraction reconstructs
/// the pre-withdrawal balance exactly; nonce, code and storage are untouched.
#[derive(Clone)]
struct PreWithdrawalsDb {
    inner: StoreVmDatabase,
    credits: Arc<FxHashMap<Address, U256>>,
}

impl PreWithdrawalsDb {
    fn credits_of(withdrawals: &[Withdrawal]) -> FxHashMap<Address, U256> {
        let mut credits: FxHashMap<Address, U256> = FxHashMap::default();
        for withdrawal in withdrawals {
            if withdrawal.amount == 0 {
                continue;
            }
            let credit = U256::from(withdrawal.amount).saturating_mul(U256::from(GWEI_TO_WEI));
            let entry = credits.entry(withdrawal.address).or_default();
            *entry = entry.saturating_add(credit);
        }
        credits
    }
}

impl VmDatabase for PreWithdrawalsDb {
    fn get_account_state(&self, address: Address) -> Result<Option<AccountState>, EvmError> {
        let mut state = self.inner.get_account_state(address)?;
        if let (Some(state), Some(credit)) = (state.as_mut(), self.credits.get(&address)) {
            state.balance = state.balance.saturating_sub(*credit);
        }
        Ok(state)
    }

    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        self.inner.get_storage_slot(address, key)
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
        self.inner.get_block_hash(block_number)
    }

    fn get_chain_config(&self) -> Result<ChainConfig, EvmError> {
        self.inner.get_chain_config()
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, EvmError> {
        self.inner.get_account_code(code_hash)
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, EvmError> {
        self.inner.get_code_metadata(code_hash)
    }
}

impl Blockchain {
    /// The Profile 2 omission check over `il` for `block`, already imported.
    ///
    /// For every listed frame transaction absent from the block, in list order
    /// and once per transaction: excuse it unless it is a candidate with an
    /// admitted occurrence whose `max_gas` fits the block's remaining gas; then
    /// replay its validation prefix at `S_end` and, if not eligible there, at
    /// `S_start`. The first transaction eligible at either state makes the
    /// omission unjustified and ends the pass. A state whose verdict cannot be
    /// computed is recorded, not counted.
    ///
    /// `gas_fits` is judged once, against the end of the payload, for the same
    /// reason Profile 1 judges it there: gas remaining only decreases within a
    /// payload. The keyed nonces, the recent-root tuples and the signatures are
    /// judged before any frame executes, per state, so a transaction failing
    /// them costs no replay.
    pub fn check_profile2_omissions(
        &self,
        il: &[Transaction],
        block: &JudgedBlock<'_>,
        config: &ChainConfig,
        crypto: &dyn Crypto,
    ) -> Profile2Verdict {
        let header = block.header;
        let fork = config.fork(header.timestamp);
        let fill = budget_fill(il, fork, crypto);
        let gas_left = header.gas_limit.saturating_sub(header.gas_used);
        let slot_count = config.aa_vops_slot_count();
        let current_slot = config.effective_slot_number(header.slot_number, header.timestamp);
        let mut code_budget = CodeBudget::new(
            MAX_VALIDATION_CODE_BODIES,
            max_validation_code_bytes(config, header.timestamp),
        );
        let credits = Arc::new(PreWithdrawalsDb::credits_of(block.withdrawals));

        let mut verdict = Profile2Verdict::default();
        let mut judged: HashSet<H256> = HashSet::new();
        for tx in il {
            let tx_hash = tx.hash(crypto);
            // Identity is the envelope bytes; the hash is their digest. One verdict
            // per transaction however many times the list names it.
            if !judged.insert(tx_hash) || block.block_tx_hashes.contains(&tx_hash) {
                continue;
            }
            let Transaction::FrameTransaction(frame_tx) = tx else {
                continue;
            };
            let Ok(candidate) = profile2_candidate(frame_tx) else {
                continue;
            };
            if !fill.admitted.contains(&tx_hash) {
                continue;
            }
            // A frame transaction declaring another chain can never be included
            // in `B`, so its omission cannot be unjustified. Profile 1 states this
            // condition; the Profile 2 candidacy list does not, and it is applied
            // here for the same reason it holds there.
            if frame_tx.chain_id != config.chain_id {
                continue;
            }
            if frame_tx.max_gas() > gas_left {
                continue;
            }

            // S_end first: when the transaction is eligible there the result is the
            // same, and S_start need not be opened.
            let at_end = self.replay_profile2_at(
                tx,
                frame_tx,
                &candidate,
                block,
                header,
                Some(credits.clone()),
                current_slot,
                slot_count,
                &mut code_budget,
            );
            if at_end == Profile2Replay::Eligible {
                verdict.unjustified = Some(tx_hash);
                return verdict;
            }
            let at_start = self.replay_profile2_at(
                tx,
                frame_tx,
                &candidate,
                block,
                block.parent_header,
                None,
                current_slot,
                slot_count,
                &mut code_budget,
            );
            if at_start == Profile2Replay::Eligible {
                verdict.unjustified = Some(tx_hash);
                return verdict;
            }
            for (state, replay) in [("S_end", &at_end), ("S_start", &at_start)] {
                if let Profile2Replay::Undecided(reason) = replay {
                    warn!(
                        %tx_hash,
                        state,
                        reason,
                        "FOCIL Profile 2: could not decide eligibility; omission excused and recorded"
                    );
                    verdict
                        .undecided
                        .push((tx_hash, format!("{state}: {reason}")));
                }
            }
        }
        verdict
    }

    /// One replay of `frame_tx`'s validation prefix at the state rooted by
    /// `state_header`, under `block.header`'s context. `withdrawal_credits` is
    /// `Some` for `S_end`, whose committed root must have the block's withdrawal
    /// credits discounted.
    #[allow(clippy::too_many_arguments)]
    fn replay_profile2_at(
        &self,
        tx: &Transaction,
        frame_tx: &FrameTransaction,
        candidate: &Profile2Candidate,
        block: &JudgedBlock<'_>,
        state_header: &BlockHeader,
        withdrawal_credits: Option<Arc<FxHashMap<Address, U256>>>,
        current_slot: u64,
        slot_count: u64,
        code_budget: &mut CodeBudget,
    ) -> Profile2Replay {
        // Eligibility condition 3, before any EVM: the code at RECENT_ROOT_ADDRESS
        // is the canonical one and every tuple of a leading recent-root verifier
        // frame holds at this state at `B`'s slot. A storage read that fails is the
        // evaluator's failure, not the transaction's.
        if candidate.prefix.recent_root_index.is_some() {
            match self.check_recent_root_frame_at_root(
                frame_tx,
                current_slot,
                state_header.state_root,
            ) {
                Ok(()) => {}
                Err(MempoolError::StoreError(err)) => {
                    return Profile2Replay::Undecided(format!("recent-root read failed: {err}"));
                }
                Err(err) => return Profile2Replay::Ineligible(err.to_string()),
            }
        }

        // A state that cannot be opened is not evidence about the transaction.
        let store_db = match StoreVmDatabase::new(self.storage.clone(), state_header.clone()) {
            Ok(db) => db,
            Err(err) => {
                return Profile2Replay::Undecided(format!("state cannot be opened: {err}"));
            }
        };
        // Each replay gets its own database, so nothing it changes is observed by
        // or persisted for any other replay; only the code budget is shared.
        let mut evm = match withdrawal_credits {
            Some(credits) if !credits.is_empty() => Evm::new_for_l1(
                PreWithdrawalsDb {
                    inner: store_db,
                    credits,
                },
                Arc::new(NativeCrypto),
            ),
            _ => Evm::new_for_l1(store_db, Arc::new(NativeCrypto)),
        };
        match evm.replay_profile2_validation_prefix(
            tx,
            block.header,
            &candidate.prefix,
            candidate.payer,
            slot_count,
            code_budget,
        ) {
            Ok(replay) => replay,
            Err(err) => Profile2Replay::Undecided(err.to_string()),
        }
    }
}
