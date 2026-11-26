//! Adapter of the measurement harness for EIP-8141 with declarative claims.
//!
//! The signature travels beside the transaction. The transaction body declares
//! claims, `[verifier, key, msg, gas_limit, witness_len]`; the leanSPHINCS
//! signature that satisfies a claim, its witness, is submitted, pooled and
//! exchanged with the transaction in its pooled representation,
//! `0x06 || rlp([tx_body, wrapper_version, witnesses, blob_sidecar])`, and the
//! block carries it in `claim_witnesses`, one byte string of its body that the
//! header commits to.
//!
//! Transactions here are `[VERIFY, SENDER]` with one claim for each key of the
//! account, `[LEANSPHINCS_VERIFIER, key, "", 0, 0]`: a signature of the
//! transaction's own signature hash under that key. The account's code asks for
//! each claim with `HASCLAIM`, by verifier, key and message, so nothing depends
//! on the position of a claim and the validation frame carries no data, as in
//! the other alternatives.

use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_common::{
    Address, H256, U256,
    constants::TX_MAX_GAS_LIMIT_AMSTERDAM,
    types::{
        Block, Claim, Dependency, FRAME_TX_MAX_VERIFY_GAS, Frame, FrameMode, FrameTransaction,
        Genesis, LEANSPHINCS_SIGNATURE_SIZE, LEANSPHINCS_VERIFICATION_GAS, LEANSPHINCS_VERIFIER,
        MAX_BLOCK_WITNESS_BYTES, MAX_CLAIM_WITNESS_BYTES, MAX_CLAIMS, P2PTransaction,
        PooledFrameTransaction, Transaction, WITNESS_BYTE_GAS, block_dependencies,
        claim_witnesses_len_for_payload, compute_claim_witnesses_hash, decode_claim_witnesses,
        encode_claim_witnesses, max_pooled_frame_tx_len,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::{
    decode::RLPDecode,
    encode::{RLPEncode, bytes_length},
};

use super::lifecycle::{
    AccountKind, BlockExtras, Fault, FaultCase, FaultyBlock, FaultySubmission, GasProfile,
    ImportPath, KeyPair, Layer, Limit, Submission, TxFacts, TxRequest, indexed_address,
    pooled_announcement_bytes, signature,
};

pub const ALTERNATIVE: &str = "Alternative 4";
pub const ALTERNATIVE_ID: &str = "a4";
pub const PROPOSAL: &str = "EIP-8141 with declarative claims";
pub const MILESTONE: u8 = 1;
pub const SIGNATURE_SCHEME: &str = "leanSPHINCS";
pub const KEY_CACHE_NAMESPACE: &str = "leansphincs";
pub const VERIFIER_PACKAGE: &str = "lean_sphincs";

/// Where this implementation verifies signatures, stage by stage.
pub const VERIFICATION: &[(&str, &str)] = &[
    (
        "pool admission",
        "the witness of each claim is verified once, natively, on the calling thread, one \
         after another in the order of the claims, before any state is read and before the \
         validation prefix is simulated; the revalidation after a block does not repeat it",
    ),
    (
        "block construction",
        "none: the builder takes the witnesses the pool already verified; it verifies again \
         only the claims of custom verifiers, which these transactions do not have",
    ),
    (
        "block import, add_block",
        "one loop over the block's dependencies on the calling thread, before execution",
    ),
    (
        "block import, add_block_pipeline",
        "the same loop on the calling thread, before execution starts; only the \
         execution of the transactions uses the thread pool",
    ),
];

/// Gas one leanSPHINCS claim adds to its transaction whatever its key: its
/// verification, and `WITNESS_BYTE_GAS` for each byte of its witness. The
/// calldata cost of the claim's verifier address and key comes on top.
const LEANSPHINCS_CLAIM_GAS: u64 =
    LEANSPHINCS_VERIFICATION_GAS + WITNESS_BYTE_GAS * LEANSPHINCS_SIGNATURE_SIZE as u64;

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

/// The call the client makes for every witness of a leanSPHINCS claim, on
/// admission to the pool and on import of a block: the native verifier of the
/// dependency, over `key || msg || witness`. It wraps
/// `ethrex_crypto::leansphincs::leansphincs_verify`.
pub fn verify(public: &[u8], message: &[u8; 32], signature: &[u8]) -> bool {
    if public.len() != H256::len_bytes() {
        return false;
    }
    let dependency = leansphincs_dependency(H256::from_slice(public), H256(*message));
    ethrex_vm::claims::verify_protocol_dependency(&dependency, signature, &NativeCrypto)
        == Some(true)
}

fn public_key(key: &KeyPair) -> H256 {
    H256::from_slice(&key.public)
}

/// The dependency a leanSPHINCS signature of `msg` under `key` satisfies.
fn leansphincs_dependency(key: H256, msg: H256) -> Dependency {
    Dependency {
        verifier: LEANSPHINCS_VERIFIER,
        key,
        msg,
        gas_limit: LEANSPHINCS_VERIFICATION_GAS,
        witness_len: LEANSPHINCS_SIGNATURE_SIZE as u64,
    }
}

// ---------------------------------------------------------------------------
// The chain and its accounts
// ---------------------------------------------------------------------------

/// Claims are part of Hegota: nothing else is needed in the genesis.
pub fn prepare_genesis(_genesis: &mut Genesis) {}

pub fn supports(_kind: AccountKind) -> Result<(), &'static str> {
    Ok(())
}

pub fn account_address(index: u64, _keys: &[KeyPair]) -> Address {
    indexed_address(index)
}

/// Validation code of an account controlled by leanSPHINCS keys.
///
/// It approves execution and payment when, for each key of the account,
/// `HASCLAIM(LEANSPHINCS_VERIFIER, key, 0)` holds: the transaction claims a
/// signature of its own signature hash under every key. It reverts otherwise.
/// With `AccountKind::KeyInStorage` the key is read from storage slot 0.
///
/// For one key in the code this is, byte for byte, the 68-byte account code
/// the branch documents and its sender tool deploys.
pub fn account_code(kind: AccountKind, keys: &[KeyPair]) -> Vec<u8> {
    let mut code = Vec::new();
    for (position, key) in key_sources(kind, keys).into_iter().enumerate() {
        // PUSH0: the msg, where zero stands for the transaction's signature hash
        code.push(0x5f);
        match key {
            // PUSH0, SLOAD: the key in slot 0
            None => code.extend_from_slice(&[0x5f, 0x54]),
            Some(key) => {
                code.push(0x7f); // PUSH32 key
                code.extend_from_slice(&key.public);
            }
        }
        code.push(0x73); // PUSH20 verifier
        code.extend_from_slice(LEANSPHINCS_VERIFIER.as_bytes());
        code.push(0xb4); // HASCLAIM
        if position > 0 {
            code.push(0x16); // AND with the claims before it
        }
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

/// Claims of a leanSPHINCS signature of the transaction's signature hash under
/// each of `keys`. The gas and the witness length of a claim are zero: the
/// protocol verifier fixes both.
fn claims(keys: &[H256]) -> Vec<Claim> {
    keys.iter()
        .map(|key| Claim {
            verifier: LEANSPHINCS_VERIFIER,
            key: Bytes::copy_from_slice(key.as_bytes()),
            msg: Bytes::new(),
            gas_limit: 0,
            witness_len: 0,
        })
        .collect()
}

/// `[VERIFY, SENDER]` for `request`, declaring `claims`.
fn transaction(request: &TxRequest, claims: Vec<Claim>) -> FrameTransaction {
    let frames = vec![
        Frame {
            mode: u8::from(FrameMode::Verify),
            flags: 0x03,
            target: Some(request.sender),
            gas_limit: request.verify_gas_limit,
            state_gas_limit: 0,
            value: U256::zero(),
            data: Bytes::new(),
        },
        Frame {
            mode: u8::from(FrameMode::Sender),
            flags: 0,
            target: Some(request.call.target),
            gas_limit: request.call.gas_limit,
            state_gas_limit: request.call.state_gas_limit,
            value: request.call.value,
            data: request.call.data.clone(),
        },
    ];
    let mut tx = FrameTransaction {
        chain_id: request.chain_id,
        nonce_keys: Vec::new(),
        nonce_seq: request.nonce,
        sender: request.sender,
        claims,
        frames,
        max_priority_fee_per_gas: U256::from(request.max_priority_fee_per_gas),
        blob_versioned_hashes: Vec::new(),
        ..Default::default()
    };
    tx.max_fee = U256::from(request.max_fee_per_gas) * U256::from(tx.max_gas());
    tx
}

/// One witness for each claim of `tx`, in the order of its claims, each signed
/// by the key among `signers` that the claim names.
fn witnesses(tx: &FrameTransaction, signers: &[KeyPair]) -> Vec<Bytes> {
    tx.claim_dependencies()
        .expect("the claims are well formed")
        .iter()
        .map(|dependency| {
            let signer = signers
                .iter()
                .find(|key| public_key(key) == dependency.key)
                .expect("a key for every claim");
            Bytes::from(signature(signer, dependency.msg))
        })
        .collect()
}

/// The pooled representation of `tx` with `witnesses`.
fn pooled(tx: &FrameTransaction, witnesses: Vec<Bytes>) -> P2PTransaction {
    P2PTransaction::FrameTransaction(PooledFrameTransaction {
        tx: tx.clone(),
        witnesses,
        blobs_bundle: None,
    })
}

/// The form a user submits:
/// `FRAME_TX_TYPE || rlp([tx_body, wrapper_version, witnesses, blob_sidecar])`.
fn submitted(tx: &FrameTransaction, witnesses: Vec<Bytes>) -> Vec<u8> {
    pooled(tx, witnesses).encode_canonical_to_vec()
}

/// The item of a `PooledTransactions` message: the same bytes as one RLP string.
fn exchanged(tx: &FrameTransaction, witnesses: Vec<Bytes>) -> Vec<u8> {
    pooled(tx, witnesses).encode_to_vec()
}

fn gas_profile(tx: &FrameTransaction) -> GasProfile {
    let claims = tx.claims.len() as u64;
    GasProfile {
        signatures: claims,
        signature_bytes_in_transaction: 0,
        signature_bytes_beside_transaction: claims * LEANSPHINCS_SIGNATURE_SIZE as u64,
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

pub fn build(request: &TxRequest) -> Submission {
    let keys: Vec<H256> = request.keys.iter().map(public_key).collect();
    let tx = transaction(request, claims(&keys));
    let witnesses = witnesses(&tx, request.keys);
    let witness_bytes = witnesses.iter().map(Bytes::len).sum();
    let pooled = pooled(&tx, witnesses);
    let rpc = pooled.encode_canonical_to_vec();
    Submission {
        gas: gas_profile(&tx),
        transaction: Transaction::FrameTransaction(tx),
        // NewPooledTransactionHashes announces the type, the size of the pooled
        // representation and the transaction hash.
        announcement_bytes: pooled_announcement_bytes(rpc.len()),
        witness_bytes,
        peer: pooled.encode_to_vec(),
        rpc,
    }
}

/// A peer delivers a frame transaction only in its pooled representation, as
/// an item of a `PooledTransactions` message. The handler of that message
/// holds it to its size cap and hands it to the pool.
pub async fn admit_from_peer(blockchain: &Blockchain, peer: &[u8]) -> Result<H256, String> {
    let pooled = match P2PTransaction::decode(peer).map_err(|error| format!("{error:?}"))? {
        P2PTransaction::FrameTransaction(pooled) => pooled,
        other => {
            return Err(format!(
                "not a frame transaction in its pooled representation: {:?}",
                other.tx_type()
            ));
        }
    };
    let (actual, limit) = (pooled.wire_len(), pooled.max_wire_len());
    if actual > limit {
        return Err(format!(
            "the pooled representation takes {actual} bytes, more than the {limit} accepted"
        ));
    }
    blockchain
        .add_frame_transaction_to_pool(pooled)
        .await
        .map_err(|error| format!("{error:?}: {error}"))
}

// ---------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------

/// The witnesses a block carries, one for each of its dependencies.
fn block_witnesses(block: &Block) -> Vec<Bytes> {
    decode_claim_witnesses(block.body.claim_witnesses.as_deref().unwrap_or_default())
        .expect("the claim witnesses of a block the builder made decode")
}

pub fn block_extras(block: &Block) -> BlockExtras {
    let witnesses = block_witnesses(block);
    BlockExtras {
        witnesses: witnesses.len(),
        witness_bytes: witnesses.iter().map(Bytes::len).sum(),
        aggregate_proof_bytes: 0,
        // The claim witnesses are part of the body, and so of the block's RLP
        // encoding.
        outside_block_rlp_bytes: 0,
    }
}

/// Bytes one leanSPHINCS witness takes in a block's `claim_witnesses`: the
/// signature and the header of its RLP string.
fn witness_bytes_in_block() -> u64 {
    bytes_length(LEANSPHINCS_SIGNATURE_SIZE, 0xff) as u64
}

/// leanSPHINCS witnesses the `claim_witnesses` of a block holds within
/// `MAX_BLOCK_WITNESS_BYTES`, by the length the builder computes for it.
fn witnesses_a_block_holds() -> u64 {
    let one = bytes_length(LEANSPHINCS_SIGNATURE_SIZE, 0xff);
    let mut count = MAX_BLOCK_WITNESS_BYTES / one;
    while count > 0 && claim_witnesses_len_for_payload(count * one) > MAX_BLOCK_WITNESS_BYTES {
        count -= 1;
    }
    count as u64
}

/// The most bytes the node accepts for the pooled representation of a
/// transaction with `signatures` leanSPHINCS claims.
fn pooled_representation_cap(signatures: u64) -> u64 {
    let keys = vec![H256::repeat_byte(0x01); usize::try_from(signatures).expect("fits")];
    let tx = FrameTransaction {
        claims: claims(&keys),
        ..Default::default()
    };
    max_pooled_frame_tx_len(&tx, false) as u64
}

pub fn limits(facts: &TxFacts) -> Vec<Limit> {
    vec![
        Limit {
            name: "MAX_CLAIMS",
            value: MAX_CLAIMS as u64,
            unit: "signatures",
            scope: "transaction",
            source: "EIP-8141 with declarative claims, static constraints",
            layer: "consensus",
            enforced: true,
            enforced_at: "static constraints of the transaction, on pool admission and on \
                          execution",
            per_transaction: Some(facts.signatures),
        },
        Limit {
            name: "MAX_CLAIM_WITNESS_BYTES",
            value: MAX_CLAIM_WITNESS_BYTES,
            unit: "bytes",
            scope: "transaction",
            source: "EIP-8141 with declarative claims, static constraints",
            layer: "consensus",
            enforced: true,
            enforced_at: "static constraints, on the witness length a custom verifier's claim \
                          declares; it does not apply to a leanSPHINCS claim, whose witness \
                          the verifier fixes at 7,740 bytes",
            per_transaction: None,
        },
        Limit {
            name: "MAX_BLOCK_WITNESS_BYTES, on the block's claim_witnesses",
            value: MAX_BLOCK_WITNESS_BYTES as u64,
            unit: "bytes",
            scope: "block",
            source: "EIP-8141 with declarative claims, Block Witnesses (provisional)",
            layer: "consensus",
            enforced: true,
            enforced_at: "payload builder, which leaves out a transaction whose witnesses do \
                          not fit, and block import; pool admission refuses a transaction \
                          whose own witnesses exceed it",
            per_transaction: Some(facts.signatures * witness_bytes_in_block()),
        },
        Limit {
            name: "leanSPHINCS witnesses in a block under MAX_BLOCK_WITNESS_BYTES",
            value: witnesses_a_block_holds(),
            unit: "signatures",
            scope: "block",
            source: "EIP-8141 with declarative claims, Block Witnesses: the limit over the \
                     bytes one witness takes in claim_witnesses",
            layer: "consensus",
            enforced: true,
            enforced_at: "payload builder and block import, through MAX_BLOCK_WITNESS_BYTES",
            per_transaction: Some(facts.signatures),
        },
        Limit {
            name: "leanSPHINCS claims in a block by gas: block gas limit / \
                   (LEANSPHINCS_VERIFICATION_GAS + WITNESS_BYTE_GAS x LEANSPHINCS_SIGNATURE_SIZE)",
            value: facts.block_gas_limit / LEANSPHINCS_CLAIM_GAS,
            unit: "signatures",
            scope: "block",
            source: "EIP-8141 with declarative claims, Gas (WITNESS_BYTE_GAS is provisional)",
            layer: "consensus",
            enforced: false,
            enforced_at: "no check of its own: it follows from the block gas limit, since both \
                          charges are intrinsic gas of the transaction",
            per_transaction: Some(facts.signatures),
        },
        Limit {
            name: "leanSPHINCS claims one transaction can pay for under the EIP-7825 gas cap, \
                   at LEANSPHINCS_VERIFICATION_GAS + WITNESS_BYTE_GAS x LEANSPHINCS_SIGNATURE_SIZE \
                   each",
            value: TX_MAX_GAS_LIMIT_AMSTERDAM / LEANSPHINCS_CLAIM_GAS,
            unit: "signatures",
            scope: "transaction",
            source: "EIP-8141 with declarative claims, Gas",
            layer: "consensus",
            enforced: true,
            enforced_at: "transaction validation, through the gas cap; MAX_CLAIMS is lower",
            per_transaction: Some(facts.signatures),
        },
        Limit {
            name: "verification gas of a block's claims against its gas limit, before any \
                   verifier runs",
            value: facts.block_gas_limit,
            unit: "gas",
            scope: "block",
            source: "decision of this implementation; the proposal bounds this work only once \
                     the transactions execute",
            layer: "consensus",
            enforced: true,
            enforced_at: "block import, in the check of the claim witnesses; a block within \
                          its gas limit never fails it",
            per_transaction: Some(facts.signatures * LEANSPHINCS_VERIFICATION_GAS),
        },
        Limit {
            name: "MAX_VERIFY_GAS (the verification gas of every claim counts)",
            value: FRAME_TX_MAX_VERIFY_GAS,
            unit: "gas",
            scope: "transaction",
            source: "EIP-8141, public mempool rules; the proposal adds claim_verification_cost \
                     to the validation prefix's execution limits",
            layer: "mempool policy",
            enforced: true,
            enforced_at: "pool admission, BlockchainOptions::max_verify_gas",
            per_transaction: Some(
                facts.verify_gas_limit + facts.signatures * LEANSPHINCS_VERIFICATION_GAS,
            ),
        },
        Limit {
            name: "leanSPHINCS claims of one transaction the public mempool admits: \
                   (MAX_VERIFY_GAS - validation frame gas) / LEANSPHINCS_VERIFICATION_GAS",
            value: FRAME_TX_MAX_VERIFY_GAS.saturating_sub(facts.verify_gas_limit)
                / LEANSPHINCS_VERIFICATION_GAS,
            unit: "signatures",
            scope: "transaction",
            source: "EIP-8141, public mempool rules",
            layer: "mempool policy",
            enforced: true,
            enforced_at: "pool admission, BlockchainOptions::max_verify_gas",
            per_transaction: Some(facts.signatures),
        },
        Limit {
            name: "largest pooled representation the node accepts for this transaction",
            value: pooled_representation_cap(facts.signatures),
            unit: "bytes",
            scope: "transaction",
            source: "client policy for the transaction (MAX_TX_SIZE) plus what the proposal's \
                     Networking section fixes for the witnesses of its claims",
            layer: "mempool policy",
            enforced: true,
            enforced_at: "eth_sendRawTransaction, the PooledTransactions handler and pool \
                          admission, before any witness is verified",
            per_transaction: Some(facts.rpc_bytes),
        },
    ]
}

// ---------------------------------------------------------------------------
// Faults
// ---------------------------------------------------------------------------

/// The block with `claim_witnesses` in place of its own, the header committing
/// to them and the block hash following the header, so that only the claim
/// witnesses can make it invalid.
fn with_claim_witnesses(block: &Block, claim_witnesses: Bytes) -> Block {
    let mut block = block.clone();
    block.header.claim_witnesses_hash = Some(compute_claim_witnesses_hash(
        &claim_witnesses,
        &NativeCrypto,
    ));
    block.header.hash = Default::default();
    block.body.claim_witnesses = Some(claim_witnesses);
    block
}

fn flipped(witness: &Bytes) -> Bytes {
    let mut altered = witness.to_vec();
    altered[100] ^= 0x01;
    Bytes::from(altered)
}

fn truncated(witness: &Bytes) -> Bytes {
    witness.slice(..witness.len() - 1)
}

fn extended(witness: &Bytes) -> Bytes {
    let mut longer = witness.to_vec();
    longer.push(0);
    Bytes::from(longer)
}

fn case<T>(variant: &'static str, input: T, expected_error: &'static str) -> FaultCase<T> {
    FaultCase {
        variant,
        input,
        expected_error,
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
    const NOT_SATISFIED: &str = "does not satisfy its claim";
    const WRONG_LENGTH: &str = "bytes long and its claim requires 7740";
    // The decoder of the pooled representation refuses a list of witnesses
    // longer than the claims of the transaction allow, before it reads it.
    const MORE_THAN_CLAIMED: &str = "the claims of the transaction allow";
    const NOT_APPROVED: &str = "validation prefix frame reverted";
    let own: Vec<H256> = request.keys.iter().map(public_key).collect();
    let tx = transaction(request, claims(&own));
    let valid = witnesses(&tx, request.keys);
    let rpc = |witnesses: Vec<Bytes>| FaultySubmission::Rpc(submitted(&tx, witnesses));
    let peer = |witnesses: Vec<Bytes>| FaultySubmission::Peer(exchanged(&tx, witnesses));
    match fault {
        Fault::InvalidSignature => vec![
            case(
                "one bit of the witness flipped",
                rpc(vec![flipped(&valid[0])]),
                NOT_SATISFIED,
            ),
            case(
                "one bit of the witness flipped, from a peer",
                peer(vec![flipped(&valid[0])]),
                NOT_SATISFIED,
            ),
        ],
        Fault::WrongKey => {
            let forged = Bytes::from(signature(other, tx.compute_sig_hash()));
            // The other key claimed too: the witness satisfies the claim, and
            // the account finds no claim on its own key.
            let foreign_keys = [other.clone()];
            let foreign = build(&TxRequest {
                keys: &foreign_keys,
                ..request.clone()
            });
            vec![
                case(
                    "witness signed with another key",
                    rpc(vec![forged]),
                    NOT_SATISFIED,
                ),
                case(
                    "another key claimed and signing",
                    FaultySubmission::Rpc(foreign.rpc),
                    NOT_APPROVED,
                ),
            ]
        }
        Fault::MalformedSignature => vec![
            case(
                "witness one byte short",
                rpc(vec![truncated(&valid[0])]),
                WRONG_LENGTH,
            ),
            case(
                "witness one byte long",
                rpc(vec![extended(&valid[0])]),
                MORE_THAN_CLAIMED,
            ),
            case(
                "witness one byte long, from a peer",
                peer(vec![extended(&valid[0])]),
                MORE_THAN_CLAIMED,
            ),
            case("empty witness", rpc(vec![Bytes::new()]), WRONG_LENGTH),
        ],
        Fault::MissingSignature => {
            let unclaimed = transaction(request, Vec::new());
            vec![
                case(
                    "transaction body submitted without its pooled representation",
                    FaultySubmission::Rpc(
                        Transaction::FrameTransaction(tx.clone()).encode_canonical_to_vec(),
                    ),
                    "submitted in its pooled representation",
                ),
                case(
                    "empty witness list",
                    rpc(Vec::new()),
                    "was submitted with 0 witnesses",
                ),
                case(
                    "empty witness list, from a peer",
                    peer(Vec::new()),
                    "was submitted with 0 witnesses",
                ),
                case(
                    "no claim and no witness",
                    FaultySubmission::Rpc(submitted(&unclaimed, Vec::new())),
                    NOT_APPROVED,
                ),
            ]
        }
        Fault::DeclaredSuppliedMismatch => {
            let unclaimed = transaction(request, Vec::new());
            // A second claim, on the other key, and the witnesses of both.
            let signers = [request.keys[0].clone(), other.clone()];
            let both_keys: Vec<H256> = signers.iter().map(public_key).collect();
            let two_claims = transaction(request, claims(&both_keys));
            let both = witnesses(&two_claims, &signers);
            let other_message = Bytes::from(signature(&request.keys[0], H256::repeat_byte(0x11)));
            vec![
                case(
                    "two witnesses for one claim",
                    rpc(vec![valid[0].clone(), valid[0].clone()]),
                    MORE_THAN_CLAIMED,
                ),
                case(
                    "witness for a transaction that declares no claim",
                    FaultySubmission::Rpc(submitted(&unclaimed, valid.clone())),
                    MORE_THAN_CLAIMED,
                ),
                case(
                    "one witness for two claims",
                    FaultySubmission::Rpc(submitted(&two_claims, vec![both[0].clone()])),
                    "has 2 claims and was submitted with 1 witnesses",
                ),
                case(
                    "witnesses in another order than their claims",
                    FaultySubmission::Rpc(submitted(
                        &two_claims,
                        vec![both[1].clone(), both[0].clone()],
                    )),
                    NOT_SATISFIED,
                ),
                case(
                    "witness that signs another message than the claim's",
                    rpc(vec![other_message]),
                    NOT_SATISFIED,
                ),
            ]
        }
    }
}

pub fn faulty_blocks(
    fault: Fault,
    block: &Block,
    _request: &TxRequest,
    other: &KeyPair,
) -> Vec<FaultCase<FaultyBlock>> {
    const NOT_SATISFIED: &str = "ClaimNotSatisfied(0)";
    const COUNT: &str = "ClaimWitnessesCountMismatch";
    const MALFORMED: &str = "InvalidClaimWitnesses";
    let valid = block_witnesses(block);
    let dependency = block_dependencies(&block.body.transactions)
        .expect("the claims of the block are well formed")[0];
    // The block with other claim witnesses, refused on both import paths.
    let carrying = |variant, claim_witnesses: Bytes, expected_error| {
        case(
            variant,
            FaultyBlock {
                block: with_claim_witnesses(block, claim_witnesses),
                paths: &ImportPath::ALL,
            },
            expected_error,
        )
    };
    let listed = |variant, witnesses: Vec<Bytes>, expected_error| {
        carrying(variant, encode_claim_witnesses(&witnesses), expected_error)
    };
    match fault {
        Fault::InvalidSignature => vec![listed(
            "one bit of the witness flipped",
            vec![flipped(&valid[0])],
            NOT_SATISFIED,
        )],
        Fault::WrongKey => vec![listed(
            "witness signed with another key",
            vec![Bytes::from(signature(other, dependency.msg))],
            NOT_SATISFIED,
        )],
        Fault::MalformedSignature => {
            // The valid witness under the version byte of another form.
            let mut other_version = encode_claim_witnesses(&valid).to_vec();
            other_version[0] = 0x01;
            vec![
                listed(
                    "witness one byte short",
                    vec![truncated(&valid[0])],
                    NOT_SATISFIED,
                ),
                listed(
                    "witness one byte long",
                    vec![extended(&valid[0])],
                    NOT_SATISFIED,
                ),
                listed("empty witness", vec![Bytes::new()], NOT_SATISFIED),
                carrying(
                    "witness under a version byte the fork does not define",
                    Bytes::from(other_version),
                    MALFORMED,
                ),
            ]
        }
        Fault::MissingSignature => vec![
            listed("empty claim witnesses", Vec::new(), COUNT),
            carrying(
                "an empty witness list spelled out",
                Bytes::from_static(&[0x00, 0xc0]),
                MALFORMED,
            ),
        ],
        Fault::DeclaredSuppliedMismatch => {
            // The valid claim witnesses under a header that commits to other
            // bytes. Both import paths compare them in the claim check.
            let mut other_commitment = block.clone();
            other_commitment.header.claim_witnesses_hash = Some(H256::repeat_byte(0x11));
            other_commitment.header.hash = Default::default();
            vec![
                listed(
                    "two witnesses for one dependency",
                    vec![valid[0].clone(), valid[0].clone()],
                    COUNT,
                ),
                case(
                    "header commitment that is not the claim witnesses'",
                    FaultyBlock {
                        block: other_commitment,
                        paths: &ImportPath::ALL,
                    },
                    "ClaimWitnessesHashMismatch",
                ),
            ]
        }
    }
}
