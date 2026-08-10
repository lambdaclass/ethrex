//! Concrete [`IlProfile2Evaluator`] for EIP-8369 Profile 2 (FOCIL AA-VOPS)
//! inclusion-list omissions, backed by a real [`Blockchain`]/[`Store`].
//!
//! [`BlockchainProfile2Evaluator`] is deliberately the only thing in this
//! crate that both implements [`IlProfile2Evaluator`] and depends on
//! `ethrex-vm`: [`crate::inclusion_list_validator`] stays VM-free, and this
//! module supplies the one implementation that actually replays a
//! transaction's validation prefix.

use std::cell::RefCell;

use ethrex_common::H256;
use ethrex_common::types::{BlockHeader, FrameMode, FrameTransaction, Transaction};
use ethrex_vm::{CodeBodyBudget, FocilVopsSurface, Profile2Replay};

use crate::{
    Blockchain,
    focil_eligibility::{
        MAX_VALIDATION_CODE_BODIES, MAX_VERIFY_GAS_PER_TX, max_validation_code_bytes,
        profile_2_payer,
    },
    inclusion_list_validator::{IlProfile2Evaluator, Profile2Eligibility},
    vm::StoreVmDatabase,
};

/// Decides EIP-8369 Profile 2 eligibility for one omitted inclusion-list
/// frame transaction, by replaying its validation prefix against `header`'s
/// post-execution state.
///
/// `header` MUST be the block the inclusion list is being judged against, and
/// `gas_left` its `gas_limit - gas_used`. Every read this evaluator performs
/// goes through `header.state_root` explicitly (never a canonical block
/// number), which is what makes it safe to use before `header` is canonical —
/// see [`Blockchain::check_recent_root_references_at_root`].
pub struct BlockchainProfile2Evaluator<'a> {
    blockchain: &'a Blockchain,
    header: &'a BlockHeader,
    pre_state_root: H256,
    gas_left: u64,
    /// Code bodies this inclusion list's replays may still load. One evaluator
    /// judges one list, so the ledger lives here and is carried across every
    /// candidate and both evaluation states; `evaluate` takes `&self`, hence the
    /// cell. A body already charged is free to load again, which is what makes
    /// the shared-verifier shape affordable.
    code_budget: RefCell<CodeBodyBudget>,
}

impl<'a> BlockchainProfile2Evaluator<'a> {
    /// `header` is the block being judged, `pre_state_root` the state root the
    /// block executes from, and `gas_left` its `gas_limit - gas_used`.
    pub fn new(
        blockchain: &'a Blockchain,
        header: &'a BlockHeader,
        pre_state_root: H256,
        gas_left: u64,
    ) -> Self {
        Self {
            blockchain,
            header,
            pre_state_root,
            gas_left,
            code_budget: RefCell::new(CodeBodyBudget::new(
                MAX_VALIDATION_CODE_BODIES,
                max_validation_code_bytes(),
            )),
        }
    }
}

/// Union of the verdicts at the two evaluation states: an omission is
/// unjustified when the transaction was includable at *either* endpoint.
///
/// `Eligible` at either state wins outright. Otherwise an `Undecided` dominates,
/// because a state that could not be evaluated is not evidence that the
/// transaction was uneligible there — treating it as such would excuse an
/// omission on the strength of a local failure.
fn union(a: Profile2Eligibility, b: Profile2Eligibility) -> Profile2Eligibility {
    use Profile2Eligibility::{Eligible, Ineligible, Undecided};
    match (a, b) {
        (Eligible, _) | (_, Eligible) => Eligible,
        (Undecided(why), _) | (_, Undecided(why)) => Undecided(why),
        (Ineligible(at_end), Ineligible(at_start)) => Ineligible(format!(
            "ineligible at both endpoints: end-of-payload: {at_end}; start-of-payload: {at_start}"
        )),
    }
}

impl<'a> IlProfile2Evaluator for BlockchainProfile2Evaluator<'a> {
    /// An omission is unjustified when the candidate was includable at the state
    /// the payload started from OR the state it ended at.
    ///
    /// Judging at a single point admits a payload in which the transaction fails
    /// at that point. The complete rule is the union over every index, which is
    /// unaffordable; these two are the endpoints the evaluator already holds, so
    /// the extra cost is one replay and no state reconstruction. The union closes
    /// two holes an end-of-payload-only rule leaves open: a payer solvent early
    /// and drained by a later transaction of the same block, and a queued keyed
    /// nonce that only becomes valid late in the payload.
    ///
    /// `gas_fits` is deliberately NOT part of this union and is evaluated once,
    /// against the end of the payload. Gas remaining decreases monotonically
    /// within a block, so evaluating it at the start would mark every full block
    /// unsatisfied and turn the inclusion list into a hard claim on block space.
    fn evaluate(&self, tx: &FrameTransaction) -> Profile2Eligibility {
        if tx.total_gas_limit() > self.gas_left {
            return Profile2Eligibility::Ineligible(format!(
                "total gas limit {} exceeds the block's remaining gas {}",
                tx.total_gas_limit(),
                self.gas_left
            ));
        }

        let at_end = self.evaluate_at(tx, self.header.state_root);
        if matches!(at_end, Profile2Eligibility::Eligible) {
            return at_end;
        }
        union(at_end, self.evaluate_at(tx, self.pre_state_root))
    }
}

