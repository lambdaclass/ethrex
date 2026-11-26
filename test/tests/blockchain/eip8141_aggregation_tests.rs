//! EIP-8141 claim witnesses from LStar on: one aggregate proof for a block's
//! leanSPHINCS claims.
//!
//! A frame transaction is unchanged by the fork. It declares the same claims, is
//! submitted with the same witnesses and is kept with them in the pool. What
//! changes is the block (EIP-8141 §Block Witnesses, §Aggregation and EIP-8288
//! compatibility): its `claim_witnesses` opens with version byte `0x01` and
//! holds one aggregate proof in place of the witnesses of every
//! `LEANSPHINCS_VERIFIER` dependency, beside the raw witnesses of the other
//! dependencies. The builder makes the proof from the witnesses in its pool, and
//! every node that imports the block verifies it against the leanSPHINCS
//! dependencies of the block's transactions.

use std::{fs::File, io::BufReader, path::PathBuf, sync::Arc};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    claim_witnesses::aggregate_claim_witnesses,
    error::MempoolError,
    payload::{BuildPayloadArgs, PayloadBuildContext, PayloadBuildResult, create_payload},
};
use ethrex_common::{
    Address, H160, H256,
    errors::InvalidBlockError,
    types::{
        Block, BlockHeader, Claim, ClaimWitnesses, DEFAULT_BUILDER_GAS_CEIL, Dependency,
        ELASTICITY_MULTIPLIER, Fork, Genesis, GenesisAccount, LEANSPHINCS_SIGNATURE_SIZE,
        LEANSPHINCS_VERIFICATION_GAS, LEANSPHINCS_VERIFIER, MAX_AGGREGATE_PROOF_BYTES,
        MAX_BLOCK_WITNESS_BYTES, P2PTransaction, PooledFrameTransaction, SECP256K1_VERIFIER,
        Transaction, block_access_list::BlockAccessList, block_dependencies,
        compute_claim_witnesses_hash, decode_claim_witnesses, decode_claim_witnesses_at,
        encode_aggregated_claim_witnesses, encode_claim_witnesses,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::claims::verify_aggregated_dependencies;
use ethrex_rlp::encode::RLPEncode;
use ethrex_rpc::{
    engine::payload::{GetPayloadV6Request, NewPayloadV5Request},
    rpc::RpcHandler,
    test_utils::default_context_with_storage,
    types::payload::ExecutionPayload,
};
use ethrex_storage::{EngineType, Store};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::eip8141_claims_tests::{
    CUSTOM_ACCOUNT, CUSTOM_KEY, CUSTOM_WITNESS, Key, assert_rejected_on_both_paths, blockchain,
    claim_transaction, custom_claim, custom_pooled, custom_verifier_code, frame_transaction,
    funded, import_on_both_paths, key, leansphincs_claim, make_head, pooled, sign, tx_hash,
    with_claim_witnesses,
};
use crate::levm::eip8141_claims_tests::claim_account_code;

/// Seconds between a block and its parent on the chains of these tests.
const BLOCK_TIME: u64 = 12;
/// LStar from the first block on: the genesis block is the only Hegota block.
const LSTAR_AT_FIRST_BLOCK: Option<u64> = Some(BLOCK_TIME);
/// A custom verifier that is part of the genesis state, so that a claim it
/// checks holds in the pre-state of the first block.
const CUSTOM_VERIFIER: Address = H160([0x7E; 20]);

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// `l1-hegota.json` (every fork through Hegota active at genesis) with each
/// key's account funded and running the validation code for that key, plus
/// `accounts`. LStar activates `lstar_after` seconds after genesis, or never
/// when it is `None`.
async fn setup_store(
    keys: &[&Key],
    accounts: &[(Address, GenesisAccount)],
    lstar_after: Option<u64>,
) -> (Store, u64) {
    let file = File::open(workspace_root().join("fixtures/genesis/l1-hegota.json"))
        .expect("open l1-hegota genesis");
    let mut genesis: Genesis =
        serde_json::from_reader(BufReader::new(file)).expect("parse l1-hegota genesis");
    genesis.config.lstar_time = lstar_after.map(|seconds| genesis.timestamp + seconds);
    for key in keys {
        genesis.alloc.insert(
            key.account,
            funded(crate::levm::eip8141_claims_tests::leansphincs_account_code(
                key.public,
            )),
        );
    }
    for (address, account) in accounts {
        genesis.alloc.insert(*address, account.clone());
    }
    let chain_id = genesis.config.chain_id;
    let mut store = Store::new("store.db", EngineType::InMemory).expect("build in-memory store");
    store
        .add_initial_state(genesis)
        .await
        .expect("add genesis state");
    (store, chain_id)
}

/// A secp256k1 key and the account without code it controls: the default code
/// approves on a secp256k1 claim for the account's own address.
struct Secp256k1Key {
    signing: k256::ecdsa::SigningKey,
    account: Address,
}

fn secp256k1_key(seed: u8) -> Secp256k1Key {
    let signing = k256::ecdsa::SigningKey::from_bytes(&[seed; 32].into()).unwrap();
    let uncompressed = signing.verifying_key().to_encoded_point(false);
    let public_key_hash = ethrex_crypto::keccak::keccak_hash(&uncompressed.as_bytes()[1..]);
    Secp256k1Key {
        signing,
        account: Address::from_slice(&public_key_hash[12..]),
    }
}

/// The witness of a secp256k1 claim: `v || r || s`.
fn secp256k1_sign(key: &Secp256k1Key, message: H256) -> Bytes {
    let (signature, recovery_id) = key
        .signing
        .sign_prehash_recoverable(message.as_bytes())
        .unwrap();
    let mut witness = vec![recovery_id.to_byte()];
    witness.extend_from_slice(&signature.to_bytes());
    Bytes::from(witness)
}

/// The genesis entry of the key's account: funded, without code.
fn secp256k1_account(key: &Secp256k1Key) -> (Address, GenesisAccount) {
    (key.account, funded(Vec::new()))
}

/// A transaction from the key's account on its secp256k1 claim, in its pooled
/// representation.
fn secp256k1_pooled(chain_id: u64, key: &Secp256k1Key) -> PooledFrameTransaction {
    let tx = frame_transaction(
        chain_id,
        key.account,
        vec![Claim {
            verifier: SECP256K1_VERIFIER,
            ..Default::default()
        }],
    );
    let witness = secp256k1_sign(key, tx.compute_sig_hash());
    PooledFrameTransaction {
        tx,
        witnesses: vec![witness],
        blobs_bundle: None,
    }
}

/// The genesis entries of the custom verifier and of the account that approves
/// on a claim it checks.
fn custom_accounts() -> [(Address, GenesisAccount); 2] {
    [
        (CUSTOM_VERIFIER, funded(custom_verifier_code())),
        (
            CUSTOM_ACCOUNT,
            funded(claim_account_code(CUSTOM_VERIFIER, CUSTOM_KEY)),
        ),
    ]
}

/// The block a builder starts from on top of `parent`.
fn payload_on(store: &Store, parent: &BlockHeader) -> Block {
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + BLOCK_TIME,
        fee_recipient: H160::zero(),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: Some(parent.number + 1),
        version: 1,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    create_payload(&args, store, Bytes::new()).unwrap()
}

fn build_block(store: &Store, blockchain: &Blockchain, parent: &BlockHeader) -> PayloadBuildResult {
    blockchain.build_payload(payload_on(store, parent)).unwrap()
}

fn genesis_header(store: &Store) -> BlockHeader {
    store.get_block_header(0).unwrap().unwrap()
}

/// The dependency a leanSPHINCS signature of `message` under `key` satisfies.
fn leansphincs_dependency(key: &Key, message: H256) -> Dependency {
    Dependency {
        verifier: LEANSPHINCS_VERIFIER,
        key: key.public,
        msg: message,
        gas_limit: LEANSPHINCS_VERIFICATION_GAS,
        witness_len: LEANSPHINCS_SIGNATURE_SIZE as u64,
    }
}

/// An aggregate proof of the given `(key, message)` claims, each signed here.
fn proof_of(claims: &[(&Key, H256)]) -> Bytes {
    let signed: Vec<_> = claims
        .iter()
        .map(|(key, message)| (leansphincs_dependency(key, *message), sign(key, *message)))
        .collect();
    aggregate_claim_witnesses(&signed).expect("valid signatures aggregate")
}

/// What the claim witnesses of an LStar block hold.
fn aggregated(block: &Block) -> ClaimWitnesses {
    decode_claim_witnesses_at(
        Fork::LStar,
        block
            .body
            .claim_witnesses
            .as_ref()
            .expect("the block has claim witnesses"),
    )
    .expect("the claim witnesses of an LStar block decode")
}

/// The block with `claim_witnesses` in place of its own, the header committing
/// to them and the block hash following the header, so only the claim witnesses
/// themselves can make it invalid.
fn with_bytes(block: Block, claim_witnesses: Bytes) -> Block {
    let mut block = with_claim_witnesses(block, claim_witnesses);
    block.header.hash = Default::default();
    block
}

/// [`with_bytes`] for the aggregated form of `aggregate_proof` and `witnesses`.
fn with_aggregated(block: Block, aggregate_proof: &[u8], witnesses: &[Bytes]) -> Block {
    with_bytes(
        block,
        encode_aggregated_claim_witnesses(aggregate_proof, witnesses),
    )
}

/// An LStar block with two transactions authorized by leanSPHINCS claims and
/// one authorized by a secp256k1 claim. It has three dependencies: the
/// secp256k1 one comes first in `block_dependencies` and keeps a raw witness,
/// and the aggregate proof covers the other two.
struct LStarBlock {
    block: Block,
    bal: Arc<BlockAccessList>,
    /// The signature hashes of the two leanSPHINCS transactions, in the order
    /// of the keys.
    messages: [H256; 2],
    /// The aggregate proof the block carries.
    proof: Bytes,
    /// The raw witness the block carries: the secp256k1 signature.
    witness: Bytes,
    /// What a node starts from to import the block.
    accounts: [(Address, GenesisAccount); 1],
}

async fn lstar_block(first: &Key, second: &Key, signer: &Secp256k1Key) -> LStarBlock {
    let accounts = [secp256k1_account(signer)];
    let (store, chain_id) = setup_store(&[first, second], &accounts, LSTAR_AT_FIRST_BLOCK).await;
    let blockchain = blockchain(store.clone());
    let mut messages = [H256::zero(); 2];
    for (index, sender) in [first, second].into_iter().enumerate() {
        let tx = claim_transaction(chain_id, sender, Vec::new());
        messages[index] = tx.compute_sig_hash();
        blockchain
            .add_frame_transaction_to_pool(pooled(tx, &[sender]))
            .await
            .expect("the leanSPHINCS transaction enters the pool");
    }
    blockchain
        .add_frame_transaction_to_pool(secp256k1_pooled(chain_id, signer))
        .await
        .expect("the secp256k1 transaction enters the pool");
    let built = build_block(&store, &blockchain, &genesis_header(&store));
    assert_eq!(built.payload.body.transactions.len(), 3);
    assert!(built.receipts.iter().all(|receipt| receipt.succeeded));
    let ClaimWitnesses {
        aggregate_proof,
        witnesses,
    } = aggregated(&built.payload);
    assert_eq!(witnesses.len(), 1);
    LStarBlock {
        block: built.payload,
        bal: Arc::new(built.block_access_list.expect("an LStar block has a BAL")),
        messages,
        proof: aggregate_proof,
        witness: witnesses[0].clone(),
        accounts,
    }
}

/// Asserts that both import paths reject every block of `invalid` with an
/// invalid-block error that `expected` recognizes, on nodes that start from the
/// genesis state of [`lstar_block`].
async fn assert_all_rejected(
    built: &LStarBlock,
    keys: &[&Key],
    invalid: Vec<(&str, Block)>,
    expected: impl Fn(&InvalidBlockError) -> bool,
) {
    let fresh_store = || async {
        setup_store(keys, &built.accounts, LSTAR_AT_FIRST_BLOCK)
            .await
            .0
    };
    for (what, block) in invalid {
        assert_rejected_on_both_paths(block, &built.bal, fresh_store, what, &expected).await;
    }
}

// ==================== The LStar block ====================

#[tokio::test]
async fn an_lstar_block_carries_one_proof_for_its_leansphincs_claims_and_raw_witnesses_for_the_rest()
 {
    let first = key(0xA1);
    let second = key(0xA2);
    let third = key(0xA3);
    let shared = key(0xA4);
    let signer = secp256k1_key(0xA5);
    let keys = [&first, &second, &third];
    let mut accounts = custom_accounts().to_vec();
    accounts.push(secp256k1_account(&signer));

    // The first two transactions also claim a signature of the same message
    // under a fourth key. The third claims its own signature alone. One more
    // transaction is authorized by a secp256k1 claim, and one carries a claim a
    // custom verifier checks.
    let message = H256::repeat_byte(0x77);
    let pooled_txs = |chain_id: u64| {
        let mut pooled_txs = Vec::new();
        for sender in [&first, &second] {
            let tx = claim_transaction(
                chain_id,
                sender,
                vec![leansphincs_claim(&shared, Some(message))],
            );
            pooled_txs.push(pooled(tx, &[sender, &shared]));
        }
        pooled_txs.push(pooled(
            claim_transaction(chain_id, &third, Vec::new()),
            &[&third],
        ));
        pooled_txs.push(secp256k1_pooled(chain_id, &signer));
        pooled_txs.push(custom_pooled(
            frame_transaction(
                chain_id,
                CUSTOM_ACCOUNT,
                vec![custom_claim(CUSTOM_VERIFIER)],
            ),
            CUSTOM_WITNESS,
        ));
        pooled_txs
    };

    let (store, chain_id) = setup_store(&keys, &accounts, LSTAR_AT_FIRST_BLOCK).await;
    let blockchain = blockchain(store.clone());
    let pooled_txs = pooled_txs(chain_id);
    for pooled_tx in &pooled_txs {
        blockchain
            .add_frame_transaction_to_pool(pooled_tx.clone())
            .await
            .expect("the transaction enters the pool with its witnesses");
    }
    let secp256k1_witness = pooled_txs[3].witnesses[0].clone();

    let built = build_block(&store, &blockchain, &genesis_header(&store));
    let block = built.payload;
    assert_eq!(block.body.transactions.len(), 5);
    assert!(
        built.receipts.iter().all(|receipt| receipt.succeeded),
        "the transactions must succeed: {:?}",
        built.receipts
    );

    // Seven claims and six distinct dependencies: the shared one counts once.
    // Four of them are leanSPHINCS dependencies.
    let dependencies = block_dependencies(&block.body.transactions).unwrap();
    assert_eq!(dependencies.len(), 6);
    let (leansphincs, others): (Vec<Dependency>, Vec<Dependency>) = dependencies
        .iter()
        .partition(|dependency| dependency.verifier == LEANSPHINCS_VERIFIER);
    assert_eq!(leansphincs.len(), 4);
    assert_eq!(
        others
            .iter()
            .map(|dependency| dependency.verifier)
            .collect::<Vec<_>>(),
        vec![SECP256K1_VERIFIER, CUSTOM_VERIFIER]
    );

    // The block's claim witnesses open with the version byte of the aggregated
    // form and hold one proof and exactly the two raw witnesses, in the order
    // of their dependencies. No leanSPHINCS signature is in the block.
    let claim_witnesses = block
        .body
        .claim_witnesses
        .clone()
        .expect("an LStar block has claim witnesses");
    assert_eq!(claim_witnesses[0], 0x01);
    let ClaimWitnesses {
        aggregate_proof,
        witnesses,
    } = aggregated(&block);
    assert_eq!(
        witnesses,
        vec![
            secp256k1_witness,
            Bytes::copy_from_slice(CUSTOM_WITNESS.as_bytes())
        ]
    );
    assert!(!aggregate_proof.is_empty());
    assert!(aggregate_proof.len() <= MAX_AGGREGATE_PROOF_BYTES);
    assert!(built.unaggregated_claim_witnesses.is_empty());
    assert_eq!(
        claim_witnesses,
        encode_aggregated_claim_witnesses(&aggregate_proof, &witnesses)
    );
    assert!(claim_witnesses.len() <= MAX_BLOCK_WITNESS_BYTES);
    // The proof is one of exactly the block's leanSPHINCS dependencies.
    assert!(verify_aggregated_dependencies(
        &leansphincs,
        &aggregate_proof
    ));
    assert!(!verify_aggregated_dependencies(
        &leansphincs[1..],
        &aggregate_proof
    ));
    // The header field and what it commits to are what they were before LStar.
    assert_eq!(
        block.header.claim_witnesses_hash,
        Some(H256(ethrex_crypto::keccak::keccak_hash(&claim_witnesses)))
    );

    // The transactions in the block are the bare bodies, with the hashes they
    // had in the pool, and the pool still holds each one with the witnesses it
    // was submitted with: the block did not take the signatures, it proved them.
    for pooled_tx in &pooled_txs {
        let hash = tx_hash(&pooled_tx.tx);
        assert!(
            block
                .body
                .transactions
                .iter()
                .any(|tx| tx.hash(&NativeCrypto) == hash)
        );
        assert_eq!(
            blockchain.mempool.get_claim_witnesses(hash).unwrap(),
            Some(pooled_tx.witnesses.clone())
        );
    }

    // It imports on every path, and on the one that collects the execution
    // witness of the block.
    let bal = Arc::new(built.block_access_list.expect("an LStar block has a BAL"));
    let fresh_store = || async { setup_store(&keys, &accounts, LSTAR_AT_FIRST_BLOCK).await.0 };
    for (path, result) in ["add_block", "add_block_pipeline"]
        .into_iter()
        .zip(import_on_both_paths(&block, &bal, fresh_store).await)
    {
        result.unwrap_or_else(|error| panic!("{path}: {error}"));
    }
    let execution_witness = self::blockchain(fresh_store().await)
        .add_block_pipeline_with_witness(block.clone(), Some(bal.clone()))
        .expect("the block imports while its execution witness is collected");
    // Stateless execution repeats the check from that witness. This build links
    // the leanSPHINCS verifiers, so it verifies the proof as a node does.
    ethrex_guest_program::l1::validate_blocks_statelessly(
        std::slice::from_ref(&block),
        execution_witness,
        Arc::new(NativeCrypto),
    )
    .expect("the block's claims are verified statelessly");

    // The same transactions in a Hegota block, on a chain without LStar: the
    // block carries a raw witness for each of the six dependencies.
    let (hegota_store, hegota_chain_id) = setup_store(&keys, &accounts, None).await;
    assert_eq!(hegota_chain_id, chain_id);
    let hegota_blockchain = self::blockchain(hegota_store.clone());
    for pooled_tx in &pooled_txs {
        hegota_blockchain
            .add_frame_transaction_to_pool(pooled_tx.clone())
            .await
            .expect("the transaction enters the pool with its witnesses");
    }
    let hegota_built = build_block(
        &hegota_store,
        &hegota_blockchain,
        &genesis_header(&hegota_store),
    );
    let hegota_block = hegota_built.payload;
    assert_eq!(hegota_block.body.transactions.len(), 5);
    for tx in &block.body.transactions {
        assert!(hegota_block.body.transactions.contains(tx));
    }
    let hegota_claim_witnesses = hegota_block.body.claim_witnesses.clone().unwrap();
    assert_eq!(hegota_claim_witnesses[0], 0x00);
    assert_eq!(
        decode_claim_witnesses(&hegota_claim_witnesses)
            .unwrap()
            .len(),
        6
    );
    // The gas does not depend on the form the block carries the witnesses in.
    assert_eq!(block.header.gas_used, hegota_block.header.gas_used);
    println!(
        "Five transactions, four leanSPHINCS dependencies. LStar block: {} bytes of RLP block, \
         of which {} bytes of claim_witnesses ({} bytes of proof), {} gas used. Hegota block \
         with the same transactions: {} bytes of RLP block, of which {} bytes of \
         claim_witnesses, {} gas used.",
        block.length(),
        claim_witnesses.len(),
        aggregate_proof.len(),
        block.header.gas_used,
        hegota_block.length(),
        hegota_claim_witnesses.len(),
        hegota_block.header.gas_used,
    );
}

#[tokio::test]
async fn an_lstar_block_without_dependencies_carries_empty_claim_witnesses() {
    let first = key(0xA6);
    let second = key(0xA7);
    let signer = secp256k1_key(0xA8);
    let keys = [&first, &second];
    // A valid proof, of the claims of another block.
    let other = lstar_block(&first, &second, &signer).await;
    let fresh_store = || async {
        setup_store(&keys, &other.accounts, LSTAR_AT_FIRST_BLOCK)
            .await
            .0
    };

    let store = fresh_store().await;
    let built = build_block(&store, &blockchain(store.clone()), &genesis_header(&store));
    let block = built.payload;
    let bal = Arc::new(built.block_access_list.expect("an LStar block has a BAL"));
    assert!(block.body.transactions.is_empty());
    assert_eq!(block.body.claim_witnesses, Some(Bytes::new()));
    assert_eq!(
        block.header.claim_witnesses_hash,
        Some(H256(hex_literal::hex!(
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        )))
    );
    for result in import_on_both_paths(&block, &bal, fresh_store).await {
        result.expect("the block imports");
    }

    // The empty byte string is its only form: the version byte with an empty
    // proof and an empty list is invalid.
    assert_rejected_on_both_paths(
        with_bytes(block.clone(), Bytes::from(vec![0x01, 0xc2, 0x80, 0xc0])),
        &bal,
        fresh_store,
        "an empty proof and an empty list",
        |error| matches!(error, InvalidBlockError::InvalidClaimWitnesses(_)),
    )
    .await;

    // With a proof it has nothing to prove it is invalid, however valid the
    // proof is for other claims, and so it is with a witness nobody claimed.
    let proof_len = other.proof.len();
    assert_rejected_on_both_paths(
        with_aggregated(block.clone(), &other.proof, &[]),
        &bal,
        fresh_store,
        "a proof without leanSPHINCS dependencies",
        |error| matches!(error, InvalidBlockError::UnexpectedAggregateProof(len) if *len == proof_len),
    )
    .await;
    assert_rejected_on_both_paths(
        with_aggregated(block, &[], std::slice::from_ref(&other.witness)),
        &bal,
        fresh_store,
        "a stray witness",
        |error| {
            matches!(
                error,
                InvalidBlockError::ClaimWitnessesCountMismatch {
                    expected: 0,
                    found: 1
                }
            )
        },
    )
    .await;
}

// A block whose dependencies are all of verifiers that are not aggregated has
// the aggregated form with an empty proof, and a proof makes it invalid.
#[tokio::test]
async fn an_lstar_block_without_leansphincs_claims_carries_an_empty_proof() {
    let first = key(0xA9);
    let second = key(0xAA);
    let signer = secp256k1_key(0xAB);
    let other = lstar_block(&first, &second, &signer).await;
    let accounts = [secp256k1_account(&signer)];
    let fresh_store = || async { setup_store(&[], &accounts, LSTAR_AT_FIRST_BLOCK).await.0 };

    let (store, chain_id) = setup_store(&[], &accounts, LSTAR_AT_FIRST_BLOCK).await;
    let blockchain = blockchain(store.clone());
    let pooled_tx = secp256k1_pooled(chain_id, &signer);
    let witness = pooled_tx.witnesses[0].clone();
    blockchain
        .add_frame_transaction_to_pool(pooled_tx)
        .await
        .expect("the secp256k1 transaction enters the pool");
    let built = build_block(&store, &blockchain, &genesis_header(&store));
    let block = built.payload;
    let bal = Arc::new(built.block_access_list.expect("an LStar block has a BAL"));
    assert_eq!(block.body.transactions.len(), 1);
    assert_eq!(
        aggregated(&block),
        ClaimWitnesses {
            aggregate_proof: Bytes::new(),
            witnesses: vec![witness.clone()],
        }
    );
    assert_eq!(block.body.claim_witnesses.as_ref().unwrap()[0], 0x01);
    for result in import_on_both_paths(&block, &bal, fresh_store).await {
        result.expect("the block imports");
    }

    let proof_len = other.proof.len();
    assert_rejected_on_both_paths(
        with_aggregated(block, &other.proof, &[witness]),
        &bal,
        fresh_store,
        "a proof without leanSPHINCS dependencies",
        |error| matches!(error, InvalidBlockError::UnexpectedAggregateProof(len) if *len == proof_len),
    )
    .await;
}

// ==================== Blocks the proof does not hold for ====================

#[tokio::test]
async fn a_block_whose_proof_is_not_of_exactly_its_leansphincs_claims_is_rejected() {
    let first = key(0xAC);
    let second = key(0xAD);
    let other = key(0xAE);
    let signer = secp256k1_key(0xAF);
    let keys = [&first, &second];
    let built = lstar_block(&first, &second, &signer).await;
    let [first_message, second_message] = built.messages;
    let with_proof = |proof: Bytes| {
        with_aggregated(
            built.block.clone(),
            &proof,
            std::slice::from_ref(&built.witness),
        )
    };

    let invalid = vec![
        // A proof that the first transaction's claim holds says nothing about
        // the second's.
        (
            "a proof that covers one claim fewer",
            with_proof(proof_of(&[(&first, first_message)])),
        ),
        // The first key signed another message than the transaction's signature
        // hash.
        (
            "a proof for the same key and another message",
            with_proof(proof_of(&[
                (&first, H256::repeat_byte(0x11)),
                (&second, second_message),
            ])),
        ),
        // Another key signed the first transaction's signature hash.
        (
            "a proof for another key and the same message",
            with_proof(proof_of(&[
                (&other, first_message),
                (&second, second_message),
            ])),
        ),
        // The proof states exactly the claims it was made for: one that also
        // covers a claim the block does not have is not the block's.
        (
            "a proof that covers the block's claims and one more",
            with_proof(proof_of(&[
                (&first, first_message),
                (&second, second_message),
                (&other, H256::repeat_byte(0x22)),
            ])),
        ),
    ];
    assert_all_rejected(&built, &keys, invalid, |error| {
        matches!(error, InvalidBlockError::InvalidAggregateProof(2))
    })
    .await;
}

#[tokio::test]
async fn a_block_whose_proof_bytes_are_not_its_proof_is_rejected() {
    let first = key(0xB0);
    let second = key(0xB1);
    let signer = secp256k1_key(0xB2);
    let keys = [&first, &second];
    let built = lstar_block(&first, &second, &signer).await;
    let with_proof = |proof: &[u8]| {
        with_aggregated(
            built.block.clone(),
            proof,
            std::slice::from_ref(&built.witness),
        )
    };

    let mut invalid = Vec::new();
    let flipped: Vec<(String, Vec<u8>)> = [0, built.proof.len() / 2, built.proof.len() - 1]
        .into_iter()
        .map(|position| {
            let mut flipped = built.proof.to_vec();
            flipped[position] ^= 0x01;
            (format!("a flipped bit in byte {position}"), flipped)
        })
        .collect();
    for (what, flipped) in &flipped {
        invalid.push((what.as_str(), with_proof(flipped)));
    }
    invalid.push((
        "a proof cut in half",
        with_proof(&built.proof[..built.proof.len() / 2]),
    ));
    invalid.push((
        "a proof one byte short",
        with_proof(&built.proof[..built.proof.len() - 1]),
    ));
    let mut longer = built.proof.to_vec();
    longer.push(0x00);
    invalid.push(("a proof with a byte after it", with_proof(&longer)));
    // The empty proof, which only a block without leanSPHINCS claims carries.
    invalid.push(("an empty proof", with_proof(&[])));
    assert_all_rejected(&built, &keys, invalid, |error| {
        matches!(error, InvalidBlockError::InvalidAggregateProof(2))
    })
    .await;

    // EIP-8288 `MAX_DEPENDENCY_PROOF_BYTES` bounds the proof. One byte over it,
    // the claim witnesses are malformed whatever the bytes are; at the bound
    // they are judged as a proof.
    let mut oversized = built.proof.to_vec();
    oversized.resize(MAX_AGGREGATE_PROOF_BYTES + 1, 0);
    assert_all_rejected(
        &built,
        &keys,
        vec![("a proof over its size bound", with_proof(&oversized))],
        |error| {
            matches!(error, InvalidBlockError::InvalidClaimWitnesses(reason)
                if reason.contains("MAX_AGGREGATE_PROOF_BYTES"))
        },
    )
    .await;
    oversized.truncate(MAX_AGGREGATE_PROOF_BYTES);
    assert_all_rejected(
        &built,
        &keys,
        vec![("a padded proof at its size bound", with_proof(&oversized))],
        |error| matches!(error, InvalidBlockError::InvalidAggregateProof(2)),
    )
    .await;
}

// ==================== The raw witnesses ====================

#[tokio::test]
async fn an_lstar_block_carries_exactly_one_valid_raw_witness_for_each_claim_the_proof_does_not_cover()
 {
    let first = key(0xB3);
    let second = key(0xB4);
    let signer = secp256k1_key(0xB5);
    let other_signer = secp256k1_key(0xB6);
    let keys = [&first, &second];
    let built = lstar_block(&first, &second, &signer).await;
    let with_witnesses =
        |witnesses: &[Bytes]| with_aggregated(built.block.clone(), &built.proof, witnesses);

    // The secp256k1 dependency is the first of the block's three.
    let dependencies = block_dependencies(&built.block.body.transactions).unwrap();
    assert_eq!(dependencies[0].verifier, SECP256K1_VERIFIER);
    let secp256k1_message = dependencies[0].msg;

    assert_all_rejected(
        &built,
        &keys,
        vec![("a missing raw witness", with_witnesses(&[]))],
        |error| {
            matches!(
                error,
                InvalidBlockError::ClaimWitnessesCountMismatch {
                    expected: 1,
                    found: 0
                }
            )
        },
    )
    .await;
    assert_all_rejected(
        &built,
        &keys,
        vec![
            (
                "an extra raw witness",
                with_witnesses(&[built.witness.clone(), built.witness.clone()]),
            ),
            // The signature of a claim the proof already covers.
            (
                "a leanSPHINCS witness beside the proof",
                with_witnesses(&[built.witness.clone(), sign(&first, built.messages[0])]),
            ),
        ],
        |error| {
            matches!(
                error,
                InvalidBlockError::ClaimWitnessesCountMismatch {
                    expected: 1,
                    found: 2
                }
            )
        },
    )
    .await;

    // A raw witness is verified as before LStar: another key's signature of the
    // same message, a tampered signature and one of another length do not
    // satisfy the dependency, which is reported by its index among the block's.
    let mut tampered = built.witness.to_vec();
    tampered[40] ^= 0x01;
    assert_all_rejected(
        &built,
        &keys,
        vec![
            (
                "the witness of another key",
                with_witnesses(&[secp256k1_sign(&other_signer, secp256k1_message)]),
            ),
            (
                "a tampered witness",
                with_witnesses(&[Bytes::from(tampered)]),
            ),
            (
                "a witness one byte short",
                with_witnesses(&[built.witness.slice(..built.witness.len() - 1)]),
            ),
        ],
        |error| matches!(error, InvalidBlockError::ClaimNotSatisfied(0)),
    )
    .await;
}

// A custom verifier's claim keeps its raw witness from LStar, and the verifier
// still runs in the pure context against the block's pre-state.
#[tokio::test]
async fn a_custom_claim_of_an_lstar_block_is_verified_against_its_raw_witness() {
    let sender = key(0xB7);
    let keys = [&sender];
    let accounts = custom_accounts();
    let fresh_store = || async { setup_store(&keys, &accounts, LSTAR_AT_FIRST_BLOCK).await.0 };
    let (store, chain_id) = setup_store(&keys, &accounts, LSTAR_AT_FIRST_BLOCK).await;
    let blockchain = blockchain(store.clone());
    blockchain
        .add_frame_transaction_to_pool(pooled(
            claim_transaction(chain_id, &sender, Vec::new()),
            &keys,
        ))
        .await
        .expect("the leanSPHINCS transaction enters the pool");
    let custom_tx = frame_transaction(
        chain_id,
        CUSTOM_ACCOUNT,
        vec![custom_claim(CUSTOM_VERIFIER)],
    );
    blockchain
        .add_frame_transaction_to_pool(custom_pooled(custom_tx, CUSTOM_WITNESS))
        .await
        .expect("the custom claim holds");
    let built = build_block(&store, &blockchain, &genesis_header(&store));
    let block = built.payload;
    let bal = Arc::new(built.block_access_list.expect("an LStar block has a BAL"));
    assert_eq!(block.body.transactions.len(), 2);
    assert!(built.receipts.iter().all(|receipt| receipt.succeeded));
    let ClaimWitnesses {
        aggregate_proof,
        witnesses,
    } = aggregated(&block);
    assert_eq!(
        witnesses,
        vec![Bytes::copy_from_slice(CUSTOM_WITNESS.as_bytes())]
    );
    for result in import_on_both_paths(&block, &bal, fresh_store).await {
        result.expect("the block imports");
    }

    // The custom dependency comes after the leanSPHINCS one in the block's
    // dependencies: it is the second of the two, and the only raw witness.
    let dependencies = block_dependencies(&block.body.transactions).unwrap();
    assert_eq!(dependencies[1].verifier, CUSTOM_VERIFIER);
    assert_rejected_on_both_paths(
        with_aggregated(block, &aggregate_proof, &[Bytes::from(vec![0xD8; 32])]),
        &bal,
        fresh_store,
        "a witness the custom verifier rejects",
        |error| matches!(error, InvalidBlockError::ClaimNotSatisfied(1)),
    )
    .await;
}

// A claim's gas is part of its transaction's intrinsic gas, which the block's gas
// limit bounds. From LStar too that bound is checked before a verifier runs or
// the proof is read: a block whose claims take more gas than its limit is
// rejected first.
#[tokio::test]
async fn an_lstar_block_whose_claims_take_more_gas_than_its_limit_is_rejected_before_they_run() {
    /// A verifier that never returns: `JUMPDEST; PUSH0; JUMP`.
    const LOOPING_VERIFIER: Address = H160([0x1F; 20]);
    let accounts = [
        (LOOPING_VERIFIER, funded(vec![0x5b, 0x5f, 0x56])),
        (
            CUSTOM_ACCOUNT,
            funded(claim_account_code(LOOPING_VERIFIER, CUSTOM_KEY)),
        ),
    ];
    let fresh_store = || async { setup_store(&[], &accounts, LSTAR_AT_FIRST_BLOCK).await.0 };
    let (store, chain_id) = setup_store(&[], &accounts, LSTAR_AT_FIRST_BLOCK).await;
    let built = build_block(&store, &blockchain(store.clone()), &genesis_header(&store));
    let empty = built.payload;
    let bal = Arc::new(built.block_access_list.expect("an LStar block has a BAL"));
    let gas_limit = empty.header.gas_limit;

    // The empty block with one more transaction, whose one claim on the looping
    // verifier has `claim_gas`, and a witness of the length the claim declares.
    let block_claiming = |claim_gas: u64| {
        let mut block = empty.clone();
        let tx = frame_transaction(
            chain_id,
            CUSTOM_ACCOUNT,
            vec![Claim {
                gas_limit: claim_gas,
                ..custom_claim(LOOPING_VERIFIER)
            }],
        );
        block
            .body
            .transactions
            .push(Transaction::FrameTransaction(tx));
        with_aggregated(block, &[], &[Bytes::from(vec![0u8; 32])])
    };

    assert_rejected_on_both_paths(
        block_claiming(gas_limit + 1),
        &bal,
        fresh_store,
        "one claim over the block's gas limit",
        |error| {
            matches!(
                error,
                InvalidBlockError::ClaimGasExceedsGasLimit { claim_gas, gas_limit: limit }
                    if *claim_gas == gas_limit + 1 && *limit == gas_limit
            )
        },
    )
    .await;
    // At the limit the verifier runs, spends the claim's gas and fails.
    assert_rejected_on_both_paths(
        block_claiming(gas_limit),
        &bal,
        fresh_store,
        "one claim at the block's gas limit",
        |error| matches!(error, InvalidBlockError::ClaimNotSatisfied(0)),
    )
    .await;
}

// ==================== The byte string ====================

#[tokio::test]
async fn an_lstar_block_is_held_to_its_commitment_and_to_the_block_witness_limit() {
    let first = key(0xB8);
    let second = key(0xB9);
    let signer = secp256k1_key(0xBA);
    let keys = [&first, &second];
    let built = lstar_block(&first, &second, &signer).await;

    // A header that commits to other bytes than the block carries: here, to the
    // same witness beside another valid proof.
    let mut other_commitment = built.block.clone();
    other_commitment.header.claim_witnesses_hash = Some(compute_claim_witnesses_hash(
        &encode_aggregated_claim_witnesses(
            &proof_of(&[(&first, built.messages[0])]),
            std::slice::from_ref(&built.witness),
        ),
        &NativeCrypto,
    ));
    other_commitment.header.hash = Default::default();
    assert_all_rejected(
        &built,
        &keys,
        vec![("a wrong header commitment", other_commitment)],
        |error| matches!(error, InvalidBlockError::ClaimWitnessesHashMismatch),
    )
    .await;

    // Claim witnesses longer than a block may carry, whatever they hold: the
    // block's proof and witness, and a second witness that takes them over.
    let oversized = encode_aggregated_claim_witnesses(
        &built.proof,
        &[
            built.witness.clone(),
            Bytes::from(vec![0u8; MAX_BLOCK_WITNESS_BYTES]),
        ],
    );
    assert!(oversized.len() > MAX_BLOCK_WITNESS_BYTES);
    assert_all_rejected(
        &built,
        &keys,
        vec![(
            "claim witnesses over the size limit",
            with_bytes(built.block.clone(), oversized),
        )],
        |error| {
            matches!(error, InvalidBlockError::InvalidClaimWitnesses(reason)
                if reason.contains("MAX_BLOCK_WITNESS_BYTES"))
        },
    )
    .await;
}

#[tokio::test]
async fn claim_witnesses_are_valid_only_under_the_version_byte_of_the_blocks_fork() {
    let first = key(0xBB);
    let second = key(0xBC);
    let signer = secp256k1_key(0xBD);
    let keys = [&first, &second];
    let built = lstar_block(&first, &second, &signer).await;
    let claim_witnesses = built.block.body.claim_witnesses.clone().unwrap();
    assert_eq!(claim_witnesses[0], 0x01);

    // The witnesses a Hegota block carries for these transactions, in the raw
    // form: every one of them is valid, and the form is not the fork's.
    let raw_witnesses: Vec<Bytes> = block_dependencies(&built.block.body.transactions)
        .unwrap()
        .iter()
        .map(|dependency| {
            if dependency.verifier == SECP256K1_VERIFIER {
                built.witness.clone()
            } else if dependency.key == first.public {
                sign(&first, dependency.msg)
            } else {
                sign(&second, dependency.msg)
            }
        })
        .collect();
    let raw_form = encode_claim_witnesses(&raw_witnesses);
    assert_eq!(raw_form[0], 0x00);

    let mut unknown_version = claim_witnesses.to_vec();
    unknown_version[0] = 0x02;
    let mut trailing = claim_witnesses.to_vec();
    trailing.push(0x00);
    let invalid = [
        ("version 0x00 in an LStar block", raw_form.to_vec()),
        ("an unknown version byte", unknown_version),
        ("trailing bytes after the RLP", trailing),
        ("the version byte alone", vec![0x01]),
        // A list of raw witnesses under the new version byte.
        ("a list without a proof", {
            let mut bytes = raw_form.to_vec();
            bytes[0] = 0x01;
            bytes
        }),
    ]
    .into_iter()
    .map(|(what, bytes)| (what, with_bytes(built.block.clone(), Bytes::from(bytes))))
    .collect();
    assert_all_rejected(&built, &keys, invalid, |error| {
        matches!(error, InvalidBlockError::InvalidClaimWitnesses(_))
    })
    .await;

    // The same transactions on a chain where LStar never activates. Its first
    // block is a Hegota block, built with the raw form.
    let fresh_store = || async { setup_store(&keys, &built.accounts, None).await.0 };
    let (store, chain_id) = setup_store(&keys, &built.accounts, None).await;
    let blockchain = blockchain(store.clone());
    for sender in keys {
        blockchain
            .add_frame_transaction_to_pool(pooled(
                claim_transaction(chain_id, sender, Vec::new()),
                &[sender],
            ))
            .await
            .expect("the leanSPHINCS transaction enters the pool");
    }
    blockchain
        .add_frame_transaction_to_pool(secp256k1_pooled(chain_id, &signer))
        .await
        .expect("the secp256k1 transaction enters the pool");
    let hegota_built = build_block(&store, &blockchain, &genesis_header(&store));
    let hegota_block = hegota_built.payload;
    let hegota_bal = Arc::new(
        hegota_built
            .block_access_list
            .expect("a Hegota block has a BAL"),
    );
    assert_eq!(hegota_block.body.transactions.len(), 3);
    assert_eq!(
        block_dependencies(&hegota_block.body.transactions),
        block_dependencies(&built.block.body.transactions),
        "the two blocks have the same dependencies"
    );
    assert_eq!(hegota_block.body.claim_witnesses.as_ref().unwrap()[0], 0x00);
    for result in import_on_both_paths(&hegota_block, &hegota_bal, fresh_store).await {
        result.expect("the Hegota block imports with its raw witnesses");
    }

    // The aggregated form is not valid before LStar, although its proof holds
    // for the block's leanSPHINCS claims and its raw witness is the block's.
    assert_rejected_on_both_paths(
        with_bytes(hegota_block, claim_witnesses),
        &hegota_bal,
        fresh_store,
        "version 0x01 in a Hegota block",
        |error| matches!(error, InvalidBlockError::InvalidClaimWitnesses(_)),
    )
    .await;
}

// ==================== The pool ====================

// The fork changes nothing for a pending transaction: it is submitted with the
// raw witness of each claim, the pool verifies each one natively, keeps them
// and serves them to peers.
#[tokio::test]
async fn the_pool_still_takes_and_verifies_raw_leansphincs_witnesses_after_lstar() {
    let key = key(0xBE);
    let other_key = self::key(0xBF);
    // LStar at genesis: the head the pool validates against is an LStar block.
    let (store, chain_id) = setup_store(&[&key], &[], Some(0)).await;
    let genesis = genesis_header(&store);
    assert!(
        store
            .get_chain_config()
            .is_lstar_activated(genesis.timestamp)
    );
    let blockchain = blockchain(store.clone());
    let tx = claim_transaction(chain_id, &key, Vec::new());
    let valid = pooled(tx.clone(), &[&key]);

    let forged = PooledFrameTransaction {
        tx: tx.clone(),
        witnesses: vec![sign(&other_key, tx.compute_sig_hash())],
        blobs_bundle: None,
    };
    let result = blockchain.add_frame_transaction_to_pool(forged).await;
    assert!(
        matches!(result, Err(MempoolError::ClaimNotSatisfied(0))),
        "got {result:?}"
    );
    let result = blockchain
        .add_transaction_to_pool(Transaction::FrameTransaction(tx.clone()))
        .await;
    assert!(
        matches!(result, Err(MempoolError::FrameTxWithoutWitnesses)),
        "got {result:?}"
    );

    let hash = blockchain
        .add_frame_transaction_to_pool(valid.clone())
        .await
        .expect("the transaction enters the pool with its witness");
    assert_eq!(hash, tx_hash(&tx));
    assert_eq!(
        blockchain.mempool.get_claim_witnesses(hash).unwrap(),
        Some(valid.witnesses.clone())
    );
    assert_eq!(
        blockchain
            .get_p2p_transaction_by_hash(&hash)
            .unwrap()
            .encode_canonical_to_vec(),
        P2PTransaction::FrameTransaction(valid).encode_canonical_to_vec()
    );

    // The block built from it proves the signature and does not carry it.
    let built = build_block(&store, &blockchain, &genesis);
    assert_eq!(built.payload.body.transactions.len(), 1);
    let claim_witnesses = aggregated(&built.payload);
    assert!(claim_witnesses.witnesses.is_empty());
    assert!(verify_aggregated_dependencies(
        &[leansphincs_dependency(&key, tx.compute_sig_hash())],
        &claim_witnesses.aggregate_proof
    ));
}

// ==================== Across the fork ====================

#[tokio::test]
async fn an_account_keeps_authorizing_with_the_same_leansphincs_key_across_lstar() {
    let key = key(0xC0);
    let keys = [&key];
    // Hegota at genesis and for the first block; LStar from the second block on.
    let lstar_after = Some(2 * BLOCK_TIME);
    let (store, chain_id) = setup_store(&keys, &[], lstar_after).await;
    let blockchain = blockchain(store.clone());
    let genesis = genesis_header(&store);
    let account_at_genesis = store
        .get_account_state(0, key.account)
        .await
        .unwrap()
        .expect("the account is in the genesis state");

    // The account authorizes a transaction in a Hegota block: the block carries
    // the signature.
    let first_pooled = pooled(claim_transaction(chain_id, &key, Vec::new()), &keys);
    let first_witness = first_pooled.witnesses[0].clone();
    blockchain
        .add_frame_transaction_to_pool(first_pooled)
        .await
        .expect("the transaction enters the pool");
    let first_built = build_block(&store, &blockchain, &genesis);
    let first_block = first_built.payload;
    assert!(
        !store
            .get_chain_config()
            .is_lstar_activated(first_block.header.timestamp)
    );
    assert_eq!(first_block.body.transactions.len(), 1);
    assert!(first_built.receipts[0].succeeded);
    let first_claim_witnesses = first_block.body.claim_witnesses.clone().unwrap();
    assert_eq!(
        first_claim_witnesses,
        encode_claim_witnesses(std::slice::from_ref(&first_witness))
    );
    assert_eq!(first_claim_witnesses[0], 0x00);
    blockchain
        .add_block(first_block.clone())
        .expect("the Hegota block imports");
    make_head(&store, &first_block).await;
    blockchain
        .remove_block_transactions_from_pool(&first_block)
        .unwrap();

    // The account's next transaction is signed and enters the pool while the
    // head is still a Hegota block.
    let mut second_tx = claim_transaction(chain_id, &key, Vec::new());
    second_tx.nonce_seq = 1;
    let second_message = second_tx.compute_sig_hash();
    let second_pooled = pooled(second_tx.clone(), &keys);
    let second_witness = second_pooled.witnesses[0].clone();
    let hash_in_the_pool = blockchain
        .add_frame_transaction_to_pool(second_pooled)
        .await
        .expect("the transaction enters the pool before LStar");
    assert_eq!(hash_in_the_pool, tx_hash(&second_tx));

    // The chain crosses LStar. The block that includes the transaction proves
    // the signature the pool holds, the one made before the fork, and does not
    // carry it.
    assert_eq!(
        blockchain
            .mempool
            .get_claim_witnesses(hash_in_the_pool)
            .unwrap(),
        Some(vec![second_witness.clone()])
    );
    let second_built = build_block(&store, &blockchain, &first_block.header);
    let second_block = second_built.payload;
    assert!(
        store
            .get_chain_config()
            .is_lstar_activated(second_block.header.timestamp)
    );
    assert_eq!(second_block.body.transactions.len(), 1);
    assert_eq!(
        second_block.body.transactions[0].hash(&NativeCrypto),
        hash_in_the_pool,
        "the transaction has the same hash in the pool and in the block"
    );
    assert!(second_built.receipts[0].succeeded);
    let second_claim_witnesses = second_block.body.claim_witnesses.clone().unwrap();
    assert_eq!(second_claim_witnesses[0], 0x01);
    let ClaimWitnesses {
        aggregate_proof,
        witnesses,
    } = aggregated(&second_block);
    assert!(witnesses.is_empty());
    let dependency = leansphincs_dependency(&key, second_message);
    assert_eq!(
        block_dependencies(&second_block.body.transactions).unwrap(),
        vec![dependency]
    );
    assert!(verify_aggregated_dependencies(
        &[dependency],
        &aggregate_proof
    ));
    assert!(
        !second_claim_witnesses
            .windows(64)
            .any(|window| second_witness.starts_with(window)),
        "the signature is not in the block"
    );
    blockchain
        .add_block(second_block.clone())
        .expect("the LStar block imports");
    make_head(&store, &second_block).await;

    // The account is the one of the genesis state: same code, same storage.
    // Only its nonce and its balance moved.
    for (number, nonce) in [(1, 1), (2, 2)] {
        let account = store
            .get_account_state(number, key.account)
            .await
            .unwrap()
            .expect("the account is in the state");
        assert_eq!(account.code_hash, account_at_genesis.code_hash);
        assert_eq!(account.storage_root, account_at_genesis.storage_root);
        assert_eq!(account.nonce, nonce);
    }

    // Another node imports the two blocks, one of each fork, on both paths.
    let other = self::blockchain(setup_store(&keys, &[], lstar_after).await.0);
    other
        .add_block(first_block.clone())
        .expect("plain import of the Hegota block");
    other
        .add_block(second_block.clone())
        .expect("plain import of the LStar block");
    let other = self::blockchain(setup_store(&keys, &[], lstar_after).await.0);
    other
        .add_block_pipeline(
            first_block.clone(),
            first_built.block_access_list.map(Arc::new),
        )
        .expect("pipeline import of the Hegota block");
    other
        .add_block_pipeline(
            second_block.clone(),
            second_built.block_access_list.map(Arc::new),
        )
        .expect("pipeline import of the LStar block");

    // The two transactions do the same work and pay the same gas: what a claim
    // costs follows from the transaction body, which the fork does not change.
    let gas_before = first_built.receipts[0].cumulative_gas_used;
    let gas_after = second_built.receipts[0].cumulative_gas_used;
    println!(
        "Gas of the transaction in the Hegota block: {gas_before}. Gas of the transaction in \
         the LStar block: {gas_after}. Hash of the second transaction, in the pool and in the \
         block: {hash_in_the_pool:#x}."
    );
    println!(
        "Hegota block: {} bytes of RLP block, of which {} bytes of claim_witnesses. LStar \
         block: {} bytes of RLP block, of which {} bytes of claim_witnesses ({} bytes of \
         proof).",
        first_block.length(),
        first_claim_witnesses.len(),
        second_block.length(),
        second_claim_witnesses.len(),
        aggregate_proof.len(),
    );
    assert_eq!(first_block.header.gas_used, gas_before);
    assert_eq!(second_block.header.gas_used, gas_after);
    assert_eq!(gas_before, gas_after);
}

// ==================== The builder ====================

// Before LStar a block's leanSPHINCS witnesses count toward
// MAX_BLOCK_WITNESS_BYTES, 7,740 bytes each, so a block holds at most 270 of
// them. From LStar they leave the block and one proof takes their place, so
// that count does not bound the block any more.
#[tokio::test]
async fn from_lstar_the_builder_includes_more_leansphincs_claims_than_fit_a_block_as_witnesses() {
    // Sixty-four leanSPHINCS claims a transaction, just under half a megabyte
    // of witnesses each: four of these transactions fit a Hegota block and the
    // fifth does not.
    const CLAIMS: usize = 64;
    const SENDERS: u8 = 5;
    let witnesses_that_fit_a_block = MAX_BLOCK_WITNESS_BYTES / LEANSPHINCS_SIGNATURE_SIZE;
    assert_eq!(witnesses_that_fit_a_block, 270);
    assert!(usize::from(SENDERS) * CLAIMS > witnesses_that_fit_a_block);

    let senders: Vec<Key> = (0..SENDERS).map(|i| key(0xD0 + i)).collect();
    let keys: Vec<&Key> = senders.iter().collect();
    let (store, chain_id) = setup_store(&keys, &[], LSTAR_AT_FIRST_BLOCK).await;
    // The public mempool's validation budget admits seven leanSPHINCS claims a
    // transaction; this node is told to verify more.
    let blockchain = Blockchain::new(
        store.clone(),
        BlockchainOptions {
            min_tip_wei: 0,
            max_verify_gas: 2_000_000,
            ..Default::default()
        },
    );
    for sender in &senders {
        let more_claims = (1..CLAIMS)
            .map(|i| leansphincs_claim(sender, Some(H256::from_low_u64_be(i as u64))))
            .collect();
        let pooled_tx = pooled(claim_transaction(chain_id, sender, more_claims), &[sender]);
        assert_eq!(pooled_tx.witnesses.len(), CLAIMS);
        blockchain
            .add_frame_transaction_to_pool(pooled_tx)
            .await
            .expect("the transaction enters the pool");
    }

    let built = build_block(&store, &blockchain, &genesis_header(&store));
    let block = built.payload;
    assert_eq!(block.body.transactions.len(), usize::from(SENDERS));
    assert!(built.receipts.iter().all(|receipt| receipt.succeeded));
    let dependencies = block_dependencies(&block.body.transactions).unwrap();
    assert_eq!(dependencies.len(), usize::from(SENDERS) * CLAIMS);
    assert!(dependencies.len() > witnesses_that_fit_a_block);

    // The block carries no witness, and one proof that is shorter than the
    // witnesses of a single one of its transactions.
    let claim_witnesses = block.body.claim_witnesses.clone().unwrap();
    let ClaimWitnesses {
        aggregate_proof,
        witnesses,
    } = aggregated(&block);
    assert!(witnesses.is_empty());
    assert!(aggregate_proof.len() < CLAIMS * LEANSPHINCS_SIGNATURE_SIZE);
    assert!(claim_witnesses.len() <= MAX_BLOCK_WITNESS_BYTES);
    println!(
        "{} leanSPHINCS dependencies in one LStar block, {} more than fit a block as witnesses: \
         {} bytes of claim_witnesses, where their witnesses take {} bytes.",
        dependencies.len(),
        dependencies.len() - witnesses_that_fit_a_block,
        claim_witnesses.len(),
        dependencies.len() * LEANSPHINCS_SIGNATURE_SIZE,
    );

    let bal = Arc::new(built.block_access_list.expect("an LStar block has a BAL"));
    let fresh_store = || async { setup_store(&keys, &[], LSTAR_AT_FIRST_BLOCK).await.0 };
    for result in import_on_both_paths(&block, &bal, fresh_store).await {
        result.expect("the block imports");
    }
}

#[tokio::test]
async fn the_payload_build_loop_proves_the_payload_it_returns() {
    let first = key(0xC1);
    let second = key(0xC2);
    let keys = [&first, &second];
    let (store, chain_id) = setup_store(&keys, &[], LSTAR_AT_FIRST_BLOCK).await;
    let blockchain = Arc::new(blockchain(store.clone()));
    for sender in keys {
        blockchain
            .add_frame_transaction_to_pool(pooled(
                claim_transaction(chain_id, sender, Vec::new()),
                &[sender],
            ))
            .await
            .expect("the transaction enters the pool");
    }

    // The loop's first build runs up to its first await. Cancelling it, as
    // `engine_getPayload` does, ends the loop with that build.
    let token = CancellationToken::new();
    let mut building = std::pin::pin!(
        blockchain
            .clone()
            .build_payload_loop(payload_on(&store, &genesis_header(&store)), token.clone())
    );
    std::future::poll_fn(|cx| {
        assert!(
            building.as_mut().poll(cx).is_pending(),
            "the build loop finished before it was cancelled"
        );
        std::task::Poll::Ready(())
    })
    .await;
    token.cancel();
    let built = building.await.expect("the payload is built and proven");

    let block = built.payload;
    assert_eq!(block.body.transactions.len(), 2);
    assert!(built.unaggregated_claim_witnesses.is_empty());
    let claim_witnesses = block.body.claim_witnesses.clone().unwrap();
    assert!(!aggregated(&block).aggregate_proof.is_empty());
    assert_eq!(
        block.header.claim_witnesses_hash,
        Some(compute_claim_witnesses_hash(
            &claim_witnesses,
            &NativeCrypto
        ))
    );
    let bal = Arc::new(built.block_access_list.expect("an LStar block has a BAL"));
    let fresh_store = || async { setup_store(&keys, &[], LSTAR_AT_FIRST_BLOCK).await.0 };
    for result in import_on_both_paths(&block, &bal, fresh_store).await {
        result.expect("the block imports");
    }
}

// A caller that drives the build steps itself, as a block producer outside the
// payload build loop does, gets a whole block from the step that seals it: the
// proof is in the block, and nothing is left to aggregate.
#[tokio::test]
async fn finalizing_a_payload_puts_the_aggregate_proof_in_the_block() {
    use ethrex_blockchain::vm::StoreVmDatabase;
    use ethrex_guest_program::common::verify_claim_witnesses;
    use ethrex_vm::Evm;

    let first = key(0xE1);
    let second = key(0xE2);
    let keys = [&first, &second];
    let (store, chain_id) = setup_store(&keys, &[], LSTAR_AT_FIRST_BLOCK).await;
    let blockchain = blockchain(store.clone());
    for sender in keys {
        blockchain
            .add_frame_transaction_to_pool(pooled(
                claim_transaction(chain_id, sender, Vec::new()),
                &[sender],
            ))
            .await
            .expect("the transaction enters the pool");
    }

    let genesis = genesis_header(&store);
    let mut context = PayloadBuildContext::new(
        payload_on(&store, &genesis),
        &store,
        &blockchain.options.r#type,
    )
    .expect("a build context");
    blockchain
        .apply_system_operations(&mut context)
        .expect("system operations");
    blockchain
        .fill_transactions(&mut context, None)
        .expect("the transactions are added");
    blockchain.extract_requests(&mut context).expect("requests");
    blockchain
        .apply_withdrawals(&mut context)
        .expect("withdrawals");
    blockchain
        .finalize_payload(&mut context)
        .expect("the payload is sealed and proven");

    let block = context.payload;
    assert_eq!(block.body.transactions.len(), 2);
    assert!(context.unaggregated_claim_witnesses.is_empty());
    let ClaimWitnesses {
        aggregate_proof,
        witnesses,
    } = aggregated(&block);
    assert!(witnesses.is_empty());
    assert!(verify_aggregated_dependencies(
        &block_dependencies(&block.body.transactions).unwrap(),
        &aggregate_proof
    ));
    // The block's claim witnesses pass the check every import runs.
    let evm = Evm::new_for_l1(
        StoreVmDatabase::new(store, genesis).unwrap(),
        Arc::new(NativeCrypto),
    );
    verify_claim_witnesses(&block, &evm, true).expect("the claims are verified");
}

// ==================== The Engine API ====================

/// The parameters of `engine_newPayloadV5` for a block of these tests: no blobs,
/// a zero beacon root and no execution requests.
fn new_payload_params(payload: Value) -> Option<Vec<Value>> {
    Some(vec![payload, json!([]), json!(H256::zero()), json!([])])
}

/// What `engine_newPayloadV5` answers on a node that starts from `store`.
async fn new_payload(payload: Value, store: Store) -> Value {
    let context = default_context_with_storage(store).await;
    NewPayloadV5Request::parse(&new_payload_params(payload))
        .expect("well-formed newPayloadV5 params")
        .handle(context.clone())
        .await
        .expect("newPayload returns a payload status")
}

fn payload_json(block: Block, bal: &BlockAccessList) -> Value {
    serde_json::to_value(ExecutionPayload::from_block(block, Some(bal.clone())))
        .expect("payload to json")
}

#[tokio::test]
async fn aggregated_claim_witnesses_travel_through_get_payload_and_new_payload() {
    const PAYLOAD_ID: u64 = 1;
    let first = key(0xC3);
    let second = key(0xC4);
    let signer = secp256k1_key(0xC5);
    let keys = [&first, &second];
    let accounts = [secp256k1_account(&signer)];
    let (store, chain_id) = setup_store(&keys, &accounts, LSTAR_AT_FIRST_BLOCK).await;
    let builder = default_context_with_storage(store.clone()).await;
    for sender in keys {
        builder
            .blockchain
            .add_frame_transaction_to_pool(pooled(
                claim_transaction(chain_id, sender, Vec::new()),
                &[sender],
            ))
            .await
            .expect("the leanSPHINCS transaction enters the pool");
    }
    let secp256k1_tx = secp256k1_pooled(chain_id, &signer);
    let secp256k1_witness = secp256k1_tx.witnesses[0].clone();
    builder
        .blockchain
        .add_frame_transaction_to_pool(secp256k1_tx)
        .await
        .expect("the secp256k1 transaction enters the pool");

    // The builder starts a payload, as on a fork choice update with payload
    // attributes. Its first build runs before anything else on this thread.
    builder
        .blockchain
        .clone()
        .initiate_payload_build(payload_on(&store, &genesis_header(&store)), PAYLOAD_ID)
        .await;
    tokio::task::yield_now().await;

    // `engine_getPayloadV6` returns the claim witnesses under the field it had
    // before LStar, in the aggregated form: the proof and the one raw witness.
    let response = GetPayloadV6Request {
        payload_id: PAYLOAD_ID,
    }
    .handle(builder.clone())
    .await
    .expect("getPayload returns the payload");
    let payload = response["executionPayload"].clone();
    assert_eq!(payload["transactions"].as_array().map(Vec::len), Some(3));
    let claim_witnesses = payload["claimWitnesses"]
        .as_str()
        .expect("the payload has claim witnesses");
    assert!(claim_witnesses.starts_with("0x01"));
    let decoded = decode_claim_witnesses_at(
        Fork::LStar,
        &hex::decode(&claim_witnesses[2..]).expect("claim witnesses are hex"),
    )
    .expect("the claim witnesses decode");
    assert!(!decoded.aggregate_proof.is_empty());
    assert_eq!(decoded.witnesses, vec![secp256k1_witness]);
    // The header's commitment is not a payload field: the receiver derives it,
    // and the block hash covers it.
    assert!(payload.get("claimWitnessesHash").is_none());

    // `engine_newPayloadV5` on another node accepts it.
    let fresh_store = || async { setup_store(&keys, &accounts, LSTAR_AT_FIRST_BLOCK).await.0 };
    let status = new_payload(payload.clone(), fresh_store().await).await;
    assert_eq!(status["status"], "VALID", "got {status:?}");
    assert_eq!(status["latestValidHash"], payload["blockHash"]);

    // The same payload with other claim witnesses is not the block its hash
    // names.
    let mut other_bytes = payload.clone();
    other_bytes["claimWitnesses"] = json!(format!("{claim_witnesses}00"));
    let status = new_payload(other_bytes, fresh_store().await).await;
    assert_eq!(status["status"], "INVALID", "got {status:?}");
    // Nor is it without them: from Hegota on an absent field is the empty byte
    // string.
    let mut absent = payload;
    absent
        .as_object_mut()
        .expect("the payload is an object")
        .remove("claimWitnesses");
    let status = new_payload(absent, fresh_store().await).await;
    assert_eq!(status["status"], "INVALID", "got {status:?}");
}

#[tokio::test]
async fn the_engine_answers_invalid_to_a_payload_whose_aggregated_claim_witnesses_do_not_hold() {
    let first = key(0xC6);
    let second = key(0xC7);
    let other = key(0xC8);
    let signer = secp256k1_key(0xC9);
    let keys = [&first, &second];
    let built = lstar_block(&first, &second, &signer).await;
    let [first_message, second_message] = built.messages;
    let fresh_store = || async {
        setup_store(&keys, &built.accounts, LSTAR_AT_FIRST_BLOCK)
            .await
            .0
    };

    // The block as built is valid.
    let status = new_payload(
        payload_json(built.block.clone(), &built.bal),
        fresh_store().await,
    )
    .await;
    assert_eq!(status["status"], "VALID", "got {status:?}");

    // In each of these the block hash is the hash of the block the payload
    // describes, so only the claim witnesses make the payload invalid.
    let raw = std::slice::from_ref(&built.witness);
    let aggregated_form = |proof: &[u8], witnesses: &[Bytes]| {
        encode_aggregated_claim_witnesses(proof, witnesses).to_vec()
    };
    let mut flipped = built.proof.to_vec();
    flipped[built.proof.len() / 2] ^= 0x01;
    let mut oversized = built.proof.to_vec();
    oversized.resize(MAX_AGGREGATE_PROOF_BYTES + 1, 0);
    let claim_witnesses = built.block.body.claim_witnesses.clone().unwrap();
    let mut unknown_version = claim_witnesses.to_vec();
    unknown_version[0] = 0x02;
    let mut trailing = claim_witnesses.to_vec();
    trailing.push(0x00);
    let invalid = [
        (
            "a proof that covers one claim fewer",
            aggregated_form(&proof_of(&[(&first, first_message)]), raw),
        ),
        (
            "a proof for the same key and another message",
            aggregated_form(
                &proof_of(&[(&first, H256::repeat_byte(0x11)), (&second, second_message)]),
                raw,
            ),
        ),
        (
            "a proof for another key and the same message",
            aggregated_form(
                &proof_of(&[(&other, first_message), (&second, second_message)]),
                raw,
            ),
        ),
        (
            "a proof that covers the block's claims and one more",
            aggregated_form(
                &proof_of(&[
                    (&first, first_message),
                    (&second, second_message),
                    (&other, H256::repeat_byte(0x22)),
                ]),
                raw,
            ),
        ),
        ("a flipped bit in the proof", aggregated_form(&flipped, raw)),
        (
            "a truncated proof",
            aggregated_form(&built.proof[..built.proof.len() / 2], raw),
        ),
        ("an empty proof", aggregated_form(&[], raw)),
        (
            "a proof over its size bound",
            aggregated_form(&oversized, raw),
        ),
        ("a missing raw witness", aggregated_form(&built.proof, &[])),
        (
            "an extra raw witness",
            aggregated_form(
                &built.proof,
                &[built.witness.clone(), built.witness.clone()],
            ),
        ),
        (
            "a raw witness that does not satisfy its claim",
            aggregated_form(
                &built.proof,
                &[secp256k1_sign(&secp256k1_key(0xCA), first_message)],
            ),
        ),
        (
            "claim witnesses over the block's limit",
            aggregated_form(
                &built.proof,
                &[
                    built.witness.clone(),
                    Bytes::from(vec![0u8; MAX_BLOCK_WITNESS_BYTES]),
                ],
            ),
        ),
        (
            "version 0x00 in an LStar block",
            encode_claim_witnesses(&[
                built.witness.clone(),
                sign(&first, first_message),
                sign(&second, second_message),
            ])
            .to_vec(),
        ),
        ("an unknown version byte", unknown_version),
        ("trailing bytes after the RLP", trailing),
    ];
    for (what, bytes) in invalid {
        let block = with_bytes(built.block.clone(), Bytes::from(bytes));
        let status = new_payload(payload_json(block, &built.bal), fresh_store().await).await;
        assert_eq!(status["status"], "INVALID", "{what}: got {status:?}");
    }

    // A header that commits to other claim witnesses cannot be told from a wrong
    // block hash through the Engine API, since the receiver derives the
    // commitment from the field: the answer is the same.
    let mut payload = payload_json(built.block.clone(), &built.bal);
    payload["claimWitnesses"] = json!(format!(
        "0x{}",
        hex::encode(aggregated_form(&proof_of(&[(&first, first_message)]), raw))
    ));
    let status = new_payload(payload, fresh_store().await).await;
    assert_eq!(status["status"], "INVALID", "got {status:?}");

    // A proof in a block without leanSPHINCS claims.
    let store = fresh_store().await;
    let empty = build_block(&store, &blockchain(store.clone()), &genesis_header(&store));
    let empty_bal = empty.block_access_list.expect("an LStar block has a BAL");
    let status = new_payload(
        payload_json(empty.payload.clone(), &empty_bal),
        fresh_store().await,
    )
    .await;
    assert_eq!(status["status"], "VALID", "got {status:?}");
    let status = new_payload(
        payload_json(
            with_aggregated(empty.payload, &built.proof, &[]),
            &empty_bal,
        ),
        fresh_store().await,
    )
    .await;
    assert_eq!(status["status"], "INVALID", "got {status:?}");
}

// A Hegota payload carries the raw form, and the aggregated form makes it
// invalid, on a chain where LStar is scheduled for later.
#[tokio::test]
async fn the_engine_reads_claim_witnesses_under_the_fork_of_the_payload() {
    let first = key(0xCB);
    let second = key(0xCC);
    let signer = secp256k1_key(0xCD);
    let keys = [&first, &second];
    // The block of the same transactions on a chain that is in LStar already.
    let lstar = lstar_block(&first, &second, &signer).await;

    let lstar_after = Some(2 * BLOCK_TIME);
    let fresh_store = || async { setup_store(&keys, &lstar.accounts, lstar_after).await.0 };
    let (store, chain_id) = setup_store(&keys, &lstar.accounts, lstar_after).await;
    let blockchain = blockchain(store.clone());
    for sender in keys {
        blockchain
            .add_frame_transaction_to_pool(pooled(
                claim_transaction(chain_id, sender, Vec::new()),
                &[sender],
            ))
            .await
            .expect("the leanSPHINCS transaction enters the pool");
    }
    blockchain
        .add_frame_transaction_to_pool(secp256k1_pooled(chain_id, &signer))
        .await
        .expect("the secp256k1 transaction enters the pool");
    let built = build_block(&store, &blockchain, &genesis_header(&store));
    let bal = built.block_access_list.expect("a Hegota block has a BAL");
    let payload = payload_json(built.payload.clone(), &bal);
    assert!(
        payload["claimWitnesses"]
            .as_str()
            .expect("the payload has claim witnesses")
            .starts_with("0x00")
    );
    let status = new_payload(payload, fresh_store().await).await;
    assert_eq!(status["status"], "VALID", "got {status:?}");

    let aggregated_form = lstar.block.body.claim_witnesses.clone().unwrap();
    let status = new_payload(
        payload_json(with_bytes(built.payload, aggregated_form), &bal),
        fresh_store().await,
    )
    .await;
    assert_eq!(status["status"], "INVALID", "got {status:?}");
}

// ==================== Stateless execution ====================

// A build of stateless execution without the leanSPHINCS verifiers cannot check
// the aggregate proof either. It refuses an LStar block with a leanSPHINCS
// claim, as it refuses a Hegota one, and checks every other block as a node
// does.
#[tokio::test]
async fn stateless_execution_refuses_the_leansphincs_claims_of_an_lstar_block_it_cannot_verify() {
    use ethrex_blockchain::vm::StoreVmDatabase;
    use ethrex_guest_program::common::{ExecutionError, verify_claim_witnesses};
    use ethrex_vm::Evm;

    let first = key(0xCE);
    let second = key(0xCF);
    let signer = secp256k1_key(0xE0);
    let built = lstar_block(&first, &second, &signer).await;
    let evm_over = |store: Store| {
        let parent = genesis_header(&store);
        Evm::new_for_l1(
            StoreVmDatabase::new(store, parent).unwrap(),
            Arc::new(NativeCrypto),
        )
    };
    let (store, chain_id) =
        setup_store(&[&first, &second], &built.accounts, LSTAR_AT_FIRST_BLOCK).await;
    let evm = evm_over(store.clone());

    // This build links the verifiers, so the proof is checked and holds.
    const { assert!(ethrex_vm::LEANSPHINCS_VERIFIER_LINKED) };
    verify_claim_witnesses(&built.block, &evm, true).expect("the claims are verified");
    // A build that does not link them refuses the block.
    let result = verify_claim_witnesses(&built.block, &evm, false);
    assert!(
        matches!(result, Err(ExecutionError::UnverifiedClaimWitnesses(1))),
        "got {result:?}"
    );

    // An LStar block without leanSPHINCS claims is checked the same either way.
    let blockchain = blockchain(store.clone());
    let secp256k1_tx = secp256k1_pooled(chain_id, &signer);
    blockchain
        .add_frame_transaction_to_pool(secp256k1_tx)
        .await
        .expect("the secp256k1 transaction enters the pool");
    let raw_only = build_block(&store, &blockchain, &genesis_header(&store)).payload;
    assert_eq!(raw_only.body.transactions.len(), 1);
    verify_claim_witnesses(&raw_only, &evm, false).expect("the secp256k1 claim is verified");
    let invalid = with_aggregated(raw_only, &[], &[Bytes::from(vec![0x01; 65])]);
    let result = verify_claim_witnesses(&invalid, &evm, false);
    assert!(
        matches!(
            result,
            Err(ExecutionError::BlockValidation(
                InvalidBlockError::ClaimNotSatisfied(0)
            ))
        ),
        "got {result:?}"
    );
}
