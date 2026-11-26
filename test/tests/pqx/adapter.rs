//! Adapter of the measurement harness for the unmodified chain.
//!
//! The reference every alternative is compared with: an account without code
//! sends a frame transaction authorized by its secp256k1 key through the
//! default code of EIP-8141. The signature is an entry of the transaction's
//! signature list; nothing travels beside the transaction and the block has no
//! new field.
//!
//! Transactions here are `[VERIFY, SENDER]` with one secp256k1 entry in the
//! signature list. The validation frame carries no data.

use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_common::{
    Address, H256, U256,
    types::{
        Block, FRAME_SIG_SCHEME_SECP256K1, FRAME_TX_MAX_VERIFY_GAS, Fork, Frame, FrameMode,
        FrameSignature, FrameTransaction, Genesis, Transaction, compute_transactions_root,
    },
    utils::keccak,
};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use secp256k1::{Message, PublicKey, SECP256K1, SecretKey};

use super::lifecycle::{
    AccountKind, BlockExtras, Fault, FaultCase, FaultyBlock, FaultySubmission, GasProfile,
    ImportPath, KeyPair, Layer, Limit, Submission, TxFacts, TxRequest, pooled_announcement_bytes,
    signature,
};

pub const ALTERNATIVE: &str = "Baseline";
pub const ALTERNATIVE_ID: &str = "base";
pub const PROPOSAL: &str = "EIP-8141 default code with a secp256k1 signature";
/// The baseline has no milestone: it is the chain before any alternative.
pub const MILESTONE: u8 = 0;
pub const SIGNATURE_SCHEME: &str = "secp256k1";
pub const KEY_CACHE_NAMESPACE: &str = "secp256k1";
pub const VERIFIER_PACKAGE: &str = "secp256k1";

/// Where this implementation verifies signatures, stage by stage.
pub const VERIFICATION: &[(&str, &str)] = &[
    (
        "pool admission",
        "the signature list is verified once on the calling thread, before the \
         validation prefix is simulated",
    ),
    (
        "block construction",
        "the builder executes every transaction it includes, which verifies its \
         signature list again, on the builder's thread",
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

/// Length of a secp256k1 entry's signature: the recovery id, `r` and `s`.
const SIGNATURE_SIZE: usize = 65;

// ---------------------------------------------------------------------------
// The signature scheme
// ---------------------------------------------------------------------------

/// The public half of a key is the address it controls: that is what a
/// recovered signature is compared with.
pub fn generate_key(seed: &[u8; 32]) -> KeyPair {
    let secret = SecretKey::from_slice(seed).expect("the seed is a valid secret key");
    let public = PublicKey::from_secret_key(SECP256K1, &secret).serialize_uncompressed();
    KeyPair {
        secret: seed.to_vec(),
        public: keccak(&public[1..]).as_bytes()[12..].to_vec(),
    }
}

/// `v || r || s` with `v` the bare recovery id, as EIP-8141 requires.
pub fn sign(key: &KeyPair, message: &[u8; 32]) -> Vec<u8> {
    let secret = SecretKey::from_slice(&key.secret).expect("a valid secret key");
    let (recovery_id, compact) = SECP256K1
        .sign_ecdsa_recoverable(&Message::from_digest(*message), &secret)
        .serialize_compact();
    let mut signature = vec![Into::<i32>::into(recovery_id) as u8];
    signature.extend_from_slice(&compact);
    signature
}

/// The check the client runs on one entry of a signature list.
pub fn verify(public: &[u8], message: &[u8; 32], signature: &[u8]) -> bool {
    if public.len() != 20 {
        return false;
    }
    let signer = Address::from_slice(public);
    ethrex_vm::validate_frame_signatures(
        &[entry(None, signature)],
        H256(*message),
        signer,
        Fork::Hegota,
        &NativeCrypto,
    )
}

fn address(key: &KeyPair) -> Address {
    Address::from_slice(&key.public)
}

// ---------------------------------------------------------------------------
// The chain and its accounts
// ---------------------------------------------------------------------------

pub fn prepare_genesis(_genesis: &mut Genesis) {}

pub fn supports(kind: AccountKind) -> Result<(), &'static str> {
    match kind {
        AccountKind::SingleKey => Ok(()),
        AccountKind::TwoKeys => {
            Err("the default code of an account without code accepts one secp256k1 signature")
        }
        AccountKind::KeyInStorage => {
            Err("the baseline is the reference chain, not a scheme to migrate to")
        }
    }
}

/// The account is the one its key controls.
pub fn account_address(_index: u64, keys: &[KeyPair]) -> Address {
    address(&keys[0])
}

/// An account without code: the protocol's default code validates for it.
pub fn account_code(_kind: AccountKind, _keys: &[KeyPair]) -> Vec<u8> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Transactions
// ---------------------------------------------------------------------------

/// A secp256k1 entry over the transaction's signature hash. With no `signer`
/// the entry stands for the transaction's sender.
fn entry(signer: Option<Address>, signature: &[u8]) -> FrameSignature {
    FrameSignature {
        scheme: FRAME_SIG_SCHEME_SECP256K1,
        signer,
        msg: Bytes::new(),
        signature: Bytes::copy_from_slice(signature),
    }
}

/// `[VERIFY, SENDER]` for `request` with `signatures` as its signature list.
fn transaction(request: &TxRequest, signatures: Vec<FrameSignature>) -> FrameTransaction {
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
        frames,
        signatures,
        max_priority_fee_per_gas: U256::from(request.max_priority_fee_per_gas),
        blob_versioned_hashes: Vec::new(),
        ..Default::default()
    };
    tx.max_fee = U256::from(request.max_fee_per_gas) * U256::from(tx.max_gas());
    tx
}

