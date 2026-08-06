//! Concrete [`IlProfile2Evaluator`] for EIP-8369 Profile 2 (FOCIL AA-VOPS)
//! inclusion-list omissions, backed by a real [`Blockchain`]/[`Store`].
//!
//! [`BlockchainProfile2Evaluator`] is deliberately the only thing in this
//! crate that both implements [`IlProfile2Evaluator`] and depends on
//! `ethrex-vm`: [`crate::inclusion_list_validator`] stays VM-free, and this
//! module supplies the one implementation that actually replays a
//! transaction's validation prefix.

use ethrex_common::types::{BlockHeader, FrameMode, FrameTransaction, Transaction};
use ethrex_vm::FocilVopsSurface;

use crate::{
    Blockchain,
    focil_eligibility::{MAX_VERIFY_GAS_PER_TX, profile_2_payer},
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
    gas_left: u64,
}

impl<'a> BlockchainProfile2Evaluator<'a> {
    pub fn new(blockchain: &'a Blockchain, header: &'a BlockHeader, gas_left: u64) -> Self {
        Self {
            blockchain,
            header,
            gas_left,
        }
    }
}

impl<'a> IlProfile2Evaluator for BlockchainProfile2Evaluator<'a> {
    fn evaluate(&self, tx: &FrameTransaction) -> Profile2Eligibility {
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

        if tx.total_gas_limit() > self.gas_left {
            return Profile2Eligibility::Ineligible(format!(
                "total gas limit {} exceeds the block's remaining gas {}",
                tx.total_gas_limit(),
                self.gas_left
            ));
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
            self.header.state_root,
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

        let vm_db = match StoreVmDatabase::new(self.blockchain.storage.clone(), self.header.clone())
        {
            Ok(vm_db) => vm_db,
            Err(err) => {
                return Profile2Eligibility::Undecided(format!(
                    "failed to open state at header {}: {err}",
                    self.header.number
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
        match vm.simulate_frame_validation_prefix(
            &transaction,
            self.header,
            &prefix,
            None,
            MAX_VERIFY_GAS_PER_TX,
            Some(surface),
        ) {
            Err(err) => Profile2Eligibility::Undecided(format!("simulation error: {err}")),
            Ok(outcome) if !outcome.passed => Profile2Eligibility::Ineligible(
                outcome
                    .violation
                    .unwrap_or_else(|| "validation prefix did not pass".to_string()),
            ),
            Ok(_) => Profile2Eligibility::Eligible,
        }
    }
}
