//! Common measurement harness for proposals that authorize transactions with a
//! signature scheme of their own. It drives the real transaction lifecycle of
//! the client in-process: submission through the JSON-RPC dispatcher, pool
//! admission, block construction by the payload builder, and import of the
//! block on a fresh node, on both import paths.
//!
//! This file is the same, byte for byte, on every branch under comparison. It
//! uses only what all of them share (`Blockchain`, `Store`, the payload builder,
//! the RPC dispatcher, `Transaction`, `Block`). Everything a proposal defines
//! lives in `adapter.rs`, the only file that differs between branches.
//!
//! # The adapter interface
//!
//! `adapter.rs` must define the following items. A new alternative, or a later
//! milestone of an existing one, is added by writing only that file.
//!
//! Identity, recorded with every result:
//!
//! - `ALTERNATIVE: &str`, the label used in reports ("Alternative 1").
//! - `ALTERNATIVE_ID: &str`, a short identifier used in file names ("a1").
//! - `PROPOSAL: &str`, the proposal the branch implements.
//! - `MILESTONE: u8`, 1 for individually verified signatures, 2 for blocks that
//!   carry one aggregate proof.
//! - `SIGNATURE_SCHEME: &str`, the scheme that authorizes the transactions.
//! - `KEY_CACHE_NAMESPACE: &str`, the directory of the key cache this scheme's
//!   keys and signatures are stored under. Alternatives that share a scheme
//!   share the namespace, so they are measured with the same keys.
//! - `VERIFIER_PACKAGE: &str`, the name in `Cargo.lock` of the crate that
//!   verifies the scheme. Its version and source revision go in `meta.json`.
//! - `VERIFICATION: &[(&str, &str)]`, for each stage of the lifecycle, where the
//!   implementation verifies signatures and on how many threads.
//!
//! The signature scheme:
//!
//! - `generate_key(seed: &[u8; 32]) -> KeyPair`, deterministic.
//! - `sign(key: &KeyPair, message: &[u8; 32]) -> Vec<u8>`.
//! - `verify(public: &[u8], message: &[u8; 32], signature: &[u8]) -> bool`, the
//!   function the client itself calls to verify one signature.
//!
//! The chain and its accounts:
//!
//! - `prepare_genesis(genesis: &mut Genesis)`, for anything the alternative
//!   needs in the genesis beyond the accounts of the workload.
//! - `supports(kind: AccountKind) -> Result<(), &'static str>`, whether the
//!   alternative can express that kind of account; the error says why not.
//! - `account_address(index: u64, keys: &[KeyPair]) -> Address`.
//! - `account_code(kind: AccountKind, keys: &[KeyPair]) -> Vec<u8>`, the
//!   validation code of the account, empty for an account without code. For
//!   `AccountKind::KeyInStorage` the code reads the public key from storage
//!   slot 0 of the account it runs in.
//!
//! Transactions:
//!
//! - `build(request: &TxRequest) -> Submission`: builds and signs one
//!   transaction, and returns it with the bytes submitted through
//!   `eth_sendRawTransaction`, the bytes peers exchange for it, the bytes that
//!   travel beside it, and the components of the gas it is charged.
//! - `admit_from_peer(blockchain: &Blockchain, peer: &[u8]) -> Result<H256,
//!   String>` (async): decodes the bytes a peer sent and admits the transaction
//!   through the entry point the networking layer uses.
//!
//! Blocks:
//!
//! - `block_extras(block: &Block) -> BlockExtras`: what the block carries
//!   beside its transactions (individual signatures, an aggregate proof).
//! - `limits(facts: &TxFacts) -> Vec<Limit>`: every limit the proposal or this
//!   implementation of it places on such transactions, enforced or not.
//!
//! Faults, for the negative tests in `checks.rs`:
//!
//! - `faulty_submissions(fault: Fault, request: &TxRequest, other: &KeyPair)
//!   -> Vec<FaultCase<FaultySubmission>>`
//! - `faulty_blocks(fault: Fault, block: &Block, request: &TxRequest, other:
//!   &KeyPair) -> Vec<FaultCase<FaultyBlock>>`, where `block` is the valid
//!   block that holds only the transaction of `request`. Each case names the
//!   import paths that must refuse it.
//! - `fault_not_expressible(fault: Fault, layer: Layer) -> Option<&'static
//!   str>`, the reason when the alternative has no way to state a fault at a
//!   layer.
//!
//! # What the harness assumes about a branch
//!
//! - The chain starts with its forks active at genesis: `PQX_FORK` selects
//!   them, and `prepare_genesis` runs afterwards, so an adapter may schedule a
//!   fork at a later timestamp instead. Blocks are 12 seconds apart.
//! - A block is built by `Blockchain::build_payload` from the pool and imported
//!   by `Blockchain::add_block` and `Blockchain::add_block_pipeline`. Whatever a
//!   milestone adds to block construction or validation, an aggregate proof for
//!   example, is measured as part of those calls, as long as it happens inside
//!   them.
//! - The harness reads only the transactions, the header's gas fields and the
//!   RLP encoding of a block. Everything else a block carries is reported by
//!   `block_extras`, including bytes that travel with the block outside its
//!   RLP encoding.
//! - Every sender is its own payer and sends one transaction per block.
//!
//! # Running
//!
//! Measurements are ignored tests. They need a release build and run one at a
//! time:
//!
//! ```text
//! PQX_OUT=/path/to/out PQX_KEY_CACHE=/path/to/keys \
//!   cargo test --release -p ethrex-test --test ethrex_tests -- \
//!   --ignored --nocapture --test-threads=1 pqx::
//! ```
//!
//! Environment:
//!
//! - `PQX_OUT`: output directory (required by the measurements).
//! - `PQX_KEY_CACHE`: directory of the key and signature cache. Without it
//!   keys and signatures are regenerated on every run.
//! - `PQX_WORKLOAD`: comma separated workloads; default, every workload the
//!   alternative supports plus the two reference ones. Names: `transfer`,
//!   `contract_call`, `two_claims`, `eip1559_transfer`, `eip1559_contract_call`.
//! - `PQX_N`: comma separated block sizes, `max` for the most that fits;
//!   default `1,16,64,128,256,max`.
//! - `PQX_ITERATIONS` (default 30), `PQX_ITERATIONS_LARGE` (default 10, for
//!   blocks of more than 128 transactions), `PQX_WARMUP` (default 3).
//! - `PQX_GAS_LIMIT` (default 200000000): the block gas limit of the genesis
//!   and the ceiling the builder is given.
//! - `PQX_FORK` (default `hegota`; `lstar` also activates the following fork at
//!   genesis) and `PQX_GENESIS` (default `fixtures/genesis/l1-hegota.json`).
//! - `PQX_CAPACITY_MARGIN` (default 8): transactions offered beyond the number
//!   the gas limit admits, when measuring the most that fits.
//! - `PQX_PREPARE_THREADS`: threads used to generate keys and sign, outside
//!   every timed region; default, the available parallelism.
//! - `PQX_VERIFY_SAMPLES`, `PQX_VERIFY_BATCH`, `PQX_VERIFY_DISTINCT`,
//!   `PQX_VERIFY_BATCH_ITERATIONS`, `PQX_VERIFY_THREADS`: the verification
//!   micro-benchmark.
//! - `RAYON_NUM_THREADS`: read by the client, not by the harness. It sizes the
//!   thread pool the block pipeline executes transactions on. The harness
//!   records it.
//!
//! # Output
//!
//! In `PQX_OUT`, with `<id>` the adapter's `ALTERNATIVE_ID` and `<m>` its
//! milestone:
//!
//! - `<id>-m<m>.results.jsonl`: one JSON object per workload and block size,
//!   with every raw sample. Lines are appended, each with the `run_id` of the
//!   invocation that wrote it.
//! - `<id>-m<m>.microbench.jsonl`: the verification micro-benchmark.
//! - `<id>-m<m>.meta.<run_id>.json`: machine, compiler, build, branch and
//!   commit, verifier revision, thread settings and iteration counts of one
//!   invocation.
//!
//! All durations are nanoseconds. `median` is the mean of the two middle
//! samples when their number is even, `p95` is the nearest-rank percentile and
//! `sd` is the sample standard deviation.

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;
use ethrex_blockchain::{
    Blockchain, BlockchainOptions,
    payload::{BuildPayloadArgs, PayloadBuildResult, create_payload},
};
use ethrex_common::{
    Address, H160, H256, U256,
    constants::{MAX_RLP_BLOCK_SIZE, TX_MAX_GAS_LIMIT_AMSTERDAM},
    types::{
        Block, EIP1559Transaction, ELASTICITY_MULTIPLIER,
        FRAME_TX_MAX_PENDING_NONCANONICAL_PAYMASTER, Genesis, GenesisAccount, MAX_TX_SIZE, Receipt,
        Transaction, TxKind, block_access_list::BlockAccessList,
    },
    utils::keccak,
};
use ethrex_crypto::NativeCrypto;
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_levm::gas_cost::{STATE_BYTES_PER_STORAGE_SET, cost_per_state_byte};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_rpc::{
    rpc::{RpcApiContext, map_http_requests},
    test_utils::default_context_with_storage,
    utils::RpcRequest,
};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;
use serde_json::{Value, json};

use super::adapter;

// ---------------------------------------------------------------------------
// Types shared with the adapter
// ---------------------------------------------------------------------------

/// A key pair of the adapter's signature scheme, as the scheme's own bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPair {
    pub secret: Vec<u8>,
    pub public: Vec<u8>,
}

/// How an account decides that a transaction is authorized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountKind {
    /// One key, fixed in the account's code.
    SingleKey,
    /// Two keys, fixed in the account's code. Both must have signed.
    TwoKeys,
    /// One key, read from storage slot 0 of the account.
    KeyInStorage,
}

