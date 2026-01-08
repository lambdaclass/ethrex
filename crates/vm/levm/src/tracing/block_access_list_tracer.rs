use std::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;
use ethrex_common::types::block_access_list::{
    AccountChanges as BALAccountChanges, BalanceChange, BlockAccessList, CodeChange, NonceChange,
    SlotChange, StorageChange,
};
use ethrex_common::{Address, BigEndianHash, H256, U256};

use super::Tracer;

#[derive(Debug, Default)]
struct AccountChanges {
    state_reads: BTreeSet<H256>,
    state_changes: BTreeMap<H256, BTreeMap<u64, U256>>,
    balance_changes: BTreeMap<u64, U256>,
    nonce_changes: BTreeMap<u64, u64>,
    code_changes: BTreeMap<u64, Bytes>,
}

#[derive(Debug, Default)]
pub struct BlockAccessListTracer {
    account_changes: BTreeMap<Address, AccountChanges>,
    idx: u64,
}

impl BlockAccessListTracer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_result(&self) -> BlockAccessList {
        let mut res = vec![];
        for (addr, acc) in self.account_changes.iter() {
            let AccountChanges {
                state_reads,
                state_changes,
                balance_changes,
                code_changes,
                nonce_changes,
            } = acc;
            res.push(BALAccountChanges {
                address: *addr,
                balance_changes: balance_changes
                    .iter()
                    .map(|b| BalanceChange {
                        block_access_index: *b.0,
                        post_balance: *b.1,
                    })
                    .collect(),
                storage_changes: state_changes
                    .iter()
                    .map(|s| SlotChange {
                        slot: s.0.into_uint(),
                        slot_changes: s
                            .1
                            .iter()
                            .map(|c| StorageChange {
                                block_access_index: *c.0,
                                post_value: *c.1,
                            })
                            .collect(),
                    })
                    .collect(),
                storage_reads: state_reads.iter().map(|s| s.into_uint()).collect(),
                nonce_changes: nonce_changes
                    .iter()
                    .map(|n| NonceChange {
                        block_access_index: *n.0,
                        post_nonce: *n.1,
                    })
                    .collect(),
                code_changes: code_changes
                    .iter()
                    .map(|c| CodeChange {
                        block_access_index: *c.0,
                        new_code: c.1.clone(),
                    })
                    .collect(),
            });
        }
        BlockAccessList::new(res)
    }
}

impl Tracer for BlockAccessListTracer {
    fn enter(
        &mut self,
        _call_type: ethrex_common::tracing::CallType,
        _from: Address,
        _to: Address,
        _value: U256,
        _gas: u64,
        _input: &Bytes,
    ) {
    }

    fn exit(
        &mut self,
        _depth: usize,
        _gas_used: u64,
        _output: Bytes,
        _error: Option<String>,
        _revert_reason: Option<String>,
    ) -> Result<(), crate::errors::InternalError> {
        Ok(())
    }

    fn log(
        &mut self,
        _log: &ethrex_common::types::Log,
    ) -> Result<(), crate::errors::InternalError> {
        Ok(())
    }

    fn txn_start(
        &mut self,
        _env: &crate::Environment,
        _tx: &ethrex_common::types::Transaction,
        _from: Address,
        _db: &mut crate::db::gen_db::GeneralizedDatabase,
    ) {
        if self.idx == 0 {
            self.idx = self.idx.saturating_add(1);
        }
    }

    fn txn_end(
        &mut self,
        _gas_used: u64,
        err: Option<String>,
        _db: &mut crate::db::gen_db::GeneralizedDatabase,
    ) {
        self.idx = self.idx.saturating_add(1);
    }

    fn on_opcode(
        &mut self,
        _opcode: crate::opcodes::Opcode,
        _current_address: Address,
        _stack: &[U256],
        _db: &mut crate::db::gen_db::GeneralizedDatabase,
    ) -> bool {
        true
    }

    fn on_storage_access(
        &mut self,
        address: Address,
        slot: H256,
        db: &mut crate::db::gen_db::GeneralizedDatabase,
    ) {
        let a = self.account_changes.entry(address).or_default();
        if !a.state_changes.contains_key(&slot) {
            a.state_reads.insert(slot);
        }
    }

    fn on_storage_change(
        &mut self,
        address: Address,
        slot: H256,
        prev: U256,
        new: U256,
        db: &mut crate::db::gen_db::GeneralizedDatabase,
    ) {
        let v = self.account_changes.entry(address).or_default();
        let state_changes = v.state_changes.entry(slot).or_default();
        state_changes.insert(self.idx, new);
        // A slot can read first then written but it can only exist in state read or write not
        // both.
        v.state_reads.remove(&slot);
    }

    fn on_balance_change(
        &mut self,
        address: Address,
        prev: U256,
        new: U256,
        _db: &mut crate::db::gen_db::GeneralizedDatabase,
    ) {
        let v = self.account_changes.entry(address).or_default();
        v.balance_changes.insert(self.idx, new);
    }

    fn on_nonce_change(
        &mut self,
        address: Address,
        prev: u64,
        new: u64,
        _db: &mut crate::db::gen_db::GeneralizedDatabase,
    ) {
        let v = self.account_changes.entry(address).or_default();
        v.nonce_changes.insert(self.idx, new);
    }

    fn on_code_change(
        &mut self,
        _address: Address,
        prev: ethrex_common::types::Code,
        new: ethrex_common::types::Code,
        _db: &mut crate::db::gen_db::GeneralizedDatabase,
    ) {
        let v = self.account_changes.entry(_address).or_default();
        v.code_changes.insert(self.idx, new.bytecode);
    }

    fn on_account_access(
        &mut self,
        address: Address,
        _db: &mut crate::db::gen_db::GeneralizedDatabase,
    ) {
        self.account_changes.entry(address).or_default();
    }

    fn on_create(&mut self, _address: Address, _db: &mut crate::db::gen_db::GeneralizedDatabase) {}

    fn on_selfdestruct(
        &mut self,
        _address: Address,
        _db: &mut crate::db::gen_db::GeneralizedDatabase,
    ) {
    }
}
