//! Adapter of the measurement harness for the signature-list proposal.
//!
//! The dependency is declared in the EIP-8141 signature list, as the entry
//! `[LEANSPHINCS, public_key, msg, ""]`, with no frame of its own. The
//! leanSPHINCS signature that satisfies it travels beside the transaction
//! exactly as in EIP-8504: the proposal assumes an aggregate proof that does
//! not exist yet, so this branch keeps EIP-8504's witness list, its wrapper and
//! its block field in the meantime.
//!
//! Transactions here are `[VERIFY, SENDER]`. The account's code reads its
//! declarations at fixed positions: entry `i` of the signature list is the one
//! of key `i`. The validation frame therefore carries no data, as in the other
//! alternatives.

use bytes::Bytes;
use ethrex_blockchain::Blockchain;
use ethrex_common::{
    Address, H256, U256,
    constants::{MAX_RLP_BLOCK_SIZE, TX_MAX_GAS_LIMIT_AMSTERDAM},
    types::{
        Block, DEPENDENCY_TRIPLE_BYTES, FRAME_SIG_SCHEME_LEANSPHINCS, FRAME_TX_MAX_VERIFY_GAS,
        Frame, FrameMode, FrameSignature, FrameTransaction, Genesis, LEANSPHINCS_SIGNATURE_SIZE,
        LEANSPHINCS_VERIFICATION_GAS, LEANSPHINCS_WITNESS_GAS, MAX_SIGS_PER_TX, MAX_TX_SIZE,
        ProofWrapper, Transaction, block_dependencies, compute_dependency_witnesses_hash,
        proof_wrapper_overhead,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};

use super::lifecycle::{
    AccountKind, BlockExtras, Fault, FaultCase, FaultyBlock, FaultySubmission, GasProfile,
    ImportPath, KeyPair, Layer, Limit, Submission, TxFacts, TxRequest, indexed_address, signature,
};

pub const ALTERNATIVE: &str = "Alternative 3";
pub const ALTERNATIVE_ID: &str = "a3";
pub const PROPOSAL: &str = "Signature list, not proof frames";
pub const MILESTONE: u8 = 1;
pub const SIGNATURE_SCHEME: &str = "leanSPHINCS";
pub const KEY_CACHE_NAMESPACE: &str = "leansphincs";
pub const VERIFIER_PACKAGE: &str = "lean_sphincs";

/// Where this implementation verifies signatures, stage by stage.
pub const VERIFICATION: &[(&str, &str)] = &[
    (
        "pool admission",
        "each witness is verified once on the calling thread, before the validation \
         prefix is simulated; the revalidation after a block does not repeat it",
    ),
    (
        "block construction",
        "none: the builder takes the witnesses the pool already verified",
    ),
    (
        "block import, add_block",
        "one loop over the block's witnesses on the calling thread, before execution",
    ),
    (
        "block import, add_block_pipeline",
        "the same loop on the calling thread, before execution starts; only the \
         execution of the transactions uses the thread pool",
    ),
];

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

/// The verifier the client calls for every witness.
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

/// The proposal needs nothing in the genesis beyond its fork being active.
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
/// entry `i` of the signature list has the LEANSPHINCS scheme, its signer is
/// the key and its `msg` is empty: the transaction depends on a signature of
/// its own signature hash under every key. It reverts otherwise, and halts on
/// an entry whose signer is not a key. With `AccountKind::KeyInStorage` the
/// key is read from storage slot 0.
pub fn account_code(kind: AccountKind, keys: &[KeyPair]) -> Vec<u8> {
    let mut code = vec![0x60, 0x01]; // PUSH1 1: every check below is ANDed into it
    for (position, key) in key_sources(kind, keys).into_iter().enumerate() {
        let entry = u8::try_from(position).expect("fits in a PUSH1");
        code.extend_from_slice(&[
            // SIGPARAM(scheme, entry) == LEANSPHINCS
            0x60,
            0x01,
            0x60,
            entry,
            0xb4,
            0x60,
            FRAME_SIG_SCHEME_LEANSPHINCS,
            0x14,
            0x16,
            // SIGPARAM(msg, entry) == 0: the message is this transaction's
            // signature hash
            0x60,
            0x02,
            0x60,
            entry,
            0xb4,
            0x15,
            0x16,
            // SIGPARAM(raw signer, entry), compared with the account's key below
            0x60,
            0x04,
            0x60,
            entry,
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

/// Signature list entries declaring a signature of the transaction's
/// signature hash under each of `keys`. The entry's `signature` is empty: the
/// signature that satisfies the dependency never enters the transaction.
fn entries(keys: &[H256]) -> Vec<FrameSignature> {
    keys.iter()
        .map(|key| FrameSignature {
            scheme: FRAME_SIG_SCHEME_LEANSPHINCS,
            signer: None,
            signer_key: Some(*key),
            msg: Bytes::new(),
            signature: Bytes::new(),
        })
        .collect()
}

/// `[VERIFY, SENDER]` for `request`, with `declared` keys in its signature list.
fn transaction(request: &TxRequest, declared: &[H256]) -> FrameTransaction {
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
        signatures: entries(declared),
        max_priority_fee_per_gas: U256::from(request.max_priority_fee_per_gas),
        blob_versioned_hashes: Vec::new(),
        ..Default::default()
    };
    tx.max_fee = U256::from(request.max_fee_per_gas) * U256::from(tx.max_gas());
    tx
}

/// One witness for each dependency of `tx`, in the order of `dependencies`,
/// each signed by the key among `signers` that the dependency names.
fn witnesses(tx: &FrameTransaction, signers: &[KeyPair]) -> Vec<Bytes> {
    tx.dependencies()
        .iter()
        .map(|dependency| {
            let signer = signers
                .iter()
                .find(|key| public_key(key) == dependency.verification_key_hash)
                .expect("a key for every dependency");
            Bytes::from(signature(signer, dependency.data_hash))
        })
        .collect()
}

/// The form a user submits: `FRAME_TX_TYPE || rlp([tx_payload_body, witnesses])`.
fn submitted(tx: &FrameTransaction, witnesses: Vec<Bytes>) -> Vec<u8> {
    let mut raw = Vec::new();
    ProofWrapper::new(tx.clone(), None, witnesses).encode_submission(&mut raw);
    raw
}

fn gas_profile(tx: &FrameTransaction) -> GasProfile {
    let dependencies = tx.dependencies().len() as u64;
    GasProfile {
        signatures: dependencies,
        signature_bytes_in_transaction: 0,
        signature_bytes_beside_transaction: dependencies * LEANSPHINCS_SIGNATURE_SIZE as u64,
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
    let declared: Vec<H256> = request.keys.iter().map(public_key).collect();
    let tx = transaction(request, &declared);
    let witnesses = witnesses(&tx, request.keys);
    let wrapper = ProofWrapper::new(tx.clone(), None, witnesses.clone());
    let mut rpc = Vec::new();
    wrapper.encode_submission(&mut rpc);
    let peer = wrapper.encode_to_vec();
    let transaction = Transaction::FrameTransaction(tx.clone());
    // NewProofWrapperHashes announces [wrapper-hash, tx-hash, length].
    let announcement = (
        wrapper.hash(&NativeCrypto),
        transaction.hash(&NativeCrypto),
        peer.len() as u64,
    )
        .encode_to_vec()
        .len();
    Submission {
        transaction,
        rpc,
        announcement_bytes: announcement,
        witness_bytes: witnesses.iter().map(Bytes::len).sum(),
        gas: gas_profile(&tx),
        peer,
    }
}

/// A peer delivers a dependency transaction only as its wrapper.
pub async fn admit_from_peer(blockchain: &Blockchain, peer: &[u8]) -> Result<H256, String> {
    let wrapper = ProofWrapper::decode(peer).map_err(|error| format!("{error:?}"))?;
    blockchain
        .add_dependency_transaction_to_pool(wrapper)
        .await
        .map_err(|error| format!("{error:?}: {error}"))
}

// ---------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------

pub fn block_extras(block: &Block) -> BlockExtras {
    let witnesses = block
        .body
        .dependency_witnesses
        .as_deref()
        .unwrap_or_default();
    BlockExtras {
        witnesses: witnesses.len(),
        witness_bytes: witnesses.iter().map(Bytes::len).sum(),
        aggregate_proof_bytes: 0,
        // The witness list is part of the block's RLP encoding.
        outside_block_rlp_bytes: 0,
    }
}

pub fn limits(facts: &TxFacts) -> Vec<Limit> {
    vec![
        Limit {
            name: "MAX_SIGS_PER_TX",
            value: MAX_SIGS_PER_TX as u64,
            unit: "signatures",
            scope: "transaction",
            source: "EIP-8504, kept by this branch; the proposal states no limit of its own",
            layer: "mempool policy",
            enforced: true,
            enforced_at: "pool admission, BlockchainOptions::max_sigs_per_tx",
            per_transaction: Some(facts.signatures),
        },
        Limit {
            name: "dependencies one transaction can pay for under the EIP-7825 gas cap",
            value: TX_MAX_GAS_LIMIT_AMSTERDAM
                / (LEANSPHINCS_WITNESS_GAS + LEANSPHINCS_VERIFICATION_GAS),
            unit: "signatures",
            scope: "transaction",
            source: "derived from the gas this branch charges; the proposal sets no gas",
            layer: "consensus",
            enforced: true,
            enforced_at: "transaction validation, through the gas cap",
            per_transaction: Some(facts.signatures),
        },
        Limit {
            name: "MAX_VERIFY_GAS (each entry counts its verification gas)",
            value: FRAME_TX_MAX_VERIFY_GAS,
            unit: "gas",
            scope: "transaction",
            source: "EIP-8141, public mempool rules; the proposal keeps entries within it",
            layer: "mempool policy",
            enforced: true,
            enforced_at: "pool admission, BlockchainOptions::max_verify_gas",
            per_transaction: Some(
                facts.verify_gas_limit + facts.signatures * LEANSPHINCS_VERIFICATION_GAS,
            ),
        },
        Limit {
            name: "largest wrapper the pool accepts",
            value: (MAX_TX_SIZE + proof_wrapper_overhead(MAX_SIGS_PER_TX)) as u64,
            unit: "bytes",
            scope: "transaction",
            source: "client policy",
            layer: "mempool policy",
            enforced: true,
            enforced_at: "pool admission, Blockchain::max_proof_wrapper_size",
            per_transaction: Some(facts.peer_bytes),
        },
        Limit {
            name: "witnesses in a block: block gas limit / LEANSPHINCS_WITNESS_GAS",
            value: facts.block_gas_limit / LEANSPHINCS_WITNESS_GAS,
            unit: "signatures",
            scope: "block",
            source: "EIP-8504, limits on dependency_witnesses, kept by this branch",
            layer: "consensus",
            enforced: false,
            enforced_at: "no check of its own: it follows from the block gas limit and from \
                          the rule that a block carries exactly one witness per dependency",
            per_transaction: Some(facts.signatures),
        },
        Limit {
            name: "EIP-8288 MAX_DEPENDENCY_PROOF_BYTES, for the aggregate proof the proposal assumes",
            value: 1_048_576,
            unit: "bytes",
            scope: "block",
            source: "EIP-8288, Block Validity; the proposal keeps EIP-8288's proof",
            layer: "consensus",
            enforced: false,
            enforced_at: "not applicable at this milestone: the block carries individual \
                          witnesses, not the proof this limit is for",
            per_transaction: Some(facts.witness_bytes),
        },
        Limit {
            name: "witnesses in a block under EIP-7934, counting only signature bytes",
            value: MAX_RLP_BLOCK_SIZE / LEANSPHINCS_SIGNATURE_SIZE as u64,
            unit: "signatures",
            scope: "block",
            source: "EIP-8504, limits on dependency_witnesses, kept by this branch",
            layer: "consensus",
            enforced: true,
            enforced_at: "the EIP-7934 check, which measures the block with its witnesses",
            per_transaction: Some(facts.signatures),
        },
    ]
}

// ---------------------------------------------------------------------------
// Faults
// ---------------------------------------------------------------------------

/// The block with its witness list replaced and the header committing to the
/// new list, so that only the witnesses can make it invalid.
fn with_witnesses(block: &Block, witnesses: Vec<Bytes>) -> Block {
    let mut block = block.clone();
    block.header.dependency_witnesses_hash =
        Some(compute_dependency_witnesses_hash(&witnesses, &NativeCrypto));
    block.body.dependency_witnesses = Some(witnesses);
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
    const NOT_SATISFIED: &str = "does not satisfy its dependency";
    const NOT_STATED: &str = "does not state its transaction's dependencies";
    let declared: Vec<H256> = request.keys.iter().map(public_key).collect();
    let tx = transaction(request, &declared);
    let valid = witnesses(&tx, request.keys);
    let rpc = |witnesses: Vec<Bytes>| FaultySubmission::Rpc(submitted(&tx, witnesses));
    match fault {
        Fault::InvalidSignature => vec![case(
            "one bit of the witness flipped",
            rpc(vec![flipped(&valid[0])]),
            NOT_SATISFIED,
        )],
        Fault::WrongKey => {
            let forged = Bytes::from(signature(other, tx.compute_sig_hash()));
            // The other key declared too: the witness satisfies the dependency,
            // and the account refuses a dependency that is not on its own key.
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
                    "another key declared and signing",
                    FaultySubmission::Rpc(foreign.rpc),
                    "validation prefix frame reverted",
                ),
            ]
        }
        Fault::MalformedSignature => vec![
            case(
                "witness one byte short",
                rpc(vec![truncated(&valid[0])]),
                NOT_SATISFIED,
            ),
            case(
                "witness one byte long",
                rpc(vec![extended(&valid[0])]),
                NOT_SATISFIED,
            ),
            case("empty witness", rpc(vec![Bytes::new()]), NOT_SATISFIED),
        ],
        Fault::MissingSignature => vec![
            case(
                "transaction submitted without the wrapper",
                FaultySubmission::Rpc(
                    Transaction::FrameTransaction(tx.clone()).encode_canonical_to_vec(),
                ),
                "submitted without their witnesses",
            ),
            case("empty witness list", rpc(Vec::new()), NOT_STATED),
        ],
        Fault::DeclaredSuppliedMismatch => {
            let undeclared = transaction(request, &[]);
            let mut misstated = ProofWrapper::new(tx.clone(), None, valid.clone());
            misstated.deps[0] = Bytes::from(vec![0u8; DEPENDENCY_TRIPLE_BYTES]);
            vec![
                case(
                    "two witnesses for one dependency",
                    rpc(vec![valid[0].clone(), valid[0].clone()]),
                    NOT_STATED,
                ),
                case(
                    "witness for a transaction that declares no dependency",
                    FaultySubmission::Rpc(submitted(&undeclared, valid.clone())),
                    "declares no dependencies",
                ),
                case(
                    "wrapper from a peer stating other dependencies",
                    FaultySubmission::Peer(misstated.encode_to_vec()),
                    "WrapperDependenciesMismatch",
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
    const NOT_SATISFIED: &str = "DependencyNotSatisfied";
    const COUNT: &str = "DependencyWitnessesCountMismatch";
    let valid = block
        .body
        .dependency_witnesses
        .clone()
        .expect("a block of this fork has a witness list");
    let dependency = block_dependencies(&block.body.transactions)[0];
    // The block with another witness list, refused on both import paths.
    let listed = |variant, witnesses: Vec<Bytes>, expected_error| {
        case(
            variant,
            FaultyBlock {
                block: with_witnesses(block, witnesses),
                paths: &ImportPath::ALL,
            },
            expected_error,
        )
    };
    match fault {
        Fault::InvalidSignature => vec![listed(
            "one bit of the witness flipped",
            vec![flipped(&valid[0])],
            NOT_SATISFIED,
        )],
        Fault::WrongKey => vec![listed(
            "witness signed with another key",
            vec![Bytes::from(signature(other, dependency.data_hash))],
            NOT_SATISFIED,
        )],
        Fault::MalformedSignature => vec![
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
        ],
        Fault::MissingSignature => vec![listed("empty witness list", Vec::new(), COUNT)],
        Fault::DeclaredSuppliedMismatch => {
            // The valid witness list under a header that commits to another.
            // Only the pipeline compares a body with its header.
            let mut other_commitment = block.clone();
            other_commitment.header.dependency_witnesses_hash = Some(H256::repeat_byte(0x11));
            vec![
                listed(
                    "two witnesses for one dependency",
                    vec![valid[0].clone(), valid[0].clone()],
                    COUNT,
                ),
                case(
                    "header commitment that is not the witness list's",
                    FaultyBlock {
                        block: other_commitment,
                        paths: &[ImportPath::Pipeline],
                    },
                    "DependencyWitnessesHashNotMatch",
                ),
            ]
        }
    }
}