impl AccountKind {
    /// Number of keys, and of signatures, a transaction of the account needs.
    pub fn keys(self) -> usize {
        match self {
            AccountKind::SingleKey | AccountKind::KeyInStorage => 1,
            AccountKind::TwoKeys => 2,
        }
    }
}

/// The call a transaction makes once it is authorized.
#[derive(Clone, Debug)]
pub struct Call {
    pub target: Address,
    pub value: U256,
    pub data: Bytes,
    /// Execution gas the call may use.
    pub gas_limit: u64,
    /// State gas (EIP-8037) the call may use.
    pub state_gas_limit: u64,
}

/// Everything the adapter needs to build one transaction.
#[derive(Clone, Debug)]
#[allow(dead_code, reason = "an adapter reads the fields its proposal needs")]
pub struct TxRequest<'a> {
    pub chain_id: u64,
    pub sender: Address,
    pub nonce: u64,
    pub account: AccountKind,
    /// The keys that control the account, in the order its code expects them.
    pub keys: &'a [KeyPair],
    pub call: Call,
    /// Execution gas the account's validation may use.
    pub verify_gas_limit: u64,
    pub max_priority_fee_per_gas: u64,
    /// Most the sender pays per unit of gas. A transaction with one fee budget
    /// for all its gas takes this value times its gas limit.
    pub max_fee_per_gas: u64,
}

/// What a transaction carries and is charged for its signatures, under the
/// constants of its own proposal, as the branch's transaction type computes it.
#[derive(Clone, Debug, Default)]
pub struct GasProfile {
    pub signatures: u64,
    pub signature_bytes_in_transaction: u64,
    pub signature_bytes_beside_transaction: u64,
    /// Costs charged in full whatever the transaction does.
    pub mandatory_gas: u64,
    /// The transaction's data at the standard calldata price.
    pub calldata_gas: u64,
    /// The transaction's data at the calldata floor price, without the
    /// mandatory costs.
    pub calldata_floor_gas: u64,
    /// Sum of the execution gas limits the transaction declares.
    pub execution_gas_limits: u64,
    /// Sum of the state gas limits the transaction declares.
    pub state_gas_limits: u64,
    /// Mandatory costs, data at the standard price and declared limits.
    pub standard_gas_limit: u64,
    /// Mandatory costs and data at the floor price.
    pub calldata_floor_total: u64,
    /// The gas the transaction reserves in a block.
    pub max_gas: u64,
}

/// A built and signed transaction, in every form it takes before inclusion.
#[derive(Clone, Debug)]
pub struct Submission {
    /// The transaction as a block includes it.
    pub transaction: Transaction,
    /// The bytes passed to `eth_sendRawTransaction`.
    pub rpc: Vec<u8>,
    /// The item peers exchange for this transaction, without the envelope of
    /// the message that carries it.
    pub peer: Vec<u8>,
    /// Bytes of the announcement of this transaction to one peer.
    pub announcement_bytes: usize,
    /// Bytes that travel beside the transaction (0 when everything is in it).
    pub witness_bytes: usize,
    pub gas: GasProfile,
}

/// What a block carries beside its transactions.
#[derive(Clone, Debug, Default)]
pub struct BlockExtras {
    /// Individual signatures carried outside the transactions.
    pub witnesses: usize,
    pub witness_bytes: usize,
    /// Bytes of the aggregate proof, 0 when the block has none.
    pub aggregate_proof_bytes: usize,
    /// Bytes that travel with the block and are not part of its RLP encoding,
    /// 0 when the encoding holds everything. The harness adds them to the
    /// encoding's size for the size of the block as peers exchange it.
    pub outside_block_rlp_bytes: usize,
}

/// What the harness measured for one transaction of a workload.
#[derive(Clone, Debug, Default)]
#[allow(dead_code, reason = "an adapter reads the fields its limits depend on")]
pub struct TxFacts {
    pub signatures: u64,
    pub canonical_bytes: u64,
    pub rpc_bytes: u64,
    pub peer_bytes: u64,
    pub witness_bytes: u64,
    /// The transaction's gas limit, what the builder reserves for it.
    pub gas_limit: u64,
    /// The gas the EIP-7825 cap is measured on: the larger of the standard
    /// side without state gas and the calldata floor side.
    pub capped_gas: u64,
    /// Block gas one transaction used.
    pub block_gas: u64,
    /// Bytes one transaction adds to the serialized block.
    pub block_bytes: u64,
    /// Execution gas limits that count as validation work for the mempool.
    pub verify_gas_limit: u64,
    /// The gas limit of the block the transaction was included in.
    pub block_gas_limit: u64,
}

/// A limit that applies to the transactions of an alternative.
#[derive(Clone, Debug)]
pub struct Limit {
    pub name: &'static str,
    pub value: u64,
    /// `gas`, `bytes`, `signatures`, `transactions` or `frames`.
    pub unit: &'static str,
    /// `block`, `transaction` or `pool`.
    pub scope: &'static str,
    /// Where the limit is stated.
    pub source: &'static str,
    /// `consensus`, `builder`, `mempool policy` or `transport`.
    pub layer: &'static str,
    /// Whether this implementation enforces it.
    pub enforced: bool,
    /// Where it is enforced, or why it is not.
    pub enforced_at: &'static str,
    /// Units of the limit one transaction of the workload consumes.
    pub per_transaction: Option<u64>,
}

/// The ways a signature can fail to authorize a transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// A signature of the right size whose bytes were altered.
    InvalidSignature,
    /// A genuine signature made with another key.
    WrongKey,
    /// A signature of the wrong size.
    MalformedSignature,
    /// No signature where one is required.
    MissingSignature,
    /// What is declared and what is supplied do not correspond.
    DeclaredSuppliedMismatch,
}

impl Fault {
    pub fn name(self) -> &'static str {
        match self {
            Fault::InvalidSignature => "invalid signature",
            Fault::WrongKey => "signature of another key",
            Fault::MalformedSignature => "malformed or truncated signature",
            Fault::MissingSignature => "missing signature",
            Fault::DeclaredSuppliedMismatch => "declared and supplied do not match",
        }
    }
}

/// Where a fault must be caught.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Pool,
    Block,
}

/// A faulty transaction, in the form of the entry point it is sent to.
#[derive(Clone, Debug)]
#[allow(
    dead_code,
    reason = "an adapter uses the entry points its proposal has"
)]
pub enum FaultySubmission {
    /// Bytes for `eth_sendRawTransaction`.
    Rpc(Vec<u8>),
    /// Bytes as a peer would send them.
    Peer(Vec<u8>),
}

/// A faulty block and the import paths that must refuse it.
#[derive(Clone, Debug)]
pub struct FaultyBlock {
    pub block: Block,
    pub paths: &'static [ImportPath],
}

/// One concrete way of producing a fault.
#[derive(Clone, Debug)]
pub struct FaultCase<T> {
    pub variant: &'static str,
    pub input: T,
    /// Text the rejection must contain, so that a rejection for an unrelated
    /// reason does not pass for the expected one.
    pub expected_error: &'static str,
}

/// The transactions a block is filled with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workload {
    /// A transfer of ether authorized with the adapter's scheme.
    Transfer,
    /// A call that writes one new storage slot, authorized likewise.
    ContractCall,
    /// The transfer, from an account that requires two signatures.
    TwoClaims,
    /// Reference: the transfer as an EIP-1559 transaction signed with
    /// secp256k1, built by this file, the same on every branch.
    Eip1559Transfer,
    /// Reference: the storage write as an EIP-1559 transaction.
    Eip1559ContractCall,
}

impl Workload {
    pub const ALL: [Workload; 5] = [
        Workload::Transfer,
        Workload::ContractCall,
        Workload::TwoClaims,
        Workload::Eip1559Transfer,
        Workload::Eip1559ContractCall,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Workload::Transfer => "transfer",
            Workload::ContractCall => "contract_call",
            Workload::TwoClaims => "two_claims",
            Workload::Eip1559Transfer => "eip1559_transfer",
            Workload::Eip1559ContractCall => "eip1559_contract_call",
        }
    }

    pub fn from_name(name: &str) -> Option<Workload> {
        Workload::ALL
            .into_iter()
            .find(|workload| workload.name() == name)
    }

    /// The account the workload's senders use, `None` for the reference
    /// workloads, whose senders are plain secp256k1 accounts.
    pub fn account_kind(self) -> Option<AccountKind> {
        match self {
            Workload::Transfer | Workload::ContractCall => Some(AccountKind::SingleKey),
            Workload::TwoClaims => Some(AccountKind::TwoKeys),
            Workload::Eip1559Transfer | Workload::Eip1559ContractCall => None,
        }
    }

    /// Whether this branch can run the workload, and why not otherwise.
    pub fn supported(self) -> Result<(), &'static str> {
        match self.account_kind() {
            Some(kind) => adapter::supports(kind),
            None => Ok(()),
        }
    }

    /// What authorizes the workload's transactions.
    pub fn authorization(self) -> String {
        match self.account_kind() {
            Some(_) => adapter::SIGNATURE_SCHEME.to_string(),
            None => "secp256k1 (EIP-1559 reference)".to_string(),
        }
    }

    fn writes_storage(self) -> bool {
        matches!(self, Workload::ContractCall | Workload::Eip1559ContractCall)
    }
}

// ---------------------------------------------------------------------------
// Fixed parts of every chain
// ---------------------------------------------------------------------------

/// Receives the transfers. Funded at genesis, so a transfer creates no account.
pub const RECIPIENT: Address = H160([0x86; 20]);
/// `SSTORE(CALLER, 1)`: every sender writes a slot of its own, new each time.
pub const STORAGE_CONTRACT: Address = H160([0x87; 20]);
pub const STORAGE_CONTRACT_CODE: [u8; 5] = [0x60, 0x01, 0x33, 0x55, 0x00];
/// Receives the priority fees. Funded at genesis for the same reason.
pub const FEE_RECIPIENT: Address = H160([0x88; 20]);

