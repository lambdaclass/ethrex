//! Integration test for the EXECUTE precompile (Native Rollups EIP-8079 PoC).
//!
//! Demonstrates:
//! - Building an ExecutionWitness from a simple genesis state
//! - Re-executing a block with a transfer transaction via the EXECUTE precompile
//! - Applying a deposit (L1→L2 balance credit) via the anchor mechanism
//! - Verifying the final state root matches

#![cfg(feature = "native-rollups")]
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    dead_code
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        Account, AccountInfo, AccountState, Block, BlockBody, BlockHeader, ChainConfig, Code,
        CodeMetadata, EIP1559Transaction, EIP4844Transaction, Receipt, Transaction, TxKind,
        Withdrawal, block_execution_witness::ExecutionWitness,
    },
};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_levm::{
    db::gen_db::GeneralizedDatabase,
    environment::Environment,
    errors::TxResult,
    execute_precompile::{Deposit, ExecutePrecompileInput, execute_inner, execute_precompile},
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_trie::Trie;
use k256::ecdsa::{SigningKey, signature::hazmat::PrehashSigner};
use rustc_hash::FxHashMap;

/// Helper: derive Ethereum address from a k256 signing key.
fn address_from_key(key: &SigningKey) -> Address {
    use k256::ecdsa::VerifyingKey;
    let verifying_key = VerifyingKey::from(key);
    let pubkey_bytes = verifying_key.to_encoded_point(false);
    // Skip the 0x04 prefix byte
    let hash = keccak_hash(&pubkey_bytes.as_bytes()[1..]);
    Address::from_slice(&hash[12..])
}

/// Helper: sign an EIP-1559 transaction.
fn sign_eip1559_tx(tx: &mut EIP1559Transaction, key: &SigningKey) {
    use ethrex_rlp::structs::Encoder;

    // Build the unsigned message: tx_type || RLP(chain_id, nonce, max_priority_fee, max_fee, gas_limit, to, value, data, access_list)
    let mut buf = vec![0x02u8]; // EIP-1559 type
    Encoder::new(&mut buf)
        .encode_field(&tx.chain_id)
        .encode_field(&tx.nonce)
        .encode_field(&tx.max_priority_fee_per_gas)
        .encode_field(&tx.max_fee_per_gas)
        .encode_field(&tx.gas_limit)
        .encode_field(&tx.to)
        .encode_field(&tx.value)
        .encode_field(&tx.data)
        .encode_field(&tx.access_list)
        .finish();

    let msg_hash = keccak_hash(&buf);

    let (sig, recid) = key.sign_prehash(&msg_hash).expect("signing failed");

    let sig_bytes = sig.to_bytes();
    tx.signature_r = U256::from_big_endian(&sig_bytes[..32]);
    tx.signature_s = U256::from_big_endian(&sig_bytes[32..64]);
    tx.signature_y_parity = recid.to_byte() != 0;
}

/// Helper: insert an account into the state trie.
fn insert_account(trie: &mut Trie, address: Address, state: &AccountState) {
    let hashed_addr = keccak_hash(address.to_fixed_bytes()).to_vec();
    trie.insert(hashed_addr, state.encode_to_vec())
        .expect("trie insert failed");
}

/// Helper: get the root node from a trie for use in ExecutionWitness.
fn get_trie_root_node(trie: &Trie) -> Option<ethrex_trie::Node> {
    trie.hash_no_commit();
    trie.root_node()
        .expect("root_node failed")
        .map(|arc_node| (*arc_node).clone())
}

/// The main integration test: execute a simple transfer + deposit via the EXECUTE precompile.
#[test]
fn test_execute_precompile_transfer_and_deposit() {
    // ===== Setup: Keys and Addresses =====
    let alice_key = SigningKey::from_bytes(&[1u8; 32].into()).expect("valid key");
    let alice = address_from_key(&alice_key);

    let bob = Address::from_low_u64_be(0xB0B);
    let charlie = Address::from_low_u64_be(0xC4A);
    let coinbase = Address::from_low_u64_be(0xC01);

    let chain_id: u64 = 1;
    let base_fee: u64 = 1_000_000_000; // 1 gwei

    // ===== Genesis State =====
    let alice_balance = U256::from(10) * U256::from(10).pow(U256::from(18)); // 10 ETH
    let alice_state = AccountState {
        nonce: 0,
        balance: alice_balance,
        ..Default::default()
    };

    let coinbase_state = AccountState::default();

    let mut state_trie = Trie::new_temp();
    insert_account(&mut state_trie, alice, &alice_state);
    insert_account(&mut state_trie, coinbase, &coinbase_state);
    // Bob and Charlie don't need to be in the trie initially (they're empty accounts)
    // But we need them for the witness to work — the precompile will try to look them up
    insert_account(&mut state_trie, bob, &AccountState::default());
    insert_account(&mut state_trie, charlie, &AccountState::default());

    let pre_state_root = state_trie.hash_no_commit();

    // ===== Parent Block Header (genesis) =====
    let parent_header = BlockHeader {
        number: 0,
        state_root: pre_state_root,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(base_fee),
        timestamp: 1_000_000,
        ..Default::default()
    };

    // ===== Build Transfer Transaction: Alice → Bob, 1 ETH =====
    let transfer_value = U256::from(10).pow(U256::from(18)); // 1 ETH
    let gas_limit: u64 = 21_000; // Simple transfer

    let mut tx = EIP1559Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: 1_000_000_000, // 1 gwei priority
        max_fee_per_gas: 2_000_000_000,          // 2 gwei max fee
        gas_limit,
        to: TxKind::Call(bob),
        value: transfer_value,
        data: Bytes::new(),
        access_list: vec![],
        ..Default::default()
    };
    sign_eip1559_tx(&mut tx, &alice_key);

    let transaction = Transaction::EIP1559Transaction(tx);

    // Verify we can recover Alice's address from the signed transaction
    let recovered_sender = transaction.sender().expect("sender recovery failed");
    assert_eq!(recovered_sender, alice, "Sender recovery mismatch");

    // ===== Compute Block Fields =====
    // Gas used for a simple transfer
    let gas_used: u64 = 21_000;
    // Effective gas price: min(max_priority_fee + base_fee, max_fee_per_gas)
    let effective_gas_price: u64 = std::cmp::min(1_000_000_000 + base_fee, 2_000_000_000);
    let gas_cost = U256::from(gas_used) * U256::from(effective_gas_price);

    // Priority fee goes to coinbase
    let priority_fee_per_gas: u64 = effective_gas_price.saturating_sub(base_fee);
    let coinbase_reward = U256::from(gas_used) * U256::from(priority_fee_per_gas);

    // Compute transactions root
    let transactions = vec![transaction.clone()];
    let transactions_root = ethrex_common::types::compute_transactions_root(&transactions);

    // Compute receipts root (successful transfer)
    let receipt = Receipt::new(transaction.tx_type(), true, gas_used, vec![]);
    let receipts_root = ethrex_common::types::compute_receipts_root(&[receipt]);

    // ===== Block Header =====
    let block_header = BlockHeader {
        parent_hash: parent_header.compute_block_hash(),
        number: 1,
        gas_used,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(base_fee),
        timestamp: 1_000_012, // 12 seconds after parent
        coinbase,
        transactions_root,
        receipts_root,
        // State root will be computed after we know the post-state
        // For now set a placeholder — we'll compute it properly
        state_root: H256::zero(),
        withdrawals_root: Some(ethrex_common::types::compute_withdrawals_root(&[])),
        ..Default::default()
    };

    let block = Block {
        header: block_header,
        body: BlockBody {
            transactions,
            ommers: vec![],
            withdrawals: Some(vec![]),
        },
    };

    // ===== Compute Expected Post-State (after transfer + deposit) =====
    // After the transfer:
    // - Alice: alice_balance - transfer_value - gas_cost
    // - Bob: transfer_value
    // - Coinbase: coinbase_reward
    // After the deposit:
    // - Charlie: 5 ETH
    let deposit_amount = U256::from(5) * U256::from(10).pow(U256::from(18)); // 5 ETH

    let alice_post = AccountState {
        nonce: 1,
        balance: alice_balance - transfer_value - gas_cost,
        ..Default::default()
    };
    let bob_post = AccountState {
        balance: transfer_value,
        ..Default::default()
    };
    let coinbase_post = AccountState {
        balance: coinbase_reward,
        ..Default::default()
    };
    let charlie_post = AccountState {
        balance: deposit_amount,
        ..Default::default()
    };

    let mut post_trie = Trie::new_temp();
    insert_account(&mut post_trie, alice, &alice_post);
    insert_account(&mut post_trie, bob, &bob_post);
    insert_account(&mut post_trie, coinbase, &coinbase_post);
    insert_account(&mut post_trie, charlie, &charlie_post);
    let post_state_root = post_trie.hash_no_commit();

    // ===== Update block header with correct state root =====
    let mut block = block;
    block.header.state_root = post_state_root;

    // ===== Build ExecutionWitness =====
    let chain_config = ChainConfig {
        chain_id,
        // Activate all pre-merge forks at block 0
        homestead_block: Some(0),
        eip150_block: Some(0),
        eip155_block: Some(0),
        eip158_block: Some(0),
        byzantium_block: Some(0),
        constantinople_block: Some(0),
        petersburg_block: Some(0),
        istanbul_block: Some(0),
        berlin_block: Some(0),
        london_block: Some(0),
        // Post-merge
        terminal_total_difficulty: Some(0),
        terminal_total_difficulty_passed: true,
        // Activate Shanghai at timestamp 0 (for withdrawals support)
        shanghai_time: Some(0),
        ..Default::default()
    };

    let parent_header_bytes = parent_header.encode_to_vec();
    let block_header_bytes = block.header.encode_to_vec();

    let witness = ExecutionWitness {
        codes: vec![],
        block_headers_bytes: vec![parent_header_bytes, block_header_bytes],
        first_block_number: 1,
        chain_config,
        state_trie_root: get_trie_root_node(&state_trie),
        storage_trie_roots: BTreeMap::new(),
        keys: vec![],
    };

    // ===== Deposits =====
    let deposits = vec![Deposit {
        address: charlie,
        amount: deposit_amount,
    }];

    // ===== Build ABI-encoded calldata and execute via execute_precompile() =====
    let block_rlp = block.encode_to_vec();
    let witness_json = serde_json::to_vec(&witness).expect("witness JSON serialization failed");
    let calldata = build_precompile_calldata(pre_state_root, &deposits, &block_rlp, &witness_json);
    println!("ABI-encoded EXECUTE calldata: {} bytes", calldata.len());

    let mut gas_remaining: u64 = 1_000_000;
    let result = execute_precompile(
        &Bytes::from(calldata),
        &mut gas_remaining,
        ethrex_common::types::Fork::Prague,
    );
    match &result {
        Ok(output) => {
            assert_eq!(output.len(), 64, "Expected 64-byte ABI-encoded return");
            let returned_root = H256::from_slice(&output[..32]);
            let returned_block_num = U256::from_big_endian(&output[32..]);
            assert_eq!(
                returned_root, post_state_root,
                "Returned state root mismatch"
            );
            assert_eq!(
                returned_block_num,
                U256::from(1),
                "Returned block number mismatch"
            );
            println!("EXECUTE precompile succeeded!");
            println!("  Pre-state root:  {pre_state_root:?}");
            println!("  Post-state root: {post_state_root:?}");
            println!("  Alice sent 1 ETH to Bob");
            println!("  Charlie received 5 ETH deposit");
        }
        Err(e) => {
            panic!("EXECUTE precompile failed: {e}");
        }
    }
}

