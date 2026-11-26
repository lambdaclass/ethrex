//! Adapter of the measurement harness for the PROOF frame proposal, from IStar.
//!
//! The signature travels inside the transaction. A mode-4 proof frame carries
//! `key || msg || proof` and names its verifier as its target; the protocol
//! runs the verifier, here the leanSPHINCS verifier precompile, before any
//! other frame, and a claim that does not hold invalidates the transaction.
//! Nothing travels beside the transaction.
//!
//! From IStar a block does not carry those signatures. Its builder executes
//! the transactions with their proofs and then strips every leanSPHINCS proof
//! the protocol may strip: the `data` of a stripped frame is the list
//! `[claim, proof_length]`, and the verifier is not run for it. One aggregate
//! proof stands for the proofs that left. The block carries it as the last
//! item of its body, inside the block's RLP encoding and outside the EIP-7934
//! size check, and the header commits to it with `aggregate_proof_hash`. Gas
//! does not change: the payer is still charged for the proof by its length.
//!
//! A `Submission` therefore describes the transaction with its proofs, the form
//! that is signed, submitted, pooled and exchanged by peers. The block holds
//! the stripped form, whose hash is another one: only the signature hash names
//! the transaction from its submission to its inclusion.
//!
//! Transactions here are `[PROOF.., VERIFY, SENDER]`. The account's code reads
//! its claims at fixed positions: proof frame `i` is the one of key `i`. The
//! validation frame therefore carries no data, as in the other alternatives.

use std::{collections::BTreeMap, sync::Mutex};

use bytes::Bytes;
use ethrex_blockchain::{Blockchain, aggregate_proof::aggregate_signatures};
use ethrex_common::{
    Address, H256, U256,
    constants::{MAX_RLP_BLOCK_SIZE, TX_MAX_GAS_LIMIT_AMSTERDAM},
    types::{
        Block, FRAME_TX_LEANSPHINCS_VERIFIER, FRAME_TX_MAX_FRAMES, FRAME_TX_MAX_VERIFY_GAS,
        FRAME_TX_SECP256K1_VERIFIER, Frame, FrameMode, FrameTransaction, Genesis,
        LEANSPHINCS_VERIFIER_GAS, MAX_AGGREGATE_PROOF_BYTES, PROOF_CLAIM_LENGTH, PROOF_READ_FLAG,
        SECP256K1_VERIFIER_GAS, Transaction, compute_aggregate_proof_hash,
        compute_transactions_root,
    },
};
use ethrex_crypto::{NativeCrypto, leansphincs::LEANSPHINCS_SIGNATURE_SIZE};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

use super::lifecycle::{
    AccountKind, BlockExtras, Fault, FaultCase, FaultyBlock, FaultySubmission, GasProfile,
    ImportPath, KeyPair, Layer, Limit, Submission, TxFacts, TxRequest, indexed_address,
    pooled_announcement_bytes, signature,
};

pub const ALTERNATIVE: &str = "Alternative 2";
pub const ALTERNATIVE_ID: &str = "a2";
pub const PROPOSAL: &str = "EIP-8141 PROOF frame mode";
pub const MILESTONE: u8 = 2;
pub const SIGNATURE_SCHEME: &str = "leanSPHINCS";
pub const KEY_CACHE_NAMESPACE: &str = "leansphincs";
pub const VERIFIER_PACKAGE: &str = "lean_sphincs";

/// Where this implementation verifies signatures from IStar, stage by stage.
pub const VERIFICATION: &[(&str, &str)] = &[
    (
        "pool admission",
        "as before IStar: each proof a precompile verifies is checked once, natively, on \
         the calling thread, before any state is read; the prefix simulation and the \
         revalidation after a block skip it. A transaction with a stripped proof is \
         refused",
    ),
    (
        "block construction",
        "the builder executes every transaction it includes with its proofs, which \
         verifies each proof again, natively, on the builder's thread. \
         Blockchain::build_payload then strips every proof the protocol may strip and \
         proves the stripped claims with one aggregate proof \
         (PayloadBuildResult::with_aggregate_proof), on the calling thread. The prover \
         builds the proof as a tree, one node at a time, with a leaf for every 512 \
         signatures; each node is proven on the prover's own thread pool \
         (LEANVM_NUM_THREADS, by default sized from the machine's cores), and the \
         process proves one block at a time",
    ),
    (
        "block import, add_block",
        "one verification of the aggregate proof against the claims of the block's \
         stripped proof frames, on the calling thread, before the block executes. \
         Executing a transaction does not verify a stripped proof; a proof that stayed \
         in its frame is verified inside the execution, on the calling thread",
    ),
    (
        "block import, add_block_pipeline",
        "the same verification of the aggregate proof, on the calling thread, before \
         execution starts. A proof that stayed in its frame is verified inside the \
         execution of its transaction, on the thread pool that executes the block's \
         transactions (RAYON_NUM_THREADS)",
    ),
];