pub const GWEI: u64 = 1_000_000_000;
/// One ether, far more than any transaction here can spend.
pub const SENDER_BALANCE: u64 = 1_000_000_000_000_000_000;
pub const MAX_PRIORITY_FEE_PER_GAS: u64 = GWEI;
pub const MAX_FEE_PER_GAS: u64 = 10 * GWEI;
/// Execution gas the account's validation may use, the same for every
/// alternative.
pub const VERIFY_GAS_LIMIT: u64 = 50_000;
/// Execution gas the workload's call may use, the same for every alternative.
pub const CALL_GAS_LIMIT: u64 = 50_000;
/// Gas limits of the reference EIP-1559 transactions. They have a single limit
/// for both gas dimensions, so the storage write needs room for its state gas.
pub const EIP1559_TRANSFER_GAS_LIMIT: u64 = 60_000;
pub const EIP1559_CONTRACT_CALL_GAS_LIMIT: u64 = 300_000;

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

// ---------------------------------------------------------------------------
// Environment
// ---------------------------------------------------------------------------

pub fn env_string(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

pub fn env_u64(name: &str, default: u64) -> u64 {
    match env_string(name) {
        Some(value) => value
            .parse()
            .unwrap_or_else(|_| panic!("{name} must be an unsigned integer, got {value:?}")),
        None => default,
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    usize::try_from(env_u64(name, default as u64)).expect("the value fits in usize")
}

pub fn gas_limit() -> u64 {
    env_u64("PQX_GAS_LIMIT", 200_000_000)
}

pub fn available_threads() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}

/// Threads of the pool the client executes a block's transactions on when it
/// imports through the pipeline: `RAYON_NUM_THREADS` when it is a positive
/// number, the available parallelism otherwise. This is the rule of the
/// thread pool library, restated here because the test crate cannot query it.
pub fn pipeline_threads() -> usize {
    env_string("RAYON_NUM_THREADS")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|threads| *threads > 0)
        .unwrap_or_else(available_threads)
}

fn prepare_threads() -> usize {
    env_usize("PQX_PREPARE_THREADS", available_threads()).max(1)
}

/// Applies `work` to every item on several threads and returns the results in
/// order. Used only outside timed regions, to generate keys and to sign.
pub fn parallel_map<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    if items.is_empty() {
        return Vec::new();
    }
    let threads = prepare_threads().min(items.len());
    let chunk = items.len().div_ceil(threads);
    let work = &work;
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|part| scope.spawn(move || part.iter().map(work).collect::<Vec<R>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("a preparation thread panicked"))
            .collect()
    })
}

// ---------------------------------------------------------------------------
// Keys and signatures, cached
// ---------------------------------------------------------------------------

fn key_cache_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env_string("PQX_KEY_CACHE")?).join(adapter::KEY_CACHE_NAMESPACE);
    fs::create_dir_all(&dir).expect("create the key cache directory");
    Some(dir)
}

/// Writes a cache file so that a reader never sees part of it.
fn write_cache_file(path: &Path, contents: &[u8]) {
    let unique = format!(
        "{}.{:?}.tmp",
        std::process::id(),
        std::thread::current().id()
    );
    let temporary = path.with_extension(unique);
    fs::write(&temporary, contents).expect("write a key cache file");
    fs::rename(&temporary, path).expect("move a key cache file into place");
}

/// The seed of key number `index`, the same on every branch.
fn key_seed(index: u64) -> [u8; 32] {
    let mut input = b"pqx key ".to_vec();
    input.extend_from_slice(&index.to_be_bytes());
    keccak(input).0
}

fn keys_in_memory() -> &'static Mutex<HashMap<u64, KeyPair>> {
    static KEYS: OnceLock<Mutex<HashMap<u64, KeyPair>>> = OnceLock::new();
    KEYS.get_or_init(Default::default)
}

fn signatures_in_memory() -> &'static Mutex<HashMap<H256, Vec<u8>>> {
    static SIGNATURES: OnceLock<Mutex<HashMap<H256, Vec<u8>>>> = OnceLock::new();
    SIGNATURES.get_or_init(Default::default)
}

/// Key number `index` of the adapter's scheme. Derived from the index alone,
/// so every branch that shares the scheme uses the same keys.
pub fn key(index: u64) -> KeyPair {
    if let Some(found) = keys_in_memory().lock().expect("key cache").get(&index) {
        return found.clone();
    }
    let seed = key_seed(index);
    let path = key_cache_dir().map(|dir| dir.join(format!("key-{}.txt", hex::encode(seed))));
    let cached = path.as_ref().and_then(|path| {
        let text = fs::read_to_string(path).ok()?;
        let mut lines = text.lines();
        let secret = hex::decode(lines.next()?).ok()?;
        let public = hex::decode(lines.next()?).ok()?;
        Some(KeyPair { secret, public })
    });
    let pair = cached.unwrap_or_else(|| {
        let pair = adapter::generate_key(&seed);
        if let Some(path) = &path {
            let text = format!(
                "{}\n{}\n",
                hex::encode(&pair.secret),
                hex::encode(&pair.public)
            );
            write_cache_file(path, text.as_bytes());
        }
        pair
    });
    keys_in_memory()
        .lock()
        .expect("key cache")
        .insert(index, pair.clone());
    pair
}

/// A signature of `message` by `key`, from the cache when it holds a valid one.
pub fn signature(key: &KeyPair, message: H256) -> Vec<u8> {
    let mut id = key.public.clone();
    id.extend_from_slice(message.as_bytes());
    let id = keccak(id);
    if let Some(found) = signatures_in_memory()
        .lock()
        .expect("signature cache")
        .get(&id)
    {
        return found.clone();
    }
    let path = key_cache_dir().map(|dir| dir.join(format!("signature-{}.bin", hex::encode(id))));
    // A cached signature is used only if it still verifies, so a cache left by
    // another scheme or a damaged file is replaced instead of trusted.
    let cached = path
        .as_ref()
        .and_then(|path| fs::read(path).ok())
        .filter(|cached| adapter::verify(&key.public, message.as_fixed_bytes(), cached));
    let signature = cached.unwrap_or_else(|| {
        let signature = adapter::sign(key, message.as_fixed_bytes());
        if let Some(path) = &path {
            write_cache_file(path, &signature);
        }
        signature
    });
    signatures_in_memory()
        .lock()
        .expect("signature cache")
        .insert(id, signature.clone());
    signature
}

/// The address of account number `index` when it is not derived from a key.
#[allow(dead_code, reason = "for adapters whose accounts are contracts")]
pub fn indexed_address(index: u64) -> Address {
    let mut bytes = [0u8; 20];
    bytes[0] = 0x51;
    bytes[12..].copy_from_slice(&index.to_be_bytes());
    H160(bytes)
}

/// The size in bytes of one transaction's entry in an eth/68
/// `NewPooledTransactionHashes` announcement: its type, its size and its hash.
pub fn pooled_announcement_bytes(transaction_size: usize) -> usize {
    1 + (transaction_size as u64).encode_to_vec().len() + 33
}

// ---------------------------------------------------------------------------
// The chain
// ---------------------------------------------------------------------------

/// A genesis and what the harness needs to know about it.
#[derive(Clone)]
pub struct Chain {
    genesis: Genesis,
    pub chain_id: u64,
    pub gas_limit: u64,
    pub fork: String,
    pub genesis_file: String,
}

impl Chain {
    /// The genesis named by the environment, with the block gas limit of the
    /// measurement, the recipient, the storage contract and `accounts`.
    pub fn new(accounts: Vec<(Address, GenesisAccount)>) -> Chain {
        let path = env_string("PQX_GENESIS").map_or_else(
            || workspace_root().join("fixtures/genesis/l1-hegota.json"),
            PathBuf::from,
        );
        let file = fs::File::open(&path)
            .unwrap_or_else(|error| panic!("open the genesis {}: {error}", path.display()));
        let mut genesis: Genesis =
            serde_json::from_reader(std::io::BufReader::new(file)).expect("parse the genesis");
        genesis.gas_limit = gas_limit();
        match env_string("PQX_FORK")
            .unwrap_or_else(|| "hegota".to_string())
            .to_lowercase()
            .as_str()
        {
            "hegota" => {}
            "lstar" => genesis.config.lstar_time = Some(0),
            other => panic!("PQX_FORK must be hegota or lstar, got {other:?}"),
        }
        adapter::prepare_genesis(&mut genesis);
        let existing = |balance: u64| GenesisAccount {
            code: Bytes::new(),
            storage: BTreeMap::new(),
            balance: U256::from(balance),
            nonce: 0,
        };
        genesis.alloc.insert(RECIPIENT, existing(1));
        genesis.alloc.insert(FEE_RECIPIENT, existing(1));
        genesis.alloc.insert(
            STORAGE_CONTRACT,
            GenesisAccount {
                code: Bytes::from_static(&STORAGE_CONTRACT_CODE),
                storage: BTreeMap::new(),
                balance: U256::zero(),
                nonce: 1,
            },
        );
        for (address, account) in accounts {
            genesis.alloc.insert(address, account);
        }
        let fork = format!("{:?}", genesis.config.fork(genesis.timestamp));
        Chain {
            chain_id: genesis.config.chain_id,
            gas_limit: genesis.gas_limit,
            fork,
            genesis_file: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            genesis,
        }
    }

    /// A node that knows only the genesis.
    pub async fn node(&self) -> Store {
        let mut store =
            Store::new("store.db", EngineType::InMemory).expect("build an in-memory store");
        store
            .add_initial_state(self.genesis.clone())
            .await
            .expect("add the genesis state");
        store
    }
}

