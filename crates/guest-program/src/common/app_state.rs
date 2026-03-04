use std::collections::BTreeMap;

use ethrex_common::types::AccountState;
use ethrex_common::{Address, H256, U256};

use super::app_types::{AccountProof, StorageProof};

/// Errors that can occur during app state operations.
#[derive(Debug, thiserror::Error)]
pub enum AppStateError {
    #[error("Account not found: {0:?}")]
    AccountNotFound(Address),
    #[error("Storage slot not found: {address:?} slot {slot:?}")]
    StorageSlotNotFound { address: Address, slot: H256 },
    #[error("Insufficient balance: {address:?} has {balance}, needs {required}")]
    InsufficientBalance {
        address: Address,
        balance: U256,
        required: U256,
    },
    #[error("Nonce mismatch: {address:?} expected {expected}, got {actual}")]
    NonceMismatch {
        address: Address,
        expected: u64,
        actual: u64,
    },
}

/// In-circuit state management based on storage proofs.
///
/// Instead of maintaining a full state trie (as the EVM does), `AppState`
/// tracks only the accounts and storage slots that are relevant to the
/// current batch of app operations. State is initialized from Merkle proofs
/// and updated in-place; the final state root is computed via incremental
/// MPT updates at the end.
pub struct AppState {
    /// Previous state root (verified against storage proofs).
    prev_state_root: H256,

    /// Account states (nonce, balance, storage_root, code_hash).
    accounts: BTreeMap<Address, AccountState>,

    /// Current storage values, keyed by (address, slot).
    storage: BTreeMap<(Address, H256), U256>,

    /// Tracks which accounts have been modified.
    dirty_accounts: BTreeMap<Address, bool>,

    /// Tracks which storage slots have been modified (address -> slots).
    dirty_storage: BTreeMap<Address, BTreeMap<H256, U256>>,

    /// Original account proofs (for state root recomputation).
    account_proofs: Vec<AccountProof>,

    /// Original storage proofs (for state root recomputation).
    storage_proofs: Vec<StorageProof>,
}

impl AppState {
    /// Create a new `AppState` from storage and account proofs.
    ///
    /// The proofs are verified separately (in `incremental_mpt`); this
    /// constructor only populates the in-memory state.
    pub fn from_proofs(
        prev_state_root: H256,
        account_proofs: Vec<AccountProof>,
        storage_proofs: Vec<StorageProof>,
    ) -> Self {
        let mut accounts = BTreeMap::new();
        let mut storage = BTreeMap::new();

        // Populate accounts from account proofs.
        for ap in &account_proofs {
            accounts.insert(
                ap.address,
                AccountState {
                    nonce: ap.nonce,
                    balance: ap.balance,
                    storage_root: ap.storage_root,
                    code_hash: ap.code_hash,
                },
            );
        }

        // Populate storage values and ensure accounts exist.
        for sp in &storage_proofs {
            storage.insert((sp.address, sp.slot), sp.value);
        }

        Self {
            prev_state_root,
            accounts,
            storage,
            dirty_accounts: BTreeMap::new(),
            dirty_storage: BTreeMap::new(),
            account_proofs,
            storage_proofs,
        }
    }

    /// Get the previous state root.
    pub fn prev_state_root(&self) -> H256 {
        self.prev_state_root
    }

    // ── Account operations ────────────────────────────────────────

    /// Get account state. Returns error if account is not in the proof set.
    pub fn get_account(&self, address: Address) -> Result<&AccountState, AppStateError> {
        self.accounts
            .get(&address)
            .ok_or(AppStateError::AccountNotFound(address))
    }

    /// Get account balance.
    pub fn get_balance(&self, address: Address) -> Result<U256, AppStateError> {
        Ok(self.get_account(address)?.balance)
    }

    /// Set account balance (marks account as dirty).
    pub fn set_balance(&mut self, address: Address, balance: U256) -> Result<(), AppStateError> {
        let account = self
            .accounts
            .get_mut(&address)
            .ok_or(AppStateError::AccountNotFound(address))?;
        account.balance = balance;
        self.dirty_accounts.insert(address, true);
        Ok(())
    }

    /// Add to account balance (e.g., deposits).
    pub fn credit_balance(&mut self, address: Address, amount: U256) -> Result<(), AppStateError> {
        let balance = self.get_balance(address)?;
        self.set_balance(address, balance + amount)
    }

    /// Subtract from account balance with insufficient-balance check.
    pub fn debit_balance(&mut self, address: Address, amount: U256) -> Result<(), AppStateError> {
        let balance = self.get_balance(address)?;
        if balance < amount {
            return Err(AppStateError::InsufficientBalance {
                address,
                balance,
                required: amount,
            });
        }
        self.set_balance(address, balance - amount)
    }

    /// Get account nonce.
    pub fn get_nonce(&self, address: Address) -> Result<u64, AppStateError> {
        Ok(self.get_account(address)?.nonce)
    }

    /// Verify nonce matches expected value and increment.
    pub fn verify_and_increment_nonce(
        &mut self,
        address: Address,
        expected_nonce: u64,
    ) -> Result<(), AppStateError> {
        let account = self
            .accounts
            .get_mut(&address)
            .ok_or(AppStateError::AccountNotFound(address))?;
        if account.nonce != expected_nonce {
            return Err(AppStateError::NonceMismatch {
                address,
                expected: expected_nonce,
                actual: account.nonce,
            });
        }
        account.nonce += 1;
        self.dirty_accounts.insert(address, true);
        Ok(())
    }

    // ── ETH transfer ──────────────────────────────────────────────

