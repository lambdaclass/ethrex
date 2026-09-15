//! FOCIL Profile 2 end to end: real blocks built by the payload builder and
//! imported through `add_block_pipeline_with_il`, with an EIP-8141 frame
//! transaction in the inclusion list, in both directions of the verdict.
//!
//! The senders are contracts whose runtime is `APPROVE(3)` (approve execution
//! and payment for themselves), so the transactions need no signature and the
//! tests stay focused on the omission rule. Case numbers refer to the Test
//! Cases table of `docs/eip-focil-frametx.md`.

use std::collections::HashSet;
use std::{fs::File, io::BufReader, path::PathBuf};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    error::ChainError,
    payload::{BuildPayloadArgs, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        Block, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, ELASTICITY_MULTIPLIER, Frame, FrameMode,
        FrameTransaction, Genesis, GenesisAccount, Transaction,
    },
    validation::BlockValidationContext,
};
use ethrex_crypto::NativeCrypto;
use ethrex_storage::{EngineType, Store};

/// Runtime `APPROVE(3)`: `PUSH1 3, PUSH1 0, PUSH1 0, APPROVE`.
const APPROVE_BOTH_CODE: &[u8] = &[0x60, 0x03, 0x60, 0x00, 0x60, 0x00, 0xAA];
const FRAME_SENDER: Address = Address::repeat_byte(0xC5);
/// A sender whose approval reads storage slot 3, the last one inside the
/// surface at `AA_VOPS_SLOT_COUNT = 4`.
const SLOT_3_READER: Address = Address::repeat_byte(0xC6);
/// A sender whose approval reads storage slot 4, the first one outside.
const SLOT_4_READER: Address = Address::repeat_byte(0xC7);
const TEST_MAX_FEE_PER_GAS: u64 = 10_000_000_000;
/// EIP-8037 `STATE_BYTES_PER_NEW_ACCOUNT * CPSB`: what a frame pays to create the
/// account it funds.
const NEW_ACCOUNT_STATE_GAS: u64 = 120 * 1530;
/// EIP-8250 `KEYED_NONCE_FIRST_USE_STATE_GAS`: what the approving frame pays to
/// create a fresh key's NONCE_MANAGER slot.
const KEYED_NONCE_FIRST_USE_STATE_GAS: u64 = 64 * 1530;
/// 100 ETH.
fn funded() -> U256 {
    U256::from(10u64).pow(U256::from(20u64))
}