/// Helper: build a minimal ExecutePrecompileInput for rejection tests.
///
/// Creates a genesis state with a single account, builds an ExecutionWitness,
/// and wraps the given block in an ExecutePrecompileInput.
fn build_rejection_test_input(block: Block) -> ExecutePrecompileInput {
    let account = Address::from_low_u64_be(0xA);
    let chain_id: u64 = 1;

    let mut state_trie = Trie::new_temp();
    insert_account(
        &mut state_trie,
        account,
        &AccountState {
            balance: U256::from(10) * U256::from(10).pow(U256::from(18)),
            ..Default::default()
        },
    );
    let pre_state_root = state_trie.hash_no_commit();

    let parent_header = BlockHeader {
        number: block.header.number.saturating_sub(1),
        state_root: pre_state_root,
        gas_limit: 30_000_000,
        timestamp: block.header.timestamp.saturating_sub(12),
        ..Default::default()
    };

    let chain_config = ChainConfig {
        chain_id,
        homestead_block: Some(0),
        eip150_block: Some(0),
        eip155_block: Some(0),
        eip158_block: Some(0),
        byzantium_block: Some(0),
        constantinople_block: Some(0),
        petersburg_block: Some(0),
        istanbul_block: Some(0),
        berlin_block: Some(0),
        london_block: Some(0),
        terminal_total_difficulty: Some(0),
        terminal_total_difficulty_passed: true,
        shanghai_time: Some(0),
        ..Default::default()
    };

    let witness = ExecutionWitness {
        codes: vec![],
        block_headers_bytes: vec![parent_header.encode_to_vec(), block.header.encode_to_vec()],
        first_block_number: block.header.number,
        chain_config,
        state_trie_root: get_trie_root_node(&state_trie),
        storage_trie_roots: BTreeMap::new(),
        keys: vec![],
    };

    ExecutePrecompileInput {
        pre_state_root,
        deposits: vec![],
        execution_witness: witness,
        block,
    }
}