/// Gas a proof frame needs for the leanSPHINCS verifier: its cost and the
/// access of a warm precompile.
const PROOF_FRAME_GAS_LIMIT: u64 = LEANSPHINCS_VERIFIER_GAS + 100;

// ---------------------------------------------------------------------------
// The signature scheme
// ---------------------------------------------------------------------------

pub fn generate_key(seed: &[u8; 32]) -> KeyPair {
    KeyPair {
        secret: seed.to_vec(),
        public: lean_sphincs::key_gen(seed).to_bytes().to_vec(),
    }
}

pub fn sign(key: &KeyPair, message: &[u8; 32]) -> Vec<u8> {
    let secret: [u8; 32] = key.secret.as_slice().try_into().expect("a 32-byte secret");
    lean_sphincs::sign(&secret, message).to_bytes().to_vec()
}

/// The verifier the precompile calls for every proof.
pub fn verify(public: &[u8], message: &[u8; 32], signature: &[u8]) -> bool {
    let Ok(public) = <&[u8; 32]>::try_from(public) else {
        return false;
    };
    ethrex_crypto::leansphincs::leansphincs_verify(public, message, signature)
}

fn public_key(key: &KeyPair) -> H256 {
    H256::from_slice(&key.public)
}

// ---------------------------------------------------------------------------
// The chain and its accounts
// ---------------------------------------------------------------------------

/// The verifier is a precompile, so the genesis needs no account. IStar is
/// active from genesis whatever fork the run selected, so that every block of a
/// run is one whose proofs the builder strips and aggregates.
pub fn prepare_genesis(genesis: &mut Genesis) {
    genesis.config.istar_time = Some(0);
}

pub fn supports(_kind: AccountKind) -> Result<(), &'static str> {
    Ok(())
}

pub fn account_address(index: u64, _keys: &[KeyPair]) -> Address {
    indexed_address(index)
}

/// Validation code of an account controlled by leanSPHINCS keys.
///
/// It approves execution and payment when, for each key `i` of the account,
/// frame `i` is a proof frame whose verifier is the leanSPHINCS verifier, whose
/// key is the account's and whose `msg` is zero: the transaction carries a
/// signature of its own signature hash under every key. The protocol verified
/// each claim before this code runs, natively or through the block's aggregate
/// proof, so reading it is enough. It reverts otherwise, and halts on a frame
/// that is not a proof frame. With `AccountKind::KeyInStorage` the key is read
/// from storage slot 0.
pub fn account_code(kind: AccountKind, keys: &[KeyPair]) -> Vec<u8> {
    let verifier = FRAME_TX_LEANSPHINCS_VERIFIER.as_bytes();
    assert!(
        verifier[..19].iter().all(|byte| *byte == 0),
        "the verifier address fits in a PUSH1"
    );
    let mut code = vec![0x60, 0x01]; // PUSH1 1: every check below is ANDed into it
    for (position, key) in key_sources(kind, keys).into_iter().enumerate() {
        let frame = u8::try_from(position).expect("fits in a PUSH1");
        code.extend_from_slice(&[
            // PROOFPARAM(verifier, frame) == LEANSPHINCS_VERIFIER
            0x60,
            0x01,
            0x60,
            frame,
            0xb4,
            0x60,
            verifier[19],
            0x14,
            0x16,
            // PROOFPARAM(msg, frame) == 0: the message is this transaction's
            // signature hash
            0x60,
            0x02,
            0x60,
            frame,
            0xb4,
            0x15,
            0x16,
            // PROOFPARAM(key, frame), compared with the account's key below
            0x60,
            0x00,
            0x60,
            frame,
            0xb4,
        ]);
        match key {
            // PUSH0, SLOAD: the key in slot 0
            None => code.extend_from_slice(&[0x5f, 0x54]),
            Some(key) => {
                code.push(0x7f); // PUSH32 key
                code.extend_from_slice(&key.public);
            }
        }
        code.extend_from_slice(&[0x14, 0x16]); // EQ, AND
    }
    // JUMPI to the approval, 6 bytes ahead: PUSH1 dest, JUMPI, PUSH0, PUSH0, REVERT.
    let approve_at = u8::try_from(code.len() + 6).expect("the code is shorter than 256 bytes");
    code.extend_from_slice(&[0x60, approve_at, 0x57, 0x5f, 0x5f, 0xfd]);
    // JUMPDEST; APPROVE(offset 0, length 0, scope 3)
    code.extend_from_slice(&[0x5b, 0x60, 0x03, 0x5f, 0x5f, 0xaa]);
    code
}

