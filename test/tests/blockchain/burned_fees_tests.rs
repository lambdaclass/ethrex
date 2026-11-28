//! EIP-8079 `burned_fees`: a header field that exists from LStar on. A header carrying it
//! before LStar, or lacking it from LStar on, does not match its fork's schema, and at
//! LStar its value must equal the fees execution burned.

use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    error::ChainError,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    H160, H256,
    types::{
        Block, BlockHeader, ChainConfig, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, Genesis,
    },
    validate_burned_fees,
};
use ethrex_storage::{EngineType, Store};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// An Amsterdam-at-genesis store, with LStar (and Hegotá before it) also active at
/// genesis when `lstar` is set.
async fn setup_store(lstar: bool) -> Store {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-bal.json"))
        .expect("open l1-bal genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-bal genesis");
    if lstar {
        genesis.config.hegota_time = Some(0);
        genesis.config.lstar_time = Some(0);
    }
    let mut store = Store::new("store.db", EngineType::InMemory).expect("in-memory store");
    store.add_initial_state(genesis).await.expect("genesis");
    store
}

/// A valid empty block on genesis, built by ethrex's builder (which sets `burned_fees`
/// at LStar).
fn build_block(store: &Store) -> Block {
    let blockchain = Blockchain::new(store.clone(), BlockchainOptions::default());
    let genesis_header = store.get_block_header(0).unwrap().unwrap();
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
    let payload = create_payload(&args, store, Bytes::new()).unwrap();
    blockchain.build_payload(payload).unwrap().payload
}

fn with_burned_fees(mut block: Block, burned_fees: Option<u64>) -> Block {
    block.header.burned_fees = burned_fees;
    block.header.hash = Default::default();
    block
}

fn import(store: Store, block: Block) -> Result<(), String> {
    Blockchain::new(store, BlockchainOptions::default())
        .add_block_pipeline(block, None)
        .map_err(|e: ChainError| format!("{e:?}"))
}

/// An Amsterdam block whose header carries a 24th field is rejected before execution.
#[tokio::test]
async fn amsterdam_block_with_burned_fees_is_rejected() {
    let store = setup_store(false).await;
    let block = build_block(&store);
    assert_eq!(
        block.header.burned_fees, None,
        "the builder adds no LStar field"
    );

    let err = import(store, with_burned_fees(block, Some(1)))
        .expect_err("a pre-LStar header carrying burned_fees is schema-invalid");
    assert!(err.contains("BurnedFeesPresent"), "got: {err}");
}

/// An LStar block imports with the burned fees it commits to, and is rejected when it
/// claims another value or none.
#[tokio::test]
async fn lstar_block_burned_fees_are_authenticated() {
    let store = setup_store(true).await;
    let block = build_block(&store);
    let committed = block
        .header
        .burned_fees
        .expect("the builder sets burned_fees at LStar");

    import(setup_store(true).await, block.clone()).expect("the builder's block is valid");

    let err = import(
        setup_store(true).await,
        with_burned_fees(block.clone(), Some(committed + 1)),
    )
    .expect_err("a wrong burned_fees value must be rejected");
    assert!(err.contains("BurnedFeesMismatch"), "got: {err}");

    let err = import(setup_store(true).await, with_burned_fees(block, None))
        .expect_err("an LStar header without burned_fees must be rejected");
    assert!(err.contains("BurnedFeesMismatch"), "got: {err}");
}

/// The post-execution comparison only applies at LStar, where a header without the field
/// fails it too.
#[test]
fn validate_burned_fees_compares_only_at_lstar() {
    let lstar = ChainConfig {
        cancun_time: Some(0),
        prague_time: Some(0),
        osaka_time: Some(0),
        amsterdam_time: Some(0),
        hegota_time: Some(0),
        lstar_time: Some(0),
        ..Default::default()
    };
    let amsterdam = ChainConfig {
        hegota_time: None,
        lstar_time: None,
        ..lstar
    };
    let header = BlockHeader {
        burned_fees: Some(5),
        ..Default::default()
    };

    validate_burned_fees(&header, &lstar, Some(5)).expect("equal values pass");
    assert!(validate_burned_fees(&header, &lstar, Some(6)).is_err());
    assert!(validate_burned_fees(&header, &lstar, None).is_err());
    validate_burned_fees(&header, &amsterdam, None).expect("not compared before LStar");
}
