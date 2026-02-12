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

    // ===== Build binary calldata and execute via execute_precompile() =====
    let block_rlp = block.encode_to_vec();
    let witness_json = serde_json::to_vec(&witness).expect("witness JSON serialization failed");
    let calldata = build_precompile_calldata(
        pre_state_root,
        post_state_root,
        &deposits,
        &block_rlp,
        &witness_json,
    );
    println!("Binary EXECUTE calldata: {} bytes", calldata.len());

    let mut gas_remaining: u64 = 1_000_000;
    let result = execute_precompile(
        &Bytes::from(calldata),
        &mut gas_remaining,
        ethrex_common::types::Fork::Prague,
    );
    match &result {
        Ok(output) => {
            assert_eq!(output.as_ref(), &[0x01], "Expected success byte 0x01");
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
        post_state_root: pre_state_root, // Doesn't matter — will fail before state check
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
///   - 512a5ca0: advance(bytes32,uint256,uint256,bytes,bytes)
///   - 57e871e7: blockNumber()
///   - f340fa01: deposit(address)
///   - 7b898939: depositIndex()
///   - a7932794: pendingDeposits(uint256)
///   - 9588eca2: stateRoot()
///
/// Source: crates/vm/levm/contracts/NativeRollup.sol
/// Compile: cd crates/vm/levm/contracts && solc --bin-runtime NativeRollup.sol -o build --overwrite
const NATIVE_ROLLUP_RUNTIME_HEX: &str = "608060405260043610610054575f3560e01c8063512a5ca01461005857806357e871e7146100805780637b898939146100aa5780639588eca2146100d4578063a7932794146100fe578063f340fa011461013b575b5f5ffd5b348015610063575f5ffd5b5061007e6004803603810190610079919061068b565b610157565b005b34801561008b575f5ffd5b50610094610427565b6040516100a19190610751565b60405180910390f35b3480156100b5575f5ffd5b506100be61042d565b6040516100cb9190610751565b60405180910390f35b3480156100df575f5ffd5b506100e8610433565b6040516100f59190610779565b60405180910390f35b348015610109575f5ffd5b50610124600480360381019061011f9190610792565b610438565b6040516101329291906107fc565b60405180910390f35b6101556004803603810190610150919061084d565b610486565b005b5f6003549050600280549050868261016f91906108a5565b11156101b0576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016101a790610932565b60405180910390fd5b5f5f5489886040516020016101c7939291906109b3565b60405160208183030381529060405290505f5f90505b8781101561028c575f600282856101f491906108a5565b81548110610205576102046109ef565b5b905f5260205f209060020201905082815f015f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff16826001015460405160200161024d929190610a81565b60405160208183030381529060405260405160200161026d929190610afe565b60405160208183030381529060405292505080806001019150506101dd565b5080868690506040516020016102a29190610b21565b604051602081830303815290604052878787876040516020016102ca96959493929190610b6d565b604051602081830303815290604052905086826102e791906108a5565b6003819055505f5f61010173ffffffffffffffffffffffffffffffffffffffff16836040516103169190610bae565b5f604051808303815f865af19150503d805f811461034f576040519150601f19603f3d011682016040523d82523d5f602084013e610354565b606091505b5091509150818015610367575060018151145b801561039657506001815f81518110610383576103826109ef565b5b602001015160f81c60f81b60f81c60ff16145b6103d5576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016103cc90610c34565b60405180910390fd5b8a5f8190555089600181905550897fe41acc52c5cd3ab398bfed63f4130976083bea5288e3bf4bf489ccbb3bd20c858c6040516104129190610779565b60405180910390a25050505050505050505050565b60015481565b60035481565b5f5481565b60028181548110610447575f80fd5b905f5260205f2090600202015f91509050805f015f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff16908060010154905082565b5f34116104c8576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016104bf90610c9c565b60405180910390fd5b600260405180604001604052808373ffffffffffffffffffffffffffffffffffffffff16815260200134815250908060018154018082558091505060019003905f5260205f2090600202015f909190919091505f820151815f015f6101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055506020820151816001015550508073ffffffffffffffffffffffffffffffffffffffff167f741a0277a612f71e4836430fe80cc831a4e28c01d2121c0ab1a4451bc88f909e346040516105b19190610751565b60405180910390a250565b5f5ffd5b5f5ffd5b5f819050919050565b6105d6816105c4565b81146105e0575f5ffd5b50565b5f813590506105f1816105cd565b92915050565b5f819050919050565b610609816105f7565b8114610613575f5ffd5b50565b5f8135905061062481610600565b92915050565b5f5ffd5b5f5ffd5b5f5ffd5b5f5f83601f84011261064b5761064a61062a565b5b8235905067ffffffffffffffff8111156106685761066761062e565b5b60208301915083600182028301111561068457610683610632565b5b9250929050565b5f5f5f5f5f5f5f60a0888a0312156106a6576106a56105bc565b5b5f6106b38a828b016105e3565b97505060206106c48a828b01610616565b96505060406106d58a828b01610616565b955050606088013567ffffffffffffffff8111156106f6576106f56105c0565b5b6107028a828b01610636565b9450945050608088013567ffffffffffffffff811115610725576107246105c0565b5b6107318a828b01610636565b925092505092959891949750929550565b61074b816105f7565b82525050565b5f6020820190506107645f830184610742565b92915050565b610773816105c4565b82525050565b5f60208201905061078c5f83018461076a565b92915050565b5f602082840312156107a7576107a66105bc565b5b5f6107b484828501610616565b91505092915050565b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6107e6826107bd565b9050919050565b6107f6816107dc565b82525050565b5f60408201905061080f5f8301856107ed565b61081c6020830184610742565b9392505050565b61082c816107dc565b8114610836575f5ffd5b50565b5f8135905061084781610823565b92915050565b5f60208284031215610862576108616105bc565b5b5f61086f84828501610839565b91505092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f6108af826105f7565b91506108ba836105f7565b92508282019050808211156108d2576108d1610878565b5b92915050565b5f82825260208201905092915050565b7f4e6f7420656e6f756768206465706f73697473000000000000000000000000005f82015250565b5f61091c6013836108d8565b9150610927826108e8565b602082019050919050565b5f6020820190508181035f83015261094981610910565b9050919050565b5f819050919050565b61096a610965826105c4565b610950565b82525050565b5f63ffffffff82169050919050565b5f8160e01b9050919050565b5f6109958261097f565b9050919050565b6109ad6109a882610970565b61098b565b82525050565b5f6109be8286610959565b6020820191506109ce8285610959565b6020820191506109de828461099c565b600482019150819050949350505050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b5f8160601b9050919050565b5f610a3282610a1c565b9050919050565b5f610a4382610a28565b9050919050565b610a5b610a56826107dc565b610a39565b82525050565b5f819050919050565b610a7b610a76826105f7565b610a61565b82525050565b5f610a8c8285610a4a565b601482019150610a9c8284610a6a565b6020820191508190509392505050565b5f81519050919050565b5f81905092915050565b8281835e5f83830152505050565b5f610ad882610aac565b610ae28185610ab6565b9350610af2818560208601610ac0565b80840191505092915050565b5f610b098285610ace565b9150610b158284610ace565b91508190509392505050565b5f610b2c828461099c565b60048201915081905092915050565b828183375f83830152505050565b5f610b548385610ab6565b9350610b61838584610b3b565b82840190509392505050565b5f610b788289610ace565b9150610b848288610ace565b9150610b91828688610b49565b9150610b9e828486610b49565b9150819050979650505050505050565b5f610bb98284610ace565b915081905092915050565b7f4558454355544520707265636f6d70696c6520766572696669636174696f6e205f8201527f6661696c65640000000000000000000000000000000000000000000000000000602082015250565b5f610c1e6026836108d8565b9150610c2982610bc4565b604082019050919050565b5f6020820190508181035f830152610c4b81610c12565b9050919050565b7f4d7573742073656e6420455448000000000000000000000000000000000000005f82015250565b5f610c86600d836108d8565b9150610c9182610c52565b602082019050919050565b5f6020820190508181035f830152610cb381610c7a565b905091905056fea264697066735822122040e0f068cbebd248a66c1109af1d33b2f050abf02ecd629724d7325a869dd42764736f6c634300081f0033";

/// NativeRollup.sol full deployment bytecode (constructor + runtime).
///
/// Constructor takes bytes32 _initialStateRoot as argument.
/// Deployment data = DEPLOY_BYTECODE + ABI-encoded constructor arg (32 bytes).
///
/// Compile: cd crates/vm/levm/contracts && solc --bin NativeRollup.sol -o build --overwrite
const NATIVE_ROLLUP_DEPLOY_HEX: &str = "6080604052348015600e575f5ffd5b50604051610d8e380380610d8e8339818101604052810190602e9190606b565b805f81905550506091565b5f5ffd5b5f819050919050565b604d81603d565b81146056575f5ffd5b50565b5f815190506065816046565b92915050565b5f60208284031215607d57607c6039565b5b5f6088848285016059565b91505092915050565b610cf08061009e5f395ff3fe608060405260043610610054575f3560e01c8063512a5ca01461005857806357e871e7146100805780637b898939146100aa5780639588eca2146100d4578063a7932794146100fe578063f340fa011461013b575b5f5ffd5b348015610063575f5ffd5b5061007e6004803603810190610079919061068b565b610157565b005b34801561008b575f5ffd5b50610094610427565b6040516100a19190610751565b60405180910390f35b3480156100b5575f5ffd5b506100be61042d565b6040516100cb9190610751565b60405180910390f35b3480156100df575f5ffd5b506100e8610433565b6040516100f59190610779565b60405180910390f35b348015610109575f5ffd5b50610124600480360381019061011f9190610792565b610438565b6040516101329291906107fc565b60405180910390f35b6101556004803603810190610150919061084d565b610486565b005b5f6003549050600280549050868261016f91906108a5565b11156101b0576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016101a790610932565b60405180910390fd5b5f5f5489886040516020016101c7939291906109b3565b60405160208183030381529060405290505f5f90505b8781101561028c575f600282856101f491906108a5565b81548110610205576102046109ef565b5b905f5260205f209060020201905082815f015f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff16826001015460405160200161024d929190610a81565b60405160208183030381529060405260405160200161026d929190610afe565b60405160208183030381529060405292505080806001019150506101dd565b5080868690506040516020016102a29190610b21565b604051602081830303815290604052878787876040516020016102ca96959493929190610b6d565b604051602081830303815290604052905086826102e791906108a5565b6003819055505f5f61010173ffffffffffffffffffffffffffffffffffffffff16836040516103169190610bae565b5f604051808303815f865af19150503d805f811461034f576040519150601f19603f3d011682016040523d82523d5f602084013e610354565b606091505b5091509150818015610367575060018151145b801561039657506001815f81518110610383576103826109ef565b5b602001015160f81c60f81b60f81c60ff16145b6103d5576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016103cc90610c34565b60405180910390fd5b8a5f8190555089600181905550897fe41acc52c5cd3ab398bfed63f4130976083bea5288e3bf4bf489ccbb3bd20c858c6040516104129190610779565b60405180910390a25050505050505050505050565b60015481565b60035481565b5f5481565b60028181548110610447575f80fd5b905f5260205f2090600202015f91509050805f015f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff16908060010154905082565b5f34116104c8576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016104bf90610c9c565b60405180910390fd5b600260405180604001604052808373ffffffffffffffffffffffffffffffffffffffff16815260200134815250908060018154018082558091505060019003905f5260205f2090600202015f909190919091505f820151815f015f6101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055506020820151816001015550508073ffffffffffffffffffffffffffffffffffffffff167f741a0277a612f71e4836430fe80cc831a4e28c01d2121c0ab1a4451bc88f909e346040516105b19190610751565b60405180910390a250565b5f5ffd5b5f5ffd5b5f819050919050565b6105d6816105c4565b81146105e0575f5ffd5b50565b5f813590506105f1816105cd565b92915050565b5f819050919050565b610609816105f7565b8114610613575f5ffd5b50565b5f8135905061062481610600565b92915050565b5f5ffd5b5f5ffd5b5f5ffd5b5f5f83601f84011261064b5761064a61062a565b5b8235905067ffffffffffffffff8111156106685761066761062e565b5b60208301915083600182028301111561068457610683610632565b5b9250929050565b5f5f5f5f5f5f5f60a0888a0312156106a6576106a56105bc565b5b5f6106b38a828b016105e3565b97505060206106c48a828b01610616565b96505060406106d58a828b01610616565b955050606088013567ffffffffffffffff8111156106f6576106f56105c0565b5b6107028a828b01610636565b9450945050608088013567ffffffffffffffff811115610725576107246105c0565b5b6107318a828b01610636565b925092505092959891949750929550565b61074b816105f7565b82525050565b5f6020820190506107645f830184610742565b92915050565b610773816105c4565b82525050565b5f60208201905061078c5f83018461076a565b92915050565b5f602082840312156107a7576107a66105bc565b5b5f6107b484828501610616565b91505092915050565b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6107e6826107bd565b9050919050565b6107f6816107dc565b82525050565b5f60408201905061080f5f8301856107ed565b61081c6020830184610742565b9392505050565b61082c816107dc565b8114610836575f5ffd5b50565b5f8135905061084781610823565b92915050565b5f60208284031215610862576108616105bc565b5b5f61086f84828501610839565b91505092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f6108af826105f7565b91506108ba836105f7565b92508282019050808211156108d2576108d1610878565b5b92915050565b5f82825260208201905092915050565b7f4e6f7420656e6f756768206465706f73697473000000000000000000000000005f82015250565b5f61091c6013836108d8565b9150610927826108e8565b602082019050919050565b5f6020820190508181035f83015261094981610910565b9050919050565b5f819050919050565b61096a610965826105c4565b610950565b82525050565b5f63ffffffff82169050919050565b5f8160e01b9050919050565b5f6109958261097f565b9050919050565b6109ad6109a882610970565b61098b565b82525050565b5f6109be8286610959565b6020820191506109ce8285610959565b6020820191506109de828461099c565b600482019150819050949350505050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b5f8160601b9050919050565b5f610a3282610a1c565b9050919050565b5f610a4382610a28565b9050919050565b610a5b610a56826107dc565b610a39565b82525050565b5f819050919050565b610a7b610a76826105f7565b610a61565b82525050565b5f610a8c8285610a4a565b601482019150610a9c8284610a6a565b6020820191508190509392505050565b5f81519050919050565b5f81905092915050565b8281835e5f83830152505050565b5f610ad882610aac565b610ae28185610ab6565b9350610af2818560208601610ac0565b80840191505092915050565b5f610b098285610ace565b9150610b158284610ace565b91508190509392505050565b5f610b2c828461099c565b60048201915081905092915050565b828183375f83830152505050565b5f610b548385610ab6565b9350610b61838584610b3b565b82840190509392505050565b5f610b788289610ace565b9150610b848288610ace565b9150610b91828688610b49565b9150610b9e828486610b49565b9150819050979650505050505050565b5f610bb98284610ace565b915081905092915050565b7f4558454355544520707265636f6d70696c6520766572696669636174696f6e205f8201527f6661696c65640000000000000000000000000000000000000000000000000000602082015250565b5f610c1e6026836108d8565b9150610c2982610bc4565b604082019050919050565b5f6020820190508181035f830152610c4b81610c12565b9050919050565b7f4d7573742073656e6420455448000000000000000000000000000000000000005f82015250565b5f610c86600d836108d8565b9150610c9182610c52565b602082019050919050565b5f6020820190508181035f830152610cb381610c7a565b905091905056fea264697066735822122040e0f068cbebd248a66c1109af1d33b2f050abf02ecd629724d7325a869dd42764736f6c634300081f0033";

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

/// Encode a call to NativeRollup.advance(bytes32, uint256, uint256, bytes, bytes).
///
/// ABI encoding:
///   - 4 bytes: function selector (0x512a5ca0)
///   - 32 bytes: _newStateRoot
///   - 32 bytes: _newBlockNumber
///   - 32 bytes: _depositsCount
///   - 32 bytes: offset to _block bytes
///   - 32 bytes: offset to _witness bytes
///   - dynamic: _block length + data (padded)
///   - dynamic: _witness length + data (padded)
fn encode_advance_call(
    new_state_root: H256,
    new_block_number: u64,
    deposits_count: u64,
    block_rlp: &[u8],
    witness_json: &[u8],
) -> Vec<u8> {
    let mut encoded = Vec::new();

    // Function selector: advance(bytes32,uint256,uint256,bytes,bytes) = 0x512a5ca0
    encoded.extend_from_slice(&[0x51, 0x2a, 0x5c, 0xa0]);

    // _newStateRoot (bytes32)
    encoded.extend_from_slice(new_state_root.as_bytes());

    // _newBlockNumber (uint256)
    let mut block_num_bytes = [0u8; 32];
    block_num_bytes[24..].copy_from_slice(&new_block_number.to_be_bytes());
    encoded.extend_from_slice(&block_num_bytes);

    // _depositsCount (uint256)
    let mut deposits_count_bytes = [0u8; 32];
    deposits_count_bytes[24..].copy_from_slice(&deposits_count.to_be_bytes());
    encoded.extend_from_slice(&deposits_count_bytes);

    // Offset to _block: 5 static params * 32 = 160 = 0xa0
    let mut block_offset = [0u8; 32];
    block_offset[31] = 0xa0;
    encoded.extend_from_slice(&block_offset);

    // Offset to _witness: 0xa0 + 32 (block length) + padded block data
    let padded_block_len = block_rlp.len() + ((32 - (block_rlp.len() % 32)) % 32);
    let witness_offset: u64 = 160 + 32 + padded_block_len as u64;
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

/// Build binary calldata for the EXECUTE precompile.
///
/// Format:
///   [32 bytes] pre_state_root
///   [32 bytes] post_state_root
///   [4  bytes] num_deposits (uint32 big-endian)
///   [52 * num_deposits] deposits (20 address + 32 amount each)
///   [4  bytes] block_rlp_length (uint32 big-endian)
///   [block_rlp_length] block RLP
///   [remaining] witness JSON
fn build_precompile_calldata(
    pre_state_root: H256,
    post_state_root: H256,
    deposits: &[Deposit],
    block_rlp: &[u8],
    witness_json: &[u8],
) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(pre_state_root.as_bytes());
    data.extend_from_slice(post_state_root.as_bytes());
    data.extend_from_slice(&(deposits.len() as u32).to_be_bytes());
    for deposit in deposits {
        data.extend_from_slice(deposit.address.as_bytes());
        data.extend_from_slice(&deposit.amount.to_big_endian());
    }
    data.extend_from_slice(&(block_rlp.len() as u32).to_be_bytes());
    data.extend_from_slice(block_rlp);
    data.extend_from_slice(witness_json);
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
        post_state_root,
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

    // === TX 2: advance(postStateRoot, 1, 1, blockRlp, witnessJson) ===
    let advance_calldata = encode_advance_call(post_state_root, 1, 1, &block_rlp, &witness_json);

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