/// Test that blob transactions (EIP-4844) are rejected in native rollup blocks.
#[test]
fn test_execute_precompile_rejects_blob_transactions() {
    let blob_tx = Transaction::EIP4844Transaction(EIP4844Transaction {
        to: Address::from_low_u64_be(0xB0B),
        blob_versioned_hashes: vec![H256::zero()],
        ..Default::default()
    });

    let block = Block {
        header: BlockHeader {
            number: 1,
            gas_limit: 30_000_000,
            timestamp: 1_000_012,
            ..Default::default()
        },
        body: BlockBody {
            transactions: vec![blob_tx],
            ommers: vec![],
            withdrawals: Some(vec![]),
        },
    };

    let result = execute_inner(build_rejection_test_input(block));
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Blob transactions"),
        "Expected blob transaction rejection"
    );
}

/// Test that blocks with non-empty withdrawals are rejected in native rollups.
#[test]
fn test_execute_precompile_rejects_withdrawals() {
    let block = Block {
        header: BlockHeader {
            number: 1,
            gas_used: 0,
            gas_limit: 30_000_000,
            timestamp: 1_000_012,
            ..Default::default()
        },
        body: BlockBody {
            transactions: vec![],
            ommers: vec![],
            withdrawals: Some(vec![Withdrawal {
                index: 0,
                validator_index: 0,
                address: Address::from_low_u64_be(0xA),
                amount: 1000,
            }]),
        },
    };

    let result = execute_inner(build_rejection_test_input(block));
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("must not contain withdrawals"),
        "Expected withdrawal rejection"
    );
}

// ===== Contract-based demo test infrastructure =====

/// Minimal Database implementation for L1 VM tests.
///
/// Provides chain config and default values for everything else.
/// Actual account state is loaded into GeneralizedDatabase's cache.
struct TestDb {
    chain_config: ChainConfig,
}

impl ethrex_levm::db::Database for TestDb {
    fn get_account_state(
        &self,
        _address: Address,
    ) -> Result<AccountState, ethrex_levm::errors::DatabaseError> {
        Ok(AccountState::default())
    }
    fn get_storage_value(
        &self,
        _address: Address,
        _key: H256,
    ) -> Result<U256, ethrex_levm::errors::DatabaseError> {
        Ok(U256::zero())
    }
    fn get_block_hash(
        &self,
        _block_number: u64,
    ) -> Result<H256, ethrex_levm::errors::DatabaseError> {
        Ok(H256::zero())
    }
    fn get_chain_config(&self) -> Result<ChainConfig, ethrex_levm::errors::DatabaseError> {
        Ok(self.chain_config)
    }
    fn get_account_code(
        &self,
        _code_hash: H256,
    ) -> Result<Code, ethrex_levm::errors::DatabaseError> {
        Ok(Code::default())
    }
    fn get_code_metadata(
        &self,
        _code_hash: H256,
    ) -> Result<CodeMetadata, ethrex_levm::errors::DatabaseError> {
        Ok(CodeMetadata { length: 0 })
    }
}

