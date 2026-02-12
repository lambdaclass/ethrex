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
const NATIVE_ROLLUP_DEPLOY_HEX: &str = "6080604052348015600e575f5ffd5b50604051610e60380380610e608339818101604052810190602e9190606b565b805f81905550506091565b5f5ffd5b5f819050919050565b604d81603d565b81146056575f5ffd5b50565b5f815190506065816046565b92915050565b5f60208284031215607d57607c6039565b5b5f6088848285016059565b91505092915050565b610dc28061009e5f395ff3fe608060405260043610610058575f3560e01c806357e871e7146101965780637b898939146101c05780639588eca2146101ea578063a793279414610214578063ed3133f214610251578063f340fa011461027957610192565b36610192575f341161009f576040517f08c379a0000000000000000000000000000000000000000000000000000000008152600401610096906106fa565b60405180910390fd5b600260405180604001604052803373ffffffffffffffffffffffffffffffffffffffff16815260200134815250908060018154018082558091505060019003905f5260205f2090600202015f909190919091505f820151815f015f6101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055506020820151816001015550503373ffffffffffffffffffffffffffffffffffffffff167f741a0277a612f71e4836430fe80cc831a4e28c01d2121c0ab1a4451bc88f909e346040516101889190610730565b60405180910390a2005b5f5ffd5b3480156101a1575f5ffd5b506101aa610295565b6040516101b79190610730565b60405180910390f35b3480156101cb575f5ffd5b506101d461029b565b6040516101e19190610730565b60405180910390f35b3480156101f5575f5ffd5b506101fe6102a1565b60405161020b9190610761565b60405180910390f35b34801561021f575f5ffd5b5061023a600480360381019061023591906107ac565b6102a6565b604051610248929190610816565b60405180910390f35b34801561025c575f5ffd5b506102776004803603810190610272919061089e565b6102f4565b005b610293600480360381019061028e9190610959565b61056a565b005b60015481565b60035481565b5f5481565b600281815481106102b5575f80fd5b905f5260205f2090600202015f91509050805f015f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff16908060010154905082565b5f6003549050600280549050868261030c91906109b1565b111561034d576040517f08c379a000000000000000000000000000000000000000000000000000000000815260040161034490610a2e565b60405180910390fd5b60605f5f90505b87811015610403575f6002828561036b91906109b1565b8154811061037c5761037b610a4c565b5b905f5260205f209060020201905082815f015f9054906101000a900473ffffffffffffffffffffffffffffffffffffffff1682600101546040516020016103c4929190610ade565b6040516020818303038152906040526040516020016103e4929190610b5b565b6040516020818303038152906040529250508080600101915050610354565b50868261041091906109b1565b6003819055505f5f54878787878660405160200161043396959493929190610c10565b60405160208183030381529060405290505f5f61010173ffffffffffffffffffffffffffffffffffffffff168360405161046d9190610c6c565b5f604051808303815f865af19150503d805f81146104a6576040519150601f19603f3d011682016040523d82523d5f602084013e6104ab565b606091505b50915091508180156104be575060408151145b6104fd576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016104f490610cf2565b60405180910390fd5b5f5f828060200190518101906105139190610d4e565b91509150815f8190555080600181905550807fe41acc52c5cd3ab398bfed63f4130976083bea5288e3bf4bf489ccbb3bd20c85836040516105549190610761565b60405180910390a2505050505050505050505050565b5f34116105ac576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004016105a3906106fa565b60405180910390fd5b600260405180604001604052808373ffffffffffffffffffffffffffffffffffffffff16815260200134815250908060018154018082558091505060019003905f5260205f2090600202015f909190919091505f820151815f015f6101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055506020820151816001015550508073ffffffffffffffffffffffffffffffffffffffff167f741a0277a612f71e4836430fe80cc831a4e28c01d2121c0ab1a4451bc88f909e346040516106959190610730565b60405180910390a250565b5f82825260208201905092915050565b7f4d7573742073656e6420455448000000000000000000000000000000000000005f82015250565b5f6106e4600d836106a0565b91506106ef826106b0565b602082019050919050565b5f6020820190508181035f830152610711816106d8565b9050919050565b5f819050919050565b61072a81610718565b82525050565b5f6020820190506107435f830184610721565b92915050565b5f819050919050565b61075b81610749565b82525050565b5f6020820190506107745f830184610752565b92915050565b5f5ffd5b5f5ffd5b61078b81610718565b8114610795575f5ffd5b50565b5f813590506107a681610782565b92915050565b5f602082840312156107c1576107c061077a565b5b5f6107ce84828501610798565b91505092915050565b5f73ffffffffffffffffffffffffffffffffffffffff82169050919050565b5f610800826107d7565b9050919050565b610810816107f6565b82525050565b5f6040820190506108295f830185610807565b6108366020830184610721565b9392505050565b5f5ffd5b5f5ffd5b5f5ffd5b5f5f83601f84011261085e5761085d61083d565b5b8235905067ffffffffffffffff81111561087b5761087a610841565b5b60208301915083600182028301111561089757610896610845565b5b9250929050565b5f5f5f5f5f606086880312156108b7576108b661077a565b5b5f6108c488828901610798565b955050602086013567ffffffffffffffff8111156108e5576108e461077e565b5b6108f188828901610849565b9450945050604086013567ffffffffffffffff8111156109145761091361077e565b5b61092088828901610849565b92509250509295509295909350565b610938816107f6565b8114610942575f5ffd5b50565b5f813590506109538161092f565b92915050565b5f6020828403121561096e5761096d61077a565b5b5f61097b84828501610945565b91505092915050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52601160045260245ffd5b5f6109bb82610718565b91506109c683610718565b92508282019050808211156109de576109dd610984565b5b92915050565b7f4e6f7420656e6f756768206465706f73697473000000000000000000000000005f82015250565b5f610a186013836106a0565b9150610a23826109e4565b602082019050919050565b5f6020820190508181035f830152610a4581610a0c565b9050919050565b7f4e487b71000000000000000000000000000000000000000000000000000000005f52603260045260245ffd5b5f8160601b9050919050565b5f610a8f82610a79565b9050919050565b5f610aa082610a85565b9050919050565b610ab8610ab3826107f6565b610a96565b82525050565b5f819050919050565b610ad8610ad382610718565b610abe565b82525050565b5f610ae98285610aa7565b601482019150610af98284610ac7565b6020820191508190509392505050565b5f81519050919050565b5f81905092915050565b8281835e5f83830152505050565b5f610b3582610b09565b610b3f8185610b13565b9350610b4f818560208601610b1d565b80840191505092915050565b5f610b668285610b2b565b9150610b728284610b2b565b91508190509392505050565b5f82825260208201905092915050565b828183375f83830152505050565b5f601f19601f8301169050919050565b5f610bb78385610b7e565b9350610bc4838584610b8e565b610bcd83610b9c565b840190509392505050565b5f610be282610b09565b610bec8185610b7e565b9350610bfc818560208601610b1d565b610c0581610b9c565b840191505092915050565b5f608082019050610c235f830189610752565b8181036020830152610c36818789610bac565b90508181036040830152610c4b818587610bac565b90508181036060830152610c5f8184610bd8565b9050979650505050505050565b5f610c778284610b2b565b915081905092915050565b7f4558454355544520707265636f6d70696c6520766572696669636174696f6e205f8201527f6661696c65640000000000000000000000000000000000000000000000000000602082015250565b5f610cdc6026836106a0565b9150610ce782610c82565b604082019050919050565b5f6020820190508181035f830152610d0981610cd0565b9050919050565b610d1981610749565b8114610d23575f5ffd5b50565b5f81519050610d3481610d10565b92915050565b5f81519050610d4881610782565b92915050565b5f5f60408385031215610d6457610d6361077a565b5b5f610d7185828601610d26565b9250506020610d8285828601610d3a565b915050925092905056fea264697066735822122032f2db442daf298b9e99572230200f4a1f4248d37377159a59baf92b1f49ba4f64736f6c634300081f0033";

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

/// Encode ABI: advance(uint256, bytes, bytes)
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

    // 6. Call advance(1, block_rlp, witness_json)
    let advance_calldata = encode_advance_call(1, &block_rlp, &witness_json);

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
    assert_eq!(stored_deposit_index, U256::from(1), "depositIndex mismatch");

    println!("\nNativeRollup integration test passed!");
    println!("  Pre-state root:  {pre_state_root:?}");
    println!("  Post-state root: {post_state_root:?}");
    println!("  Block number:    1");
    println!("  Deposit index:   1");
    println!("  Contract:        {contract_address:?}");
}