/// Where the code finds each key of the account: in itself, or in storage
/// slot 0 for `AccountKind::KeyInStorage`.
fn key_sources(kind: AccountKind, keys: &[KeyPair]) -> Vec<Option<&KeyPair>> {
    match kind {
        AccountKind::KeyInStorage => vec![None],
        AccountKind::SingleKey | AccountKind::TwoKeys => {
            assert_eq!(keys.len(), kind.keys(), "one key per signature required");
            keys.iter().map(Some).collect()
        }
    }
}

// ---------------------------------------------------------------------------
// Transactions
// ---------------------------------------------------------------------------

/// A proof frame for the leanSPHINCS verifier claiming `key` for `msg` (zero
/// for the transaction's signature hash), with `proof` as its proof.
fn proof_frame(key: H256, msg: H256, proof: &[u8]) -> Frame {
    let mut data = Vec::with_capacity(PROOF_CLAIM_LENGTH + proof.len());
    data.extend_from_slice(key.as_bytes());
    data.extend_from_slice(msg.as_bytes());
    data.extend_from_slice(proof);
    Frame {
        mode: u8::from(FrameMode::Proof),
        flags: 0,
        target: Some(FRAME_TX_LEANSPHINCS_VERIFIER),
        gas_limit: PROOF_FRAME_GAS_LIMIT,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::from(data),
    }
}

/// `[PROOF.., VERIFY, SENDER]` for `request`, with the given proof frames.
fn transaction(request: &TxRequest, proof_frames: Vec<Frame>) -> FrameTransaction {
    let mut frames = proof_frames;
    frames.push(Frame {
        mode: u8::from(FrameMode::Verify),
        flags: 0x03,
        target: Some(request.sender),
        gas_limit: request.verify_gas_limit,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    });
    frames.push(Frame {
        mode: u8::from(FrameMode::Sender),
        flags: 0,
        target: Some(request.call.target),
        gas_limit: request.call.gas_limit,
        state_gas_limit: request.call.state_gas_limit,
        value: request.call.value,
        data: request.call.data.clone(),
    });
    let mut tx = FrameTransaction {
        chain_id: request.chain_id,
        nonce_keys: Vec::new(),
        nonce_seq: request.nonce,
        sender: request.sender,
        frames,
        max_priority_fee_per_gas: U256::from(request.max_priority_fee_per_gas),
        blob_versioned_hashes: Vec::new(),
        ..Default::default()
    };
    tx.max_fee = U256::from(request.max_fee_per_gas) * U256::from(tx.max_gas());
    tx
}

/// The transaction of `request` claiming each of `claimed` keys, with proofs
/// of the final length that nobody signed. The signature hash elides every
/// proof and commits to its length, so this is the layout that is signed, and
/// stripping it gives the transaction a block carries once it is signed.
fn unsigned(request: &TxRequest, claimed: &[H256]) -> FrameTransaction {
    let placeholder = vec![0u8; LEANSPHINCS_SIGNATURE_SIZE];
    let placeholders = claimed
        .iter()
        .map(|key| proof_frame(*key, H256::zero(), &placeholder))
        .collect();
    transaction(request, placeholders)
}

/// The transaction of `request` claiming each of `claimed` keys, each claim
/// proved by the signature of the corresponding entry of `signers`.
fn signed(request: &TxRequest, claimed: &[H256], signers: &[KeyPair]) -> FrameTransaction {
    let mut tx = unsigned(request, claimed);
    let sig_hash = tx.compute_sig_hash();
    for (index, (key, signer)) in claimed.iter().zip(signers).enumerate() {
        tx.frames[index] = proof_frame(*key, H256::zero(), &signature(signer, sig_hash));
    }
    tx
}

/// `tx` with the proof of its first proof frame replaced.
fn with_proof(tx: &FrameTransaction, proof: &[u8]) -> FrameTransaction {
    let mut altered = tx.clone();
    let mut data = tx.frames[0].proof_claim().to_vec();
    data.extend_from_slice(proof);
    altered.frames[0].data = Bytes::from(data);
    // Drop the hash and encoding cached for the original transaction.
    altered.inner_hash = Default::default();
    altered.cached_canonical = Default::default();
    altered
}

/// `tx` as a block from IStar carries it: every proof the protocol may strip
/// left its frame, which keeps the claim and the length of the proof.
fn stripped(tx: &FrameTransaction) -> FrameTransaction {
    let mut stripped = tx.clone();
    stripped.strip_proofs();
    stripped
}

fn gas_profile(tx: &FrameTransaction) -> GasProfile {
    let proof_frames: Vec<&Frame> = tx.frames.iter().filter(|frame| frame.is_proof()).collect();
    GasProfile {
        signatures: proof_frames.len() as u64,
        signature_bytes_in_transaction: proof_frames
            .iter()
            .map(|frame| frame.proof_bytes().len() as u64)
            .sum(),
        signature_bytes_beside_transaction: 0,
        mandatory_gas: tx.mandatory_gas(),
        calldata_gas: tx.data_cost(),
        calldata_floor_gas: tx.calldata_floor_gas(),
        execution_gas_limits: tx.total_frame_execution_gas(),
        state_gas_limits: tx.total_frame_state_gas(),
        standard_gas_limit: tx.standard_gas_limit(),
        calldata_floor_total: tx.calldata_floor_total(),
        max_gas: tx.max_gas(),
    }
}

