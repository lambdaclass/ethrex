use ethrex_common::types::{AccountInfo, AccountUpdate, ChainConfig};
use ethrex_common::{Address as CoreAddress, BigEndianHash, H256, U256};
use ethrex_vm::{DynVmDatabase, EvmError, VmDatabase};
use revm::db::states::bundle_state::BundleRetention;
use revm::db::AccountStatus;
use revm::primitives::{
    AccountInfo as RevmAccountInfo, Address as RevmAddress, Bytecode as RevmBytecode, Bytes as RevmBytes, B256 as RevmB256, U256 as RevmU256
};

/// State used when running the EVM. The state can be represented with a [VmDbWrapper] database
///
/// Encapsulates state behaviour to be agnostic to the evm implementation for crate users.
pub struct EvmState {
    pub inner: revm::db::State<RevmDynVmDatabase>,
}

#[derive(Clone)]
pub struct RevmDynVmDatabase(DynVmDatabase);

// Needed because revm::db::State is not cloneable and we need to
// restore the previous EVM state after executing a transaction in L2 mode whose resulting state diff doesn't fit in a blob.
impl Clone for EvmState {
    fn clone(&self) -> Self {
        let inner = revm::db::State::<RevmDynVmDatabase> {
            cache: self.inner.cache.clone(),
            database: self.inner.database.clone(),
            transition_state: self.inner.transition_state.clone(),
            bundle_state: self.inner.bundle_state.clone(),
            use_preloaded_bundle: self.inner.use_preloaded_bundle,
            block_hashes: self.inner.block_hashes.clone(),
        };

        Self { inner }
    }
}

impl EvmState {
    /// Gets the stored chain config
    pub fn chain_config(&self) -> Result<ChainConfig, EvmError> {
        self.inner.database.0.get_chain_config()
    }
}

/// Builds EvmState from a Store
pub fn evm_state(db: DynVmDatabase) -> EvmState {
    let inner = revm::db::State::builder()
        .with_database(RevmDynVmDatabase(db))
        .with_bundle_update()
        .without_state_clear()
        .build();
    EvmState { inner }
}

impl revm::Database for RevmDynVmDatabase {
    type Error = EvmError;

    fn basic(&mut self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let acc_info = match <dyn VmDatabase>::get_account_info(
            self.0.as_ref(),
            CoreAddress::from(address.0.as_ref()),
        )? {
            None => return Ok(None),
            Some(acc_info) => acc_info,
        };
        let code = self.code_by_hash(acc_info.code_hash.0.into())?;

        Ok(Some(RevmAccountInfo {
            balance: RevmU256::from_limbs(acc_info.balance.0),
            nonce: acc_info.nonce,
            code_hash: RevmB256::from(acc_info.code_hash.0),
            code: Some(code),
        }))
    }

    fn code_by_hash(&mut self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        let code =
            <dyn VmDatabase>::get_account_code(self.0.as_ref(), H256::from(code_hash.as_ref()))?;
        Ok(RevmBytecode::new_raw(RevmBytes(code)))
    }

    fn storage(&mut self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        Ok(<dyn VmDatabase>::get_storage_slot(
            self.0.as_ref(),
            CoreAddress::from(address.0.as_ref()),
            H256::from(index.to_be_bytes()),
        )?
        .map(|value| RevmU256::from_limbs(value.0))
        .unwrap_or_else(|| RevmU256::ZERO))
    }

    fn block_hash(&mut self, number: u64) -> Result<RevmB256, Self::Error> {
        <dyn VmDatabase>::get_block_hash(self.0.as_ref(), number)
            .map(|hash| RevmB256::from_slice(&hash.0))
    }
}

impl revm::DatabaseRef for RevmDynVmDatabase {
    type Error = EvmError;

    fn basic_ref(&self, address: RevmAddress) -> Result<Option<RevmAccountInfo>, Self::Error> {
        let acc_info = match <dyn VmDatabase>::get_account_info(
            self.0.as_ref(),
            CoreAddress::from(address.0.as_ref()),
        )? {
            None => return Ok(None),
            Some(acc_info) => acc_info,
        };
        let code = self.code_by_hash_ref(acc_info.code_hash.0.into())?;

        Ok(Some(RevmAccountInfo {
            balance: RevmU256::from_limbs(acc_info.balance.0),
            nonce: acc_info.nonce,
            code_hash: RevmB256::from(acc_info.code_hash.0),
            code: Some(code),
        }))
    }