/// NativeRollup.sol runtime bytecode (compiled with solc 0.8.31).
///
/// Contract storage layout:
///   - Slot 0: stateRoot (bytes32)
///   - Slot 1: blockNumber (uint256)
///   - Slot 2: pendingDeposits.length (dynamic array)
///   - Slot 3: depositIndex (uint256)
///
/// Function selectors:
///   - ed3133f2: advance(uint256,bytes,bytes)
///   - 57e871e7: blockNumber()
///   - f340fa01: deposit(address)
///   - 7b898939: depositIndex()
///   - a7932794: pendingDeposits(uint256)
///   - 9588eca2: stateRoot()
///
/// Source: crates/vm/levm/contracts/NativeRollup.sol
/// Compile: cd crates/vm/levm/contracts && solc --bin-runtime NativeRollup.sol -o build --overwrite
const NATIVE_ROLLUP_RUNTIME_HEX: &str = "608060405260043610610058575f3560e01c806357e871e7146101965780637b898939146101c05780639588eca2146101ea578063a793279414610214578063ed3133f214610251578063f340fa011461027957610192565b36610192575f341161009f576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610096906106fa565b60405180910390fd5b600260405180604001604052803373ffffffffffffffffffffffffffffffffffffffff16815260200134815250908060018154018082558091505060019003905f5260205f2090600202015f909190919091505f820151815f015f6101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055506020820151816001015550503373ffffffffffffffffffffffffffffffffffffffff167f741a0277a612f71e4836430fe80cc831a4e28c01d2121c0ab1a4451bc88f909e346040516101889190610730565b60405180910390a2005b5f5ffd5b3480156101a1575f5ffd5b506101aa610295565b6040516101b79190610730565b60405180910390f35b3480156101cb575f5ffd5b506101d461029b565b6040516101e19190610730565b60405180910390f35b3480156101f5575f5ffd5b506101fe6102a1565b60405161020b9190610761565b60405180910390f35b34801561021f575f5ffd5b5061023a600480360381019061023591906107ac565b6102a6565b604051610248929190610816565b60405180910390f35b34801561025c575f5ffd5b506102776004803603810190610272919061089e565b6102f4565b005b610293600480360381019061028e9190610959565b61056a565b005b60015481565b60035481565b5f5481565b600281815481106102b5575f80fd5b905f5260205f2090600202015f91509050805f015f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff16908060010154905082565b5f6003549050600280549050868261030c91906109b1565b111561034d576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161034490610a2e565b60405180910390fd5b60605f5f90505b87811015610403575f6002828561036b91906109b1565b8154811061037c5761037b610a4c565b5b905f5260205f209060020201905082815f015f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff1682600101546040516020016103c4929190610ade565b6040516020818303038152906040526040516020016103e4929190610b5b565b6040516020818303038152906040529250508080600101915050610354565b50868261041091906109b1565b6003819055505f5f54878787878660405160200161043396959493929190610c10565b60405160208183030381529060405290505f5f61010173ffffffffffffffffffffffffffffffffffffffff168360405161046d9190610c6c565b5f604051808303815f865af19150503d805f81146104a6576040519150601f19603f3d011682016040523d82523d5f602084013e6104ab565b606091505b50915091508180156104be575060408151145b6104fd576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016104f490610cf2565b60405180910390fd5b5f5f828060200190518101906105139190610d4e565b91509150815f8190555080600181905550807fe41acc52c5cd3ab398bfed63f4130976083bea5288e3bf4bf489ccbb3bd20c85836040516105549190610761565b60405180910390a2505050505050505050505050565b5f34116105ac576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016105a3906106fa565b60405180910390fd5b600260405180604001604052808373ffffffffffffffffffffffffffffffffffffffff16815260200134815250908060018154018082558091505060019003905f5260205f2090600202015f909190919091505f820151815f015f6101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055506020820151816001015550508073ffffffffffffffffffffffffffffffffffffffff167f741a0277a612f71e4836430fe80cc831a4e28c01d2121c0ab1a4451bc88f909e346040516106959190610730565b60405180910390a250565b5f82825260208201905092915050565b7f4d7573742073656e6420455448000000000000000000000000000000000000005f82015250565b5f6106e4600d836106a0565b91506106ef826106b0565b602082019050919050565b5f6020820190508181035f830152610711816106d8565b9050919050565b5f819050919050565b61072a81610718565b82525050565b5f6020820190506107435f830184610721565b92915050565b5f819050919050565b61075b81610749565b82525050565b5f6020820190506107745f830184610752565b92915050565b5f5ffd5b5f5ffd5b61078b81610718565b8114610795575f5ffd5b50565b5f813590506107a681610782565b92915050565b5f602082840312156107c1576107c061077a565b5b5f6107ce84828501610798565b91505092915050565b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f610800826107d7565b9050919050565b610810816107f6565b82525050565b5f6040820190506108295f830185610807565b6108366020830184610721565b9392505050565b5f5ffd5b5f5ffd5b5f5ffd5b5f5f83601f84011261085e5761085d61083d565b5b8235905067ffffffffffffffff81111561087b5761087a610841565b5b60208301915083600182028301111561089757610896610845565b5b9250929050565b5f5f5f5f5f606086880312156108b7576108b661077a565b5b5f6108c488828901610798565b955050602086013567ffffffffffffffff8111156108e5576108e461077e565b5b6108f188828901610849565b9450945050604086013567ffffffffffffffff8111156109145761091361077e565b5b61092088828901610849565b92509250509295509295909350565b610938816107f6565b8114610942575f5ffd5b50565b5f813590506109538161092f565b92915050565b5f6020828403121561096e5761096d61077a565b5b5f61097b84828501610945565b91505092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f6109bb82610718565b91506109c683610718565b92508282019050808211156109de576109dd610984565b5b92915050565b7f4e6f7420656e6f756768206465706f73697473000000000000000000000000005f82015250565b5f610a186013836106a0565b9150610a23826109e4565b602082019050919050565b5f6020820190508181035f830152610a4581610a0c565b9050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b5f8160601b9050919050565b5f610a8f82610a79565b9050919050565b5f610aa082610a85565b9050919050565b610ab8610ab3826107f6565b610a96565b82525050565b5f819050919050565b610ad8610ad382610718565b610abe565b82525050565b5f610ae98285610aa7565b601482019150610af98284610ac7565b6020820191508190509392505050565b5f81519050919050565b5f81905092915050565b8281835e5f83830152505050565b5f610b3582610b09565b610b3f8185610b13565b9350610b4f818560208601610b1d565b80840191505092915050565b5f610b668285610b2b565b9150610b728284610b2b565b91508190509392505050565b5f82825260208201905092915050565b828183375f83830152505050565b5f601f19601f8301169050919050565b5f610bb78385610b7e565b9350610bc4838584610b8e565b610bcd83610b9c565b840190509392505050565b5f610be282610b09565b610bec8185610b7e565b9350610bfc818560208601610b1d565b610c0581610b9c565b840191505092915050565b5f608082019050610c235f830189610752565b8181036020830152610c36818789610bac565b90508181036040830152610c4b818587610bac565b90508181036060830152610c5f8184610bd8565b9050979650505050505050565b5f610c778284610b2b565b915081905092915050565b7f4558454355544520707265636f6d70696c6520766572696669636174696f6e205f8201527f6661696c65640000000000000000000000000000000000000000000000000000602082015250565b5f610cdc6026836106a0565b9150610ce782610c82565b604082019050919050565b5f6020820190508181035f830152610d0981610cd0565b9050919050565b610d1981610749565b8114610d23575f5ffd5b50565b5f81519050610d3481610d10565b92915050565b5f81519050610d4881610782565b92915050565b5f5f60408385031215610d6457610d6361077a565b5b5f610d7185828601610d26565b9250506020610d8285828601610d3a565b915050925092905056fea264697066735822122032f2db442daf298b9e99572230200f4a1f4248d37377159a59baf92b1f49ba4f64736f6c634300081f0033";

