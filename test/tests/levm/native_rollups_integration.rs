//! Integration test for the NativeRollup contract on a real L1.
//!
//! Requires:
//!   1. Start L1: `NATIVE_ROLLUPS=1 make -C crates/l2 init-l1`
//!   2. Run: `cargo test -p ethrex-test --features native-rollups -- native_rollups_integration --ignored --nocapture`
//!
//! The test deploys NativeRollup.sol, builds an L2 state transition,
//! calls advance(), and verifies the contract state was updated.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::arithmetic_side_effects
)]

use std::collections::BTreeMap;

use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    types::{
        AccountState, Block, BlockBody, BlockHeader, ChainConfig, EIP1559Transaction, Receipt,
        Transaction, TxKind, TxType, block_execution_witness::ExecutionWitness,
    },
};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_l2_rpc::signer::{LocalSigner, Signer};
use ethrex_l2_sdk::{
    build_generic_tx, create_deploy, send_generic_transaction, wait_for_transaction_receipt,
};
use ethrex_levm::execute_precompile::{Deposit, ExecutePrecompileInput};
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::clients::eth::{EthClient, Overrides};
use ethrex_rpc::types::block_identifier::{BlockIdentifier, BlockTag};
use ethrex_trie::Trie;
use k256::ecdsa::{SigningKey, signature::hazmat::PrehashSigner};
use reqwest::Url;
use secp256k1::SecretKey;

const L1_RPC_URL: &str = "http://localhost:8545";
/// Private key from crates/l2/Makefile (pre-funded in L1 genesis).
const L1_PRIVATE_KEY: &str = "385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924";

/// NativeRollup.sol deployment bytecode (constructor + runtime).
/// Compiled with solc 0.8.31.
/// Source: crates/vm/levm/contracts/NativeRollup.sol
const NATIVE_ROLLUP_DEPLOY_HEX: &str = "6080604052348015600e575f5ffd5b506040516105603803806105608339818101604052810190602e9190606b565b805f81905550506091565b5f5ffd5b5f819050919050565b604d81603d565b81146056575f5ffd5b50565b5f815190506065816046565b92915050565b5f60208284031215607d57607c6039565b5b5f6088848285016059565b91505092915050565b6104c28061009e5f395ff3fe608060405234801561000f575f5ffd5b506004361061003f575f3560e01c806335eb2ee71461004357806357e871e71461005f5780639588eca21461007d575b5f5ffd5b61005d600480360381019061005891906102ac565b61009b565b005b6100676101d2565b604051610074919061032c565b60405180910390f35b6100856101d8565b6040516100929190610354565b60405180910390f35b5f5f61010173ffffffffffffffffffffffffffffffffffffffff1684846040516100c69291906103a9565b5f604051808303815f865af19150503d805f81146100ff576040519150601f19603f3d011682016040523d82523d5f602084013e610104565b606091505b5091509150818015610117575060018151145b801561014657506001815f81518110610133576101326103c1565b5b602001015160f81c60f81b60f81c60ff16145b610185576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161017c9061046e565b60405180910390fd5b855f8190555084600181905550847fe41acc52c5cd3ab398bfed63f4130976083bea5288e3bf4bf489ccbb3bd20c85876040516101c29190610354565b60405180910390a2505050505050565b60015481565b5f5481565b5f5ffd5b5f5ffd5b5f819050919050565b6101f7816101e5565b8114610201575f5ffd5b50565b5f81359050610212816101ee565b92915050565b5f819050919050565b61022a81610218565b8114610234575f5ffd5b50565b5f8135905061024581610221565b92915050565b5f5ffd5b5f5ffd5b5f5ffd5b5f5f83601f84011261026c5761026b61024b565b5b8235905067ffffffffffffffff8111156102895761028861024f565b5b6020830191508360018202830111156102a5576102a4610253565b5b9250929050565b5f5f5f5f606085870312156102c4576102c36101dd565b5b5f6102d187828801610204565b94505060206102e287828801610237565b935050604085013567ffffffffffffffff811115610303576103026101e1565b5b61030f87828801610257565b925092505092959194509250565b61032681610218565b82525050565b5f60208201905061033f5f83018461031d565b92915050565b61034e816101e5565b82525050565b5f6020820190506103675f830184610345565b92915050565b5f81905092915050565b828183375f83830152505050565b5f610390838561036d565b935061039d838584610377565b82840190509392505050565b5f6103b5828486610385565b91508190509392505050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b5f82825260208201905092915050565b7f4558454355544520707265636f6d70696c6520766572696669636174696f6e205f8201527f6661696c65640000000000000000000000000000000000000000000000000000602082015250565b5f6104586026836103ee565b9150610463826103fe565b604082019050919050565b5f6020820190508181035f8301526104858161044c565b905091905056fea2646970667358221220db626fb96e6dadcaa318411badc9db5f31ac790f8309b9bfe367af6e15bd680f64736f6c634300081f0033";

