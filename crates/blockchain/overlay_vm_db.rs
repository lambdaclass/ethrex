use std::collections::HashMap;

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_KECCACK_HASH,
    types::{AccountState, AccountUpdate, ChainConfig, Code, CodeMetadata},
};
use ethrex_vm::{EvmError, VmDatabase};

use crate::vm::StoreVmDatabase;

/// An in-memory overlay on top of `StoreVmDatabase` that intercepts state
/// queries with overrides and accumulated state transitions.
///
/// Used by `eth_simulateV1` to apply per-block state overrides and propagate
/// state changes across multiple simulated blocks.
#[derive(Clone)]
pub struct OverlayVmDatabase {
    inner: StoreVmDatabase,
    account_overrides: HashMap<Address, AccountOverrideState>,
    code_overrides: HashMap<H256, Code>,
    block_hash_overrides: HashMap<u64, H256>,
}

#[derive(Clone, Default)]
struct AccountOverrideState {
    /// True when the account was destroyed via SELFDESTRUCT / EIP-161 clearing.
    /// `get_account_state` returns `None` for deleted accounts.
    deleted: bool,
    balance: Option<U256>,
    nonce: Option<u64>,
    code_hash: Option<H256>,
    /// Full storage replacement (from `state` override).
    full_storage: Option<HashMap<H256, U256>>,
    /// Partial storage diffs (from `stateDiff` or accumulated `AccountUpdate`s).
    storage_diff: HashMap<H256, U256>,
}

impl OverlayVmDatabase {
    pub fn new(inner: StoreVmDatabase) -> Self {
        Self {
            inner,
            account_overrides: HashMap::new(),
            code_overrides: HashMap::new(),
            block_hash_overrides: HashMap::new(),
        }
    }

    /// Override an account's balance.
    pub fn set_balance(&mut self, address: Address, balance: U256) {
        self.account_overrides
            .entry(address)
            .or_default()
            .balance = Some(balance);
    }

    /// Override an account's nonce.
    pub fn set_nonce(&mut self, address: Address, nonce: u64) {
        self.account_overrides
            .entry(address)
            .or_default()
            .nonce = Some(nonce);
    }

    /// Override an account's code.
    pub fn set_code(&mut self, address: Address, bytecode: Bytes) {
        let code = Code::from_bytecode(bytecode);
        self.account_overrides
            .entry(address)
            .or_default()
            .code_hash = Some(code.hash);
        self.code_overrides.insert(code.hash, code);
    }

    /// Full storage replacement for an account.
    pub fn set_full_storage(&mut self, address: Address, storage: HashMap<H256, U256>) {
        let entry = self.account_overrides.entry(address).or_default();
        entry.full_storage = Some(storage);
        entry.storage_diff.clear();
    }

    /// Partial storage diff for an account.
    pub fn set_storage_diff(&mut self, address: Address, diff: HashMap<H256, U256>) {
        let entry = self.account_overrides.entry(address).or_default();
        for (k, v) in diff {
            entry.storage_diff.insert(k, v);
        }
    }

    /// Merge `AccountUpdate`s from a simulated block's execution into the overlay.
    pub fn merge_account_updates(&mut self, updates: &[AccountUpdate]) {
        for update in updates {
            let entry = self
                .account_overrides
                .entry(update.address)
                .or_default();

            if update.removed {
                entry.deleted = true;
                entry.balance = Some(U256::zero());
                entry.nonce = Some(0);
                entry.code_hash = Some(*EMPTY_KECCACK_HASH);
                entry.full_storage = Some(HashMap::new());
                entry.storage_diff.clear();
            }

            if let Some(info) = &update.info {
                // Account has state after this update, so it's not deleted
                // (even if it was deleted earlier in the same update, e.g. SELFDESTRUCT + CREATE).
                entry.deleted = false;
                entry.balance = Some(info.balance);
                entry.nonce = Some(info.nonce);
                entry.code_hash = Some(info.code_hash);
            }

            if let Some(code) = &update.code {
                self.code_overrides.insert(code.hash, code.clone());
            }

            if update.removed_storage {
                entry.full_storage = Some(HashMap::new());
                entry.storage_diff.clear();
            }

            for (key, value) in &update.added_storage {
                if let Some(full) = &mut entry.full_storage {
                    full.insert(*key, *value);
                } else {
                    entry.storage_diff.insert(*key, *value);
                }
            }
        }
    }

    /// Register a simulated block hash for BLOCKHASH opcode resolution.
    pub fn set_block_hash(&mut self, number: u64, hash: H256) {
        self.block_hash_overrides.insert(number, hash);
    }
}

impl VmDatabase for OverlayVmDatabase {
    fn get_account_state(&self, address: Address) -> Result<Option<AccountState>, EvmError> {
        let Some(overrides) = self.account_overrides.get(&address) else {
            return self.inner.get_account_state(address);
        };

        // Account was destroyed and not recreated.
        if overrides.deleted {
            return Ok(None);
        }

        // Start from the real account or a blank one.
        let mut state = self
            .inner
            .get_account_state(address)?
            .unwrap_or_default();

        if let Some(balance) = overrides.balance {
            state.balance = balance;
        }
        if let Some(nonce) = overrides.nonce {
            state.nonce = nonce;
        }
        if let Some(code_hash) = overrides.code_hash {
            state.code_hash = code_hash;
        }

        Ok(Some(state))
    }

    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        if let Some(overrides) = self.account_overrides.get(&address) {
            // Full storage replacement: only look here.
            if let Some(full) = &overrides.full_storage {
                return Ok(Some(*full.get(&key).unwrap_or(&U256::zero())));
            }
            // Partial diff: check diff first, then fall through.
            if let Some(value) = overrides.storage_diff.get(&key) {
                return Ok(Some(*value));
            }
        }
        self.inner.get_storage_slot(address, key)
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
        if let Some(hash) = self.block_hash_overrides.get(&block_number) {
            return Ok(*hash);
        }
        self.inner.get_block_hash(block_number)
    }

    fn get_chain_config(&self) -> Result<ChainConfig, EvmError> {
        self.inner.get_chain_config()
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, EvmError> {
        if let Some(code) = self.code_overrides.get(&code_hash) {
            return Ok(code.clone());
        }
        self.inner.get_account_code(code_hash)
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, EvmError> {
        if let Some(code) = self.code_overrides.get(&code_hash) {
            return Ok(CodeMetadata {
                length: code.bytecode.len() as u64,
            });
        }
        self.inner.get_code_metadata(code_hash)
    }
}