/// NativeRollup.sol full deployment bytecode (constructor + runtime).
///
/// Constructor takes bytes32 _initialStateRoot as argument.
/// Deployment data = DEPLOY_BYTECODE + ABI-encoded constructor arg (32 bytes).
///
/// Compile: cd crates/vm/levm/contracts && solc --bin NativeRollup.sol -o build --overwrite
const NATIVE_ROLLUP_DEPLOY_HEX: &str = "6080604052348015600e575f5ffd5b50604051610e60380380610e608339818101604052810190602e9190606b565b805f81905550506091565b5f5ffd5b5f819050919050565b604d81603d565b81146056575f5ffd5b50565b5f815190506065816046565b92915050565b5f60208284031215607d57607c6039565b5b5f6088848285016059565b91505092915050565b610dc28061009e5f395ff3fe608060405260043610610058575f3560e01c806357e871e7146101965780637b898939146101c05780639588eca2146101ea578063a793279414610214578063ed3133f214610251578063f340fa011461027957610192565b36610192575f341161009f576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610096906106fa565b60405180910390fd5b600260405180604001604052803373ffffffffffffffffffffffffffffffffffffffff16815260200134815250908060018154018082558091505060019003905f5260205f2090600202015f909190919091505f820151815f015f6101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055506020820151816001015550503373ffffffffffffffffffffffffffffffffffffffff167f741a0277a612f71e4836430fe80cc831a4e28c01d2121c0ab1a4451bc88f909e346040516101889190610730565b60405180910390a2005b5f5ffd5b3480156101a1575f5ffd5b506101aa610295565b6040516101b79190610730565b60405180910390f35b3480156101cb575f5ffd5b506101d461029b565b6040516101e19190610730565b60405180910390f35b3480156101f5575f5ffd5b506101fe6102a1565b60405161020b9190610761565b60405180910390f35b34801561021f575f5ffd5b5061023a600480360381019061023591906107ac565b6102a6565b604051610248929190610816565b60405180910390f35b34801561025c575f5ffd5b506102776004803603810190610272919061089e565b6102f4565b005b610293600480360381019061028e9190610959565b61056a565b005b60015481565b60035481565b5f5481565b600281815481106102b5575f80fd5b905f5260205f2090600202015f91509050805f015f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff16908060010154905082565b5f6003549050600280549050868261030c91906109b1565b111561034d576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161034490610a2e565b60405180910390fd5b60605f5f90505b87811015610403575f6002828561036b91906109b1565b8154811061037c5761037b610a4c565b5b905f5260205f209060020201905082815f015f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff1682600101546040516020016103c4929190610ade565b6040516020818303038152906040526040516020016103e4929190610b5b565b6040516020818303038152906040529250508080600101915050610354565b50868261041091906109b1565b6003819055505f5f54878787878660405160200161043396959493929190610c10565b60405160208183030381529060405290505f5f61010173ffffffffffffffffffffffffffffffffffffffff168360405161046d9190610c6c565b5f604051808303815f865af19150503d805f81146104a6576040519150601f19603f3d011682016040523d82523d5f602084013e6104ab565b606091505b50915091508180156104be575060408151145b6104fd576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016104f490610cf2565b60405180910390fd5b5f5f828060200190518101906105139190610d4e565b91509150815f8190555080600181905550807fe41acc52c5cd3ab398bfed63f4130976083bea5288e3bf4bf489ccbb3bd20c85836040516105549190610761565b60405180910390a2505050505050505050505050565b5f34116105ac576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016105a3906106fa565b60405180910390fd5b600260405180604001604052808373ffffffffffffffffffffffffffffffffffffffff16815260200134815250908060018154018082558091505060019003905f5260205f2090600202015f909190919091505f820151815f015f6101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055506020820151816001015550508073ffffffffffffffffffffffffffffffffffffffff167f741a0277a612f71e4836430fe80cc831a4e28c01d2121c0ab1a4451bc88f909e346040516106959190610730565b60405180910390a250565b5f82825260208201905092915050565b7f4d7573742073656e6420455448000000000000000000000000000000000000005f82015250565b5f6106e4600d836106a0565b91506106ef826106b0565b602082019050919050565b5f6020820190508181035f830152610711816106d8565b9050919050565b5f819050919050565b61072a81610718565b82525050565b5f6020820190506107435f830184610721565b92915050565b5f819050919050565b61075b81610749565b82525050565b5f6020820190506107745f830184610752565b92915050565b5f5ffd5b5f5ffd5b61078b81610718565b8114610795575f5ffd5b50565b5f813590506107a681610782565b92915050565b5f602082840312156107c1576107c061077a565b5b5f6107ce84828501610798565b91505092915050565b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f610800826107d7565b9050919050565b610810816107f6565b82525050565b5f6040820190506108295f830185610807565b6108366020830184610721565b9392505050565b5f5ffd5b5f5ffd5b5f5ffd5b5f5f83601f84011261085e5761085d61083d565b5b8235905067ffffffffffffffff81111561087b5761087a610841565b5b60208301915083600182028301111561089757610896610845565b5b9250929050565b5f5f5f5f5f606086880312156108b7576108b661077a565b5b5f6108c488828901610798565b955050602086013567ffffffffffffffff8111156108e5576108e461077e565b5b6108f188828901610849565b9450945050604086013567ffffffffffffffff8111156109145761091361077e565b5b61092088828901610849565b92509250509295509295909350565b610938816107f6565b8114610942575f5ffd5b50565b5f813590506109538161092f565b92915050565b5f6020828403121561096e5761096d61077a565b5b5f61097b84828501610945565b91505092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f6109bb82610718565b91506109c683610718565b92508282019050808211156109de576109dd610984565b5b92915050565b7f4e6f7420656e6f756768206465706f73697473000000000000000000000000005f82015250565b5f610a186013836106a0565b9150610a23826109e4565b602082019050919050565b5f6020820190508181035f830152610a4581610a0c565b9050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b5f8160601b9050919050565b5f610a8f82610a79565b9050919050565b5f610aa082610a85565b9050919050565b610ab8610ab3826107f6565b610a96565b82525050565b5f819050919050565b610ad8610ad382610718565b610abe565b82525050565b5f610ae98285610aa7565b601482019150610af98284610ac7565b6020820191508190509392505050565b5f81519050919050565b5f81905092915050565b8281835e5f83830152505050565b5f610b3582610b09565b610b3f8185610b13565b9350610b4f818560208601610b1d565b80840191505092915050565b5f610b668285610b2b565b9150610b728284610b2b565b91508190509392505050565b5f82825260208201905092915050565b828183375f83830152505050565b5f601f19601f8301169050919050565b5f610bb78385610b7e565b9350610bc4838584610b8e565b610bcd83610b9c565b840190509392505050565b5f610be282610b09565b610bec8185610b7e565b9350610bfc818560208601610b1d565b610c0581610b9c565b840191505092915050565b5f608082019050610c235f830189610752565b8181036020830152610c36818789610bac565b90508181036040830152610c4b818587610bac565b90508181036060830152610c5f8184610bd8565b9050979650505050505050565b5f610c778284610b2b565b915081905092915050565b7f4558454355544520707265636f6d70696c6520766572696669636174696f6e205f8201527f6661696c65640000000000000000000000000000000000000000000000000000602082015250565b5f610cdc6026836106a0565b9150610ce782610c82565b604082019050919050565b5f6020820190508181035f830152610d0981610cd0565b9050919050565b610d1981610749565b8114610d23575f5ffd5b50565b5f81519050610d3481610d10565b92915050565b5f81519050610d4881610782565b92915050565b5f5f60408385031215610d6457610d6361077a565b5b5f610d7185828601610d26565b9250506020610d8285828601610d3a565b915050925092905056fea264697066735822122032f2db442daf298b9e99572230200f4a1f4248d37377159a59baf92b1f49ba4f64736f6c634300081f0033";