/// Bytes a transaction built here takes in a block once its proofs are
/// stripped, by its canonical size with them and its number of proofs.
/// `limits` is given the sizes of the submitted form alone.
static STRIPPED_BYTES: Mutex<BTreeMap<(u64, u64), u64>> = Mutex::new(BTreeMap::new());

/// The transaction with its proofs is its own submitted and pooled form, and
/// the one every size and the gas profile of a `Submission` describe. A block
/// from IStar holds it stripped, under another hash.
fn submission(tx: FrameTransaction) -> Submission {
    let gas = gas_profile(&tx);
    let stripped_bytes = Transaction::FrameTransaction(stripped(&tx))
        .encode_to_vec()
        .len();
    let transaction = Transaction::FrameTransaction(tx);
    let rpc = transaction.encode_canonical_to_vec();
    STRIPPED_BYTES
        .lock()
        .expect("stripped sizes")
        .insert((rpc.len() as u64, gas.signatures), stripped_bytes as u64);
    Submission {
        peer: transaction.encode_to_vec(),
        announcement_bytes: pooled_announcement_bytes(rpc.len()),
        witness_bytes: 0,
        gas,
        transaction,
        rpc,
    }
}

pub fn build(request: &TxRequest) -> Submission {
    let claimed: Vec<H256> = request.keys.iter().map(public_key).collect();
    submission(signed(request, &claimed, request.keys))
}

/// A peer delivers the transaction in the ordinary transaction messages.
pub async fn admit_from_peer(blockchain: &Blockchain, peer: &[u8]) -> Result<H256, String> {
    let transaction = Transaction::decode(peer).map_err(|error| format!("{error:?}"))?;
    blockchain
        .add_transaction_to_pool(transaction)
        .await
        .map_err(|error| format!("{error:?}: {error}"))
}

// ---------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------

/// The aggregate proof is the last item of the block's RLP encoding, so
/// nothing travels outside it. No signature travels beside a transaction on
/// this branch: the ones reported as remaining are the proofs still inside
/// their proof frames, which the block's encoding counts with their
/// transactions. A block the builder of this branch made from the
/// transactions of this adapter has none.
pub fn block_extras(block: &Block) -> BlockExtras {
    let kept: Vec<usize> = block
        .body
        .transactions
        .iter()
        .filter_map(|tx| match tx {
            Transaction::FrameTransaction(tx) => Some(tx),
            _ => None,
        })
        .flat_map(|tx| {
            tx.frames
                .iter()
                .enumerate()
                .filter(move |(index, frame)| {
                    frame.is_proof() && tx.stripped_proof_len(*index).is_none()
                })
                .map(|(_, frame)| frame.proof_bytes().len())
        })
        .collect();
    BlockExtras {
        witnesses: kept.len(),
        witness_bytes: kept.iter().sum(),
        aggregate_proof_bytes: block.body.aggregate_proof.as_ref().map_or(0, Bytes::len),
        outside_block_rlp_bytes: 0,
    }
}