/// A funded account.
pub fn funded_account(code: Vec<u8>) -> GenesisAccount {
    GenesisAccount {
        code: Bytes::from(code),
        storage: BTreeMap::new(),
        balance: U256::from(SENDER_BALANCE),
        nonce: 0,
    }
}

/// The call each workload makes. The state gas of the storage write is what
/// one new slot costs at the chain's gas limit.
pub fn workload_call(workload: Workload, gas_limit: u64) -> Call {
    if workload.writes_storage() {
        Call {
            target: STORAGE_CONTRACT,
            value: U256::zero(),
            data: Bytes::new(),
            gas_limit: CALL_GAS_LIMIT,
            state_gas_limit: STATE_BYTES_PER_STORAGE_SET * cost_per_state_byte(gas_limit),
        }
    } else {
        Call {
            target: RECIPIENT,
            value: U256::one(),
            data: Bytes::new(),
            gas_limit: CALL_GAS_LIMIT,
            state_gas_limit: 0,
        }
    }
}

/// The keys of sender number `index`. A second key comes from a range of
/// indices no first key uses.
pub fn sender_keys(kind: AccountKind, index: u64) -> Vec<KeyPair> {
    (0..kind.keys() as u64)
        .map(|position| key(position * 1_000_000 + index))
        .collect()
}

/// A workload's chain and its signed transactions, one sender each.
pub struct Prepared {
    pub workload: Workload,
    pub chain: Chain,
    pub submissions: Vec<Submission>,
}

/// Builds the chain for `count` senders of `workload` and signs one
/// transaction from each. Nothing here is timed.
pub async fn prepare(workload: Workload, count: usize) -> Prepared {
    let indices: Vec<u64> = (0..count as u64).collect();
    match workload.account_kind() {
        Some(kind) => {
            let senders: Vec<(Address, Vec<KeyPair>)> = parallel_map(&indices, |index| {
                let keys = sender_keys(kind, *index);
                (adapter::account_address(*index, &keys), keys)
            });
            let accounts = senders
                .iter()
                .map(|(address, keys)| {
                    (*address, funded_account(adapter::account_code(kind, keys)))
                })
                .collect();
            let chain = Chain::new(accounts);
            let submissions = parallel_map(&senders, |(address, keys)| {
                adapter::build(&TxRequest {
                    chain_id: chain.chain_id,
                    sender: *address,
                    nonce: 0,
                    account: kind,
                    keys,
                    call: workload_call(workload, chain.gas_limit),
                    verify_gas_limit: VERIFY_GAS_LIMIT,
                    max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
                    max_fee_per_gas: MAX_FEE_PER_GAS,
                })
            });
            Prepared {
                workload,
                chain,
                submissions,
            }
        }
        None => {
            let signers: Vec<LocalSigner> = indices
                .iter()
                .map(|index| LocalSigner::new(reference_secret_key(*index)))
                .collect();
            let accounts = signers
                .iter()
                .map(|signer| (signer.address, funded_account(Vec::new())))
                .collect();
            let chain = Chain::new(accounts);
            let mut submissions = Vec::with_capacity(count);
            for signer in signers {
                submissions.push(reference_submission(workload, &chain, signer).await);
            }
            Prepared {
                workload,
                chain,
                submissions,
            }
        }
    }
}

/// The secp256k1 key of reference sender number `index`.
pub fn reference_secret_key(index: u64) -> SecretKey {
    let mut input = b"pqx secp256k1 ".to_vec();
    input.extend_from_slice(&index.to_be_bytes());
    SecretKey::from_slice(keccak(input).as_bytes()).expect("the digest is a valid secret key")
}

/// The reference transaction of `workload`: the same call as the measured
/// workload, as an EIP-1559 transaction signed with secp256k1.
async fn reference_submission(
    workload: Workload,
    chain: &Chain,
    signer: LocalSigner,
) -> Submission {
    let call = workload_call(workload, chain.gas_limit);
    let gas_limit = if workload.writes_storage() {
        EIP1559_CONTRACT_CALL_GAS_LIMIT
    } else {
        EIP1559_TRANSFER_GAS_LIMIT
    };
    let mut transaction = Transaction::EIP1559Transaction(EIP1559Transaction {
        chain_id: chain.chain_id,
        nonce: 0,
        max_priority_fee_per_gas: MAX_PRIORITY_FEE_PER_GAS,
        max_fee_per_gas: MAX_FEE_PER_GAS,
        gas_limit,
        to: TxKind::Call(call.target),
        value: call.value,
        data: call.data,
        ..Default::default()
    });
    let signer: Signer = signer.into();
    transaction
        .sign_inplace(&signer)
        .await
        .expect("sign the reference transaction");
    let rpc = transaction.encode_canonical_to_vec();
    let peer = transaction.encode_to_vec();
    Submission {
        announcement_bytes: pooled_announcement_bytes(rpc.len()),
        witness_bytes: 0,
        gas: GasProfile {
            signatures: 1,
            // r and s, 32 bytes each, and the parity.
            signature_bytes_in_transaction: 65,
            standard_gas_limit: gas_limit,
            max_gas: gas_limit,
            ..Default::default()
        },
        transaction,
        rpc,
        peer,
    }
}

// ---------------------------------------------------------------------------
// The lifecycle, one step at a time
// ---------------------------------------------------------------------------

/// What the harness shares between nodes: the parts of the RPC context that
/// do not depend on the node, and the thread pool nodes compute state roots
/// on, built once so that no timed region pays for starting it.
pub struct Harness<'a> {
    rpc: RpcApiContext,
    new_blockchain: &'a dyn Fn(Store) -> Blockchain,
}

impl<'a> Harness<'a> {
    /// `new_blockchain` builds the `Blockchain` of every node. Callers pass a
    /// closure over one state-root thread pool, which this file cannot name.
    pub async fn new(new_blockchain: &'a dyn Fn(Store) -> Blockchain) -> Harness<'a> {
        let store = Store::new("store.db", EngineType::InMemory).expect("build a store");
        // Detached: the context is cloned for every node, and a guard that
        // joined its worker on drop would wait for those clones.
        let rpc = default_context_with_storage(store).await.into_detached();
        Harness {
            rpc,
            new_blockchain,
        }
    }

    /// A node on `chain` with an empty pool, and the RPC context that serves it.
    pub async fn node(&self, chain: &Chain) -> Node {
        let store = chain.node().await;
        let blockchain = Arc::new((self.new_blockchain)(store.clone()));
        let mut rpc = self.rpc.clone();
        rpc.storage = store.clone();
        rpc.blockchain = blockchain.clone();
        Node {
            store,
            blockchain,
            rpc,
            gas_limit: chain.gas_limit,
        }
    }
}

/// How a `Blockchain` is configured for every node of the harness: the
/// client's defaults, which are also what the RPC test context uses.
pub fn blockchain_options() -> BlockchainOptions {
    BlockchainOptions::default()
}

pub struct Node {
    pub store: Store,
    pub blockchain: Arc<Blockchain>,
    rpc: RpcApiContext,
    gas_limit: u64,
}

/// A block as the builder returned it.
pub struct Built {
    pub block: Block,
    pub receipts: Vec<Receipt>,
    pub access_list: Option<Arc<BlockAccessList>>,
}

/// The two ways the client imports a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportPath {
    /// `Blockchain::add_block`: executes the block on the calling thread. It
    /// does not compare the body with the header's commitments to it; its
    /// callers do, when they receive the body.
    AddBlock,
    /// `Blockchain::add_block_pipeline` with the block's access list, the path
    /// `engine_newPayload` takes: transactions execute on the thread pool, and
    /// the body is checked against the header.
    Pipeline,
}

impl ImportPath {
    pub const ALL: [ImportPath; 2] = [ImportPath::AddBlock, ImportPath::Pipeline];

    pub fn name(self) -> &'static str {
        match self {
            ImportPath::AddBlock => "add_block",
            ImportPath::Pipeline => "add_block_pipeline",
        }
    }
}

impl Node {
    /// Submits raw transaction bytes through the dispatcher the HTTP server
    /// calls for every request, and times the call: method lookup, decoding of
    /// the parameter, and admission to the pool.
    pub async fn submit(&self, raw: &[u8]) -> (Result<Value, String>, Duration) {
        let request: RpcRequest = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_sendRawTransaction",
            "params": [format!("0x{}", hex::encode(raw))],
        }))
        .expect("a well-formed request");
        let context = self.rpc.clone();
        let started = Instant::now();
        let result = map_http_requests(&request, context).await;
        let elapsed = started.elapsed();
        (result.map_err(|error| format!("{error:?}")), elapsed)
    }

    /// Admits the bytes a peer would send for a transaction of the adapter,
    /// and times the call: decoding and admission to the pool.
    pub async fn receive(&self, peer: &[u8]) -> (Result<H256, String>, Duration) {
        let started = Instant::now();
        let result = adapter::admit_from_peer(&self.blockchain, peer).await;
        (result, started.elapsed())
    }

    /// The same for a transaction that travels in the ordinary transaction
    /// messages, as the reference transactions do.
    pub async fn receive_plain(&self, peer: &[u8]) -> (Result<H256, String>, Duration) {
        let started = Instant::now();
        let result = match Transaction::decode(peer) {
            Ok(transaction) => self
                .blockchain
                .add_transaction_to_pool(transaction)
                .await
                .map_err(|error| format!("{error:?}: {error}")),
            Err(error) => Err(format!("{error:?}")),
        };
        (result, started.elapsed())
    }

    /// Builds a block on the node's head from its pool, with the chain's gas
    /// limit as ceiling, and times the builder.
    pub fn build(&self) -> (Built, Duration) {
        let number = self
            .store
            .get_latest_block_number()
            .expect("read the head's number");
        let head = self
            .store
            .get_block_header(number)
            .expect("read the head")
            .expect("the head exists");
        let args = BuildPayloadArgs {
            parent: head.hash(),
            timestamp: head.timestamp + 12,
            fee_recipient: FEE_RECIPIENT,
            random: H256::zero(),
            withdrawals: Some(Vec::new()),
            beacon_root: Some(H256::zero()),
            slot_number: Some(head.slot_number.unwrap_or_default() + 1),
            version: 1,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
            gas_ceil: self.gas_limit,
        };
        let started = Instant::now();
        let payload = create_payload(&args, &self.store, Bytes::new()).expect("create a payload");
        let result: PayloadBuildResult = self
            .blockchain
            .build_payload(payload)
            .expect("build the payload");
        let elapsed = started.elapsed();
        let built = Built {
            block: result.payload,
            receipts: result.receipts,
            access_list: result.block_access_list.map(Arc::new),
        };
        (built, elapsed)
    }

    /// Imports `block` on `path` and times it. The clone of the block is made
    /// before the clock starts.
    pub fn import(&self, built: &Built, path: ImportPath) -> (Result<(), String>, Duration) {
        self.import_block(built.block.clone(), built.access_list.clone(), path)
    }

    pub fn import_block(
        &self,
        block: Block,
        access_list: Option<Arc<BlockAccessList>>,
        path: ImportPath,
    ) -> (Result<(), String>, Duration) {
        let started = Instant::now();
        let result = match path {
            ImportPath::AddBlock => self.blockchain.add_block(block),
            ImportPath::Pipeline => self.blockchain.add_block_pipeline(block, access_list),
        };
        let elapsed = started.elapsed();
        (result.map_err(|error| format!("{error:?}")), elapsed)
    }

    /// Makes `block`, already imported, the head of the node's chain.
    pub async fn set_head(&self, block: &Block) {
        ethrex_blockchain::fork_choice::apply_fork_choice(
            &self.store,
            block.hash(),
            H256::zero(),
            H256::zero(),
            None,
        )
        .await
        .expect("make the block the head");
    }
}