/// Encode a call to NativeRollup.deposit(address).
///
/// ABI encoding for deposit(address _recipient):
///   - 4 bytes: function selector (0xf340fa01)
///   - 32 bytes: _recipient (address, left-padded)
fn encode_deposit_call(recipient: Address) -> Vec<u8> {
    let mut encoded = Vec::new();

    // Function selector: deposit(address) = 0xf340fa01
    encoded.extend_from_slice(&[0xf3, 0x40, 0xfa, 0x01]);

    // _recipient (address, left-padded to 32 bytes)
    let mut addr_bytes = [0u8; 32];
    addr_bytes[12..].copy_from_slice(recipient.as_bytes());
    encoded.extend_from_slice(&addr_bytes);

    encoded
}

/// Encode a call to NativeRollup.advance(uint256, bytes, bytes).
///
/// ABI encoding:
///   - 4 bytes: function selector (0xed3133f2)
///   - 32 bytes: _depositsCount
///   - 32 bytes: offset to _block bytes
///   - 32 bytes: offset to _witness bytes
///   - dynamic: _block length + data (padded)
///   - dynamic: _witness length + data (padded)
fn encode_advance_call(deposits_count: u64, block_rlp: &[u8], witness_json: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::new();

    // Function selector: advance(uint256,bytes,bytes) = 0xed3133f2
    encoded.extend_from_slice(&[0xed, 0x31, 0x33, 0xf2]);

    // _depositsCount (uint256)
    let mut deposits_count_bytes = [0u8; 32];
    deposits_count_bytes[24..].copy_from_slice(&deposits_count.to_be_bytes());
    encoded.extend_from_slice(&deposits_count_bytes);

    // Offset to _block: 3 static params * 32 = 96 = 0x60
    let mut block_offset = [0u8; 32];
    block_offset[31] = 0x60;
    encoded.extend_from_slice(&block_offset);

    // Offset to _witness: 0x60 + 32 (block length) + padded block data
    let padded_block_len = block_rlp.len() + ((32 - (block_rlp.len() % 32)) % 32);
    let witness_offset: u64 = 96 + 32 + padded_block_len as u64;
    let mut witness_offset_bytes = [0u8; 32];
    witness_offset_bytes[24..].copy_from_slice(&witness_offset.to_be_bytes());
    encoded.extend_from_slice(&witness_offset_bytes);

    // _block: length + data (padded to 32-byte boundary)
    let mut block_len = [0u8; 32];
    block_len[24..].copy_from_slice(&(block_rlp.len() as u64).to_be_bytes());
    encoded.extend_from_slice(&block_len);
    encoded.extend_from_slice(block_rlp);
    let block_padding = (32 - (block_rlp.len() % 32)) % 32;
    encoded.resize(encoded.len() + block_padding, 0);

    // _witness: length + data (padded to 32-byte boundary)
    let mut witness_len = [0u8; 32];
    witness_len[24..].copy_from_slice(&(witness_json.len() as u64).to_be_bytes());
    encoded.extend_from_slice(&witness_len);
    encoded.extend_from_slice(witness_json);
    let witness_padding = (32 - (witness_json.len() % 32)) % 32;
    encoded.resize(encoded.len() + witness_padding, 0);

    encoded
}

/// Build ABI-encoded calldata for the EXECUTE precompile.
///
/// Format: abi.encode(bytes32 preStateRoot, bytes blockRlp, bytes witnessJson, bytes deposits)
///
/// ABI layout:
///   [32 bytes] preStateRoot (static)
///   [32 bytes] offset to blockRlp
///   [32 bytes] offset to witnessJson
///   [32 bytes] offset to deposits
///   [dynamic]  blockRlp: [32 length][data][padding]
///   [dynamic]  witnessJson: [32 length][data][padding]
///   [dynamic]  deposits: [32 length][packed deposit data: (20 addr + 32 amount) * N]
fn build_precompile_calldata(
    pre_state_root: H256,
    deposits: &[Deposit],
    block_rlp: &[u8],
    witness_json: &[u8],
) -> Vec<u8> {
    // Build packed deposits bytes
    let mut deposits_data = Vec::new();
    for deposit in deposits {
        deposits_data.extend_from_slice(deposit.address.as_bytes());
        deposits_data.extend_from_slice(&deposit.amount.to_big_endian());
    }

    // Helper: pad to 32-byte boundary
    fn pad32(len: usize) -> usize {
        len + ((32 - (len % 32)) % 32)
    }

    // Calculate offsets (relative to start of calldata, after the 4 static words = 128 bytes)
    let block_offset: usize = 128; // 4 * 32
    let block_padded = pad32(block_rlp.len());
    let witness_offset: usize = block_offset + 32 + block_padded;
    let witness_padded = pad32(witness_json.len());
    let deposits_offset: usize = witness_offset + 32 + witness_padded;

    let mut data = Vec::new();

    // 1. preStateRoot (bytes32)
    data.extend_from_slice(pre_state_root.as_bytes());

    // 2. offset to blockRlp
    let mut offset_bytes = [0u8; 32];
    offset_bytes[24..].copy_from_slice(&(block_offset as u64).to_be_bytes());
    data.extend_from_slice(&offset_bytes);

    // 3. offset to witnessJson
    let mut offset_bytes = [0u8; 32];
    offset_bytes[24..].copy_from_slice(&(witness_offset as u64).to_be_bytes());
    data.extend_from_slice(&offset_bytes);

    // 4. offset to deposits
    let mut offset_bytes = [0u8; 32];
    offset_bytes[24..].copy_from_slice(&(deposits_offset as u64).to_be_bytes());
    data.extend_from_slice(&offset_bytes);

    // 5. blockRlp: length + data + padding
    let mut len_bytes = [0u8; 32];
    len_bytes[24..].copy_from_slice(&(block_rlp.len() as u64).to_be_bytes());
    data.extend_from_slice(&len_bytes);
    data.extend_from_slice(block_rlp);
    data.resize(data.len() + (block_padded - block_rlp.len()), 0);

    // 6. witnessJson: length + data + padding
    let mut len_bytes = [0u8; 32];
    len_bytes[24..].copy_from_slice(&(witness_json.len() as u64).to_be_bytes());
    data.extend_from_slice(&len_bytes);
    data.extend_from_slice(witness_json);
    data.resize(data.len() + (witness_padded - witness_json.len()), 0);

    // 7. deposits: length + data + padding
    let mut len_bytes = [0u8; 32];
    len_bytes[24..].copy_from_slice(&(deposits_data.len() as u64).to_be_bytes());
    data.extend_from_slice(&len_bytes);
    data.extend_from_slice(&deposits_data);
    let deposits_padded = pad32(deposits_data.len());
    data.resize(data.len() + (deposits_padded - deposits_data.len()), 0);

    data
}

