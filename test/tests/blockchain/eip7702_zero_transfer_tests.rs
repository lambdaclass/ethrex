use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        AuthorizationTuple, Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction,
        EIP7702Transaction, ELASTICITY_MULTIPLIER, GenesisAccount, Transaction, TxKind,
    },
    utils::keccak,
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{EngineType, Store};
use secp256k1::{Message as SecpMessage, SECP256K1, SecretKey};

const TEST_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";
const AUTHORITY_PRIVATE_KEY_BYTES: [u8; 32] = [0x42u8; 32];
const TEST_MAX_FEE_PER_GAS: u64 = 10_000_000_000;
const TEST_GAS_LIMIT: u64 = 100_000;
const EIP_7702_MAGIC: u8 = 0x05;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

async fn setup_store(sender: Address) -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let mut genesis: ethrex_common::types::Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");

    let chain_id = genesis.config.chain_id;

    genesis.alloc.insert(
        sender,
        GenesisAccount {
            balance: U256::from(10).pow(U256::from(20)),
            code: Bytes::new(),
            storage: Default::default(),
            nonce: 0,
        },
    );

    let mut store =
        Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");

    store
        .add_initial_state(genesis)
        .await
        .expect("Failed to add genesis state");

    (store, chain_id)
}

async fn build_block(store: &Store, blockchain: &Blockchain, parent_header: &BlockHeader) -> Block {
    let args = BuildPayloadArgs {
        parent: parent_header.hash(),
        timestamp: parent_header.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: None,
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        target_gas_limit: None,
    };

    let block = create_payload(&args, store, Bytes::new()).unwrap();
    let result = blockchain.build_payload(block).unwrap();
    result.payload
}

fn sign_auth_tuple(
    chain_id: u64,
    address: Address,
    nonce: u64,
    secret_key: &SecretKey,
) -> AuthorizationTuple {
    let mut rlp_buf = Vec::new();
    rlp_buf.push(EIP_7702_MAGIC);
    (U256::from(chain_id), address, nonce).encode(&mut rlp_buf);
    let hash = keccak(&rlp_buf);

    let msg = SecpMessage::from_digest(hash.0);
    let (recovery_id, sig) = SECP256K1
        .sign_ecdsa_recoverable(&msg, secret_key)
        .serialize_compact();

    let r = U256::from_big_endian(&sig[..32]);
    let s = U256::from_big_endian(&sig[32..64]);
    let y_parity = U256::from(Into::<i32>::into(recovery_id) as u64);

    AuthorizationTuple {
        chain_id: U256::from(chain_id),
        address,
        nonce,
        y_parity,
        r_signature: r,
        s_signature: s,
    }
}

async fn create_zero_value_tx(
    chain_id: u64,
    nonce: u64,
    to: Address,
    signer: &Signer,
) -> Transaction {
    let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        gas_limit: TEST_GAS_LIMIT,
        to: TxKind::Call(to),
        value: U256::zero(),
        data: Bytes::new(),
        ..Default::default()
    });
    tx.sign_inplace(signer).await.unwrap();
    tx
}

async fn create_eip7702_tx(
    chain_id: u64,
    nonce: u64,
    to: Address,
    auth_list: Vec<AuthorizationTuple>,
    signer: &Signer,
) -> Transaction {
    let mut tx = Transaction::EIP7702Transaction(EIP7702Transaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: TEST_MAX_FEE_PER_GAS,
        gas_limit: TEST_GAS_LIMIT,
        to,
        value: U256::zero(),
        data: Bytes::new(),
        access_list: vec![],
        authorization_list: auth_list,
        ..Default::default()
    });
    tx.sign_inplace(signer).await.unwrap();
    tx
}

async fn run_scenario(
    sender: Address,
    sender_signer: &Signer,
    authority_sk: &SecretKey,
    authority: Address,
    precede_with_zero_value: bool,
) -> u64 {
    let (store, chain_id) = setup_store(sender).await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    let mut sender_nonce = 0u64;
    let mut tx_index = 0usize;

    if precede_with_zero_value {
        let tx_zero = create_zero_value_tx(chain_id, sender_nonce, authority, sender_signer).await;
        sender_nonce += 1;
        tx_index += 1;
        blockchain
            .add_transaction_to_pool(tx_zero)
            .await
            .expect("zero-value tx should enter pool");
    }

    let auth_tuple = sign_auth_tuple(chain_id, sender, 0, authority_sk);
    let tx_7702 = create_eip7702_tx(
        chain_id,
        sender_nonce,
        sender,
        vec![auth_tuple],
        sender_signer,
    )
    .await;
    blockchain
        .add_transaction_to_pool(tx_7702)
        .await
        .expect("EIP-7702 tx should enter pool");

    let block = build_block(&store, &blockchain, &genesis_header).await;
    let expected_tx_count = tx_index + 1;
    assert_eq!(
        block.body.transactions.len(),
        expected_tx_count,
        "block must include all submitted transactions"
    );

    let block_number = block.header.number;
    let block_hash = block.hash();
    blockchain
        .add_block(block.clone())
        .expect("block should be valid");
    store
        .forkchoice_update(vec![], block_number, block_hash, None, None)
        .await
        .unwrap();

    let prev_cumulative = if tx_index == 0 {
        0
    } else {
        store
            .get_receipt(block_number, (tx_index - 1) as u64)
            .await
            .unwrap()
            .expect("preceding receipt should exist")
            .cumulative_gas_used
    };
    let receipt_7702 = store
        .get_receipt(block_number, tx_index as u64)
        .await
        .unwrap()
        .expect("EIP-7702 receipt should exist");

    assert!(receipt_7702.succeeded, "EIP-7702 tx must succeed");

    receipt_7702.cumulative_gas_used - prev_cumulative
}

#[tokio::test]
async fn zero_value_transfer_does_not_pollute_eip7702_authority_exists() {
    let sender_sk =
        SecretKey::from_slice(&hex::decode(TEST_PRIVATE_KEY).unwrap()).expect("valid sender key");
    let sender = LocalSigner::new(sender_sk).address;
    let sender_signer: Signer = LocalSigner::new(sender_sk).into();

    let authority_sk =
        SecretKey::from_slice(&AUTHORITY_PRIVATE_KEY_BYTES).expect("valid authority key");
    let authority = LocalSigner::new(authority_sk).address;
    assert_ne!(sender, authority, "sender and authority must differ");

    let gas_control = run_scenario(sender, &sender_signer, &authority_sk, authority, false).await;
    let gas_polluted = run_scenario(sender, &sender_signer, &authority_sk, authority, true).await;

    assert_eq!(
        gas_polluted, gas_control,
        "0-value transfer must not pollute authority.exists for the subsequent EIP-7702 auth: \
         control gas={gas_control}, polluted gas={gas_polluted}"
    );
}