/// The transaction of `request` signed by `signer`.
fn signed(request: &TxRequest, signer: &KeyPair) -> FrameTransaction {
    // The signature hash commits to the length of the signature, not to its
    // bytes, so the transaction is laid out with a signature of the final
    // length and signed last.
    let mut tx = transaction(request, vec![entry(None, &[0u8; SIGNATURE_SIZE])]);
    let sig_hash = tx.compute_sig_hash();
    tx.signatures = vec![entry(None, &signature(signer, sig_hash))];
    tx
}

/// `tx` with another signature list.
fn with_signatures(tx: &FrameTransaction, signatures: Vec<FrameSignature>) -> FrameTransaction {
    let mut altered = tx.clone();
    altered.signatures = signatures;
    // Drop the hash and encoding cached for the original transaction.
    altered.inner_hash = Default::default();
    altered.cached_canonical = Default::default();
    altered
}

fn gas_profile(tx: &FrameTransaction) -> GasProfile {
    GasProfile {
        signatures: tx.signatures.len() as u64,
        signature_bytes_in_transaction: tx
            .signatures
            .iter()
            .map(|entry| entry.signature.len() as u64)
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

pub fn build(request: &TxRequest) -> Submission {
    let tx = signed(request, &request.keys[0]);
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
    vec![Limit {
        name: "MAX_VERIFY_GAS (each signature counts its verification gas)",
        value: FRAME_TX_MAX_VERIFY_GAS,
        unit: "gas",
        scope: "transaction",
        source: "EIP-8141, public mempool rules",
        layer: "mempool policy",
        enforced: true,
        enforced_at: "pool admission, BlockchainOptions::max_verify_gas",
        // 2800 for one secp256k1 entry.
        per_transaction: Some(facts.verify_gas_limit + facts.signatures * 2800),
    }]
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

fn faulty_transactions(fault: Fault, request: &TxRequest, other: &KeyPair) -> Vec<Faulty> {
    let valid = signed(request, &request.keys[0]);
    let good = valid.signatures[0].signature.to_vec();
    let invalid = |variant, tx| Faulty {
        variant,
        tx,
        pool_error: "Invalid frame transaction signature",
        block_error: "signature validation failed",
    };
    match fault {
        Fault::InvalidSignature => {
            let mut altered = good;
            altered[10] ^= 0x01;
            vec![invalid(
                "one bit of the signature flipped",
                with_signatures(&valid, vec![entry(None, &altered)]),
            )]
        }
        Fault::WrongKey => vec![invalid("signed with another key", signed(request, other))],
        Fault::MalformedSignature => vec![invalid(
            "signature one byte short",
            with_signatures(&valid, vec![entry(None, &good[..SIGNATURE_SIZE - 1])]),
        )],
        Fault::MissingSignature => vec![Faulty {
            variant: "empty signature list",
            tx: with_signatures(&valid, Vec::new()),
            pool_error: "validation prefix frame reverted",
            block_error: "VERIFY frame did not call APPROVE",
        }],
        // The entry names the other key's account as its signer, while the
        // sender's key signed.
        Fault::DeclaredSuppliedMismatch => vec![invalid(
            "entry naming another signer than the one that signed",
            with_signatures(&valid, vec![entry(Some(address(other)), &good)]),
        )],
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