/// Build the L2 state transition used by both the direct test and the contract test.
///
/// Returns:
///   - ExecutePrecompileInput (for direct execute_inner calls)
///   - block RLP bytes (for binary calldata / contract call)
///   - witness JSON bytes (for binary calldata / contract call)
///   - pre_state_root
///   - post_state_root
fn build_l2_state_transition() -> (ExecutePrecompileInput, Vec<u8>, Vec<u8>, H256, H256) {
    let alice_key = SigningKey::from_bytes(&[1u8; 32].into()).expect("valid key");
    let alice = address_from_key(&alice_key);
    let bob = Address::from_low_u64_be(0xB0B);
    let charlie = Address::from_low_u64_be(0xC4A);
    let coinbase = Address::from_low_u64_be(0xC01);
    let chain_id: u64 = 1;
    let base_fee: u64 = 1_000_000_000;

    // Genesis state
    let alice_balance = U256::from(10) * U256::from(10).pow(U256::from(18));
    let mut state_trie = Trie::new_temp();
    insert_account(
        &mut state_trie,
        alice,
        &AccountState {
            nonce: 0,
            balance: alice_balance,
            ..Default::default()
        },
    );
    insert_account(&mut state_trie, coinbase, &AccountState::default());
    insert_account(&mut state_trie, bob, &AccountState::default());
    insert_account(&mut state_trie, charlie, &AccountState::default());
    let pre_state_root = state_trie.hash_no_commit();

    // Parent block
    let parent_header = BlockHeader {
        number: 0,
        state_root: pre_state_root,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(base_fee),
        timestamp: 1_000_000,
        ..Default::default()
    };

    // Transfer: Alice → Bob, 1 ETH
    let transfer_value = U256::from(10).pow(U256::from(18));
    let mut tx = EIP1559Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: 1_000_000_000,
        max_fee_per_gas: 2_000_000_000,
        gas_limit: 21_000,
        to: TxKind::Call(bob),
        value: transfer_value,
        data: Bytes::new(),
        access_list: vec![],
        ..Default::default()
    };
    sign_eip1559_tx(&mut tx, &alice_key);
    let transaction = Transaction::EIP1559Transaction(tx);
    let transactions = vec![transaction.clone()];

    // Compute block fields
    let gas_used: u64 = 21_000;
    let effective_gas_price: u64 = std::cmp::min(1_000_000_000 + base_fee, 2_000_000_000);
    let gas_cost = U256::from(gas_used) * U256::from(effective_gas_price);
    let priority_fee_per_gas: u64 = effective_gas_price.saturating_sub(base_fee);
    let coinbase_reward = U256::from(gas_used) * U256::from(priority_fee_per_gas);

    let transactions_root = ethrex_common::types::compute_transactions_root(&transactions);
    let receipt = Receipt::new(transaction.tx_type(), true, gas_used, vec![]);
    let receipts_root = ethrex_common::types::compute_receipts_root(&[receipt]);

    // Post-state (after transfer + deposit)
    let deposit_amount = U256::from(5) * U256::from(10).pow(U256::from(18));
    let mut post_trie = Trie::new_temp();
    insert_account(
        &mut post_trie,
        alice,
        &AccountState {
            nonce: 1,
            balance: alice_balance - transfer_value - gas_cost,
            ..Default::default()
        },
    );
    insert_account(
        &mut post_trie,
        bob,
        &AccountState {
            balance: transfer_value,
            ..Default::default()
        },
    );
    insert_account(
        &mut post_trie,
        coinbase,
        &AccountState {
            balance: coinbase_reward,
            ..Default::default()
        },
    );
    insert_account(
        &mut post_trie,
        charlie,
        &AccountState {
            balance: deposit_amount,
            ..Default::default()
        },
    );
    let post_state_root = post_trie.hash_no_commit();

    // Block
    let block = Block {
        header: BlockHeader {
            parent_hash: parent_header.compute_block_hash(),
            number: 1,
            gas_used,
            gas_limit: 30_000_000,
            base_fee_per_gas: Some(base_fee),
            timestamp: 1_000_012,
            coinbase,
            transactions_root,
            receipts_root,
            state_root: post_state_root,
            withdrawals_root: Some(ethrex_common::types::compute_withdrawals_root(&[])),
            ..Default::default()
        },
        body: BlockBody {
            transactions,
            ommers: vec![],
            withdrawals: Some(vec![]),
        },
    };

    // Witness
    let chain_config = ChainConfig {
        chain_id,
        homestead_block: Some(0),
        eip150_block: Some(0),
        eip155_block: Some(0),
        eip158_block: Some(0),
        byzantium_block: Some(0),
        constantinople_block: Some(0),
        petersburg_block: Some(0),
        istanbul_block: Some(0),
        berlin_block: Some(0),
        london_block: Some(0),
        terminal_total_difficulty: Some(0),
        terminal_total_difficulty_passed: true,
        shanghai_time: Some(0),
        ..Default::default()
    };

    let witness = ExecutionWitness {
        codes: vec![],
        block_headers_bytes: vec![parent_header.encode_to_vec(), block.header.encode_to_vec()],
        first_block_number: 1,
        chain_config,
        state_trie_root: get_trie_root_node(&state_trie),
        storage_trie_roots: BTreeMap::new(),
        keys: vec![],
    };

    let block_rlp = block.encode_to_vec();
    let witness_json = serde_json::to_vec(&witness).expect("witness JSON serialization failed");

    let input = ExecutePrecompileInput {
        pre_state_root,
        deposits: vec![Deposit {
            address: charlie,
            amount: deposit_amount,
        }],
        execution_witness: witness,
        block,
    };

    (
        input,
        block_rlp,
        witness_json,
        pre_state_root,
        post_state_root,
    )
}