// ===== Helpers (duplicated from crates/vm/levm/tests/native_rollups.rs) =====

fn address_from_key(key: &SigningKey) -> Address {
    use k256::ecdsa::VerifyingKey;
    let verifying_key = VerifyingKey::from(key);
    let pubkey_bytes = verifying_key.to_encoded_point(false);
    let hash = keccak_hash(&pubkey_bytes.as_bytes()[1..]);
    Address::from_slice(&hash[12..])
}

fn sign_eip1559_tx(tx: &mut EIP1559Transaction, key: &SigningKey) {
    use ethrex_rlp::structs::Encoder;

    let mut buf = vec![0x02u8];
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

fn insert_account(trie: &mut Trie, address: Address, state: &AccountState) {
    let hashed_addr = keccak_hash(address.to_fixed_bytes()).to_vec();
    trie.insert(hashed_addr, state.encode_to_vec())
        .expect("trie insert failed");
}

fn get_trie_root_node(trie: &Trie) -> Option<ethrex_trie::Node> {
    trie.hash_no_commit();
    trie.root_node()
        .expect("root_node failed")
        .map(|arc_node| (*arc_node).clone())
}

/// Build the L2 state transition (Alice→Bob transfer + Charlie deposit).
fn build_l2_state_transition() -> (ExecutePrecompileInput, H256, H256) {
    let alice_key = SigningKey::from_bytes(&[1u8; 32].into()).expect("valid key");
    let alice = address_from_key(&alice_key);
    let bob = Address::from_low_u64_be(0xB0B);
    let charlie = Address::from_low_u64_be(0xC4A);
    let coinbase = Address::from_low_u64_be(0xC01);
    let chain_id: u64 = 1;
    let base_fee: u64 = 1_000_000_000;

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

    let parent_header = BlockHeader {
        number: 0,
        state_root: pre_state_root,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(base_fee),
        timestamp: 1_000_000,
        ..Default::default()
    };

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

    let gas_used: u64 = 21_000;
    let effective_gas_price: u64 = std::cmp::min(1_000_000_000 + base_fee, 2_000_000_000);
    let gas_cost = U256::from(gas_used) * U256::from(effective_gas_price);
    let priority_fee_per_gas: u64 = effective_gas_price.saturating_sub(base_fee);
    let coinbase_reward = U256::from(gas_used) * U256::from(priority_fee_per_gas);

    let transactions_root = ethrex_common::types::compute_transactions_root(&transactions);
    let receipt = Receipt::new(transaction.tx_type(), true, gas_used, vec![]);
    let receipts_root = ethrex_common::types::compute_receipts_root(&[receipt]);

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

    (input, pre_state_root, post_state_root)
}

/// Encode ABI: advance(bytes32 _newStateRoot, uint256 _newBlockNumber, bytes _precompileInput)
fn encode_advance_call(
    new_state_root: H256,
    new_block_number: u64,
    precompile_input: &[u8],
) -> Vec<u8> {
    let mut encoded = Vec::new();

    // Function selector: advance(bytes32,uint256,bytes) = 0x35eb2ee7
    encoded.extend_from_slice(&[0x35, 0xeb, 0x2e, 0xe7]);

    // _newStateRoot (bytes32)
    encoded.extend_from_slice(new_state_root.as_bytes());

    // _newBlockNumber (uint256, left-padded to 32 bytes)
    let mut block_num_bytes = [0u8; 32];
    block_num_bytes[24..].copy_from_slice(&new_block_number.to_be_bytes());
    encoded.extend_from_slice(&block_num_bytes);

    // Offset to bytes data: 3 static params * 32 = 96 = 0x60
    let mut offset = [0u8; 32];
    offset[31] = 0x60;
    encoded.extend_from_slice(&offset);

    // Length of bytes data
    let mut len = [0u8; 32];
    len[24..].copy_from_slice(&(precompile_input.len() as u64).to_be_bytes());
    encoded.extend_from_slice(&len);

    // Bytes data (padded to 32-byte boundary)
    encoded.extend_from_slice(precompile_input);
    let padding = (32 - (precompile_input.len() % 32)) % 32;
    encoded.resize(encoded.len() + padding, 0);

    encoded
}

/// Integration test: deploy NativeRollup on a real L1 and advance it with one block.
///
/// Prerequisites:
///   1. Start L1: `NATIVE_ROLLUPS=1 make -C crates/l2 init-l1`
///   2. Run: `cargo test -p ethrex-test --features native-rollups -- native_rollups_integration --ignored --nocapture`
#[tokio::test]
#[ignore = "requires running L1 (NATIVE_ROLLUPS=1 make -C crates/l2 init-l1)"]
async fn test_native_rollup_on_l1() {
    // 1. Connect to L1
    let eth_client = EthClient::new(Url::parse(L1_RPC_URL).unwrap()).unwrap();
    let secret_key =
        SecretKey::from_slice(&hex::decode(L1_PRIVATE_KEY).expect("invalid hex")).expect("key");
    let signer: Signer = LocalSigner::new(secret_key).into();

    println!("Connected to L1 at {L1_RPC_URL}");
    println!("Deployer: {:?}", signer.address());

    // 2. Build L2 state transition
    let (input, pre_state_root, post_state_root) = build_l2_state_transition();
    let precompile_input = serde_json::to_vec(&input).expect("JSON serialization failed");
    println!(
        "Serialized EXECUTE calldata: {} bytes",
        precompile_input.len()
    );

    // 3. Deploy NativeRollup(pre_state_root)
    let deploy_bytecode = hex::decode(NATIVE_ROLLUP_DEPLOY_HEX).expect("invalid hex");
    let constructor_arg = pre_state_root.as_bytes().to_vec();
    let init_code: Bytes = [deploy_bytecode, constructor_arg].concat().into();

    let (deploy_tx_hash, contract_address) =
        create_deploy(&eth_client, &signer, init_code, Overrides::default())
            .await
            .expect("Failed to deploy NativeRollup");

    let deploy_receipt = wait_for_transaction_receipt(deploy_tx_hash, &eth_client, 30)
        .await
        .expect("Deploy receipt not found");
    assert!(
        deploy_receipt.receipt.status,
        "NativeRollup deployment failed"
    );

    println!("NativeRollup deployed at: {contract_address:?}");
    println!("  Deploy tx: {deploy_tx_hash:?}");

    // 4. Verify initial state: stateRoot = pre_state_root
    let stored_root = eth_client
        .get_storage_at(
            contract_address,
            U256::zero(),
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_storage_at failed");
    assert_eq!(
        H256::from(stored_root.to_big_endian()),
        pre_state_root,
        "Initial stateRoot mismatch after deployment"
    );
    println!("  Initial stateRoot verified: {pre_state_root:?}");

    // 5. Call advance(post_state_root, 1, precompile_input)
    let advance_calldata = encode_advance_call(post_state_root, 1, &precompile_input);

    let advance_tx = build_generic_tx(
        &eth_client,
        TxType::EIP1559,
        contract_address,
        signer.address(),
        Bytes::from(advance_calldata),
        Overrides {
            gas_limit: Some(1_000_000_000), // 1B gas for the precompile
            ..Default::default()
        },
    )
    .await
    .expect("Failed to build advance tx");

    let advance_tx_hash = send_generic_transaction(&eth_client, advance_tx, &signer)
        .await
        .expect("Failed to send advance tx");

    let advance_receipt = wait_for_transaction_receipt(advance_tx_hash, &eth_client, 30)
        .await
        .expect("Advance receipt not found");
    assert!(
        advance_receipt.receipt.status,
        "NativeRollup.advance() reverted!"
    );

    println!("  advance() tx: {advance_tx_hash:?}");
    println!(
        "  Gas used: {}",
        advance_receipt.receipt.cumulative_gas_used
    );

    // 6. Verify updated state
    let stored_root = eth_client
        .get_storage_at(
            contract_address,
            U256::zero(),
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_storage_at failed");
    assert_eq!(
        H256::from(stored_root.to_big_endian()),
        post_state_root,
        "Post stateRoot mismatch"
    );

    let stored_block_num = eth_client
        .get_storage_at(
            contract_address,
            U256::from(1),
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_storage_at failed");
    assert_eq!(stored_block_num, U256::from(1), "blockNumber mismatch");

    println!("\nNativeRollup integration test passed!");
    println!("  Pre-state root:  {pre_state_root:?}");
    println!("  Post-state root: {post_state_root:?}");
    println!("  Block number:    1");
    println!("  Contract:        {contract_address:?}");
}
