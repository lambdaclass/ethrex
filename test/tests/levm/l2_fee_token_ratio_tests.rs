//! Regression test for audit Finding Q: Fee-Token Ratio Fetched in Both
//! Prepare and Finalize → Inconsistent Lock vs. Settlement.
//!
//! The bug: `get_fee_token_ratio` is called once in `prepare_execution_fee_token`
//! and again in `finalize_non_privileged_execution`. Both calls clone `vm.db` at
//! the time of the call. If the transaction's execution modifies the ratio
//! contract's storage between prepare and finalize, the two phases use different
//! ratios — causing over-locking, under-refunding, or accounting errors.
//!
//! The fix: cache the ratio from prepare and pass it through to finalize.

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    constants::EMPTY_TRIE_HASH,
    types::{
        Account, AccountState, ChainConfig, Code, CodeMetadata, EIP1559Transaction, Fork,
        Transaction, TxKind, fee_config::FeeConfig,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    db::{Database, gen_db::GeneralizedDatabase},
    environment::{EVMConfig, Environment},
    hooks::l2_hook::{
        COMMON_BRIDGE_L2_ADDRESS, FEE_TOKEN_RATIO_ADDRESS, FEE_TOKEN_REGISTRY_ADDRESS,
    },
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use rustc_hash::FxHashMap;
use std::sync::Arc;

// ==================== Test Database ====================

struct TestDatabase {
    accounts: FxHashMap<Address, Account>,
}

impl TestDatabase {
    fn new() -> Self {
        Self {
            accounts: FxHashMap::default(),
        }
    }
}

impl Database for TestDatabase {
    fn get_account_state(
        &self,
        address: Address,
    ) -> Result<AccountState, ethrex_levm::errors::DatabaseError> {
        Ok(self
            .accounts
            .get(&address)
            .map(|acc| AccountState {
                nonce: acc.info.nonce,
                balance: acc.info.balance,
                storage_root: *EMPTY_TRIE_HASH,
                code_hash: acc.info.code_hash,
            })
            .unwrap_or_default())
    }

    fn get_storage_value(
        &self,
        address: Address,
        key: H256,
    ) -> Result<U256, ethrex_levm::errors::DatabaseError> {
        Ok(self
            .accounts
            .get(&address)
            .and_then(|acc| acc.storage.get(&key).copied())
            .unwrap_or_default())
    }

    fn get_block_hash(
        &self,
        _block_number: u64,
    ) -> Result<H256, ethrex_levm::errors::DatabaseError> {
        Ok(H256::zero())
    }

    fn get_chain_config(&self) -> Result<ChainConfig, ethrex_levm::errors::DatabaseError> {
        Ok(ChainConfig::default())
    }

    fn get_account_code(
        &self,
        code_hash: H256,
    ) -> Result<Code, ethrex_levm::errors::DatabaseError> {
        for acc in self.accounts.values() {
            if acc.info.code_hash == code_hash {
                return Ok(acc.code.clone());
            }
        }
        Ok(Code::default())
    }

    fn get_code_metadata(
        &self,
        code_hash: H256,
    ) -> Result<CodeMetadata, ethrex_levm::errors::DatabaseError> {
        for acc in self.accounts.values() {
            if acc.info.code_hash == code_hash {
                return Ok(CodeMetadata {
                    length: acc.code.bytecode.len() as u64,
                });
            }
        }
        Ok(CodeMetadata { length: 0 })
    }
}

// ==================== Helpers ====================

fn eoa(balance: U256) -> Account {
    Account::new(balance, Code::default(), 0, FxHashMap::default())
}

fn contract_with_storage(code: Bytes, storage: FxHashMap<H256, U256>) -> Account {
    Account::new(
        U256::zero(),
        Code::from_bytecode(code, &NativeCrypto),
        0,
        storage,
    )
}

fn contract(code: Bytes) -> Account {
    Account::new(
        U256::zero(),
        Code::from_bytecode(code, &NativeCrypto),
        0,
        FxHashMap::default(),
    )
}

/// Registry contract: always returns `true` (1) as a 32-byte word.
/// Used for `isFeeToken(address)` — unconditionally returns true.
///
/// Bytecode: PUSH1 1, PUSH1 0, MSTORE, PUSH1 32, PUSH1 0, RETURN
fn return_true_bytecode() -> Bytes {
    Bytes::from_static(&[0x60, 0x01, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3])
}

/// Ratio contract: reads storage slot 0 and returns it, OR writes to slot 0
/// if the selector is not `getFeeTokenRatio` (0xc6ab85d8).
///
/// ```text
/// // Load selector from calldata
/// PUSH1 0x00        // 60 00
/// CALLDATALOAD      // 35
/// PUSH1 0xe0        // 60 e0
/// SHR               // 1c
///
/// // Compare with getFeeTokenRatio selector
/// PUSH4 0xc6ab85d8  // 63 c6ab85d8
/// EQ                // 14
/// PUSH1 0x16        // 60 16  (jump to getter at offset 0x16)
/// JUMPI             // 57
///
/// // Setter path: CALLDATALOAD(4) → SSTORE(0)
/// PUSH1 0x04        // 60 04
/// CALLDATALOAD      // 35
/// PUSH1 0x00        // 60 00
/// SSTORE            // 55
/// STOP              // 00
///
/// // Getter path: SLOAD(0) → MSTORE → RETURN
/// JUMPDEST          // 5b
/// PUSH1 0x00        // 60 00
/// SLOAD             // 54
/// PUSH1 0x00        // 60 00
/// MSTORE            // 52
/// PUSH1 0x20        // 60 20
/// PUSH1 0x00        // 60 00
/// RETURN            // f3
/// ```
fn ratio_contract_bytecode() -> Bytes {
    Bytes::from_static(&[
        0x60, 0x00, 0x35, 0x60, 0xe0, 0x1c, // load selector
        0x63, 0xc6, 0xab, 0x85, 0xd8, 0x14, // push getFeeTokenRatio selector, EQ
        0x60, 0x16, 0x57, // PUSH1 0x16, JUMPI (to getter)
        // Setter path
        0x60, 0x04, 0x35, // PUSH1 4, CALLDATALOAD
        0x60, 0x00, 0x55, // PUSH1 0, SSTORE
        0x00, // STOP
        // Getter path (offset 0x16 = 22)
        0x5b, // JUMPDEST
        0x60, 0x00, 0x54, // PUSH1 0, SLOAD
        0x60, 0x00, 0x52, // PUSH1 0, MSTORE
        0x60, 0x20, 0x60, 0x00, 0xf3, // PUSH1 32, PUSH1 0, RETURN
    ])
}

/// Fee token contract: a no-op that always succeeds.
/// `lockFee` and `payFee` calls succeed without meaningful state changes.
///
/// Bytecode: STOP
fn fee_token_bytecode() -> Bytes {
    Bytes::from_static(&[0x00])
}

/// "Attacker" contract deployed at the `to` address. When called, it invokes
/// the ratio contract with a setter to change storage slot 0 to U256::MAX.
///
/// This simulates a transaction whose execution changes the fee-token ratio
/// between the prepare and finalize phases.
///
/// ```text
/// // Store U256::MAX at memory[4..36] (setter payload: any non-getFeeTokenRatio selector + value)
/// PUSH32 0xFFFF...FF   // 7f ff...ff
/// PUSH1 0x04           // 60 04
/// MSTORE               // 52
///
/// // memory[0..4] = 0x00000000 (default, serves as non-matching selector)
/// // memory[4..36] = U256::MAX
///
/// // CALL(gas=0xffff, to=RATIO_ADDR, value=0, argsOff=0, argsLen=36, retOff=0, retLen=0)
/// PUSH1 0x00           // retSize
/// PUSH1 0x00           // retOffset
/// PUSH1 0x24           // argsSize (36 = 4 selector + 32 value)
/// PUSH1 0x00           // argsOffset
/// PUSH1 0x00           // value
/// PUSH20 <RATIO_ADDR>  // address
/// PUSH2 0xffff         // gas
/// CALL
/// STOP
/// ```
fn ratio_modifier_bytecode() -> Bytes {
    let mut code = Vec::new();

    // PUSH32 U256::MAX (all 0xff bytes)
    code.push(0x7f); // PUSH32
    code.extend_from_slice(&[0xff; 32]);

    // PUSH1 0x04, MSTORE — store at memory[4..36]
    code.extend_from_slice(&[0x60, 0x04, 0x52]);

    // CALL args (pushed in reverse order for stack)
    code.extend_from_slice(&[0x60, 0x00]); // retSize = 0
    code.extend_from_slice(&[0x60, 0x00]); // retOffset = 0
    code.extend_from_slice(&[0x60, 0x24]); // argsSize = 36
    code.extend_from_slice(&[0x60, 0x00]); // argsOffset = 0
    code.extend_from_slice(&[0x60, 0x00]); // value = 0

    // PUSH20 FEE_TOKEN_RATIO_ADDRESS (0x000...fffb)
    code.push(0x73); // PUSH20
    code.extend_from_slice(&FEE_TOKEN_RATIO_ADDRESS.0);

    code.extend_from_slice(&[0x61, 0xff, 0xff]); // PUSH2 gas = 0xffff
    code.push(0xf1); // CALL
    code.push(0x00); // STOP

    Bytes::from(code)
}

// ==================== Test ====================

/// Regression test: if the tx's execution modifies the ratio contract's storage
/// such that `get_fee_token_ratio` returns a value > u64::MAX, the finalize
/// phase fails with "Failed to convert fee token ratio" because it re-fetches
/// the now-corrupted ratio instead of using the cached value from prepare.
///
/// After the fix (caching the ratio), finalize uses the original ratio=2 and
/// the transaction completes successfully.
#[test]
fn fee_token_ratio_cached_between_prepare_and_finalize() {
    let sender = Address::from_low_u64_be(0x1000);
    let to_addr = Address::from_low_u64_be(0x2000);
    let fee_token_addr = Address::from_low_u64_be(0x3000);
    let coinbase = Address::from_low_u64_be(0xCCC);

    // Initial ratio = 2, stored at slot 0 of the ratio contract
    let mut ratio_storage = FxHashMap::default();
    ratio_storage.insert(H256::zero(), U256::from(2));

    let mut db = TestDatabase::new();

    // Sender with enough ETH balance for gas and enough to pass checks
    db.accounts
        .insert(sender, eoa(U256::from(10_000_000_000u64)));

    // Coinbase
    db.accounts.insert(coinbase, eoa(U256::zero()));

    // Fee token registry: always returns true
    db.accounts
        .insert(FEE_TOKEN_REGISTRY_ADDRESS, contract(return_true_bytecode()));

    // Fee token ratio contract: returns slot 0 value (initially 2), can be modified
    db.accounts.insert(
        FEE_TOKEN_RATIO_ADDRESS,
        contract_with_storage(ratio_contract_bytecode(), ratio_storage),
    );

    // Fee token contract: no-op (always succeeds)
    db.accounts
        .insert(fee_token_addr, contract(fee_token_bytecode()));

    // The "to" contract: modifies ratio contract slot 0 to U256::MAX during execution
    db.accounts
        .insert(to_addr, contract(ratio_modifier_bytecode()));

    // Common bridge needs an account (used as origin in simulate_common_bridge_call)
    db.accounts
        .insert(COMMON_BRIDGE_L2_ADDRESS, eoa(U256::from(10_000_000_000u64)));

    let gas_limit: u64 = 100_000;
    let gas_price: u64 = 1000;

    let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: gas_price,
        max_fee_per_gas: gas_price,
        gas_limit,
        to: TxKind::Call(to_addr),
        value: U256::zero(),
        data: Bytes::new(),
        ..Default::default()
    });

    let fee_config = FeeConfig {
        base_fee_vault: None,
        operator_fee_config: None,
        l1_fee_config: None,
    };

    let env = Environment {
        origin: sender,
        gas_limit,
        config: EVMConfig::new(Fork::Prague, EVMConfig::canonical_values(Fork::Prague)),
        coinbase,
        chain_id: U256::from(1),
        base_fee_per_gas: U256::from(gas_price),
        gas_price: U256::from(gas_price),
        tx_max_fee_per_gas: Some(U256::from(gas_price)),
        tx_max_priority_fee_per_gas: Some(U256::from(gas_price)),
        block_gas_limit: 30_000_000,
        fee_token: Some(fee_token_addr),
        ..Default::default()
    };

    let mut db = GeneralizedDatabase::new(Arc::new(db));

    let mut vm = VM::new(
        env,
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L2(fee_config),
        &NativeCrypto,
    )
    .expect("VM creation should succeed");

    // Execute the transaction. The `to` contract modifies the ratio contract's
    // storage to U256::MAX during execution. Without the fix, finalize re-fetches
    // the ratio, gets U256::MAX, and fails on the u64 conversion.
    // With the fix, finalize uses the cached ratio=2 from prepare and succeeds.
    let result = vm.execute();

    assert!(
        result.is_ok(),
        "Expected Ok: finalize should use cached ratio from prepare, not re-fetch. Got: {result:?}"
    );
}