/// Demo: NativeRollup contract with deposit + advance to verify an L2 state transition.
///
/// This test shows the full end-to-end flow:
///   1. deposit(charlie) with 5 ETH → records pending deposit
///   2. advance(postStateRoot, 1, 1, blockRlp, witnessJson)
///      → NativeRollup builds binary calldata → CALL to 0x0101 → EXECUTE precompile
///      → parse binary calldata → re-execute L2 block → verify state roots
///      → success → contract updates stateRoot, blockNumber, depositIndex
#[test]
fn test_native_rollup_contract() {
    let (_input, block_rlp, witness_json, pre_state_root, post_state_root) =
        build_l2_state_transition();

    let charlie = Address::from_low_u64_be(0xC4A);
    let deposit_amount = U256::from(5) * U256::from(10).pow(U256::from(18)); // 5 ETH

    // Deploy NativeRollup contract on "L1" (pre-loaded with runtime bytecode + initial storage)
    let contract_address = Address::from_low_u64_be(0xFFFF);
    let sender = Address::from_low_u64_be(0x1234);

    let runtime_bytecode =
        Bytes::from(hex::decode(NATIVE_ROLLUP_RUNTIME_HEX).expect("invalid hex"));
    let contract_code_hash = H256(keccak_hash(runtime_bytecode.as_ref()));
    let contract_code = Code::from_bytecode(runtime_bytecode);

    // Pre-populate contract storage: slot 0 = stateRoot (pre_state_root)
    let mut contract_storage: FxHashMap<H256, U256> = FxHashMap::default();
    contract_storage.insert(
        H256::zero(),
        U256::from_big_endian(pre_state_root.as_bytes()),
    );

    let mut accounts: FxHashMap<Address, Account> = FxHashMap::default();
    accounts.insert(
        contract_address,
        Account {
            info: AccountInfo {
                code_hash: contract_code_hash,
                balance: U256::zero(),
                nonce: 1,
            },
            code: contract_code,
            storage: contract_storage,
        },
    );
    accounts.insert(
        sender,
        Account {
            info: AccountInfo {
                balance: U256::MAX,
                ..Default::default()
            },
            code: Code::default(),
            storage: FxHashMap::default(),
        },
    );

    let l1_chain_config = ChainConfig {
        chain_id: 1,
        homestead_block: Some(0),
        eip150_block: Some(0),
        eip155_block: Some(0),
        eip158_block: Some(0),
        byzantium_block: Some(0),
        constantinople_block: Some(0),
        petersburg_block: Some(0),
        istanbul_block: Some(0),
        berlin_block: Some(0),
        london_block: Some(0),
        terminal_total_difficulty: Some(0),
        terminal_total_difficulty_passed: true,
        shanghai_time: Some(0),
        ..Default::default()
    };

    let store: Arc<dyn ethrex_levm::db::Database> = Arc::new(TestDb {
        chain_config: l1_chain_config,
    });
    let mut db = GeneralizedDatabase::new_with_account_state(store, accounts);

    // === TX 1: deposit(charlie) with 5 ETH ===
    let deposit_calldata = encode_deposit_call(charlie);

    let deposit_tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 1_000_000_000,
        to: TxKind::Call(contract_address),
        value: deposit_amount,
        data: Bytes::from(deposit_calldata),
        access_list: vec![],
        ..Default::default()
    });

    let deposit_env = Environment {
        origin: sender,
        gas_limit: 1_000_000_000,
        block_gas_limit: 1_000_000_000,
        tx_nonce: 0,
        chain_id: U256::from(1),
        ..Default::default()
    };

    let mut vm = VM::new(
        deposit_env,
        &mut db,
        &deposit_tx,
        LevmCallTracer::disabled(),
        VMType::L1,
    )
    .expect("VM creation failed");

    let deposit_report = vm.execute().expect("VM execution failed");
    assert!(
        matches!(deposit_report.result, TxResult::Success),
        "Deposit transaction reverted: {:?}",
        deposit_report.result
    );
    println!("Deposit TX succeeded (5 ETH for charlie)");

    // === TX 2: advance(1, blockRlp, witnessJson) ===
    let advance_calldata = encode_advance_call(1, &block_rlp, &witness_json);

    let advance_tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: 1,
        nonce: 1,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 1_000_000_000,
        to: TxKind::Call(contract_address),
        value: U256::zero(),
        data: Bytes::from(advance_calldata),
        access_list: vec![],
        ..Default::default()
    });

    let advance_env = Environment {
        origin: sender,
        gas_limit: 1_000_000_000,
        block_gas_limit: 1_000_000_000,
        tx_nonce: 1,
        chain_id: U256::from(1),
        ..Default::default()
    };

    let mut vm = VM::new(
        advance_env,
        &mut db,
        &advance_tx,
        LevmCallTracer::disabled(),
        VMType::L1,
    )
    .expect("VM creation failed");

    let report = vm.execute().expect("VM execution failed");

    assert!(
        matches!(report.result, TxResult::Success),
        "L1 advance transaction reverted: {:?}",
        report.result
    );

    // Verify the contract updated its storage
    let contract_account = db.get_account(contract_address).expect("account not found");
    let stored_state_root = contract_account
        .storage
        .get(&H256::zero())
        .copied()
        .unwrap_or_default();
    let stored_block_number = contract_account
        .storage
        .get(&H256::from_low_u64_be(1))
        .copied()
        .unwrap_or_default();
    let stored_deposit_index = contract_account
        .storage
        .get(&H256::from_low_u64_be(3))
        .copied()
        .unwrap_or_default();

    // Convert stored U256 back to H256 for comparison
    let root_bytes = stored_state_root.to_big_endian();
    assert_eq!(
        H256::from(root_bytes),
        post_state_root,
        "Contract stateRoot mismatch"
    );
    assert_eq!(
        stored_block_number,
        U256::from(1),
        "Contract blockNumber mismatch"
    );
    assert_eq!(
        stored_deposit_index,
        U256::from(1),
        "Contract depositIndex mismatch"
    );

    println!("NativeRollup contract demo succeeded!");
    println!("  L2 state transition verified via deposit() + advance():");
    println!("    Pre-state root:  {pre_state_root:?}");
    println!("    Post-state root: {post_state_root:?}");
    println!("    Block number:    1");
    println!("    Deposit index:   1");
    println!("  Gas used: {}", report.gas_used);
}
