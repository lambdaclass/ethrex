//! Adapter of the measurement harness for the PROOF frame proposal.
//!
//! The signature travels inside the transaction. A mode-4 proof frame carries
//! `key || msg || proof` and names its verifier as its target; the protocol
//! runs the verifier, here the leanSPHINCS verifier precompile, before any
//! other frame, and a claim that does not hold invalidates the transaction.
//! Nothing travels beside the transaction and the block has no new field.
//!
//! Transactions here are `[PROOF.., VERIFY, SENDER]`. The account's code reads
//! its claims at fixed positions: proof frame `i` is the one of key `i`. The
//! validation frame therefore carries no data, as in the other alternatives.

use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_common::{
    Address, H256, U256,
    constants::TX_MAX_GAS_LIMIT_AMSTERDAM,
    types::{
        Block, FRAME_TX_LEANSPHINCS_VERIFIER, FRAME_TX_MAX_FRAMES, FRAME_TX_MAX_VERIFY_GAS,
        FRAME_TX_SECP256K1_VERIFIER, Frame, FrameMode, FrameTransaction, Genesis,
        LEANSPHINCS_VERIFIER_GAS, PROOF_CLAIM_LENGTH, SECP256K1_VERIFIER_GAS, Transaction,
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
pub const MILESTONE: u8 = 1;
pub const SIGNATURE_SCHEME: &str = "leanSPHINCS";
pub const KEY_CACHE_NAMESPACE: &str = "leansphincs";
pub const VERIFIER_PACKAGE: &str = "lean_sphincs";

/// Where this implementation verifies signatures, stage by stage.
pub const VERIFICATION: &[(&str, &str)] = &[
    (
        "pool admission",
        "each proof a precompile verifies is checked once on the calling thread, before \
         any state is read; the prefix simulation and the revalidation after a block \
         skip it",
    ),
    (
        "block construction",
        "the builder executes every transaction it includes, which verifies its proofs \
         again, on the builder's thread",
    ),
    (
        "block import, add_block",
        "inside the execution of each transaction, on the calling thread",
    ),
    (
        "block import, add_block_pipeline",
        "inside the execution of each transaction, on the thread pool that executes the \
         block's transactions (RAYON_NUM_THREADS)",
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

/// The verifier is a precompile, so the genesis needs nothing.
pub fn prepare_genesis(_genesis: &mut Genesis) {}

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
/// each claim before this code runs, so reading it is enough. It reverts
/// otherwise, and halts on a frame that is not a proof frame. With
/// `AccountKind::KeyInStorage` the key is read from storage slot 0.
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

/// The transaction of `request` claiming each of `claimed` keys, each claim
/// proved by the signature of the corresponding entry of `signers`.
fn signed(request: &TxRequest, claimed: &[H256], signers: &[KeyPair]) -> FrameTransaction {
    // The signature hash elides every proof and commits to its length, so the
    // transaction is laid out with proofs of the final length and signed last.
    let placeholder = vec![0u8; LEANSPHINCS_SIGNATURE_SIZE];
    let placeholders = claimed
        .iter()
        .map(|key| proof_frame(*key, H256::zero(), &placeholder))
        .collect();
    let mut tx = transaction(request, placeholders);
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

/// The transaction is its own submitted and pooled form.
fn submission(tx: FrameTransaction) -> Submission {
    let gas = gas_profile(&tx);
    let transaction = Transaction::FrameTransaction(tx);
    let rpc = transaction.encode_canonical_to_vec();
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

/// The block carries nothing beside its transactions.
pub fn block_extras(_block: &Block) -> BlockExtras {
    BlockExtras::default()
}

pub fn limits(facts: &TxFacts) -> Vec<Limit> {
    // What one proof frame adds to the calldata floor, as the transaction
    // type prices its data: the claim and a signature.
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
    vec![
        Limit {
            name: "MAX_FRAMES (each proof takes one frame)",
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
            enforced_at: "transaction validation, through the gas cap",
            per_transaction: Some(facts.signatures),
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
/// What the import reports for a transaction whose claim does not hold.
const INVALID_PROOF: &str = "the claim of a proof frame does not hold";
/// What the import reports for a transaction nothing approved.
const NOT_APPROVED: &str = "VERIFY frame did not call APPROVE";

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
                "one bit of the proof flipped",
                with_proof(&valid, &altered),
            )]
        }
        Fault::WrongKey => {
            let others = [other.clone()];
            vec![
                // The claim names the account's key; another key signed.
                unproved(
                    "proof signed with another key",
                    signed(request, &claimed, &others),
                ),
                // The claim names the other key and holds; the account refuses
                // a claim that is not on its own key.
                unapproved(
                    "another key claimed and signing",
                    signed(request, &[public_key(other)], &others),
                ),
            ]
        }
        Fault::MalformedSignature => {
            let mut longer = proof.clone();
            longer.push(0);
            vec![
                unproved(
                    "proof one byte short",
                    with_proof(&valid, &proof[..proof.len() - 1]),
                ),
                unproved("proof one byte long", with_proof(&valid, &longer)),
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
                    "claim of another message than the signed one",
                    other_message,
                ),
                unproved(
                    "verifier of another scheme than the proof's",
                    other_verifier,
                ),
            ]
        }
    }
}

pub fn fault_not_expressible(_fault: Fault, _layer: Layer) -> Option<&'static str> {
    None
}

pub fn faulty_submissions(
    fault: Fault,
    request: &TxRequest,
    other: &KeyPair,
) -> Vec<FaultCase<FaultySubmission>> {
    faulty_transactions(fault, request, other)
        .into_iter()
        .map(|faulty| FaultCase {
            variant: faulty.variant,
            input: FaultySubmission::Rpc(
                Transaction::FrameTransaction(faulty.tx).encode_canonical_to_vec(),
            ),
            expected_error: faulty.pool_error,
        })
        .collect()
}

/// The faulty transaction takes the place of the valid one in the block, and
/// the header commits to it, so that only the transaction can make the block
/// invalid.
pub fn faulty_blocks(
    fault: Fault,
    block: &Block,
    request: &TxRequest,
    other: &KeyPair,
) -> Vec<FaultCase<FaultyBlock>> {
    faulty_transactions(fault, request, other)
        .into_iter()
        .map(|faulty| {
            let mut block = block.clone();
            block.body.transactions = vec![Transaction::FrameTransaction(faulty.tx)];
            block.header.transactions_root =
                compute_transactions_root(&block.body.transactions, &NativeCrypto);
            FaultCase {
                variant: faulty.variant,
                input: FaultyBlock {
                    block,
                    paths: &ImportPath::ALL,
                },
                expected_error: faulty.block_error,
            }
        })
        .collect()
}
