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
const NATIVE_ROLLUP_DEPLOY_HEX: &str = "6080604052348015600e575f5ffd5b50604051610d8e380380610d8e8339818101604052810190602e9190606b565b805f81905550506091565b5f5ffd5b5f819050919050565b604d81603d565b81146056575f5ffd5b50565b5f815190506065816046565b92915050565b5f60208284031215607d57607c6039565b5b5f6088848285016059565b91505092915050565b610cf08061009e5f395ff3fe608060405260043610610054575f3560e01c8063512a5ca01461005857806357e871e7146100805780637b898939146100aa5780639588eca2146100d4578063a7932794146100fe578063f340fa011461013b575b5f5ffd5b348015610063575f5ffd5b5061007e6004803603810190610079919061068b565b610157565b005b34801561008b575f5ffd5b50610094610427565b6040516100a19190610751565b60405180910390f35b3480156100b5575f5ffd5b506100be61042d565b6040516100cb9190610751565b60405180910390f35b3480156100df575f5ffd5b506100e8610433565b6040516100f59190610779565b60405180910390f35b348015610109575f5ffd5b50610124600480360381019061011f9190610792565b610438565b6040516101329291906107fc565b60405180910390f35b6101556004803603810190610150919061084d565b610486565b005b5f6003549050600280549050868261016f91906108a5565b11156101b0576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016101a790610932565b60405180910390fd5b5f5f5489886040516020016101c7939291906109b3565b60405160208183030381529060405290505f5f90505b8781101561028c575f600282856101f491906108a5565b81548110610205576102046109ef565b5b905f5260205f209060020201905082815f015f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff16826001015460405160200161024d929190610a81565b60405160208183030381529060405260405160200161026d929190610afe565b60405160208183030381529060405292505080806001019150506101dd565b5080868690506040516020016102a29190610b21565b604051602081830303815290604052878787876040516020016102ca96959493929190610b6d565b604051602081830303815290604052905086826102e791906108a5565b6003819055505f5f61010173ffffffffffffffffffffffffffffffffffffffff16836040516103169190610bae565b5f604051808303815f865af19150503d805f811461034f576040519150601f19603f3d011682016040523d82523d5f602084013e610354565b606091505b5091509150818015610367575060018151145b801561039657506001815f81518110610383576103826109ef565b5b602001015160f81c60f81b60f81c60ff16145b6103d5576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016103cc90610c34565b60405180910390fd5b8a5f8190555089600181905550897fe41acc52c5cd3ab398bfed63f4130976083bea5288e3bf4bf489ccbb3bd20c858c6040516104129190610779565b60405180910390a25050505050505050505050565b60015481565b60035481565b5f5481565b60028181548110610447575f80fd5b905f5260205f2090600202015f91509050805f015f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff16908060010154905082565b5f34116104c8576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016104bf90610c9c565b60405180910390fd5b600260405180604001604052808373ffffffffffffffffffffffffffffffffffffffff16815260200134815250908060018154018082558091505060019003905f5260205f2090600202015f909190919091505f820151815f015f6101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055506020820151816001015550508073ffffffffffffffffffffffffffffffffffffffff167f741a0277a612f71e4836430fe80cc831a4e28c01d2121c0ab1a4451bc88f909e346040516105b19190610751565b60405180910390a250565b5f5ffd5b5f5ffd5b5f819050919050565b6105d6816105c4565b81146105e0575f5ffd5b50565b5f813590506105f1816105cd565b92915050565b5f819050919050565b610609816105f7565b8114610613575f5ffd5b50565b5f8135905061062481610600565b92915050565b5f5ffd5b5f5ffd5b5f5ffd5b5f5f83601f84011261064b5761064a61062a565b5b8235905067ffffffffffffffff8111156106685761066761062e565b5b60208301915083600182028301111561068457610683610632565b5b9250929050565b5f5f5f5f5f5f5f60a0888a0312156106a6576106a56105bc565b5b5f6106b38a828b016105e3565b97505060206106c48a828b01610616565b96505060406106d58a828b01610616565b955050606088013567ffffffffffffffff8111156106f6576106f56105c0565b5b6107028a828b01610636565b9450945050608088013567ffffffffffffffff811115610725576107246105c0565b5b6107318a828b01610636565b925092505092959891949750929550565b61074b816105f7565b82525050565b5f6020820190506107645f830184610742565b92915050565b610773816105c4565b82525050565b5f60208201905061078c5f83018461076a565b92915050565b5f602082840312156107a7576107a66105bc565b5b5f6107b484828501610616565b91505092915050565b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f6107e6826107bd565b9050919050565b6107f6816107dc565b82525050565b5f60408201905061080f5f8301856107ed565b61081c6020830184610742565b9392505050565b61082c816107dc565b8114610836575f5ffd5b50565b5f8135905061084781610823565b92915050565b5f60208284031215610862576108616105bc565b5b5f61086f84828501610839565b91505092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f6108af826105f7565b91506108ba836105f7565b92508282019050808211156108d2576108d1610878565b5b92915050565b5f82825260208201905092915050565b7f4e6f7420656e6f756768206465706f73697473000000000000000000000000005f82015250565b5f61091c6013836108d8565b9150610927826108e8565b602082019050919050565b5f6020820190508181035f83015261094981610910565b9050919050565b5f819050919050565b61096a610965826105c4565b610950565b82525050565b5f63ffffffff82169050919050565b5f8160e01b9050919050565b5f6109958261097f565b9050919050565b6109ad6109a882610970565b61098b565b82525050565b5f6109be8286610959565b6020820191506109ce8285610959565b6020820191506109de828461099c565b600482019150819050949350505050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b5f8160601b9050919050565b5f610a3282610a1c565b9050919050565b5f610a4382610a28565b9050919050565b610a5b610a56826107dc565b610a39565b82525050565b5f819050919050565b610a7b610a76826105f7565b610a61565b82525050565b5f610a8c8285610a4a565b601482019150610a9c8284610a6a565b6020820191508190509392505050565b5f81519050919050565b5f81905092915050565b8281835e5f83830152505050565b5f610ad882610aac565b610ae28185610ab6565b9350610af2818560208601610ac0565b80840191505092915050565b5f610b098285610ace565b9150610b158284610ace565b91508190509392505050565b5f610b2c828461099c565b60048201915081905092915050565b828183375f83830152505050565b5f610b548385610ab6565b9350610b61838584610b3b565b82840190509392505050565b5f610b788289610ace565b9150610b848288610ace565b9150610b91828688610b49565b9150610b9e828486610b49565b9150819050979650505050505050565b5f610bb98284610ace565b915081905092915050565b7f4558454355544520707265636f6d70696c6520766572696669636174696f6e205f8201527f6661696c65640000000000000000000000000000000000000000000000000000602082015250565b5f610c1e6026836108d8565b9150610c2982610bc4565b604082019050919050565b5f6020820190508181035f830152610c4b81610c12565b9050919050565b7f4d7573742073656e6420455448000000000000000000000000000000000000005f82015250565b5f610c86600d836108d8565b9150610c9182610c52565b602082019050919050565b5f6020820190508181035f830152610cb381610c7a565b905091905056fea264697066735822122040e0f068cbebd248a66c1109af1d33b2f050abf02ecd629724d7325a869dd42764736f6c634300081f0033";

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
fn build_l2_state_transition() -> (ExecutePrecompileInput, Vec<u8>, Vec<u8>, H256, H256) {
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

    (input, block_rlp, witness_json, pre_state_root, post_state_root)
}