pub fn limits(facts: &TxFacts) -> Vec<Limit> {
    // What one proof frame adds to the calldata floor, as the transaction
    // type prices its data: the claim and a signature. A stripped proof is
    // priced by the length it had, so this is also what it adds in a block.
    let floor = |frames: Vec<Frame>| {
        FrameTransaction {
            frames,
            ..Default::default()
        }
        .calldata_floor_gas()
    };
    let placeholder = proof_frame(
        H256::repeat_byte(1),
        H256::zero(),
        &[0u8; LEANSPHINCS_SIGNATURE_SIZE],
    );
    let proof_frame_floor = floor(vec![placeholder]) - floor(Vec::new());
    let stripped_bytes = STRIPPED_BYTES
        .lock()
        .expect("stripped sizes")
        .get(&(facts.canonical_bytes, facts.signatures))
        .copied();
    vec![
        Limit {
            name: "MAX_FRAMES (each proof takes one frame, stripped or not)",
            value: FRAME_TX_MAX_FRAMES as u64,
            unit: "frames",
            scope: "transaction",
            source: "EIP-8141",
            layer: "consensus",
            enforced: true,
            enforced_at: "static constraints of the transaction",
            // The proof frames, the validation frame and the call.
            per_transaction: Some(facts.signatures + 2),
        },
        Limit {
            name: "MAX_VERIFY_GAS (proof frames count their execution limits)",
            value: FRAME_TX_MAX_VERIFY_GAS,
            unit: "gas",
            scope: "transaction",
            source: "EIP-8141 with the PROOF frame mode, public mempool rules",
            layer: "mempool policy",
            enforced: true,
            enforced_at: "pool admission, BlockchainOptions::max_verify_gas",
            per_transaction: Some(
                facts.verify_gas_limit + facts.signatures * PROOF_FRAME_GAS_LIMIT,
            ),
        },
        Limit {
            name: "proofs one transaction can pay for under the EIP-7825 gas cap",
            value: TX_MAX_GAS_LIMIT_AMSTERDAM / proof_frame_floor,
            unit: "signatures",
            scope: "transaction",
            source: "EIP-8141: the calldata floor is checked against TX_MAX_GAS_LIMIT",
            layer: "consensus",
            enforced: true,
            enforced_at: "transaction validation, through the gas cap; a stripped proof \
                          counts with the length it had",
            per_transaction: Some(facts.signatures),
        },
        Limit {
            name: "proofs in a block: block gas limit / calldata floor of one proof frame",
            value: facts.block_gas_limit / proof_frame_floor,
            unit: "signatures",
            scope: "block",
            source: "EIP-8141, Aggregation: stripping a proof does not change the charge",
            layer: "consensus",
            enforced: false,
            enforced_at: "no check of its own: the payer is still charged for a stripped \
                          proof by its length, so the block gas limit admits as many \
                          proofs as before IStar. Nothing else bounds the stripped proof \
                          frames of a block but the size of the proof that stands for them",
            per_transaction: Some(facts.signatures),
        },
        Limit {
            name: "MAX_AGGREGATE_PROOF_BYTES: the block's aggregate proof",
            value: MAX_AGGREGATE_PROOF_BYTES as u64,
            unit: "bytes",
            scope: "block",
            source: "this implementation; EIP-8141 leaves the block's side of aggregation \
                     to a later EIP, and the value is the bound EIP-8288 gives the proof \
                     of a block's dependencies",
            layer: "consensus",
            enforced: true,
            enforced_at: "block import, before the proof is hashed or verified. The \
                          builder reserves nothing for the proof and does not include \
                          fewer transactions to stay under the bound: a proof over it \
                          fails the build",
            // The proof's size is not a sum over the transactions it stands for.
            per_transaction: None,
        },
        Limit {
            name: "EIP-7934 block size as the builder counts it: transactions with \
                   their proofs",
            value: MAX_RLP_BLOCK_SIZE,
            unit: "bytes",
            scope: "block",
            source: "protocol: EIP-7934; this implementation strips the proofs after it \
                     has filled the block",
            layer: "builder",
            enforced: true,
            enforced_at: "payload builder, while it adds transactions: each counts with \
                          the size it has in the pool, although the block will hold it \
                          stripped",
            per_transaction: Some(facts.canonical_bytes),
        },
        Limit {
            name: "EIP-7934 block size from IStar: the block's RLP without its \
                   aggregate proof",
            value: MAX_RLP_BLOCK_SIZE,
            unit: "bytes",
            scope: "block",
            source: "protocol: EIP-7934; this implementation leaves the aggregate proof \
                     out of the size, under its own bound",
            layer: "consensus",
            enforced: true,
            enforced_at: "block import",
            // The stripped transaction as the block's transaction list holds it.
            per_transaction: stripped_bytes,
        },
    ]
}

// ---------------------------------------------------------------------------
// Faults
// ---------------------------------------------------------------------------

/// A transaction with a fault, and what refuses it at each layer.
struct Faulty {
    variant: &'static str,
    tx: FrameTransaction,
    pool_error: &'static str,
    block_error: &'static str,
}

/// The pool's message for a claim a precompile refuses.
const CLAIM_DOES_NOT_HOLD: &str = "the claim of a proof frame does not hold";
/// The pool's message for a validation frame that does not approve.
const VALIDATION_FAILED: &str = "validation prefix frame reverted";
/// The pool's message for a transaction with a stripped proof.
const STRIPPED_OUTSIDE_A_BLOCK: &str = "a stripped proof frame is valid only in a block";
/// What the import reports for a transaction whose claim does not hold.
const INVALID_PROOF: &str = "the claim of a proof frame does not hold";
/// What the import reports for a transaction nothing approved.
const NOT_APPROVED: &str = "VERIFY frame did not call APPROVE";
/// What the import reports for an aggregate proof that does not hold for the
/// claims of the block's stripped proof frames, one or two of them.
const ONE_CLAIM_NOT_PROVEN: &str = "InvalidAggregateProof(1)";
const TWO_CLAIMS_NOT_PROVEN: &str = "InvalidAggregateProof(2)";
/// What the import reports for an aggregate proof the header does not commit to.
const PROOF_NOT_COMMITTED: &str = "AggregateProofHashNotMatch";
/// What the import reports for an aggregate proof with no stripped frame to
/// stand for.
const NOTHING_TO_PROVE: &str = "AggregateProofNotEmpty";
/// What the import reports for a stripped frame the protocol may not strip.
const NOT_THE_LEANSPHINCS_VERIFIER: &str = "only a proof for LEANSPHINCS_VERIFIER can be stripped";
const READABLE_PROOF: &str = "a proof with PROOF_READ_FLAG cannot be stripped";
const NOT_ONE_SIGNATURE_LONG: &str = "only a proof of 7740 bytes can be stripped";

