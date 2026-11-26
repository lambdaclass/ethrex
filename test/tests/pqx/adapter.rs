//! Adapter of the measurement harness for EIP-8504 from IStar, where the block
//! carries the dependency proof of EIP-8288: milestone 2.
//!
//! The signature travels beside the transaction. A mode-4 proof frame declares
//! the dependency `(scheme, data_hash, verification_key_hash)`; the leanSPHINCS
//! signature that satisfies it, the witness, is submitted and pooled with the
//! transaction in a wrapper. None of that changes at IStar. The block does: it
//! has no list of witnesses. Its builder aggregates the witnesses of the
//! block's dependencies into one proof, which the block carries as the last
//! item of its body, and the header commits to the proof and to the
//! dependencies it proves.
//!
//! Transactions here are `[VERIFY, PROOF, SENDER]`, byte for byte those of the
//! milestone-1 adapter. The account's code reads its declarations at fixed
//! positions: the proof frame is frame 1 and the triple of key `i` starts at
//! byte `96 * i` of its data. The validation frame therefore carries no data,
//! as in the other alternatives.

use bytes::Bytes;
use ethrex_blockchain::{Blockchain, dependency_proof::prove_dependencies};
use ethrex_common::{
    Address, H256, U256,
    constants::{MAX_RLP_BLOCK_SIZE, TX_MAX_GAS_LIMIT_AMSTERDAM},
    types::{
        Block, DEPENDENCY_TRIPLE_BYTES, Dependency, FRAME_TX_MAX_VERIFY_GAS, Fork, Frame,
        FrameMode, FrameTransaction, Genesis, LEANSPHINCS_SCHEME, LEANSPHINCS_SIGNATURE_SIZE,
        LEANSPHINCS_VERIFICATION_GAS, LEANSTARK_VERIFICATION_GAS, MAX_DEPENDENCIES_PER_FRAME,
        MAX_DEPENDENCY_PROOF_BYTES, MAX_SIGS_PER_TX, MAX_TX_SIZE, PROOF_FRAME_MODE, ProofWrapper,
        Transaction, block_dependencies, compute_dependency_proof_hash, proof_wrapper_overhead,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::{
    decode::RLPDecode,
    encode::{RLPEncode, list_length},
};

use super::lifecycle::{
    AccountKind, BlockExtras, Fault, FaultCase, FaultyBlock, FaultySubmission, GasProfile,
    ImportPath, KeyPair, Layer, Limit, Submission, TxFacts, TxRequest, indexed_address, signature,
};

pub const ALTERNATIVE: &str = "Alternative 1";
pub const ALTERNATIVE_ID: &str = "a1";
pub const PROPOSAL: &str = "EIP-8504";
pub const MILESTONE: u8 = 2;
pub const SIGNATURE_SCHEME: &str = "leanSPHINCS";
pub const KEY_CACHE_NAMESPACE: &str = "leansphincs";
pub const VERIFIER_PACKAGE: &str = "lean_sphincs";

/// Where this implementation verifies signatures from IStar, stage by stage.
pub const VERIFICATION: &[(&str, &str)] = &[
    (
        "pool admission",
        "as before IStar: each witness is verified once, natively, on the calling thread, \
         before the validation prefix is simulated; the revalidation after a block does \
         not repeat it",
    ),
    (
        "block construction",
        "no signature is verified: the builder takes the witnesses the pool already \
         verified and proves them. Blockchain::build_payload ends with \
         PayloadBuildResult::with_dependency_proof, which makes one proof for the block's \
         dependencies: leaves of at most 512 signatures and nodes of at most 16 proofs, \
         one after another, each on the prover's own thread pool, which is sized from \
         the machine's cores unless LEANVM_NUM_THREADS sets it, and never by \
         RAYON_NUM_THREADS; one proof at a time in the process",
    ),
    (
        "block import, add_block",
        "no signature is verified individually: validate_dependency_proof verifies the \
         block's one proof against the dependencies of its transactions, on the calling \
         thread, before execution; the verifier is single-threaded",
    ),
    (
        "block import, add_block_pipeline",
        "the same single verification on the calling thread, before execution starts; \
         only the execution of the transactions uses the thread pool",
    ),
];

/// The fork this milestone measures. A frame transaction has no gas limit field,
/// so what it is charged and reserves is a quantity of the fork of its block.
const FORK: Fork = Fork::IStar;

/// Index of the proof frame in the transactions this adapter builds.
const PROOF_FRAME_INDEX: u8 = 1;

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

/// The block of this milestone exists from IStar, so IStar is active at genesis
/// whatever `PQX_FORK` selected: a run of this adapter never measures blocks
/// that carry individual witnesses. LStar is left as the run has it: a run
/// that does not select it measures IStar blocks, which have no `burned_fees`.
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
/// It approves execution and payment when frame 1 is a proof frame and, for
/// each key `i` of the account, the triple at byte `96 * i` of that frame is
/// `(LEANSPHINCS_SCHEME, 0, key)`: the transaction depends on a signature of
/// its own signature hash under every key. It reverts otherwise. With
/// `AccountKind::KeyInStorage` the key is read from storage slot 0.
pub fn account_code(kind: AccountKind, keys: &[KeyPair]) -> Vec<u8> {
    let mut code = vec![
        // FRAMEPARAM(mode, frame 1) == PROOF_FRAME_MODE
        0x60,
        0x02,
        0x60,
        PROOF_FRAME_INDEX,
        0xb3,
        0x60,
        PROOF_FRAME_MODE,
        0x14,
    ];
    for (position, key) in key_sources(kind, keys).into_iter().enumerate() {
        let offset = u8::try_from(position * DEPENDENCY_TRIPLE_BYTES).expect("fits in a PUSH1");
        code.extend_from_slice(&[
            // FRAMEDATACOPY(memOffset 0, dataOffset, length 96, frame 1)
            0x60,
            PROOF_FRAME_INDEX,
            0x60,
            0x60,
            0x60,
            offset,
            0x5f,
            0xb2,
            // scheme word == LEANSPHINCS_SCHEME
            0x5f,
            0x51,
            0x60,
            LEANSPHINCS_SCHEME,
            0x14,
            0x16,
            // data_hash == 0: the message is this transaction's signature hash
            0x60,
            0x20,
            0x51,
            0x15,
            0x16,
            // verification_key_hash, compared with the account's key below
            0x60,
            0x40,
            0x51,
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

/// A proof frame declaring a signature of the transaction's signature hash
/// under each of `keys`.
fn proof_frame(keys: &[H256]) -> Frame {
    let mut data = Vec::with_capacity(keys.len() * DEPENDENCY_TRIPLE_BYTES);
    for key in keys {
        data.extend_from_slice(&[0u8; 31]);
        data.push(LEANSPHINCS_SCHEME);
        data.extend_from_slice(&[0u8; 32]);
        data.extend_from_slice(key.as_bytes());
    }
    Frame {
        mode: PROOF_FRAME_MODE,
        flags: 0,
        target: None,
        gas_limit: keys.len() as u64 * LEANSPHINCS_VERIFICATION_GAS,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::from(data),
    }
}

/// `[VERIFY, PROOF, SENDER]` for `request`, declaring `declared` keys.
fn transaction(request: &TxRequest, declared: &[H256]) -> FrameTransaction {
    let mut frames = vec![Frame {
        mode: u8::from(FrameMode::Verify),
        flags: 0x03,
        target: Some(request.sender),
        gas_limit: request.verify_gas_limit,
        state_gas_limit: 0,
        value: U256::zero(),
        data: Bytes::new(),
    }];
    if !declared.is_empty() {
        frames.push(proof_frame(declared));
    }
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
        signatures: Vec::new(),
        max_priority_fee_per_gas: U256::from(request.max_priority_fee_per_gas),
        blob_versioned_hashes: Vec::new(),
        ..Default::default()
    };
    // The fee budget of the milestone-1 adapter, over the `max_gas` of Hegota:
    // the transaction, its signature hash and so its signature are the same at
    // both milestones. The budget covers the smaller `max_gas` of IStar.
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
        mandatory_gas: tx.mandatory_gas_at(FORK),
        calldata_gas: tx.data_cost(),
        calldata_floor_gas: tx.calldata_floor_gas(),
        execution_gas_limits: tx.total_frame_execution_gas(),
        state_gas_limits: tx.total_frame_state_gas(),
        standard_gas_limit: tx.standard_gas_limit_at(FORK),
        calldata_floor_total: tx.calldata_floor_total_at(FORK),
        max_gas: tx.max_gas_at(FORK),
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
    // An IStar block has no witness list: these stay at zero.
    let witnesses = block
        .body
        .dependency_witnesses
        .as_deref()
        .unwrap_or_default();
    BlockExtras {
        witnesses: witnesses.len(),
        witness_bytes: witnesses.iter().map(Bytes::len).sum(),
        aggregate_proof_bytes: block.body.dependency_proof.as_ref().map_or(0, Bytes::len),
        // The proof is the last item of the block's RLP encoding and of its
        // body's, so the sizes the harness takes from those encodings already
        // count it. Only the size EIP-7934 limits leaves it out
        // (`Block::length_without_dependency_proof`).
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
            source: "EIP-8504, Admission to the Public Mempool (provisional)",
            layer: "mempool policy",
            enforced: true,
            enforced_at: "pool admission, BlockchainOptions::max_sigs_per_tx",
            per_transaction: Some(facts.signatures),
        },
        Limit {
            name: "MAX_DEPENDENCIES_PER_FRAME",
            value: MAX_DEPENDENCIES_PER_FRAME as u64,
            unit: "signatures",
            scope: "transaction",
            source: "EIP-8504, Constraints (provisional)",
            layer: "consensus",
            enforced: true,
            enforced_at: "static constraints of the transaction",
            per_transaction: Some(facts.signatures),
        },
        Limit {
            name: "declared triples one transaction can pay for under the EIP-7825 gas cap",
            value: TX_MAX_GAS_LIMIT_AMSTERDAM / LEANSTARK_VERIFICATION_GAS,
            unit: "signatures",
            scope: "transaction",
            source: "EIP-8288, dependency_proof_gas: 30,000 gas for each declared triple",
            layer: "consensus",
            enforced: true,
            enforced_at: "transaction validation, through the gas cap",
            per_transaction: Some(facts.signatures),
        },
        Limit {
            name: "MAX_VERIFY_GAS (proof frames and witness verification are exempt)",
            value: FRAME_TX_MAX_VERIFY_GAS,
            unit: "gas",
            scope: "transaction",
            source: "EIP-8141, public mempool rules; EIP-8504, Validation Prefix",
            layer: "mempool policy",
            enforced: true,
            enforced_at: "pool admission, BlockchainOptions::max_verify_gas",
            per_transaction: Some(facts.verify_gas_limit),
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
            name: "dependencies in a block: block gas limit / LEANSTARK_VERIFICATION_GAS",
            value: facts.block_gas_limit / LEANSTARK_VERIFICATION_GAS,
            unit: "signatures",
            scope: "block",
            source: "EIP-8288, dependency_proof_gas: 30,000 gas for each declared triple",
            layer: "consensus",
            enforced: false,
            enforced_at: "no check of its own: nothing bounds the dependencies of an IStar \
                          block but the block gas limit, through the gas each declaration \
                          pays, and the size bound of the proof. The builder reserves \
                          nothing for the proof, neither a number of dependencies nor a \
                          time: proving time grows with the block and is inside block \
                          construction",
            per_transaction: Some(facts.signatures),
        },
        Limit {
            name: "MAX_DEPENDENCY_PROOF_BYTES",
            value: MAX_DEPENDENCY_PROOF_BYTES as u64,
            unit: "bytes",
            scope: "block",
            source: "EIP-8288, Block Validity",
            layer: "consensus",
            enforced: true,
            enforced_at: "block import on both paths and the Engine API, before the proof is \
                          hashed. The builder does not select transactions against it: a \
                          proof over the bound fails the build",
            // One proof for the block: its size is not a sum over transactions.
            per_transaction: None,
        },
        Limit {
            name: "EIP-7934 block size, measured without the dependency proof",
            value: MAX_RLP_BLOCK_SIZE,
            unit: "bytes",
            scope: "block",
            source: "EIP-7934; EIP-8288 keeps the proof out of the RLP block",
            layer: "consensus",
            enforced: true,
            enforced_at: "payload builder and block import, on \
                          Block::length_without_dependency_proof",
            // The transaction as an item of the block's transaction list.
            per_transaction: Some(list_length(facts.canonical_bytes as usize) as u64),
        },
    ]
}

// ---------------------------------------------------------------------------
// Faults
// ---------------------------------------------------------------------------

/// The block with `change` applied to it and without the block hash computed
/// for the valid block, which the change makes stale.
fn altered(block: &Block, change: impl FnOnce(&mut Block)) -> Block {
    let mut block = block.clone();
    change(&mut block);
    block.header.hash = Default::default();
    block
}

/// The block with its dependency proof replaced and the header committing to
/// the new proof, so that only the proof itself can make it invalid.
fn with_proof(block: &Block, proof: Bytes) -> Block {
    altered(block, |block| {
        block.header.dependency_proof_hash =
            Some(compute_dependency_proof_hash(&proof, &NativeCrypto));
        block.body.dependency_proof = Some(proof);
    })
}

/// The signature of `dependency`'s message by `signer`.
fn witness(dependency: &Dependency, signer: &KeyPair) -> Bytes {
    Bytes::from(signature(signer, dependency.data_hash))
}

/// The proof the builder would make for `claims`: each dependency with the key
/// that signs its message.
fn proof_of(claims: &[(Dependency, &KeyPair)]) -> Bytes {
    let signed: Vec<(Dependency, Bytes)> = claims
        .iter()
        .map(|(dependency, signer)| (*dependency, witness(dependency, signer)))
        .collect();
    prove_dependencies(&signed).expect("valid signatures aggregate into a proof")
}

/// `bytes`, a witness or a proof, with one bit changed at `index`.
fn flipped(bytes: &Bytes, index: usize) -> Bytes {
    let mut altered = bytes.to_vec();
    altered[index] ^= 0x01;
    Bytes::from(altered)
}

fn truncated(bytes: &Bytes) -> Bytes {
    bytes.slice(..bytes.len() - 1)
}

fn extended(bytes: &Bytes) -> Bytes {
    let mut longer = bytes.to_vec();
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
            rpc(vec![flipped(&valid[0], 100)]),
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

/// From IStar a block has no individual signature to alter: each fault is stated
/// against the proof that stands for the signatures, or against the header's
/// commitments to it and to the dependencies it proves. `block` has one
/// transaction with one dependency.
pub fn faulty_blocks(
    fault: Fault,
    block: &Block,
    request: &TxRequest,
    other: &KeyPair,
) -> Vec<FaultCase<FaultyBlock>> {
    // The verifier answers yes or no, so every proof that does not prove exactly
    // the block's dependencies is refused with the same error.
    const NOT_PROVEN: &str = "InvalidDependencyProof";
    let valid = block
        .body
        .dependency_proof
        .clone()
        .expect("a block of this fork has a dependency proof");
    let dependency = block_dependencies(&block.body.transactions)[0];
    let signer = request
        .keys
        .iter()
        .find(|key| public_key(key) == dependency.verification_key_hash)
        .expect("the request holds the key the block's dependency names");
    // The same message under the key that does not control the account.
    let foreign = Dependency {
        verification_key_hash: public_key(other),
        ..dependency
    };
    // A block refused on both import paths.
    let refused = |variant, block: Block, expected_error| {
        case(
            variant,
            FaultyBlock {
                block,
                paths: &ImportPath::ALL,
            },
            expected_error,
        )
    };
    // The block with another proof, which its header commits to.
    let proven = |variant, proof: Bytes| refused(variant, with_proof(block, proof), NOT_PROVEN);
    match fault {
        Fault::InvalidSignature => vec![proven(
            "one bit of the proof flipped",
            flipped(&valid, valid.len() / 2),
        )],
        Fault::WrongKey => vec![proven(
            "proof of a signature by another key",
            proof_of(&[(foreign, other)]),
        )],
        Fault::MalformedSignature => {
            let mut oversized = valid.to_vec();
            oversized.resize(MAX_DEPENDENCY_PROOF_BYTES + 1, 0);
            vec![
                proven("proof one byte short", truncated(&valid)),
                proven("proof one byte long", extended(&valid)),
                proven(
                    "the signature itself in place of the proof",
                    witness(&dependency, signer),
                ),
                refused(
                    "proof padded to one byte over MAX_DEPENDENCY_PROOF_BYTES",
                    with_proof(block, Bytes::from(oversized)),
                    "DependencyProofTooLarge",
                ),
            ]
        }
        Fault::MissingSignature => vec![
            // One claim fewer than the block's single dependency is no claim.
            proven(
                "empty proof, the proof of a block without dependencies",
                Bytes::new(),
            ),
            refused(
                "no proof in the body",
                altered(block, |block| block.body.dependency_proof = None),
                "DependencyProofNotPresent",
            ),
        ],
        Fault::DeclaredSuppliedMismatch => vec![
            proven(
                "proof of the block's dependency and of one more",
                proof_of(&[(dependency, signer), (foreign, other)]),
            ),
            refused(
                "header commitment that is not the proof's",
                altered(block, |block| {
                    block.header.dependency_proof_hash = Some(H256::repeat_byte(0x11));
                }),
                "DependencyProofHashNotMatch",
            ),
            refused(
                "header commitment that is not the dependencies'",
                altered(block, |block| {
                    block.header.block_deps_hash = Some(H256::repeat_byte(0x11));
                }),
                "BlockDepsHashNotMatch",
            ),
            refused(
                "the witness list of a Hegota block beside the proof",
                altered(block, |block| {
                    block.body.dependency_witnesses = Some(vec![witness(&dependency, signer)]);
                }),
                "DependencyWitnessesPresent",
            ),
        ],
    }
}
