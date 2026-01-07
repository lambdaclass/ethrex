use std::collections::BTreeMap;

use bytes::Bytes;
use ethrex_common::types::block_access_list::{
    AccountChanges as BALAccountChanges, BalanceChange, BlockAccessList, CodeChange, NonceChange,
    SlotChange, StorageChange,
};
use ethrex_common::{Address, BigEndianHash, H256, U256};

use super::Tracer;

#[derive(Debug, Default)]
struct AccountChanges {
    state_reads: BTreeMap<usize, H256>,
    state_changes: BTreeMap<H256, BTreeMap<usize, U256>>,
    balance_changes: BTreeMap<usize, U256>,
    nonce_changes: BTreeMap<usize, u64>,
    code_changes: BTreeMap<usize, (H256, Bytes)>,
}

#[derive(Debug, Default)]
pub struct BlockAccessListTracer {
    account_changes: BTreeMap<Address, AccountChanges>,
    idx: usize,
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
                storage_reads: state_reads.iter().map(|s| s.1.into_uint()).collect(),
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
                        new_code: c.1.1.clone(),
                    })
                    .collect(),
            });
        }
        BlockAccessList::new(res)
    }
}

impl Tracer for BlockAccessListTracer {
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
        let v = self.account_changes.entry(address).or_default();
        v.state_reads.insert(self.idx, slot);
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