/// `PUSH1 slot, SLOAD, POP` then `APPROVE(3)`.
fn sload_then_approve_code(slot: u8) -> Bytes {
    let mut code = vec![0x60, slot, 0x54, 0x50];
    code.extend_from_slice(APPROVE_BOTH_CODE);
    Bytes::from(code)
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// The execution-api genesis with Hegotá active from genesis and the three
/// contract senders funded. Returns the store and the chain id.
async fn setup_store(store_name: &str, hegota: bool) -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/execution-api.json"))
        .expect("open genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse genesis");
    if hegota {
        // The fork schedule is validated at genesis: Hegotá requires Amsterdam.
        genesis.config.amsterdam_time = Some(0);
        genesis.config.hegota_time = Some(0);
    }
    for (address, code) in [
        (FRAME_SENDER, Bytes::from_static(APPROVE_BOTH_CODE)),
        (SLOT_3_READER, sload_then_approve_code(3)),
        (SLOT_4_READER, sload_then_approve_code(4)),
    ] {
        genesis.alloc.insert(
            address,
            GenesisAccount {
                code,
                storage: Default::default(),
                balance: funded(),
                nonce: 0,
            },
        );
    }
    let chain_id = genesis.config.chain_id;
    let mut store = Store::new(store_name, EngineType::InMemory).expect("build store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    (store, chain_id)
}

async fn hegota_chain(store_name: &str) -> (Store, Blockchain, BlockHeader, u64) {
    let (store, chain_id) = setup_store(store_name, true).await;
    let blockchain = Blockchain::new(
        store.clone(),
        BlockchainOptions {
            min_tip_wei: 0,
            ..BlockchainOptions::default()
        },
    );
    let genesis = store.get_block_header(0).unwrap().unwrap();
    (store, blockchain, genesis, chain_id)
}

fn self_verify_frame(sender: Address, state_gas_limit: u64) -> Frame {
    Frame {
        mode: FrameMode::Verify as u8,
        flags: 0x03,
        target: Some(sender),
        gas_limit: 80_000,
        state_gas_limit,
        value: U256::zero(),
        data: Bytes::new(),
    }
}

fn sender_frame(to: Address, value: U256, gas_limit: u64, state_gas_limit: u64) -> Frame {
    Frame {
        mode: FrameMode::Sender as u8,
        flags: 0,
        target: Some(to),
        gas_limit,
        state_gas_limit,
        value,
        data: Bytes::new(),
    }
}

fn frame_tx(
    chain_id: u64,
    sender: Address,
    nonce_keys: Vec<U256>,
    nonce_seq: u64,
    frames: Vec<Frame>,
) -> Transaction {
    Transaction::FrameTransaction(FrameTransaction {
        chain_id,
        nonce_keys,
        nonce_seq,
        sender,
        frames,
        signatures: vec![],
        max_priority_fee_per_gas: U256::from(1u64),
        max_fee_per_gas: U256::from(TEST_MAX_FEE_PER_GAS),
        ..Default::default()
    })
}

/// A `self_verify` transaction on the legacy nonce key followed by a no-value
/// call: the simplest Profile 2 candidate.
fn basic_frame_tx(chain_id: u64, sender: Address, nonce_seq: u64) -> Transaction {
    frame_tx(
        chain_id,
        sender,
        vec![U256::zero()],
        nonce_seq,
        vec![
            self_verify_frame(sender, 0),
            sender_frame(Address::from_low_u64_be(0xBEEF), U256::zero(), 30_000, 0),
        ],
    )
}

/// Build a block on `parent`; with `il`, the builder sequences the inclusion
/// list first; without, it fills from the mempool alone (the external builder
/// that has the list and omits it).
fn build_block(
    store: &Store,
    blockchain: &Blockchain,
    parent: &BlockHeader,
    il: Option<&[Transaction]>,
) -> Block {
    let timestamp = parent.timestamp + 12;
    // EIP-7843: an Amsterdam header carries its beacon slot (the `current_slot`
    // Profile 2 judges recent roots at); a pre-Amsterdam header must not.
    let slot_number = store
        .get_chain_config()
        .is_amsterdam_activated(timestamp)
        .then_some(1);
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number,
        version: 5,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        inclusion_list_transactions: il.map(<[Transaction]>::to_vec),
    };
    let block = create_payload(&args, store, Bytes::new()).unwrap();
    match il {
        Some(il) => blockchain.build_payload_with_il(block, il).unwrap().payload,
        None => blockchain.build_payload(block).unwrap().payload,
    }
}

fn import_with_il(
    blockchain: &Blockchain,
    block: Block,
    il: &[Transaction],
) -> Result<(), ChainError> {
    let context = BlockValidationContext::with_inclusion_list(il.to_vec());
    blockchain.add_block_pipeline_with_il(block, None, &context)
}

fn assert_unjustified(result: Result<(), ChainError>, expected: &Transaction) {
    match result {
        Err(ChainError::IlUnsatisfied { tx_hash }) => {
            assert_eq!(tx_hash, expected.hash(&NativeCrypto));
        }
        other => panic!("expected ChainError::IlUnsatisfied, got {other:?}"),
    }
}

/// Case 1: a Profile 2 candidate eligible at both states, omitted by a builder
/// that had it. The block is unsatisfied, and the shared entry point the engine
/// API consults agrees with the import pipeline.
#[tokio::test]
async fn omitted_eligible_frame_tx_is_unjustified() {
    let (store, blockchain, genesis, chain_id) = hegota_chain("focil-p2-case1").await;
    let il_tx = basic_frame_tx(chain_id, FRAME_SENDER, 0);
    let il = vec![il_tx.clone()];

    let block = build_block(&store, &blockchain, &genesis, None);
    assert!(block.body.transactions.is_empty());
    let header = block.header.clone();

    assert_unjustified(import_with_il(&blockchain, block, &il), &il_tx);

    // The engine API asks the same question again for `forkchoiceUpdated` and
    // must get the same answer from the same states.
    let satisfaction = blockchain
        .inclusion_list_satisfaction(&header, &HashSet::new(), &[], &il)
        .expect("verdict computes");
    assert_eq!(
        satisfaction.unjustified_omission,
        Some(il_tx.hash(&NativeCrypto))
    );
    assert!(satisfaction.undecided.is_empty());
}