    /// Transfer ETH between two accounts.
    pub fn transfer_eth(
        &mut self,
        from: Address,
        to: Address,
        value: U256,
    ) -> Result<(), AppStateError> {
        if value.is_zero() {
            return Ok(());
        }
        self.debit_balance(from, value)?;
        self.credit_balance(to, value)?;
        Ok(())
    }

    // ── Storage operations ────────────────────────────────────────

    /// Get a storage slot value. Returns error if slot is not in the proof set.
    pub fn get_storage(&self, address: Address, slot: H256) -> Result<U256, AppStateError> {
        self.storage
            .get(&(address, slot))
            .copied()
            .ok_or(AppStateError::StorageSlotNotFound { address, slot })
    }

    /// Set a storage slot value (marks slot as dirty).
    pub fn set_storage(
        &mut self,
        address: Address,
        slot: H256,
        value: U256,
    ) -> Result<(), AppStateError> {
        self.storage.insert((address, slot), value);
        self.dirty_storage
            .entry(address)
            .or_default()
            .insert(slot, value);
        self.dirty_accounts.insert(address, true);
        Ok(())
    }

    // ── State root computation ────────────────────────────────────

    /// Get all modified accounts and their new states.
    pub fn dirty_accounts(&self) -> impl Iterator<Item = (&Address, &AccountState)> {
        self.dirty_accounts
            .keys()
            .filter_map(move |addr| self.accounts.get(addr).map(|state| (addr, state)))
    }

    /// Get all modified storage slots grouped by account.
    pub fn dirty_storage(&self) -> &BTreeMap<Address, BTreeMap<H256, U256>> {
        &self.dirty_storage
    }

    /// Get the original account proofs (for incremental MPT update).
    pub fn account_proofs(&self) -> &[AccountProof] {
        &self.account_proofs
    }

    /// Get the original storage proofs (for incremental MPT update).
    pub fn storage_proofs(&self) -> &[StorageProof] {
        &self.storage_proofs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::H160;

    fn test_address(byte: u8) -> Address {
        H160([byte; 20])
    }

    fn test_account(nonce: u64, balance: u64) -> AccountState {
        AccountState {
            nonce,
            balance: U256::from(balance),
            storage_root: H256::zero(),
            code_hash: H256::zero(),
        }
    }

    fn make_state(accounts: Vec<(Address, AccountState)>) -> AppState {
        let account_proofs: Vec<AccountProof> = accounts
            .into_iter()
            .map(|(addr, state)| AccountProof {
                address: addr,
                nonce: state.nonce,
                balance: state.balance,
                storage_root: state.storage_root,
                code_hash: state.code_hash,
                proof: vec![],
            })
            .collect();
        AppState::from_proofs(H256::zero(), account_proofs, vec![])
    }

    #[test]
    fn eth_transfer_updates_balances() {
        let alice = test_address(1);
        let bob = test_address(2);
        let mut state = make_state(vec![
            (alice, test_account(0, 1000)),
            (bob, test_account(0, 500)),
        ]);

        state
            .transfer_eth(alice, bob, U256::from(300))
            .expect("transfer should succeed");

        assert_eq!(state.get_balance(alice).unwrap(), U256::from(700));
        assert_eq!(state.get_balance(bob).unwrap(), U256::from(800));
    }

    #[test]
    fn insufficient_balance_rejected() {
        let alice = test_address(1);
        let bob = test_address(2);
        let mut state = make_state(vec![
            (alice, test_account(0, 100)),
            (bob, test_account(0, 0)),
        ]);

        let result = state.transfer_eth(alice, bob, U256::from(200));
        assert!(matches!(
            result,
            Err(AppStateError::InsufficientBalance { .. })
        ));
    }

    #[test]
    fn nonce_verification_and_increment() {
        let alice = test_address(1);
        let mut state = make_state(vec![(alice, test_account(5, 1000))]);

        // Wrong nonce should fail.
        assert!(state.verify_and_increment_nonce(alice, 3).is_err());

        // Correct nonce should succeed and increment.
        state
            .verify_and_increment_nonce(alice, 5)
            .expect("correct nonce");
        assert_eq!(state.get_nonce(alice).unwrap(), 6);
    }

    #[test]
    fn storage_read_write() {
        let contract = test_address(0xAA);
        let slot = H256::from_low_u64_be(1);
        let storage_proofs = vec![StorageProof {
            address: contract,
            slot,
            value: U256::from(42),
            account_proof: vec![],
            storage_proof: vec![],
        }];
        let acct = test_account(0, 0);
        let account_proofs = vec![AccountProof {
            address: contract,
            nonce: acct.nonce,
            balance: acct.balance,
            storage_root: acct.storage_root,
            code_hash: acct.code_hash,
            proof: vec![],
        }];

        let mut state = AppState::from_proofs(H256::zero(), account_proofs, storage_proofs);

        assert_eq!(state.get_storage(contract, slot).unwrap(), U256::from(42));

        state
            .set_storage(contract, slot, U256::from(99))
            .expect("set storage");
        assert_eq!(state.get_storage(contract, slot).unwrap(), U256::from(99));

        // Dirty tracking.
        assert!(state.dirty_storage().contains_key(&contract));
    }

    #[test]
    fn zero_value_transfer_is_noop() {
        let alice = test_address(1);
        let bob = test_address(2);
        let mut state = make_state(vec![
            (alice, test_account(0, 100)),
            (bob, test_account(0, 0)),
        ]);

        state
            .transfer_eth(alice, bob, U256::zero())
            .expect("zero transfer");
        assert_eq!(state.get_balance(alice).unwrap(), U256::from(100));
        assert_eq!(state.get_balance(bob).unwrap(), U256::zero());
    }
}