/// Faults of a transaction that carries its proofs. The pool holds only such
/// transactions, and a block from IStar may: the builder strips every proof it
/// may, the protocol does not require it.
fn faulty_transactions(fault: Fault, request: &TxRequest, other: &KeyPair) -> Vec<Faulty> {
    let claimed: Vec<H256> = request.keys.iter().map(public_key).collect();
    let valid = signed(request, &claimed, request.keys);
    let proof = valid.frames[0].proof_bytes().to_vec();
    let unproved = |variant, tx| Faulty {
        variant,
        tx,
        pool_error: CLAIM_DOES_NOT_HOLD,
        block_error: INVALID_PROOF,
    };
    let unapproved = |variant, tx| Faulty {
        variant,
        tx,
        pool_error: VALIDATION_FAILED,
        block_error: NOT_APPROVED,
    };
    match fault {
        Fault::InvalidSignature => {
            let mut altered = proof;
            altered[100] ^= 0x01;
            vec![unproved(
                "one bit of the proof in the transaction flipped",
                with_proof(&valid, &altered),
            )]
        }
        Fault::WrongKey => {
            let others = [other.clone()];
            vec![
                // The claim names the account's key; another key signed.
                unproved(
                    "proof in the transaction signed with another key",
                    signed(request, &claimed, &others),
                ),
                // The claim names the other key and holds; the account refuses
                // a claim that is not on its own key.
                unapproved(
                    "another key claimed and signing, proof in the transaction",
                    signed(request, &[public_key(other)], &others),
                ),
            ]
        }
        Fault::MalformedSignature => {
            let mut longer = proof.clone();
            longer.push(0);
            vec![
                unproved(
                    "proof in the transaction one byte short",
                    with_proof(&valid, &proof[..proof.len() - 1]),
                ),
                unproved(
                    "proof in the transaction one byte long",
                    with_proof(&valid, &longer),
                ),
            ]
        }
        Fault::MissingSignature => vec![
            unproved("proof frame without a proof", with_proof(&valid, &[])),
            unapproved(
                "transaction without a proof frame",
                transaction(request, Vec::new()),
            ),
        ],
        Fault::DeclaredSuppliedMismatch => {
            // The claim states another message than the one that was signed.
            let mut other_message = valid.clone();
            other_message.frames[0] = proof_frame(claimed[0], H256::repeat_byte(0x77), &proof);
            other_message.inner_hash = Default::default();
            other_message.cached_canonical = Default::default();
            // The frame names a verifier of another scheme than the proof's.
            let mut other_verifier = valid;
            other_verifier.frames[0].target = Some(FRAME_TX_SECP256K1_VERIFIER);
            other_verifier.frames[0].gas_limit = SECP256K1_VERIFIER_GAS + 100;
            other_verifier.inner_hash = Default::default();
            other_verifier.cached_canonical = Default::default();
            vec![
                unproved(
                    "claim of another message than the signed one, proof in the transaction",
                    other_message,
                ),
                unproved(
                    "verifier of another scheme than the proof's, proof in the transaction",
                    other_verifier,
                ),
            ]
        }
    }
}

pub fn fault_not_expressible(_fault: Fault, _layer: Layer) -> Option<&'static str> {
    None
}

/// The pool holds transactions with their proofs at every fork, so the faults
/// are those of a proof inside its transaction, and a transaction whose proof
/// was stripped is one more way of bringing no signature.
pub fn faulty_submissions(
    fault: Fault,
    request: &TxRequest,
    other: &KeyPair,
) -> Vec<FaultCase<FaultySubmission>> {
    let mut cases: Vec<FaultCase<FaultySubmission>> = faulty_transactions(fault, request, other)
        .into_iter()
        .map(|faulty| FaultCase {
            variant: faulty.variant,
            input: FaultySubmission::Rpc(
                Transaction::FrameTransaction(faulty.tx).encode_canonical_to_vec(),
            ),
            expected_error: faulty.pool_error,
        })
        .collect();
    if fault == Fault::MissingSignature {
        let claimed: Vec<H256> = request.keys.iter().map(public_key).collect();
        let in_block = Transaction::FrameTransaction(stripped(&unsigned(request, &claimed)));
        cases.push(FaultCase {
            variant: "transaction as a block carries it, its proof stripped, submitted",
            input: FaultySubmission::Rpc(in_block.encode_canonical_to_vec()),
            expected_error: STRIPPED_OUTSIDE_A_BLOCK,
        });
        cases.push(FaultCase {
            variant: "transaction as a block carries it, its proof stripped, from a peer",
            input: FaultySubmission::Peer(in_block.encode_to_vec()),
            expected_error: STRIPPED_OUTSIDE_A_BLOCK,
        });
    }
    cases
}