/// The builder that honours the list includes the frame transaction, and the
/// block is satisfied on import.
#[tokio::test]
async fn included_frame_tx_satisfies_the_list() {
    let (store, blockchain, genesis, chain_id) = hegota_chain("focil-p2-included").await;
    let il_tx = basic_frame_tx(chain_id, FRAME_SENDER, 0);
    let il = vec![il_tx.clone()];

    let block = build_block(&store, &blockchain, &genesis, Some(&il));
    assert_eq!(
        block.body.transactions.len(),
        1,
        "the builder must include the listed frame transaction"
    );
    assert_eq!(
        block.body.transactions[0].hash(&NativeCrypto),
        il_tx.hash(&NativeCrypto)
    );
    import_with_il(&blockchain, block, &il).expect("a block carrying the listed tx is satisfied");
}

/// Case 4, in the direction the table names: a keyed nonce the transaction
/// cannot satisfy at either state. Here the sequence number is ahead of the
/// sender's nonce and no predecessor executes, so the omission is justified.
#[tokio::test]
async fn omitted_frame_tx_with_unsatisfiable_nonce_is_justified() {
    let (store, blockchain, genesis, chain_id) = hegota_chain("focil-p2-case4").await;
    let il = vec![basic_frame_tx(chain_id, FRAME_SENDER, 5)];
    let block = build_block(&store, &blockchain, &genesis, None);
    import_with_il(&blockchain, block, &il)
        .expect("a frame tx whose nonce can never match is excused");
}

/// Case 2: a queued frame transaction (`nonce_seq == 1`) whose predecessor
/// executes in the block. Ineligible at `S_start`, eligible at `S_end`: the
/// omission is unjustified. This is the case that distinguishes the two-endpoint
/// rule from a builder-claimed index, which excuses it.
#[tokio::test]
async fn queued_frame_tx_whose_predecessor_executes_in_the_block_is_unjustified() {
    let (store, blockchain, genesis, chain_id) = hegota_chain("focil-p2-case2").await;
    let predecessor = basic_frame_tx(chain_id, FRAME_SENDER, 0);
    blockchain
        .add_transaction_to_pool(predecessor.clone())
        .await
        .expect("the predecessor is admitted to the mempool");
    let queued = basic_frame_tx(chain_id, FRAME_SENDER, 1);
    let il = vec![queued.clone()];

    let block = build_block(&store, &blockchain, &genesis, None);
    assert_eq!(block.body.transactions.len(), 1);
    assert_eq!(
        block.body.transactions[0].hash(&NativeCrypto),
        predecessor.hash(&NativeCrypto)
    );

    assert_unjustified(import_with_il(&blockchain, block, &il), &queued);
}

/// Builders MUST include every listed transaction whose omission would be
/// unjustified. The queued transaction of case 2 is not includable at the front
/// of the payload and is includable at its end, once the mempool has supplied
/// the predecessor; the builder's second pass over skipped entries places it
/// there, and the block it produces satisfies its own list.
#[tokio::test]
async fn builder_retries_a_skipped_listed_tx_at_the_end_of_the_payload() {
    let (store, blockchain, genesis, chain_id) = hegota_chain("focil-p2-builder-retry").await;
    let predecessor = basic_frame_tx(chain_id, FRAME_SENDER, 0);
    blockchain
        .add_transaction_to_pool(predecessor.clone())
        .await
        .expect("the predecessor is admitted to the mempool");
    let queued = basic_frame_tx(chain_id, FRAME_SENDER, 1);
    let il = vec![queued.clone()];

    let block = build_block(&store, &blockchain, &genesis, Some(&il));
    let hashes: Vec<H256> = block
        .body
        .transactions
        .iter()
        .map(|tx| tx.hash(&NativeCrypto))
        .collect();
    assert_eq!(
        hashes,
        vec![predecessor.hash(&NativeCrypto), queued.hash(&NativeCrypto)],
        "the predecessor fills from the mempool, then the listed tx lands on the retry pass"
    );
    import_with_il(&blockchain, block, &il).expect("the builder's own block satisfies its list");
}