/// Encode a call to NativeRollup.deposit(address).
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

/// Encode ABI: advance(bytes32, uint256, uint256, bytes, bytes)
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
    let (_input, block_rlp, witness_json, pre_state_root, post_state_root) =
        build_l2_state_transition();
    let charlie = Address::from_low_u64_be(0xC4A);
    let deposit_amount = U256::from(5) * U256::from(10).pow(U256::from(18)); // 5 ETH

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

    // 5. Call deposit(charlie) with 5 ETH
    let deposit_calldata = encode_deposit_call(charlie);

    let deposit_tx = build_generic_tx(
        &eth_client,
        TxType::EIP1559,
        contract_address,
        signer.address(),
        Bytes::from(deposit_calldata),
        Overrides {
            value: Some(deposit_amount),
            ..Default::default()
        },
    )
    .await
    .expect("Failed to build deposit tx");

    let deposit_tx_hash = send_generic_transaction(&eth_client, deposit_tx, &signer)
        .await
        .expect("Failed to send deposit tx");

    let deposit_receipt = wait_for_transaction_receipt(deposit_tx_hash, &eth_client, 30)
        .await
        .expect("Deposit receipt not found");
    assert!(
        deposit_receipt.receipt.status,
        "NativeRollup.deposit() reverted!"
    );
    println!("  deposit() tx: {deposit_tx_hash:?}");

    // 6. Call advance(post_state_root, 1, 1, block_rlp, witness_json)
    let advance_calldata =
        encode_advance_call(post_state_root, 1, 1, &block_rlp, &witness_json);

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

    // 7. Verify updated state
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

    let stored_deposit_index = eth_client
        .get_storage_at(
            contract_address,
            U256::from(3),
            BlockIdentifier::Tag(BlockTag::Latest),
        )
        .await
        .expect("get_storage_at failed");
    assert_eq!(
        stored_deposit_index,
        U256::from(1),
        "depositIndex mismatch"
    );

    println!("\nNativeRollup integration test passed!");
    println!("  Pre-state root:  {pre_state_root:?}");
    println!("  Post-state root: {post_state_root:?}");
    println!("  Block number:    1");
    println!("  Deposit index:   1");
    println!("  Contract:        {contract_address:?}");
}