/// `tx` after `alter` changed it, without the hash and the encoding cached for
/// the original.
fn altered(tx: &FrameTransaction, alter: impl FnOnce(&mut FrameTransaction)) -> FrameTransaction {
    let mut altered = tx.clone();
    alter(&mut altered);
    altered.inner_hash = Default::default();
    altered.cached_canonical = Default::default();
    altered
}

/// The block with its aggregate proof replaced and the header committing to
/// the new proof, so that only the proof can make the block invalid.
fn with_aggregate_proof(block: &Block, proof: Bytes) -> Block {
    let mut block = block.clone();
    block.header.aggregate_proof_hash = Some(compute_aggregate_proof_hash(&proof, &NativeCrypto));
    block.body.aggregate_proof = Some(proof);
    block.header.hash = Default::default();
    block
}

/// The block with `tx` as its only transaction and the header committing to
/// it, so that only the transaction can make the block invalid.
fn with_transaction(block: &Block, tx: FrameTransaction) -> Block {
    let mut block = block.clone();
    block.body.transactions = vec![Transaction::FrameTransaction(tx)];
    block.header.transactions_root =
        compute_transactions_root(&block.body.transactions, &NativeCrypto);
    block.header.hash = Default::default();
    block
}

/// An aggregate proof of the claims `(key of the signer, message)`, each
/// signed here.
fn aggregate(claims: &[(&KeyPair, H256)]) -> Bytes {
    let signatures: Vec<((H256, H256), Bytes)> = claims
        .iter()
        .map(|(signer, message)| {
            (
                (public_key(signer), *message),
                Bytes::from(signature(signer, *message)),
            )
        })
        .collect();
    aggregate_signatures(&signatures).expect("valid signatures aggregate")
}

/// The block with `tx` as its only transaction and an aggregate proof that
/// holds for the claims of the stripped frames `tx` has, the claim of frame
/// `i` signed here by `signers[i]`. Only what is wrong with the stripped
/// frames themselves, or with what the transaction then does, can make the
/// block invalid.
fn with_proven_transaction(block: &Block, tx: FrameTransaction, signers: &[&KeyPair]) -> Block {
    let stripped_claims = tx.stripped_claims();
    assert_eq!(
        stripped_claims.len(),
        signers.len(),
        "one signer per stripped frame"
    );
    let claims: Vec<(&KeyPair, H256)> = signers
        .iter()
        .copied()
        .zip(stripped_claims)
        .map(|(signer, (_, message))| (signer, message))
        .collect();
    with_aggregate_proof(&with_transaction(block, tx), aggregate(&claims))
}

