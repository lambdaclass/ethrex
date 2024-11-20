use ethrex_core::{types::BlockHash, Address as CoreAddress, H256 as CoreH256};
use ethrex_storage::{error::StoreError, Store};
use revm::primitives::{
    AccountInfo as RevmAccountInfo, Address as RevmAddress, Bytecode as RevmBytecode,
    Bytes as RevmBytes, B256 as RevmB256, U256 as RevmU256,
};

pub struct StoreWrapper {
    pub store: Store,
    pub block_hash: BlockHash,
}

cfg_if::cfg_if! {
    if #[cfg(feature = "levm")] {
        use ethrex_core::{U256 as CoreU256};
        use ethrex_levm::db::Database as LevmDatabase;

        impl LevmDatabase for StoreWrapper {
            fn get_account_info(&self, address: CoreAddress) -> ethrex_levm::account::AccountInfo {
                let acc_info = self
                    .store
                    .get_account_info_by_hash(self.block_hash, address)
                    .unwrap()
                    .unwrap_or_default();

                let acc_code = self
                    .store
                    .get_account_code(acc_info.code_hash)
                    .unwrap()
                    .unwrap_or_default();

                ethrex_levm::account::AccountInfo {
                    balance: acc_info.balance,
                    nonce: acc_info.nonce,
                    bytecode: acc_code,
                }
            }

            fn get_storage_slot(&self, address: CoreAddress, key: CoreH256) -> CoreU256 {
                self.store
                    .get_storage_at_hash(self.block_hash, address, key)
                    .unwrap()
                    .unwrap_or_default()
            }

            fn get_block_hash(&self, block_number: u64) -> Option<CoreH256> {
                let a = self.store.get_block_header(block_number).unwrap();

                a.map(|a| CoreH256::from(a.compute_block_hash().0))
            }
        }
    }
}

impl revm::Database for StoreWrapper {
    type Error = StoreError;

    fn basic(&mut self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let acc_info = match self
            .store
            .get_account_info_by_hash(self.block_hash, CoreAddress::from(address.0.as_ref()))?
        {
            None => return Ok(None),
            Some(acc_info) => acc_info,
        };
        let code = self
            .store
            .get_account_code(acc_info.code_hash)?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)));

        Ok(Some(RevmAccountInfo {
            balance: RevmU256::from_limbs(acc_info.balance.0),
            nonce: acc_info.nonce,
            code_hash: RevmB256::from(acc_info.code_hash.0),
            code,
        }))
    }

    fn code_by_hash(&mut self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        self.store
            .get_account_code(CoreH256::from(code_hash.as_ref()))?
            .map(|b| RevmBytecode::new_raw(RevmBytes(b)))
            .ok_or_else(|| StoreError::Custom(format!("No code for hash {code_hash}")))
    }

    fn storage(&mut self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        Ok(self
            .store
            .get_storage_at_hash(
                self.block_hash,
                CoreAddress::from(address.0.as_ref()),
                CoreH256::from(index.to_be_bytes()),
            )?
            .map(|value| RevmU256::from_limbs(value.0))
            .unwrap_or_else(|| RevmU256::ZERO))
    }

    fn block_hash(&mut self, number: u64) -> Result<RevmB256, Self::Error> {
        self.store
            .get_block_header(number)?
            .map(|header| RevmB256::from_slice(&header.compute_block_hash().0))
            .ok_or_else(|| StoreError::Custom(format!("Block {number} not found")))
    }
}
