//! BUG-834: a type-4 transaction that runs out of gas while applying its
//! authorization must not put its recipient in the Block Access List.
//!
//! EIP-7928 pre-state validation happens before any state access, and since
//! `tests-glamsterdam-devnet@v7.1.0` EELS loads the recipient only in the
//! top-frame `prepare_dispatch` — which an EIP-7702 authorization halt
//! precedes. The recipient is therefore never accessed and must be absent.
//!
//! The VM honours this via `pending_prep_oog` (see `default_hook.rs`), so
//! ethrex *validates* such a block correctly. The block *builder* pre-records
//! the recipient before executing the transaction, so it produces a BAL the
//! validator rejects: ethrex destroys its own payloads.

use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        AuthorizationTuple, DEFAULT_BUILDER_GAS_CEIL, EIP7702Transaction, ELASTICITY_MULTIPLIER,
        Genesis, Transaction, block_access_list::BlockAccessList,
    },
    utils::keccak,
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{EngineType, Store};
use secp256k1::{Message as SecpMessage, SECP256K1, SecretKey};

const SENDER_PRIVATE_KEY: &str = "850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c";
const AUTHORITY_PRIVATE_KEY_BYTES: [u8; 32] = [0x42u8; 32];
const EIP_7702_MAGIC: u8 = 0x05;

/// The trigger from the report: enough gas to pay the 22,816 intrinsic cost but
/// not the 9,000-gas `ACCOUNT_WRITE` the authorization then needs.
const TRIGGER_GAS_LIMIT: u64 = 30_000;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// An address that appears nowhere in genesis and is touched by nothing else in
/// the block, so its presence in the BAL can only come from the recipient path.
fn recipient_address() -> Address {
    Address::from_low_u64_be(0xBA1_006)
}

async fn setup_store() -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-bal-content.json"))
        .expect("open l1-bal-content genesis");
    let genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-bal-content genesis");
    let chain_id = genesis.config.chain_id;
    let mut store = Store::new("store.db", EngineType::InMemory).expect("build in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    (store, chain_id)
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
    AuthorizationTuple {
        chain_id: U256::from(chain_id),
        address,
        nonce,
        y_parity: U256::from(Into::<i32>::into(recovery_id) as u64),
        r_signature: U256::from_big_endian(&sig[..32]),
        s_signature: U256::from_big_endian(&sig[32..64]),
    }
}

fn bal_contains(bal: &BlockAccessList, address: Address) -> bool {
    bal.accounts().iter().any(|a| a.address == address)
}

#[tokio::test]
async fn auth_oog_tx_must_not_put_its_recipient_in_the_bal() {
    let sender_sk = SecretKey::from_slice(&hex::decode(SENDER_PRIVATE_KEY).unwrap()).unwrap();
    let sender_signer: Signer = LocalSigner::new(sender_sk).into();
    let authority_sk = SecretKey::from_slice(&AUTHORITY_PRIVATE_KEY_BYTES).unwrap();

    let (store, chain_id) = setup_store().await;
    let blockchain = Blockchain::default_with_store(store.clone());
    let genesis_header = store.get_block_header(0).unwrap().unwrap();

    // Type-4 SetCode tx: one valid authorization, recipient never reached.
    let auth = sign_auth_tuple(
        chain_id,
        LocalSigner::new(sender_sk).address,
        0,
        &authority_sk,
    );
    let mut tx = Transaction::EIP7702Transaction(EIP7702Transaction {
        chain_id,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 10_000_000_000,
        gas_limit: TRIGGER_GAS_LIMIT,
        to: recipient_address(),
        value: U256::zero(),
        data: Bytes::new(),
        access_list: vec![],
        authorization_list: vec![auth],
        ..Default::default()
    });
    tx.sign_inplace(&sender_signer).await.unwrap();
    blockchain.add_transaction_to_pool(tx).await.unwrap();

    let args = BuildPayloadArgs {
        parent: genesis_header.hash(),
        timestamp: genesis_header.timestamp + 12,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: Some(1),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let payload = create_payload(&args, &store, Bytes::new()).unwrap();
    let result = blockchain.build_payload(payload).unwrap();
    let bal = result
        .block_access_list
        .expect("amsterdam block must produce a BAL");

    // Establish that we actually hit the intended path: the tx is included and
    // burned its whole gas limit. A passing assertion here with a different
    // gas figure would mean the trigger shape has drifted, not that the bug is
    // fixed.
    assert_eq!(
        result.payload.body.transactions.len(),
        1,
        "trigger tx must be included in the block"
    );
    let receipt = &result.receipts[0];
    assert!(
        !receipt.succeeded,
        "trigger tx must fail (OOG during authorization processing)"
    );
    assert_eq!(
        receipt.cumulative_gas_used, TRIGGER_GAS_LIMIT,
        "an OOG must burn the whole gas limit; got {} — trigger shape may have drifted",
        receipt.cumulative_gas_used
    );

    assert!(
        !bal_contains(&bal, recipient_address()),
        "BUG-834: builder put the recipient {:?} in the BAL for a tx that OOG'd during \
         authorization processing, before the recipient was ever accessed. EIP-7928 requires \
         it to be absent, and ethrex's own validator omits it — so this BAL fails validation \
         and the payload is destroyed. BAL accounts: {:?}",
        recipient_address(),
        bal.accounts().iter().map(|a| a.address).collect::<Vec<_>>()
    );
}