/// One pass over the lifecycle with every duration it took.
pub struct Pass {
    pub admission: Vec<Duration>,
    pub peer_admission: Vec<Duration>,
    pub build: Duration,
    pub imports: Vec<(ImportPath, Duration)>,
    pub built: Built,
}

/// Runs the lifecycle once: every submission enters the pool of a fresh node
/// through the RPC, the node builds a block, and two more fresh nodes import
/// it, one on each path. A fourth fresh node admits the same transactions as a
/// peer would deliver them. Panics on any step that fails: a measurement of
/// something that did not happen is worth nothing.
pub async fn pass(harness: &Harness<'_>, prepared: &Prepared, expect_all_included: bool) -> Pass {
    let builder = harness.node(&prepared.chain).await;
    let mut admission = Vec::with_capacity(prepared.submissions.len());
    for (index, submission) in prepared.submissions.iter().enumerate() {
        let (result, elapsed) = builder.submit(&submission.rpc).await;
        let returned = result.unwrap_or_else(|error| {
            panic!("transaction {index} was not admitted through the RPC: {error}")
        });
        let hash = submission.transaction.hash(&NativeCrypto);
        assert_eq!(
            returned,
            json!(format!("{hash:#x}")),
            "the RPC returns the transaction's hash"
        );
        admission.push(elapsed);
    }

    let (built, build) = builder.build();
    let included = built.block.body.transactions.len();
    assert!(
        built.receipts.iter().all(|receipt| receipt.succeeded),
        "every included transaction must succeed"
    );
    if expect_all_included {
        assert_eq!(
            included,
            prepared.submissions.len(),
            "the block must include every transaction offered"
        );
    }

    let mut imports = Vec::new();
    for path in ImportPath::ALL {
        let node = harness.node(&prepared.chain).await;
        let (result, elapsed) = node.import(&built, path);
        result.unwrap_or_else(|error| panic!("{} rejected the block: {error}", path.name()));
        imports.push((path, elapsed));
    }

    let peer_node = harness.node(&prepared.chain).await;
    let mut peer_admission = Vec::with_capacity(prepared.submissions.len());
    for (index, submission) in prepared.submissions.iter().enumerate() {
        let (result, elapsed) = if prepared.workload.account_kind().is_some() {
            peer_node.receive(&submission.peer).await
        } else {
            peer_node.receive_plain(&submission.peer).await
        };
        result.unwrap_or_else(|error| {
            panic!("transaction {index} was not admitted from a peer: {error}")
        });
        peer_admission.push(elapsed);
    }

    Pass {
        admission,
        peer_admission,
        build,
        imports,
        built,
    }
}

/// The size, as peers exchange it, of the block a node builds from an empty
/// pool.
pub async fn empty_block_bytes(harness: &Harness<'_>, chain: &Chain) -> usize {
    let node = harness.node(chain).await;
    let (built, _) = node.build();
    assert!(built.block.body.transactions.is_empty());
    exchanged_bytes(&built.block)
}

/// The block's RLP encoding plus whatever the adapter says travels with the
/// block outside that encoding.
pub fn exchanged_bytes(block: &Block) -> usize {
    block.encode_to_vec().len() + adapter::block_extras(block).outside_block_rlp_bytes
}

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

fn nanoseconds(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).expect("a duration that fits in 64 bits")
}

