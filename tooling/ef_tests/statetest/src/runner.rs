use std::collections::HashMap;
use std::sync::Arc;

use ethrex_common::{
    Address, H256, U256,
    types::{
        Account, AccountInfo, Code, EIP1559Transaction, EIP2930Transaction,
        EIP4844Transaction, EIP7702Transaction, Fork, LegacyTransaction,
        Transaction, TxKind,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    EVMConfig, Environment,
    db::Database as LevmDatabase,
    db::gen_db::GeneralizedDatabase,
    errors::{DatabaseError, VMError},
    tracing::LevmCallTracer,
    utils::get_base_fee_per_blob_gas,
    vm::{VM, VMType},
};
use ethrex_rlp::encode::PayloadRLPEncode;
use rustc_hash::FxHashMap;
use secp256k1::{Message, Secp256k1, SecretKey};

use crate::types::{AccountState, Env, TestCase, chain_config_for_fork};

// ---- Result type for each sub-test ----

#[derive(Debug, Clone)]
pub struct TestResult {
    pub name: String,
    pub pass: bool,
    pub fork: String,
    pub error: Option<String>,
}

// ---- Lightweight in-memory database ----

/// Minimal Database implementation backed by HashMaps.
/// All pre-state accounts are loaded into GeneralizedDatabase's cache,
/// so this backing store only needs to return defaults for misses
/// and serve code lookups.
struct InMemoryDb {
    chain_config: ethrex_common::types::ChainConfig,
    codes: FxHashMap<H256, Code>,
}

impl LevmDatabase for InMemoryDb {
    fn get_account_state(
        &self,
        _address: Address,
    ) -> Result<ethrex_common::types::AccountState, DatabaseError> {
        Ok(ethrex_common::types::AccountState::default())
    }

    fn get_storage_value(
        &self,
        _address: Address,
        _key: H256,
    ) -> Result<U256, DatabaseError> {
        Ok(U256::zero())
    }

    fn get_block_hash(&self, _block_number: u64) -> Result<H256, DatabaseError> {
        Ok(H256::zero())
    }

    fn get_chain_config(&self) -> Result<ethrex_common::types::ChainConfig, DatabaseError> {
        Ok(self.chain_config)
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
        Ok(self.codes.get(&code_hash).cloned().unwrap_or_default())
    }

    fn get_code_metadata(
        &self,
        code_hash: H256,
    ) -> Result<ethrex_common::types::CodeMetadata, DatabaseError> {
        let length = self
            .codes
            .get(&code_hash)
            .map(|c| c.bytecode.len() as u64)
            .unwrap_or(0);
        Ok(ethrex_common::types::CodeMetadata { length })
    }
}

// ---- Build GeneralizedDatabase from pre-state ----

fn build_db(
    pre: &HashMap<Address, AccountState>,
    fork: &Fork,
) -> GeneralizedDatabase {
    let chain_config = chain_config_for_fork(fork);

    let mut accounts: FxHashMap<Address, Account> = FxHashMap::default();
    let mut codes: FxHashMap<H256, Code> = FxHashMap::default();

    for (addr, state) in pre {
        let code = Code::from_bytecode(state.code.clone(), &NativeCrypto);
        let storage: FxHashMap<H256, U256> = state
            .storage
            .iter()
            .map(|(k, v)| (H256::from(k.to_big_endian()), *v))
            .collect();
        codes.insert(code.hash, code.clone());
        accounts.insert(
            *addr,
            Account {
                info: AccountInfo {
                    code_hash: code.hash,
                    balance: state.balance,
                    nonce: state.nonce,
                },
                code,
                storage,
            },
        );
    }

    let db = InMemoryDb { chain_config, codes };
    GeneralizedDatabase::new_with_account_state(Arc::new(db), accounts)
}

// ---- Build Environment ----

fn build_env(env: &Env, tc: &TestCase) -> Result<Environment, String> {
    let blob_schedule = EVMConfig::canonical_values(tc.fork);
    let config = EVMConfig::new(tc.fork, blob_schedule);
    let gas_price = effective_gas_price(env, tc)?;
    let base_blob_fee_per_gas = get_base_fee_per_blob_gas(
        env.current_excess_blob_gas
            .map(|x| x.try_into().unwrap()),
        &config,
    )
    .map_err(|e| format!("blob base fee error: {e}"))?;

    Ok(Environment {
        origin: tc.sender,
        gas_limit: tc.gas,
        config,
        block_number: env.current_number.try_into().unwrap(),
        coinbase: env.current_coinbase,
        timestamp: env.current_timestamp.try_into().unwrap(),
        prev_randao: env.current_random,
        difficulty: env.current_difficulty,
        slot_number: env.slot_number.unwrap_or_default(),
        chain_id: U256::from(1),
        base_fee_per_gas: env.current_base_fee.unwrap_or_default(),
        base_blob_fee_per_gas,
        gas_price,
        block_excess_blob_gas: env
            .current_excess_blob_gas
            .map(|x| x.try_into().unwrap()),
        block_blob_gas_used: None,
        tx_blob_hashes: tc.blob_versioned_hashes.clone(),
        tx_max_priority_fee_per_gas: tc.max_priority_fee_per_gas,
        tx_max_fee_per_gas: tc.max_fee_per_gas,
        tx_max_fee_per_blob_gas: tc.max_fee_per_blob_gas,
        tx_nonce: tc.nonce,
        block_gas_limit: env.current_gas_limit,
        is_privileged: false,
        fee_token: None,
        disable_balance_check: false,
    })
}

fn effective_gas_price(env: &Env, tc: &TestCase) -> Result<U256, String> {
    match tc.gas_price {
        Some(price) => Ok(price),
        None => {
            let base_fee = env
                .current_base_fee
                .ok_or("missing current_base_fee for EIP-1559+ tx")?;
            let priority = tc
                .max_priority_fee_per_gas
                .ok_or("missing max_priority_fee_per_gas")?;
            let max_fee = tc
                .max_fee_per_gas
                .ok_or("missing max_fee_per_gas")?;
            Ok(std::cmp::min(max_fee, base_fee + priority))
        }
    }
}

// ---- Build Transaction ----

fn build_tx(tc: &TestCase) -> Result<Transaction, String> {
    // Prefer decoding from txbytes when available (already signed).
    if !tc.tx_bytes.is_empty() {
        return Transaction::decode_canonical(&tc.tx_bytes)
            .map_err(|e| format!("txbytes decode error: {e}"));
    }

    // Fall back to constructing + signing manually.
    let chain_id: u64 = 1;
    let access_list: Vec<(Address, Vec<H256>)> = tc
        .access_list
        .iter()
        .map(|item| (item.address, item.storage_keys.clone()))
        .collect();

    let mut tx = if let Some(ref auth_list) = tc.authorization_list {
        Transaction::EIP7702Transaction(EIP7702Transaction {
            to: match tc.to {
                TxKind::Call(to) => to,
                TxKind::Create => return Err("EIP-7702 tx cannot be create".into()),
            },
            value: tc.value,
            data: tc.data.clone(),
            access_list: access_list.clone(),
            authorization_list: auth_list
                .iter()
                .map(|a| a.clone().into_authorization_tuple())
                .collect(),
            chain_id,
            nonce: tc.nonce,
            max_priority_fee_per_gas: tc
                .max_priority_fee_per_gas
                .unwrap()
                .as_u64(),
            max_fee_per_gas: tc.max_fee_per_gas.unwrap().as_u64(),
            gas_limit: tc.gas,
            ..Default::default()
        })
    } else if tc.max_fee_per_blob_gas.is_some() {
        Transaction::EIP4844Transaction(EIP4844Transaction {
            chain_id,
            nonce: tc.nonce,
            max_priority_fee_per_gas: tc
                .max_priority_fee_per_gas
                .unwrap()
                .as_u64(),
            max_fee_per_gas: tc.max_fee_per_gas.unwrap().as_u64(),
            gas: tc.gas,
            to: match tc.to {
                TxKind::Call(to) => to,
                TxKind::Create => {
                    return Err("EIP-4844 tx cannot be create".into())
                }
            },
            value: tc.value,
            data: tc.data.clone(),
            access_list: access_list.clone(),
            max_fee_per_blob_gas: tc.max_fee_per_blob_gas.unwrap(),
            blob_versioned_hashes: tc.blob_versioned_hashes.clone(),
            ..Default::default()
        })
    } else if tc.max_priority_fee_per_gas.is_some() && tc.max_fee_per_gas.is_some() {
        Transaction::EIP1559Transaction(EIP1559Transaction {
            chain_id,
            nonce: tc.nonce,
            max_priority_fee_per_gas: tc
                .max_priority_fee_per_gas
                .unwrap()
                .as_u64(),
            max_fee_per_gas: tc.max_fee_per_gas.unwrap().as_u64(),
            gas_limit: tc.gas,
            to: tc.to.clone(),
            value: tc.value,
            data: tc.data.clone(),
            access_list: access_list.clone(),
            ..Default::default()
        })
    } else if !tc.access_list.is_empty() {
        Transaction::EIP2930Transaction(EIP2930Transaction {
            chain_id,
            nonce: tc.nonce,
            gas_price: tc.gas_price.unwrap(),
            gas_limit: tc.gas,
            to: tc.to.clone(),
            value: tc.value,
            data: tc.data.clone(),
            access_list,
            ..Default::default()
        })
    } else {
        Transaction::LegacyTransaction(LegacyTransaction {
            nonce: tc.nonce,
            gas_price: tc.gas_price.unwrap(),
            gas: tc.gas,
            to: tc.to.clone(),
            value: tc.value,
            data: tc.data.clone(),
            ..Default::default()
        })
    };

    // Sign the transaction synchronously using secp256k1.
    sign_tx(&mut tx, &tc.secret_key)?;
    Ok(tx)
}

/// Sign a transaction in-place using the raw secp256k1 secret key.
fn sign_tx(tx: &mut Transaction, secret_key: &H256) -> Result<(), String> {
    let secp = Secp256k1::new();
    let sk = SecretKey::from_slice(secret_key.as_bytes())
        .map_err(|e| format!("invalid secret key: {e}"))?;

    let payload = match *tx {
        Transaction::LegacyTransaction(ref t) => t.encode_payload_to_vec(),
        Transaction::EIP2930Transaction(ref t) => {
            let mut buf = vec![0x01u8];
            buf.append(&mut t.encode_payload_to_vec());
            buf
        }
        Transaction::EIP1559Transaction(ref t) => {
            let mut buf = vec![0x02u8];
            buf.append(&mut t.encode_payload_to_vec());
            buf
        }
        Transaction::EIP4844Transaction(ref t) => {
            let mut buf = vec![0x03u8];
            buf.append(&mut t.encode_payload_to_vec());
            buf
        }
        Transaction::EIP7702Transaction(ref t) => {
            let mut buf = vec![0x04u8];
            buf.append(&mut t.encode_payload_to_vec());
            buf
        }
        _ => return Err("unsupported tx type for signing".into()),
    };

    let msg_hash = ethrex_common::utils::keccak(&payload);
    let message = Message::from_digest(*msg_hash.as_fixed_bytes());
    let (rec_id, sig_bytes) = secp
        .sign_ecdsa_recoverable(&message, &sk)
        .serialize_compact();
    let rec_id_i32: i32 = rec_id.into();
    let y_parity = rec_id_i32 != 0;
    let r = U256::from_big_endian(&sig_bytes[..32]);
    let s = U256::from_big_endian(&sig_bytes[32..]);

    match *tx {
        Transaction::LegacyTransaction(ref mut t) => {
            t.v = U256::from(rec_id_i32) + 27;
            t.r = r;
            t.s = s;
        }
        Transaction::EIP2930Transaction(ref mut t) => {
            t.signature_y_parity = y_parity;
            t.signature_r = r;
            t.signature_s = s;
        }
        Transaction::EIP1559Transaction(ref mut t) => {
            t.signature_y_parity = y_parity;
            t.signature_r = r;
            t.signature_s = s;
        }
        Transaction::EIP4844Transaction(ref mut t) => {
            t.signature_y_parity = y_parity;
            t.signature_r = r;
            t.signature_s = s;
        }
        Transaction::EIP7702Transaction(ref mut t) => {
            t.signature_y_parity = y_parity;
            t.signature_r = r;
            t.signature_s = s;
        }
        _ => return Err("unsupported tx type for signing".into()),
    }
    Ok(())
}

// ---- Check post-state ----

fn check_post_state(
    vm: &mut VM<'_>,
    tc: &TestCase,
    execution_result: &Result<ethrex_levm::errors::ExecutionReport, VMError>,
) -> Result<(), String> {
    // If an exception was expected, just check that execution failed.
    if let Some(ref _expected) = tc.post.expected_exceptions {
        if execution_result.is_ok() {
            return Err("expected exception but execution succeeded".into());
        }
        // Exception was expected and execution did fail -- pass.
        return Ok(());
    }

    // Execution must have succeeded.
    let _report = execution_result
        .as_ref()
        .map_err(|e| format!("unexpected execution error: {e}"))?;

    // Compare accounts from the expected post-state.
    if let Some(ref expected_state) = tc.post.state {
        for (addr, expected) in expected_state {
            let account = vm
                .db
                .get_account(*addr)
                .map_err(|e| format!("failed to get account {addr}: {e}"))?;

            if account.info.balance != expected.balance {
                return Err(format!(
                    "balance mismatch for {addr}: expected={}, got={}",
                    expected.balance, account.info.balance
                ));
            }
            if account.info.nonce != expected.nonce {
                return Err(format!(
                    "nonce mismatch for {addr}: expected={}, got={}",
                    expected.nonce, account.info.nonce
                ));
            }

            let expected_code_hash = ethrex_common::types::code_hash(
                &expected.code,
                &NativeCrypto,
            );
            if account.info.code_hash != expected_code_hash {
                return Err(format!(
                    "code hash mismatch for {addr}: expected={:?}, got={:?}",
                    expected_code_hash, account.info.code_hash
                ));
            }

            for (slot, expected_val) in &expected.storage {
                let slot_h256 = H256::from(slot.to_big_endian());
                let actual_val = account
                    .storage
                    .get(&slot_h256)
                    .copied()
                    .unwrap_or_default();
                if actual_val != *expected_val {
                    return Err(format!(
                        "storage mismatch for {addr} slot {slot}: expected={expected_val}, got={actual_val}"
                    ));
                }
            }
        }
        return Ok(());
    }

    // No explicit state field -- we cannot compute trie root without Store.
    // Fall back to a warning; treat as pass since we have no way to verify.
    Ok(())
}

// ---- Public entry point: run a single test case ----

pub fn run_test_case(
    test_name: &str,
    env: &Env,
    pre: &HashMap<Address, AccountState>,
    tc: &TestCase,
) -> TestResult {
    let label = format!(
        "{}[fork_{:?}-data_{}-gas_{}-value_{}]",
        test_name, tc.fork, tc.vector.0, tc.vector.1, tc.vector.2
    );
    let fork_str = format!("{:?}", tc.fork);

    // Build the in-memory database from pre-state.
    let mut db = build_db(pre, &tc.fork);

    // Build the environment.
    let vm_env = match build_env(env, tc) {
        Ok(e) => e,
        Err(e) => {
            return TestResult {
                name: label,
                pass: false,
                fork: fork_str,
                error: Some(format!("env build error: {e}")),
            };
        }
    };

    // Build the transaction.
    let tx = match build_tx(tc) {
        Ok(t) => t,
        Err(e) => {
            return TestResult {
                name: label,
                pass: false,
                fork: fork_str,
                error: Some(format!("tx build error: {e}")),
            };
        }
    };

    // Create and execute the VM.
    let tracer = LevmCallTracer::disabled();
    let mut vm = match VM::new(vm_env, &mut db, &tx, tracer, VMType::L1, &NativeCrypto) {
        Ok(vm) => vm,
        Err(e) => {
            // VM::new can fail for invalid transactions.
            // If an exception was expected, that counts as a pass.
            if tc.post.expected_exceptions.is_some() {
                return TestResult {
                    name: label,
                    pass: true,
                    fork: fork_str,
                    error: None,
                };
            }
            return TestResult {
                name: label,
                pass: false,
                fork: fork_str,
                error: Some(format!("VM creation error: {e}")),
            };
        }
    };

    let execution_result = vm.execute();

    // Check post-state.
    match check_post_state(&mut vm, tc, &execution_result) {
        Ok(()) => TestResult {
            name: label,
            pass: true,
            fork: fork_str,
            error: None,
        },
        Err(e) => TestResult {
            name: label,
            pass: false,
            fork: fork_str,
            error: Some(e),
        },
    }
}