/// `block` is the valid block: the transaction of `request` with its proof
/// stripped, and the aggregate proof of its one claim. The faults are stated
/// twice. First against what the block carries from IStar: the aggregate
/// proof, the header's commitment to it and the stripped frame. Then against a
/// proof that stayed in its transaction, which an IStar block may hold beside
/// an empty aggregate proof and which executing the transaction verifies.
pub fn faulty_blocks(
    fault: Fault,
    block: &Block,
    request: &TxRequest,
    other: &KeyPair,
) -> Vec<FaultCase<FaultyBlock>> {
    let case = |variant, block, expected_error| FaultCase {
        variant,
        input: FaultyBlock {
            block,
            paths: &ImportPath::ALL,
        },
        expected_error,
    };
    let [Transaction::FrameTransaction(included)] = block.body.transactions.as_slice() else {
        panic!("the valid block holds the one frame transaction of the request");
    };
    assert!(
        included.has_stripped_proofs(),
        "the builder strips the proof of the transaction it includes"
    );
    let proof = block
        .body
        .aggregate_proof
        .clone()
        .expect("a block from IStar carries an aggregate proof");
    let signer = &request.keys[0];
    let claimed: Vec<H256> = request.keys.iter().map(public_key).collect();
    let sig_hash = included.compute_sig_hash();

    let mut cases = match fault {
        Fault::InvalidSignature => {
            let mut flipped = proof.to_vec();
            let middle = flipped.len() / 2;
            flipped[middle] ^= 0x01;
            vec![case(
                "one bit of the aggregate proof flipped",
                with_aggregate_proof(block, Bytes::from(flipped)),
                ONE_CLAIM_NOT_PROVEN,
            )]
        }
        Fault::WrongKey => vec![
            // The stripped frame claims the account's key; the proof is a
            // genuine one of the claim another key makes on the same message.
            case(
                "aggregate proof of a signature by another key",
                with_aggregate_proof(block, aggregate(&[(other, sig_hash)])),
                ONE_CLAIM_NOT_PROVEN,
            ),
            // The stripped frame claims the other key and the aggregate proof
            // holds for it; the account refuses a claim that is not on its key.
            case(
                "another key claimed by the stripped frame, and proven",
                with_proven_transaction(
                    block,
                    stripped(&unsigned(request, &[public_key(other)])),
                    &[other],
                ),
                NOT_APPROVED,
            ),
        ],
        Fault::MalformedSignature => {
            let mut longer = proof.to_vec();
            longer.push(0);
            // The frame says the proof that left was not one signature long.
            // The signature hash commits to that length, so the claim is
            // signed and proven again for each.
            let of_length = |length: u64| {
                with_proven_transaction(
                    block,
                    altered(included, |tx| {
                        tx.stripped_proofs.insert(0, length);
                    }),
                    &[signer],
                )
            };
            let one_signature = LEANSPHINCS_SIGNATURE_SIZE as u64;
            vec![
                case(
                    "aggregate proof one byte short",
                    with_aggregate_proof(block, proof.slice(..proof.len() - 1)),
                    ONE_CLAIM_NOT_PROVEN,
                ),
                case(
                    "aggregate proof one byte long",
                    with_aggregate_proof(block, Bytes::from(longer)),
                    ONE_CLAIM_NOT_PROVEN,
                ),
                case(
                    "stripped frame of a proof one byte short",
                    of_length(one_signature - 1),
                    NOT_ONE_SIGNATURE_LONG,
                ),
                case(
                    "stripped frame of a proof one byte long",
                    of_length(one_signature + 1),
                    NOT_ONE_SIGNATURE_LONG,
                ),
            ]
        }
        Fault::MissingSignature => {
            // A transaction that makes two claims, the account's and another
            // key's, and a proof of the first alone. The header's results are
            // those of the valid block, which this transaction would not give;
            // the proof is checked before the block executes, and the error
            // expected is the proof's, for two stripped frames.
            let two_claims = stripped(&unsigned(request, &[claimed[0], public_key(other)]));
            let first_claim = aggregate(&[(signer, two_claims.compute_sig_hash())]);
            vec![
                // The proof of a block without stripped frames: for this block
                // of one claim, the proof that covers one claim fewer.
                case(
                    "empty aggregate proof beside a stripped frame",
                    with_aggregate_proof(block, Bytes::new()),
                    ONE_CLAIM_NOT_PROVEN,
                ),
                case(
                    "aggregate proof that covers one claim fewer than the two stripped frames",
                    with_aggregate_proof(&with_transaction(block, two_claims), first_claim),
                    TWO_CLAIMS_NOT_PROVEN,
                ),
            ]
        }
        Fault::DeclaredSuppliedMismatch => {
            let mut miscommitted = block.clone();
            miscommitted.header.aggregate_proof_hash = Some(H256::repeat_byte(0xab));
            miscommitted.header.hash = Default::default();
            // The stripped frame states another message than the proven one.
            let other_message = altered(included, |tx| {
                let mut claim = tx.frames[0].data.to_vec();
                claim[32..].fill(0x77);
                tx.frames[0].data = Bytes::from(claim);
            });
            // Stripped frames the protocol may not strip, each with a proof
            // that holds for the claim it makes.
            let other_verifier = altered(included, |tx| {
                tx.frames[0].target = Some(FRAME_TX_SECP256K1_VERIFIER);
            });
            let readable = altered(included, |tx| tx.frames[0].flags = PROOF_READ_FLAG);
            vec![
                case(
                    "header that commits to another aggregate proof",
                    miscommitted,
                    PROOF_NOT_COMMITTED,
                ),
                case(
                    "stripped frame that claims another message than the proven one",
                    with_transaction(block, other_message),
                    ONE_CLAIM_NOT_PROVEN,
                ),
                case(
                    "stripped frame for a verifier of another scheme",
                    with_proven_transaction(block, other_verifier, &[signer]),
                    NOT_THE_LEANSPHINCS_VERIFIER,
                ),
                case(
                    "stripped frame of a proof validation code may read",
                    with_proven_transaction(block, readable, &[signer]),
                    READABLE_PROOF,
                ),
                // The proof is the valid block's; the transaction kept its
                // signature, so the block has no stripped frame for it.
                case(
                    "aggregate proof beside a transaction that kept its proof",
                    with_transaction(block, signed(request, &claimed, request.keys)),
                    NOTHING_TO_PROVE,
                ),
            ]
        }
    };
    cases.extend(
        faulty_transactions(fault, request, other)
            .into_iter()
            .map(|faulty| {
                case(
                    faulty.variant,
                    with_aggregate_proof(&with_transaction(block, faulty.tx), Bytes::new()),
                    faulty.block_error,
                )
            }),
    );
    cases
}