/// Summary statistics of `samples`, in the samples' unit, with the samples.
pub fn statistics(samples: &[u64]) -> Value {
    if samples.is_empty() {
        return json!({ "count": 0, "samples": [] });
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let count = sorted.len();
    let median = if count % 2 == 1 {
        sorted[count / 2] as f64
    } else {
        (sorted[count / 2 - 1] as f64 + sorted[count / 2] as f64) / 2.0
    };
    // Nearest rank: the smallest sample with at least 95% of them at or below.
    let rank = (count as f64 * 0.95).ceil() as usize;
    let p95 = sorted[rank.clamp(1, count) - 1];
    let mean = sorted.iter().map(|sample| *sample as f64).sum::<f64>() / count as f64;
    let sd = if count > 1 {
        let squares: f64 = sorted
            .iter()
            .map(|sample| (*sample as f64 - mean).powi(2))
            .sum();
        (squares / (count as f64 - 1.0)).sqrt()
    } else {
        0.0
    };
    json!({
        "count": count,
        "min": sorted[0],
        "max": sorted[count - 1],
        "median": median,
        "p95": p95,
        "mean": mean,
        "sd": sd,
        "samples": samples,
    })
}

fn duration_statistics(samples: &[Duration]) -> Value {
    let samples: Vec<u64> = samples.iter().copied().map(nanoseconds).collect();
    statistics(&samples)
}

// ---------------------------------------------------------------------------
// Describing what was built
// ---------------------------------------------------------------------------

fn gas_profile_json(profile: &GasProfile) -> Value {
    json!({
        "signatures": profile.signatures,
        "signature_bytes_in_transaction": profile.signature_bytes_in_transaction,
        "signature_bytes_beside_transaction": profile.signature_bytes_beside_transaction,
        "mandatory_gas": profile.mandatory_gas,
        "calldata_gas": profile.calldata_gas,
        "calldata_floor_gas": profile.calldata_floor_gas,
        "execution_gas_limits": profile.execution_gas_limits,
        "state_gas_limits": profile.state_gas_limits,
        "standard_gas_limit": profile.standard_gas_limit,
        "calldata_floor_total": profile.calldata_floor_total,
        "max_gas": profile.max_gas,
    })
}

/// The generic limits, those every transaction of the chain is under.
fn common_limits(chain: &Chain, facts: &TxFacts, frame_transaction: bool) -> Vec<Limit> {
    let mut limits = vec![
        Limit {
            name: "block gas limit",
            value: chain.gas_limit,
            unit: "gas",
            scope: "block",
            source: "protocol; set to the value under test in the genesis",
            layer: "consensus",
            enforced: true,
            enforced_at: "payload builder and block import",
            per_transaction: Some(facts.block_gas),
        },
        Limit {
            name: "EIP-7934 block size (RLP), as the client applies it",
            value: MAX_RLP_BLOCK_SIZE,
            unit: "bytes",
            scope: "block",
            source: "protocol: EIP-7934",
            layer: "consensus",
            enforced: true,
            enforced_at: "payload builder and block import",
            per_transaction: Some(facts.block_bytes),
        },
        Limit {
            name: "EIP-7825 transaction gas cap",
            value: TX_MAX_GAS_LIMIT_AMSTERDAM,
            unit: "gas",
            scope: "transaction",
            source: "protocol: EIP-7825",
            layer: "consensus",
            enforced: true,
            enforced_at: "execution of the transaction, by the builder and at import; \
                          pool admission does not check it",
            per_transaction: Some(facts.capped_gas),
        },
        Limit {
            name: "pool: largest transaction",
            value: MAX_TX_SIZE as u64,
            unit: "bytes",
            scope: "transaction",
            source: "client policy",
            layer: "mempool policy",
            enforced: true,
            enforced_at: "pool admission",
            per_transaction: Some(facts.canonical_bytes),
        },
        Limit {
            name: "pool: most transactions",
            value: blockchain_options().max_mempool_size as u64,
            unit: "transactions",
            scope: "pool",
            source: "client policy",
            layer: "mempool policy",
            enforced: true,
            enforced_at: "pool admission",
            per_transaction: Some(1),
        },
    ];
    if frame_transaction {
        // Both rules make a block of N such transactions need N accounts.
        limits.push(Limit {
            name: "pool: pending frame transactions of one sender",
            value: 1,
            unit: "transactions",
            scope: "pool",
            source: "EIP-8141, public mempool rules",
            layer: "mempool policy",
            enforced: true,
            enforced_at: "pool admission",
            per_transaction: Some(1),
        });
        limits.push(Limit {
            name: "pool: pending frame transactions of one payer",
            value: u64::from(FRAME_TX_MAX_PENDING_NONCANONICAL_PAYMASTER),
            unit: "transactions",
            scope: "pool",
            source: "EIP-8141, MAX_PENDING_TXS_USING_NON_CANONICAL_PAYMASTER; the client \
                     applies it to every payer, a sender that pays for itself included",
            layer: "mempool policy",
            enforced: true,
            enforced_at: "pool admission",
            per_transaction: Some(1),
        });
    }
    limits
}

fn limit_json(limit: &Limit) -> Value {
    let bound = limit
        .per_transaction
        .filter(|per_transaction| *per_transaction > 0)
        .map(|per_transaction| limit.value / per_transaction);
    json!({
        "name": limit.name,
        "value": limit.value,
        "unit": limit.unit,
        "scope": limit.scope,
        "source": limit.source,
        "layer": limit.layer,
        "enforced_by_the_implementation": limit.enforced,
        "enforced_at": limit.enforced_at,
        "one_transaction_consumes": limit.per_transaction,
        // For a block or pool limit, how many transactions of the workload it
        // admits; for a transaction limit, how many times over it fits.
        "bound": bound,
    })
}

/// Everything about a built block and its transactions that is not a timing.
fn describe(prepared: &Prepared, built: &Built, empty_block_bytes: usize) -> (Value, TxFacts) {
    let block = &built.block;
    let included = block.body.transactions.len();
    let first = prepared
        .submissions
        .first()
        .expect("a workload has at least one transaction");
    let canonical: Vec<usize> = prepared
        .submissions
        .iter()
        .map(|submission| submission.transaction.encode_canonical_to_vec().len())
        .collect();
    let sum = |values: &mut dyn Iterator<Item = usize>| values.sum::<usize>();
    let rpc_total = sum(&mut prepared.submissions.iter().map(|s| s.rpc.len()));
    let peer_total = sum(&mut prepared.submissions.iter().map(|s| s.peer.len()));
    let announcement_total = sum(&mut prepared.submissions.iter().map(|s| s.announcement_bytes));
    let witness_total = sum(&mut prepared.submissions.iter().map(|s| s.witness_bytes));

    let block_bytes = block.encode_to_vec().len();
    let header_bytes = block.header.encode_to_vec().len();
    let body_bytes = block.body.encode_to_vec().len();
    let extras = adapter::block_extras(block);
    let exchanged = block_bytes + extras.outside_block_rlp_bytes;
    let per_transaction_bytes = if included > 0 {
        (exchanged.saturating_sub(empty_block_bytes)) as f64 / included as f64
    } else {
        0.0
    };

    let block_gas = block.header.gas_used;
    let receipts_gas = built
        .receipts
        .last()
        .map_or(0, |receipt| receipt.cumulative_gas_used);
    let first_receipt = built.receipts.first();
    let frames: Vec<Value> = first_receipt
        .and_then(|receipt| receipt.frame_receipts.as_ref())
        .map(|frames| {
            frames
                .iter()
                .map(|frame| {
                    json!({
                        "status": frame.status,
                        "execution_gas_used": frame.gas_used,
                        "state_gas_used": frame.state_gas_used,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let facts = TxFacts {
        signatures: first.gas.signatures,
        canonical_bytes: canonical.first().copied().unwrap_or_default() as u64,
        rpc_bytes: first.rpc.len() as u64,
        peer_bytes: first.peer.len() as u64,
        witness_bytes: first.witness_bytes as u64,
        gas_limit: first.transaction.gas_limit(),
        capped_gas: first
            .gas
            .standard_gas_limit
            .saturating_sub(first.gas.state_gas_limits)
            .max(first.gas.calldata_floor_total),
        block_gas: if included > 0 {
            block_gas / included as u64
        } else {
            0
        },
        block_bytes: per_transaction_bytes.ceil() as u64,
        verify_gas_limit: VERIFY_GAS_LIMIT,
        block_gas_limit: block.header.gas_limit,
    };
    let frame_transaction = matches!(first.transaction, Transaction::FrameTransaction(_));
    let mut limits = common_limits(&prepared.chain, &facts, frame_transaction);
    if prepared.workload.account_kind().is_some() {
        limits.extend(adapter::limits(&facts));
    }

    let description = json!({
        "sizes": {
            "transaction_canonical_bytes": canonical.first(),
            "transaction_canonical_bytes_min": canonical.iter().min(),
            "transaction_canonical_bytes_max": canonical.iter().max(),
            "witness_bytes_beside_one_transaction": first.witness_bytes,
            "rpc_submission_bytes_one_transaction": first.rpc.len(),
            "peer_exchange_bytes_one_transaction": first.peer.len(),
            "announcement_bytes_one_transaction": first.announcement_bytes,
            "rpc_submission_bytes_offered_total": rpc_total,
            "peer_exchange_bytes_offered_total": peer_total,
            "announcement_bytes_offered_total": announcement_total,
            "witness_bytes_offered_total": witness_total,
            "block_rlp_bytes": block_bytes,
            "block_header_rlp_bytes": header_bytes,
            "block_body_rlp_bytes": body_bytes,
            "block_bytes_outside_rlp": extras.outside_block_rlp_bytes,
            "block_exchanged_bytes": exchanged,
            "empty_block_exchanged_bytes": empty_block_bytes,
            "block_bytes_per_transaction": per_transaction_bytes,
            "block_witnesses": extras.witnesses,
            "block_witness_bytes": extras.witness_bytes,
            "block_aggregate_proof_bytes": extras.aggregate_proof_bytes,
        },
        "gas": {
            "block_gas_limit": block.header.gas_limit,
            "block_gas_used": block_gas,
            "block_gas_used_per_transaction": if included > 0 {
                block_gas as f64 / included as f64
            } else {
                0.0
            },
            "receipts_cumulative_gas_used": receipts_gas,
            "first_transaction_receipt_gas_used": first_receipt.map(|r| r.cumulative_gas_used),
            "first_transaction_gas_limit": first.transaction.gas_limit(),
            "first_transaction_frames": frames,
            "first_transaction_profile": gas_profile_json(&first.gas),
        },
        "limits": limits.iter().map(limit_json).collect::<Vec<_>>(),
    });
    (description, facts)
}

// ---------------------------------------------------------------------------
// The machine, the build and the run
// ---------------------------------------------------------------------------

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(workspace_root())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|text| !text.is_empty())
}

fn file_field(path: &str, field: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    text.lines()
        .find(|line| line.starts_with(field))
        .and_then(|line| line.split_once(':'))
        .map(|(_, value)| value.trim().to_string())
}

fn machine() -> Value {
    let linux = cfg!(target_os = "linux");
    let cpu = if linux {
        file_field("/proc/cpuinfo", "model name")
    } else {
        command_output("sysctl", &["-n", "machdep.cpu.brand_string"])
    };
    let memory_bytes = if linux {
        file_field("/proc/meminfo", "MemTotal")
            .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
            .map(|kilobytes| kilobytes * 1024)
    } else {
        command_output("sysctl", &["-n", "hw.memsize"]).and_then(|value| value.parse().ok())
    };
    let physical_cores = if linux {
        command_output(
            "sh",
            &[
                "-c",
                "lscpu -p=core,socket | grep -v '^#' | sort -u | wc -l",
            ],
        )
    } else {
        command_output("sysctl", &["-n", "hw.physicalcpu"])
    }
    .and_then(|value| value.trim().parse::<u64>().ok());
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "kernel": command_output("uname", &["-srm"]),
        "cpu": cpu,
        "logical_cores": available_threads(),
        "physical_cores": physical_cores,
        "memory_bytes": memory_bytes,
    })
}

/// The `[package]` entry of `name` in `Cargo.lock`: its version and source.
fn locked_package(name: &str) -> Value {
    let lock = fs::read_to_string(workspace_root().join("Cargo.lock")).unwrap_or_default();
    let wanted = format!("name = \"{name}\"");
    let entries: Vec<Value> = lock
        .split("[[package]]")
        .filter(|entry| entry.lines().any(|line| line.trim() == wanted))
        .map(|entry| {
            let field = |field: &str| {
                entry
                    .lines()
                    .find_map(|line| line.trim().strip_prefix(&format!("{field} = ")))
                    .map(|value| value.trim_matches('"').to_string())
            };
            json!({ "name": name, "version": field("version"), "source": field("source") })
        })
        .collect();
    json!(entries)
}

/// The release profile of the workspace, as its manifest states it.
fn release_profile() -> Option<String> {
    let manifest = fs::read_to_string(workspace_root().join("Cargo.toml")).ok()?;
    let start = manifest.find("[profile.release]")?;
    let rest = &manifest[start..];
    let end = rest[1..].find("\n[").map_or(rest.len(), |index| index + 1);
    Some(rest[..end].trim().to_string())
}

fn build() -> Value {
    let features: Vec<&str> = [
        ("avx2", cfg!(target_feature = "avx2")),
        ("avx512f", cfg!(target_feature = "avx512f")),
        ("bmi2", cfg!(target_feature = "bmi2")),
        ("sha", cfg!(target_feature = "sha")),
        ("aes", cfg!(target_feature = "aes")),
        ("neon", cfg!(target_feature = "neon")),
        ("sha3", cfg!(target_feature = "sha3")),
    ]
    .into_iter()
    .filter_map(|(name, enabled)| enabled.then_some(name))
    .collect();
    json!({
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "debug_assertions": cfg!(debug_assertions),
        "release_profile_in_manifest": release_profile(),
        "rustflags_at_compile_time": option_env!("RUSTFLAGS"),
        "encoded_rustflags_at_compile_time": option_env!("CARGO_ENCODED_RUSTFLAGS"),
        "cargo_config": fs::read_to_string(workspace_root().join(".cargo/config.toml")).ok(),
        "target_features_compiled_in": features,
        "rustc": command_output("rustc", &["-vV"]),
        "cargo": command_output("cargo", &["--version"]),
    })
}

fn source() -> Value {
    let digest = |path: &str| {
        command_output("shasum", &["-a", "256", path])
            .or_else(|| command_output("sha256sum", &[path]))
            .and_then(|line| line.split_whitespace().next().map(str::to_string))
    };
    json!({
        "branch": command_output("git", &["rev-parse", "--abbrev-ref", "HEAD"]),
        "commit": command_output("git", &["rev-parse", "HEAD"]),
        "uncommitted_changes": command_output("git", &["status", "--porcelain"])
            .map_or(0, |status| status.lines().count()),
        "lifecycle_rs_sha256": digest("test/tests/pqx/lifecycle.rs"),
        "checks_rs_sha256": digest("test/tests/pqx/checks.rs"),
        "adapter_rs_sha256": digest("test/tests/pqx/adapter.rs"),
        "lifecycle_rs_keccak256_at_compile_time":
            format!("{:#x}", keccak(include_bytes!("lifecycle.rs"))),
        "adapter_rs_keccak256_at_compile_time":
            format!("{:#x}", keccak(include_bytes!("adapter.rs"))),
    })
}

fn threads() -> Value {
    json!({
        "available_parallelism": available_threads(),
        "RAYON_NUM_THREADS": env_string("RAYON_NUM_THREADS"),
        "block_pipeline_execution_threads": pipeline_threads(),
        "block_pipeline_execution_threads_note":
            "derived: RAYON_NUM_THREADS when positive, else the available parallelism",
        "state_root_pool_threads": 17,
        "add_block_execution_threads": 1,
        "pool_admission_threads": 1,
        "payload_builder_execution_threads": 1,
    })
}

/// Memory of this process: its high-water mark where the system reports one
/// without a system call binding (Linux), and its current resident size.
fn memory() -> Value {
    let peak = file_field("/proc/self/status", "VmHWM")
        .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
        .map(|kilobytes| kilobytes * 1024);
    let current = command_output("ps", &["-o", "rss=", "-p", &std::process::id().to_string()])
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|kilobytes| kilobytes * 1024);
    json!({ "peak_resident_bytes": peak, "resident_bytes": current })
}

fn identity() -> Value {
    json!({
        "alternative": adapter::ALTERNATIVE,
        "alternative_id": adapter::ALTERNATIVE_ID,
        "proposal": adapter::PROPOSAL,
        "milestone": adapter::MILESTONE,
        "signature_scheme": adapter::SIGNATURE_SCHEME,
    })
}

/// One invocation of the measurements: where it writes and how it is named.
pub struct Run {
    id: String,
    out: PathBuf,
}

impl Run {
    /// Starts a run and writes its `meta` file. `None` without `PQX_OUT`.
    pub fn start(settings: Value) -> Option<Run> {
        let out = PathBuf::from(env_string("PQX_OUT")?);
        fs::create_dir_all(&out).expect("create the output directory");
        let since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("a clock after 1970");
        let id = format!("{}-{}", since_epoch.as_secs(), std::process::id());
        let run = Run { id, out };
        let meta = json!({
            "schema": 1,
            "run_id": run.id,
            "started_unix_seconds": since_epoch.as_secs(),
            "identity": identity(),
            "machine": machine(),
            "build": build(),
            "source": source(),
            "verifier": locked_package(adapter::VERIFIER_PACKAGE),
            "threads": threads(),
            "verification": adapter::VERIFICATION.iter().map(|(stage, how)| json!({
                "stage": stage, "how": how,
            })).collect::<Vec<_>>(),
            "settings": settings,
        });
        let path = run
            .out
            .join(format!("{}.meta.{}.json", run.prefix(), run.id));
        fs::write(
            &path,
            serde_json::to_string_pretty(&meta).expect("serialize the meta"),
        )
        .expect("write the meta file");
        println!("pqx: meta written to {}", path.display());
        Some(run)
    }

    fn prefix(&self) -> String {
        format!("{}-m{}", adapter::ALTERNATIVE_ID, adapter::MILESTONE)
    }

    /// Appends one JSON object as a line of `<prefix>.<kind>.jsonl`.
    pub fn append(&self, kind: &str, mut record: Value) {
        if let Some(object) = record.as_object_mut() {
            object.insert("schema".to_string(), json!(1));
            object.insert("run_id".to_string(), json!(self.id));
        }
        let path = self.out.join(format!("{}.{kind}.jsonl", self.prefix()));
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("open the results file");
        writeln!(file, "{record}").expect("write a result");
    }
}

// ---------------------------------------------------------------------------
// Measurements
// ---------------------------------------------------------------------------

/// Measurements never overlap, whatever `--test-threads` says.
pub fn exclusive() -> std::sync::MutexGuard<'static, ()> {
    static MEASURING: Mutex<()> = Mutex::new(());
    MEASURING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A debug build measures nothing worth recording.
fn require_release_build() {
    if cfg!(debug_assertions) {
        panic!("measurements need a release build");
    }
}

/// A block size to measure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockSize {
    Exactly(usize),
    /// As many as the builder includes when offered more than fit.
    Most,
}

fn block_sizes() -> Vec<BlockSize> {
    env_string("PQX_N")
        .unwrap_or_else(|| "1,16,64,128,256,max".to_string())
        .split(',')
        .map(|size| match size.trim() {
            "max" => BlockSize::Most,
            number => BlockSize::Exactly(
                number
                    .parse()
                    .unwrap_or_else(|_| panic!("PQX_N holds numbers or max, got {number:?}")),
            ),
        })
        .collect()
}

fn workloads() -> Vec<Workload> {
    match env_string("PQX_WORKLOAD") {
        Some(names) => names
            .split(',')
            .map(|name| {
                Workload::from_name(name.trim())
                    .unwrap_or_else(|| panic!("PQX_WORKLOAD names an unknown workload: {name:?}"))
            })
            .collect(),
        None => Workload::ALL.to_vec(),
    }
}

/// Timed iterations for a block of `transactions`, and the fewest the
/// comparison accepts for that size.
fn iterations(transactions: usize) -> (usize, usize) {
    if transactions > 128 {
        (env_usize("PQX_ITERATIONS_LARGE", 10), 10)
    } else {
        (env_usize("PQX_ITERATIONS", 30), 30)
    }
}

/// Measures one workload, which the alternative must support, at one block
/// size, and returns the record to write.
pub async fn measure_configuration(
    harness: &Harness<'_>,
    workload: Workload,
    size: BlockSize,
) -> Value {
    let label = match size {
        BlockSize::Exactly(count) => count.to_string(),
        BlockSize::Most => "max".to_string(),
    };
    let mut record = identity();
    let fields = record.as_object_mut().expect("the identity is an object");
    fields.insert("workload".to_string(), json!(workload.name()));
    fields.insert("authorization".to_string(), json!(workload.authorization()));
    fields.insert("n".to_string(), json!(label));
    fields.insert("supported".to_string(), json!(true));

    // How many transactions to offer. For the most that fits, a block of one
    // transaction gives the gas of one, and the pool is offered a margin over
    // what the gas limit admits.
    let (offered, gas_limited, probe_gas) = match size {
        BlockSize::Exactly(count) => (count, None, None),
        BlockSize::Most => {
            let probe = prepare(workload, 1).await;
            let gas = pass(harness, &probe, true)
                .await
                .built
                .block
                .header
                .gas_used;
            let gas_limited = usize::try_from(probe.chain.gas_limit / gas).expect("fits");
            let margin = env_usize("PQX_CAPACITY_MARGIN", 8);
            let pool = blockchain_options().max_mempool_size;
            (
                (gas_limited + margin).min(pool),
                Some(gas_limited),
                Some(gas),
            )
        }
    };
    println!(
        "pqx: {} {} n={label}: preparing {offered} transactions",
        adapter::ALTERNATIVE,
        workload.name()
    );
    let prepared = prepare(workload, offered).await;
    let empty = empty_block_bytes(harness, &prepared.chain).await;
    let expect_all = matches!(size, BlockSize::Exactly(_));

    let warmup = env_usize("PQX_WARMUP", 3);
    for _ in 0..warmup {
        pass(harness, &prepared, expect_all).await;
    }
    let mut last = pass(harness, &prepared, expect_all).await;
    let included = last.built.block.body.transactions.len();
    let (timed, minimum) = iterations(included);

    let mut admission = Vec::new();
    let mut peer_admission = Vec::new();
    let mut build = Vec::new();
    let mut imports: BTreeMap<&'static str, Vec<Duration>> = BTreeMap::new();
    let mut block_hashes = Vec::new();
    for iteration in 0..timed {
        // The pass that counted the block's transactions is the first sample.
        if iteration > 0 {
            last = pass(harness, &prepared, expect_all).await;
        }
        assert_eq!(
            last.built.block.body.transactions.len(),
            included,
            "every iteration builds a block of the same size"
        );
        admission.extend(last.admission.iter().copied());
        peer_admission.extend(last.peer_admission.iter().copied());
        build.push(last.build);
        for (path, elapsed) in &last.imports {
            imports.entry(path.name()).or_default().push(*elapsed);
        }
        block_hashes.push(last.built.block.hash());
    }
    block_hashes.sort_unstable();
    block_hashes.dedup();

    let (description, facts) = describe(&prepared, &last.built, empty);
    // What kept the builder from including more, when it was offered more than
    // it included. The builder skips a transaction when the gas left is below
    // what the transaction reserves, and stops when one more would take the
    // block over the size limit.
    let header = &last.built.block.header;
    let gas_left = header.gas_limit.saturating_sub(header.gas_used);
    let reservation = facts.gas_limit.min(TX_MAX_GAS_LIMIT_AMSTERDAM);
    let block_bytes = last.built.block.encode_to_vec().len() as u64;
    let out_of_gas = gas_left < reservation;
    let out_of_bytes = block_bytes + facts.block_bytes > MAX_RLP_BLOCK_SIZE;
    let pool_capacity = blockchain_options().max_mempool_size;
    let stopped_by = match size {
        BlockSize::Exactly(_) => None,
        BlockSize::Most if included < offered && out_of_gas => Some("block gas limit"),
        BlockSize::Most if included < offered && out_of_bytes => Some("block size limit"),
        BlockSize::Most if included == offered && offered >= pool_capacity => Some("pool size"),
        BlockSize::Most => Some("undetermined"),
    };
    let fields = record.as_object_mut().expect("the identity is an object");
    fields.insert("fork".to_string(), json!(prepared.chain.fork));
    fields.insert("genesis".to_string(), json!(prepared.chain.genesis_file));
    fields.insert("chain_id".to_string(), json!(prepared.chain.chain_id));
    fields.insert("offered".to_string(), json!(offered));
    fields.insert("included".to_string(), json!(included));
    fields.insert("warmup_iterations".to_string(), json!(warmup));
    fields.insert("iterations".to_string(), json!(timed));
    fields.insert("minimum_iterations".to_string(), json!(minimum));
    fields.insert(
        "meets_minimum_iterations".to_string(),
        json!(timed >= minimum),
    );
    fields.insert(
        "distinct_blocks_built".to_string(),
        json!(block_hashes.len()),
    );
    fields.insert("threads".to_string(), threads());
    fields.insert("memory".to_string(), memory());
    if let Some(description) = description.as_object() {
        for (name, value) in description {
            fields.insert(name.clone(), value.clone());
        }
    }
    fields.insert(
        "capacity".to_string(),
        json!({
            "gas_of_one_transaction": probe_gas.unwrap_or(facts.block_gas),
            "gas_limited_transactions": gas_limited.map_or_else(
                || prepared.chain.gas_limit.checked_div(facts.block_gas),
                |transactions| Some(transactions as u64),
            ),
            "measured_under_all_limits": matches!(size, BlockSize::Most),
            "offered": offered,
            "included_under_all_limits": matches!(size, BlockSize::Most).then_some(included),
            "stopped_by": stopped_by,
            "gas_left_in_the_block": gas_left,
            "gas_one_more_transaction_reserves": reservation,
            "block_bytes_left": MAX_RLP_BLOCK_SIZE.saturating_sub(block_bytes),
            "pool_capacity_transactions": pool_capacity,
            // The harness calls the builder directly and lets it run until the
            // pool has nothing left that fits: no getPayload request and no
            // slot deadline cuts the build short.
            "build_deadline": "none: the builder ran until nothing more fitted",
        }),
    );
    fields.insert(
        "timings_ns".to_string(),
        json!({
            "admission_rpc_per_transaction": duration_statistics(&admission),
            "admission_peer_per_transaction": duration_statistics(&peer_admission),
            "block_construction": duration_statistics(&build),
            "block_import": imports
                .iter()
                .map(|(path, samples)| (path.to_string(), duration_statistics(samples)))
                .collect::<BTreeMap<_, _>>(),
        }),
    );
    println!(
        "pqx: {} {} n={label}: included {included} of {offered}, block {} bytes, gas {}",
        adapter::ALTERNATIVE,
        workload.name(),
        last.built.block.encode_to_vec().len(),
        last.built.block.header.gas_used,
    );
    record
}

/// The lifecycle measurement: every selected workload at every selected size.
#[test]
#[ignore = "measurement: run in release, alone, with PQX_OUT set"]
fn measure() {
    let _alone = exclusive();
    require_release_build();
    // The same runtime `#[tokio::test]` builds: one thread, so that nothing
    // but the step being timed runs while it is timed.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build the runtime")
        .block_on(measure_every_configuration());
}

async fn measure_every_configuration() {
    let workloads = workloads();
    let sizes = block_sizes();
    let run = Run::start(json!({
        "workloads": workloads.iter().map(|workload| workload.name()).collect::<Vec<_>>(),
        "block_sizes": env_string("PQX_N").unwrap_or_else(|| "1,16,64,128,256,max".to_string()),
        "iterations": env_usize("PQX_ITERATIONS", 30),
        "iterations_large_blocks": env_usize("PQX_ITERATIONS_LARGE", 10),
        "warmup_iterations": env_usize("PQX_WARMUP", 3),
        "block_gas_limit": gas_limit(),
        "capacity_margin": env_usize("PQX_CAPACITY_MARGIN", 8),
        "fork": env_string("PQX_FORK").unwrap_or_else(|| "hegota".to_string()),
        "genesis": env_string("PQX_GENESIS"),
        "key_cache": env_string("PQX_KEY_CACHE").is_some(),
        "verify_gas_limit": VERIFY_GAS_LIMIT,
        "call_gas_limit": CALL_GAS_LIMIT,
        "max_priority_fee_per_gas": MAX_PRIORITY_FEE_PER_GAS,
        "max_fee_per_gas": MAX_FEE_PER_GAS,
    }))
    .expect("PQX_OUT names the directory the results are written to");

    let pool = Blockchain::build_merkle_pool();
    let new_blockchain = |store: Store| Blockchain::for_test_harness_with_pool(store, pool.clone());
    let harness = Harness::new(&new_blockchain).await;
    for workload in workloads {
        // A workload the alternative cannot express is recorded once, with the
        // reason, instead of once per block size.
        if let Err(reason) = workload.supported() {
            let mut record = identity();
            let fields = record.as_object_mut().expect("the identity is an object");
            fields.insert("workload".to_string(), json!(workload.name()));
            fields.insert("supported".to_string(), json!(false));
            fields.insert("reason".to_string(), json!(reason));
            println!("pqx: {} is not supported: {reason}", workload.name());
            run.append("results", record);
            continue;
        }
        for size in &sizes {
            let record = measure_configuration(&harness, workload, *size).await;
            run.append("results", record);
        }
    }
}

/// One verification of the adapter's scheme, and batches of them on one
/// thread and on several. This times the verifier alone: it is context for
/// the lifecycle numbers, not a substitute for them.
#[test]
#[ignore = "measurement: run in release, alone, with PQX_OUT set"]
fn verification_microbenchmark() {
    let _alone = exclusive();
    require_release_build();
    let samples = env_usize("PQX_VERIFY_SAMPLES", 1000);
    let batch = env_usize("PQX_VERIFY_BATCH", 256);
    let distinct = env_usize("PQX_VERIFY_DISTINCT", 16).max(1);
    let batch_iterations = env_usize("PQX_VERIFY_BATCH_ITERATIONS", 30);
    let most_threads = env_usize("PQX_VERIFY_THREADS", available_threads()).max(1);
    let run = Run::start(json!({
        "verify_samples": samples,
        "verify_batch": batch,
        "verify_distinct_signatures": distinct,
        "verify_batch_iterations": batch_iterations,
        "verify_threads": most_threads,
    }))
    .expect("PQX_OUT names the directory the results are written to");

    let signer = key(0);
    let messages: Vec<H256> = (0..distinct as u64)
        .map(|index| {
            let mut input = b"pqx verification ".to_vec();
            input.extend_from_slice(&index.to_be_bytes());
            keccak(input)
        })
        .collect();
    let signed: Vec<(H256, Vec<u8>)> = parallel_map(&messages, |message| {
        (*message, signature(&signer, *message))
    });
    let verify_one = |index: usize| {
        let (message, signature) = &signed[index % signed.len()];
        std::hint::black_box(adapter::verify(
            std::hint::black_box(&signer.public),
            std::hint::black_box(message.as_fixed_bytes()),
            std::hint::black_box(signature),
        ))
    };

    // A signature with one bit changed must not verify, or the numbers below
    // would time something other than a verification.
    let (message, valid) = &signed[0];
    let mut altered = valid.clone();
    altered[valid.len() / 2] ^= 0x01;
    assert!(!adapter::verify(
        &signer.public,
        message.as_fixed_bytes(),
        &altered
    ));

    for index in 0..samples.min(100) {
        assert!(verify_one(index));
    }
    let mut single = Vec::with_capacity(samples);
    for index in 0..samples {
        let started = Instant::now();
        let valid = verify_one(index);
        single.push(started.elapsed());
        assert!(valid);
    }

    let mut batches = Vec::new();
    let mut thread_counts = vec![1];
    if most_threads > 1 {
        thread_counts.push(most_threads);
    }
    for threads in thread_counts {
        let indices: Vec<usize> = (0..batch).collect();
        let chunk = batch.div_ceil(threads).max(1);
        let mut timings = Vec::with_capacity(batch_iterations);
        for _ in 0..batch_iterations {
            let verify_one = &verify_one;
            let started = Instant::now();
            let valid = std::thread::scope(|scope| {
                let handles: Vec<_> = indices
                    .chunks(chunk)
                    .map(|part| scope.spawn(move || part.iter().all(|index| verify_one(*index))))
                    .collect();
                handles
                    .into_iter()
                    .all(|handle| handle.join().expect("a verification thread panicked"))
            });
            timings.push(started.elapsed());
            assert!(valid);
        }
        batches.push(json!({
            "threads": threads,
            "signatures_per_batch": batch,
            "batch_ns": duration_statistics(&timings),
        }));
    }

    let mut record = identity();
    let fields = record.as_object_mut().expect("the identity is an object");
    fields.insert("kind".to_string(), json!("verification micro-benchmark"));
    fields.insert(
        "note".to_string(),
        json!("times the verifier function alone, outside the client's lifecycle"),
    );
    fields.insert("public_key_bytes".to_string(), json!(signer.public.len()));
    fields.insert("signature_bytes".to_string(), json!(signed[0].1.len()));
    fields.insert(
        "single_verification_ns".to_string(),
        duration_statistics(&single),
    );
    fields.insert("batches".to_string(), json!(batches));
    fields.insert("threads".to_string(), threads());
    run.append("microbench", record);
    println!(
        "pqx: {} verification micro-benchmark written",
        adapter::SIGNATURE_SCHEME
    );
}