impl<'a> BlockchainProfile2Evaluator<'a> {
    /// Replay the validation prefix against one evaluation state.
    ///
    /// `state_root` selects which state to read: the judged block's own root for
    /// the end of the payload, the root it executed from for the start.
    ///
    /// The block *context* is the judged block's in both cases — base fee,
    /// timestamp, gas limit, chain id and the EIP-7843 slot all come from
    /// `self.header`, because each endpoint asks whether the transaction could
    /// have been included in *this* block, not in its parent. Only the state
    /// differs, which is why this takes a root rather than a header.
    fn evaluate_at(&self, tx: &FrameTransaction, state_root: H256) -> Profile2Eligibility {
        let state_header = BlockHeader {
            state_root,
            ..self.header.clone()
        };
        let state_header = &state_header;
        // EIP-8369 does not model EIP-8312 at all, and a UTXO frame executes
        // AFTER the validation prefix and can invalidate it (a spent input, an
        // unproven opening), which the prefix-only replay below never
        // observes. Replaying just the prefix would therefore risk reporting
        // a transaction includable that isn't.
        if tx
            .frames
            .iter()
            .any(|frame| frame.mode == FrameMode::Utxo as u8)
        {
            return Profile2Eligibility::Undecided(
                "frame transaction carries an EIP-8312 UTXO frame, which EIP-8369 does not model"
                    .to_string(),
            );
        }

        let config = self.blockchain.storage.get_chain_config();
        let current_slot =
            config.effective_slot_number(self.header.slot_number, self.header.timestamp);
        // The block's OWN slot: a frame tx executing inside this block sees
        // this slot as `env.slot_number`, not the slot after it (that is the
        // prospective admission question `check_recent_root_references`
        // asks; this one asks whether the reference is valid AT this block).
        if let Err(err) = self.blockchain.check_recent_root_references_at_root(
            tx,
            current_slot,
            state_header.state_root,
        ) {
            return Profile2Eligibility::Ineligible(format!(
                "recent-root reference invalid: {err}"
            ));
        }

        let Some(payer) = profile_2_payer(tx) else {
            return Profile2Eligibility::Ineligible(
                "no recognized validation-prefix shape to resolve a payer".to_string(),
            );
        };

        let prefix = match tx.validation_prefix() {
            Ok(prefix) => prefix,
            Err(err) => {
                return Profile2Eligibility::Undecided(format!(
                    "failed to derive validation prefix: {err}"
                ));
            }
        };

        let vm_db =
            match StoreVmDatabase::new(self.blockchain.storage.clone(), state_header.clone()) {
                Ok(vm_db) => vm_db,
                Err(err) => {
                    return Profile2Eligibility::Undecided(format!(
                        "failed to open state at header {}: {err}",
                        state_header.number
                    ));
                }
            };
        let mut vm = match self.blockchain.new_evm(vm_db) {
            Ok(vm) => vm,
            Err(err) => {
                return Profile2Eligibility::Undecided(format!("failed to construct EVM: {err}"));
            }
        };

        let surface = FocilVopsSurface {
            payer,
            slot_count: config.aa_vops_slot_count(),
        };
        let transaction = Transaction::FrameTransaction(tx.clone());
        let outcome = vm.simulate_frame_validation_prefix(
            &transaction,
            self.header,
            &prefix,
            None,
            MAX_VERIFY_GAS_PER_TX,
            Some(Profile2Replay {
                surface,
                code_budget: self.code_budget.borrow().clone(),
            }),
        );

        match outcome {
            Err(err) => Profile2Eligibility::Undecided(format!("simulation error: {err}")),
            Ok(outcome) => {
                // Charges survive the verdict. A replay that loaded bodies and
                // then failed still made every attester read them, and refunding
                // it would let a list retry the same reads for free.
                if let Some(spent) = outcome.code_budget {
                    *self.code_budget.borrow_mut() = spent;
                }
                if outcome.passed {
                    Profile2Eligibility::Eligible
                } else {
                    Profile2Eligibility::Ineligible(
                        outcome
                            .violation
                            .unwrap_or_else(|| "validation prefix did not pass".to_string()),
                    )
                }
            }
        }
    }
}