    fn code_by_hash_ref(&self, code_hash: RevmB256) -> Result<RevmBytecode, Self::Error> {
        let code =
            <dyn VmDatabase>::get_account_code(self.0.as_ref(), H256::from(code_hash.as_ref()))?;
        Ok(RevmBytecode::new_raw(RevmBytes(code)))
    }

    fn storage_ref(&self, address: RevmAddress, index: RevmU256) -> Result<RevmU256, Self::Error> {
        Ok(<dyn VmDatabase>::get_storage_slot(
            self.0.as_ref(),
            CoreAddress::from(address.0.as_ref()),
            H256::from(index.to_be_bytes()),
        )?
        .map(|value| RevmU256::from_limbs(value.0))
        .unwrap_or_else(|| RevmU256::ZERO))
    }

    fn block_hash_ref(&self, number: u64) -> Result<RevmB256, Self::Error> {
        <dyn VmDatabase>::get_block_hash(self.0.as_ref(), number)
            .map(|hash| RevmB256::from_slice(&hash.0))
    }
}

impl EvmState {
     /// Gets the state_transitions == [AccountUpdate] from the [EvmState].
     pub fn get_state_transitions(&mut self,
    ) -> Vec<ethrex_common::types::AccountUpdate> {
        let initial_state = &mut self.inner;
        initial_state.merge_transitions(BundleRetention::PlainState);
        let bundle = initial_state.take_bundle();

        // Update accounts
        let mut account_updates = Vec::new();
        for (address, account) in bundle.state() {
            if account.status.is_not_modified() {
                continue;
            }
            let address = CoreAddress::from_slice(address.0.as_slice());
            // Remove account from DB if destroyed (Process DestroyedChanged as changed account)
            if matches!(
                account.status,
                AccountStatus::Destroyed | AccountStatus::DestroyedAgain
            ) {
                account_updates.push(AccountUpdate::removed(address));
                continue;
            }

            // If account is empty, do not add to the database
            if account
                .account_info()
                .is_some_and(|acc_info| acc_info.is_empty())
            {
                continue;
            }

            // Edge case: Account was destroyed and created again afterwards with CREATE2.
            if matches!(account.status, AccountStatus::DestroyedChanged) {
                // Push to account updates the removal of the account and then push the new state of the account.
                // This is for clearing the account's storage when it was selfdestructed in the first place.
                account_updates.push(AccountUpdate::removed(address));
                // This will always be Some though, because it is DestroyedChanged
                if let Some(new_acc_info) = account.account_info() {
                    let new_acc_update = AccountUpdate {
                        address,
                        removed: false,
                        info: Some(AccountInfo {
                            code_hash: H256::from_slice(new_acc_info.code_hash.as_slice()),
                            balance: U256::from_little_endian(new_acc_info.balance.as_le_slice()),
                            nonce: new_acc_info.nonce,
                        }),
                        code: new_acc_info.code.map(|c| c.original_bytes().0),
                        added_storage: account
                            .storage
                            .iter()
                            .map(|(key, slot)| {
                                (
                                    H256::from_uint(&U256::from_little_endian(key.as_le_slice())),
                                    U256::from_little_endian(slot.present_value().as_le_slice()),
                                )
                            })
                            .collect(),
                    };
                    account_updates.push(new_acc_update);
                }
                continue;
            }
            // Apply account changes to DB
            let mut account_update = AccountUpdate::new(address);
            // If the account was changed then both original and current info will be present in the bundle account
            if account.is_info_changed() {
                if let Some(new_acc_info) = account.account_info() {
                    // Update account info in DB
                    let code_hash = H256::from_slice(new_acc_info.code_hash.as_slice());
                    let account_info = AccountInfo {
                        code_hash,
                        balance: U256::from_little_endian(new_acc_info.balance.as_le_slice()),
                        nonce: new_acc_info.nonce,
                    };
                    account_update.info = Some(account_info);
                    // Update code in db
                    if account.is_contract_changed() {
                        if let Some(code) = new_acc_info.code {
                            account_update.code = Some(code.original_bytes().0);
                        }
                    }
                }
            }
            // Update account storage in DB
            for (key, slot) in account.storage.iter() {
                if slot.is_changed() {
                    // TODO check if we need to remove the value from our db when value is zero
                    // if slot.present_value().is_zero() {
                    //     account_update.removed_keys.push(H256::from_uint(&U256::from_little_endian(key.as_le_slice())))
                    // }
                    account_update.added_storage.insert(
                        H256::from_uint(&U256::from_little_endian(key.as_le_slice())),
                        U256::from_little_endian(slot.present_value().as_le_slice()),
                    );
                }
            }
            account_updates.push(account_update)
        }
        account_updates
    }
}