/// Case 3: the payer is solvent at `S_start` and drained by a later transaction
/// of the block. Eligible at `S_start`, ineligible at `S_end` (its `APPROVE`
/// cannot collect the maximum cost), so the omission is unjustified. An
/// end-of-payload rule alone would excuse it.
#[tokio::test]
async fn payer_drained_by_a_later_block_tx_is_still_unjustified() {
    let (store, blockchain, genesis, chain_id) = hegota_chain("focil-p2-case3").await;
    // The drain: the sender moves almost everything to a fresh account. What is
    // left after fees is a few finney.
    let keep = U256::from(5u64) * U256::from(10u64).pow(U256::from(15u64));
    let drain = frame_tx(
        chain_id,
        FRAME_SENDER,
        vec![U256::zero()],
        0,
        vec![
            self_verify_frame(FRAME_SENDER, 0),
            sender_frame(
                Address::from_low_u64_be(0xD0D0),
                funded() - keep,
                30_000,
                NEW_ACCOUNT_STATE_GAS,
            ),
        ],
    );
    blockchain
        .add_transaction_to_pool(drain.clone())
        .await
        .expect("the drain is admitted to the mempool");
    // The listed transaction: on a fresh nonce key so the drain does not consume
    // its nonce, and declaring enough gas that its maximum cost exceeds what the
    // drain leaves while fitting the block comfortably.
    let listed = frame_tx(
        chain_id,
        FRAME_SENDER,
        vec![U256::from(0x8250u64)],
        0,
        vec![
            self_verify_frame(FRAME_SENDER, KEYED_NONCE_FIRST_USE_STATE_GAS),
            sender_frame(
                Address::from_low_u64_be(0xBEEF),
                U256::zero(),
                1_000_000,
                5_000_000,
            ),
        ],
    );
    let il = vec![listed.clone()];

    let block = build_block(&store, &blockchain, &genesis, None);
    assert_eq!(block.body.transactions.len(), 1);
    assert_eq!(
        block.body.transactions[0].hash(&NativeCrypto),
        drain.hash(&NativeCrypto)
    );

    assert_unjustified(import_with_il(&blockchain, block, &il), &listed);
}

/// The surface boundary, from inside: a sender whose approval reads slot 3
/// stays inside the surface at `AA_VOPS_SLOT_COUNT = 4`, so its omission is
/// unjustified.
#[tokio::test]
async fn omitted_frame_tx_reading_a_slot_inside_the_surface_is_unjustified() {
    let (store, blockchain, genesis, chain_id) = hegota_chain("focil-p2-slot3").await;
    let il_tx = basic_frame_tx(chain_id, SLOT_3_READER, 0);
    let il = vec![il_tx.clone()];
    let block = build_block(&store, &blockchain, &genesis, None);
    assert_unjustified(import_with_il(&blockchain, block, &il), &il_tx);
}

/// Case 7: the prefix reads storage slot `AA_VOPS_SLOT_COUNT` of `sender`. The
/// same transaction executes fine in a block (the mempool's sender-only rule
/// admits any sender slot); Profile 2 excuses its omission.
#[tokio::test]
async fn omitted_frame_tx_reading_a_slot_outside_the_surface_is_justified() {
    let (store, blockchain, genesis, chain_id) = hegota_chain("focil-p2-slot4").await;
    let il = vec![basic_frame_tx(chain_id, SLOT_4_READER, 0)];
    let block = build_block(&store, &blockchain, &genesis, None);
    import_with_il(&blockchain, block, &il)
        .expect("a read outside the surface makes the transaction ineligible");
}

/// Case 21: `total_gas_limit` exceeds the block's remaining gas. Eligible at
/// both states, but block space is judged once at the end of the payload, so
/// the omission is justified. The budget sums only the prefix, so the
/// transaction is still a candidate; it is `gas_fits` that excuses it.
#[tokio::test]
async fn omitted_frame_tx_that_does_not_fit_the_block_is_justified() {
    let (store, blockchain, genesis, chain_id) = hegota_chain("focil-p2-case21").await;
    let oversized = frame_tx(
        chain_id,
        FRAME_SENDER,
        vec![U256::zero()],
        0,
        vec![
            self_verify_frame(FRAME_SENDER, 0),
            // The state dimension is outside the EIP-7825 cap, so a valid
            // transaction can declare more total gas than the block holds.
            sender_frame(
                Address::from_low_u64_be(0xBEEF),
                U256::zero(),
                30_000,
                genesis.gas_limit.saturating_add(1),
            ),
        ],
    );
    let il = vec![oversized];
    let block = build_block(&store, &blockchain, &genesis, None);
    import_with_il(&blockchain, block, &il)
        .expect("a transaction that does not fit the remaining block gas is excused");
}

/// Activation: before the fork every frame transaction omission is justified.
#[tokio::test]
async fn before_the_fork_frame_tx_omissions_are_justified() {
    let (store, chain_id) = setup_store("focil-p2-prefork", false).await;
    let blockchain = Blockchain::new(store.clone(), BlockchainOptions::default());
    let genesis = store.get_block_header(0).unwrap().unwrap();
    let il = vec![basic_frame_tx(chain_id, FRAME_SENDER, 0)];
    let block = build_block(&store, &blockchain, &genesis, None);
    import_with_il(&blockchain, block, &il)
        .expect("before FORK_TIMESTAMP a frame tx omission is justified");
}
