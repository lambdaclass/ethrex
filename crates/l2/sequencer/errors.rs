use crate::based::block_fetcher::BlockFetcherError;
use crate::sequencer::admin_server::AdminError;
use crate::sequencer::state_updater::StateUpdaterError;
use crate::utils::error::UtilsError;
use aligned_sdk::common::errors::SubmitError;
use ethereum_types::FromStrRadixErr;
use ethrex_blockchain::error::{ChainError, InvalidBlockError, InvalidForkChoice};
use ethrex_common::Address;
use ethrex_common::types::{BlobsBundleError, FakeExponentialError};
use ethrex_l2_common::privileged_transactions::PrivilegedTransactionError;
use ethrex_l2_common::prover::ProverType;
use ethrex_l2_rpc::signer::SignerError;
use ethrex_metrics::MetricsError;
use ethrex_rpc::clients::EngineClientError;
use ethrex_rpc::clients::eth::errors::{CalldataEncodeError, EthClientError};
use ethrex_storage::error::StoreError;
use ethrex_storage_rollup::RollupStoreError;
use ethrex_vm::EvmError;
use spawned_concurrency::error::GenServerError;
use tokio::task::JoinError;

#[derive(Debug, thiserror::Error)]
pub enum SequencerError {
    #[error("Failed to start L1Watcher: {0}")]
    L1WatcherError(#[from] L1WatcherError),
    #[error("Failed to start ProofCoordinator: {0}")]
    ProofCoordinatorError(#[from] ProofCoordinatorError),
    #[error("Failed to start BlockProducer: {0}")]
    BlockProducerError(#[from] BlockProducerError),
    #[error("Failed to start Committer: {0}")]
    CommitterError(#[from] CommitterError),
    #[error("Failed to start ProofSender: {0}")]
    ProofSenderError(#[from] ProofSenderError),
    #[error("Failed to start ProofVerifier: {0}")]
    ProofVerifierError(#[from] ProofVerifierError),
    #[error("Failed to start MetricsGatherer: {0}")]
    MetricsGathererError(#[from] MetricsGathererError),
    #[error("Sequencer error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("Failed to start StateUpdater: {0}")]
    StateUpdaterError(#[from] StateUpdaterError),
    #[error("Failed to start BlockFetcher: {0}")]
    BlockFetcherError(#[from] BlockFetcherError),
    #[error("Failed to access Store: {0}")]
    FailedAccessingStore(#[from] StoreError),
    #[error("Failed to access RollupStore: {0}")]
    FailedAccessingRollUpStore(#[from] RollupStoreError),
    #[error("Failed to resolve network")]
    AlignedNetworkError(String),
    #[error("Failed to start EthrexMonitor: {0}")]
    MonitorError(#[from] MonitorError),
    #[error("Failed to start admin api: {0}")]
    Admin(#[from] AdminError),
    #[error("Block gas limit cannot be greater than batch gas limit")]
    GasLimitError,
}

#[derive(Debug, thiserror::Error)]
pub enum L1WatcherError {
    #[error("L1Watcher error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("L1Watcher failed to deserialize log: {0}")]
    FailedToDeserializeLog(String),
    #[error("L1Watcher failed to parse private key: {0}")]
    FailedToDeserializePrivateKey(String),
    #[error("L1Watcher failed to retrieve chain config: {0}")]
    FailedToRetrieveChainConfig(String),
    #[error("L1Watcher failed to access Store: {0}")]
    FailedAccessingStore(#[from] StoreError),
    #[error("L1Watcher failed to access RollupStore: {0}")]
    FailedAccessingRollUpStore(#[from] RollupStoreError),
    #[error("{0}")]
    Custom(String),
    #[error("Internal Error: {0}")]
    InternalError(#[from] GenServerError),
}

#[derive(Debug, thiserror::Error)]
pub enum ProofCoordinatorError {
    #[error("ProofCoordinator connection failed: {0}")]
    ConnectionError(#[from] std::io::Error),
    #[error("ProofCoordinator failed because of an EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("ProofCoordinator failed to send transaction: {0}")]
    FailedToVerifyProofOnChain(String),
    #[error("ProofCoordinator failed to access Store: {0}")]
    FailedAccessingStore(#[from] StoreError),
    #[error("ProverServer failed to access RollupStore: {0}")]
    FailedAccessingRollupStore(#[from] RollupStoreError),
    #[error("ProofCoordinator failed to retrieve block from storaga, data is None.")]
    StorageDataIsNone,
    #[error("ProofCoordinator failed to create ProverInputs: {0}")]
    FailedToCreateProverInputs(#[from] EvmError),
    #[error("ProofCoordinator failed to create ExecutionWitness: {0}")]
    FailedToCreateExecutionWitness(#[from] ChainError),
    #[error("ProofCoordinator JoinError: {0}")]
    JoinError(#[from] JoinError),
    #[error("ProofCoordinator failed: {0}")]
    Custom(String),
    #[error("ProofCoordinator failed to write to TcpStream: {0}")]
    WriteError(String),
    #[error("ProofCoordinator failed to get data from Store: {0}")]
    ItemNotFoundInStore(String),
    #[error("Failed to encode calldata: {0}")]
    CalldataEncodeError(#[from] CalldataEncodeError),
    #[error("Unexpected Error: {0}")]
    InternalError(String),
    #[error("ProofCoordinator failed when (de)serializing JSON: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("ProofCoordinator encountered a ExecutionCacheError")]
    ExecutionCacheError(#[from] ExecutionCacheError),
    #[error("ProofCoordinator encountered a BlobsBundleError: {0}")]
    BlobsBundleError(#[from] ethrex_common::types::BlobsBundleError),
    #[error("Failed to execute command: {0}")]
    ComandError(std::io::Error),
    #[error("Missing blob for batch {0}")]
    MissingBlob(u64),
    #[error("Missing TDX private key")]
    MissingTDXPrivateKey,
    #[error("Metrics error")]
    Metrics(#[from] MetricsError),
    #[error("Missing prover input for batch {0} (version {1})")]
    MissingBatchProverInput(u64, String),
}

#[derive(Debug, thiserror::Error)]
pub enum ProofSenderError {
    #[error("Failed because of an EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("Failed to encode calldata: {0}")]
    CalldataEncodeError(#[from] CalldataEncodeError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0} proof is not present")]
    ProofNotPresent(ProverType),
    #[error("Unexpected Error: {0}")]
    UnexpectedError(String),
    #[error("Failed to parse OnChainProposer response: {0}")]
    FailedToParseOnChainProposerResponse(String),
    #[error("Internal Error: {0}")]
    InternalError(#[from] GenServerError),
    #[error("Proof Sender failed because of a rollup store error: {0}")]
    RollUpStoreError(#[from] RollupStoreError),
    #[error("Proof Sender failed to estimate Aligned fee: {0}")]
    AlignedFeeEstimateError(String),
    #[error("Proof Sender failed to get nonce from batcher: {0}")]
    AlignedGetNonceError(String),
    #[error("Proof Sender failed to submit proof(s): {0}")]
    AlignedSubmitProofError(Box<SubmitError>),
    #[error("Wrong batch proof format; should be compressed but found groth16 instead")]
    AlignedWrongProofFormat,
    #[error("Metrics error")]
    Metrics(#[from] MetricsError),
    #[error("Failed to convert integer")]
    TryIntoError(#[from] std::num::TryFromIntError),
}

impl From<SubmitError> for ProofSenderError {
    fn from(value: SubmitError) -> Self {
        ProofSenderError::AlignedSubmitProofError(value.into())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProofVerifierError {
    #[error("Failed because of an EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("Unexpected Error: {0}")]
    InternalError(String),
    #[error("ProofVerifier failed to parse beacon url")]
    ParseBeaconUrl(String),
    #[error("Failed with a SaveStateError: {0}")]
    CalldataEncodeError(#[from] CalldataEncodeError),
    #[error("Store error: {0}")]
    StoreError(#[from] StoreError),
    #[error("Block Producer failed because of a rollup store error: {0}")]
    RollupStoreError(#[from] RollupStoreError),
    #[error("Aligned does not support prover type {0}")]
    UnsupportedProverType(String),
    #[error(
        "Mismatched public inputs for batch {batch_number} and prover type {prover_type}. Existing: {existing_hex}, latest: {latest_hex}"
    )]
    MismatchedPublicInputs {
        batch_number: u64,
        prover_type: ProverType,
        existing_hex: String,
        latest_hex: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum BlockProducerError {
    #[error("Block Producer failed because of an EngineClient error: {0}")]
    EngineClientError(#[from] EngineClientError),
    #[error("Block Producer failed because of a ChainError error: {0}")]
    ChainError(#[from] ChainError),
    #[error("Block Producer failed because of a EvmError error: {0}")]
    EvmError(#[from] EvmError),
    #[error("Block Producer failed because of a InvalidForkChoice error: {0}")]
    InvalidForkChoice(#[from] InvalidForkChoice),
    #[error("Block Producer failed to produce block: {0}")]
    FailedToProduceBlock(String),
    #[error("Block Producer failed to prepare PayloadAttributes timestamp: {0}")]
    FailedToGetSystemTime(#[from] std::time::SystemTimeError),
    #[error("Block Producer failed because of a store error: {0}")]
    StoreError(#[from] StoreError),
    #[error("Block Producer failed because of a rollup store error: {0}")]
    RollupStoreError(#[from] RollupStoreError),
    #[error("Block Producer failed retrieve block from storage, data is None.")]
    StorageDataIsNone,
    #[error("Block Producer failed to read jwt_secret: {0}")]
    FailedToReadJWT(#[from] std::io::Error),
    #[error("Block Producer failed to decode jwt_secret: {0}")]
    FailedToDecodeJWT(#[from] hex::FromHexError),
    #[error("Block Producer failed because of an execution cache error")]
    ExecutionCache(#[from] ExecutionCacheError),
    #[error("Block Producer failed to convert values: {0}")]
    TryIntoError(#[from] std::num::TryFromIntError),
    #[error("{0}")]
    Custom(String),
    #[error("Failed to parse withdrawal: {0}")]
    FailedToParseWithdrawal(#[from] UtilsError),
    #[error("Failed to get data from: {0}")]
    FailedToGetDataFrom(String),
    #[error("Internal Error: {0}")]
    InternalError(#[from] GenServerError),
    #[error("EthClientError error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("Failed to encode calldata: {0}")]
    CalldataEncodeError(#[from] CalldataEncodeError),
}

impl From<InvalidBlockError> for BlockProducerError {
    fn from(err: InvalidBlockError) -> Self {
        BlockProducerError::ChainError(ChainError::from(err))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CommitterError {
    #[error("Committer failed because of an EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("Committer failed to  {0}")]
    FailedToParseLastCommittedBlock(#[from] FromStrRadixErr),
    #[error("Committer Store Error: {0}")]
    StoreError(#[from] StoreError),
    #[error("Committer failed retrieve block from rollup storage: {0}")]
    RollupStoreError(#[from] RollupStoreError),
    #[error("Committer failed because of an execution cache error")]
    ExecutionCache(#[from] ExecutionCacheError),
    #[error("Committer failed retrieve data from storage")]
    FailedToRetrieveDataFromStorage,
    #[error("Committer failed to generate blobs bundle: {0}")]
    FailedToGenerateBlobsBundle(#[from] BlobsBundleError),
    #[error("Committer failed to get information from storage: {0}")]
    FailedToGetInformationFromStorage(String),
    #[error("Committer failed to open Points file: {0}")]
    FailedToOpenPointsFile(#[from] std::io::Error),
    #[error("Committer failed to re-execute block: {0}")]
    FailedToReExecuteBlock(#[from] EvmError),
    #[error("Committer failed to send transaction: {0}")]
    FailedToSendCommitment(String),
    #[error("Committer failed to decode deposit hash")]
    FailedToDecodeDepositHash,
    #[error("Withdrawal transaction was invalid")]
    InvalidWithdrawalTransaction,
    #[error("Blob estimation failed: {0}")]
    BlobEstimationError(#[from] BlobEstimationError),
    #[error("Failed to convert integer")]
    TryIntoError(#[from] std::num::TryFromIntError),
    #[error("Failed to encode calldata: {0}")]
    CalldataEncodeError(#[from] CalldataEncodeError),
    #[error("Failed to get withdrawals: {0}")]
    FailedToGetWithdrawals(#[from] UtilsError),
    #[error("Failed to sign error: {0}")]
    FailedToSignError(#[from] SignerError),
    #[error("Privileged Transaction error: {0}")]
    PrivilegedTransactionError(#[from] PrivilegedTransactionError),
    #[error("Metrics error")]
    Metrics(#[from] MetricsError),
    #[error("Internal Error: {0}")]
    InternalError(#[from] GenServerError),
    #[error("Retrieval Error: {0}")]
    RetrievalError(String),
    #[error("Conversion Error: {0}")]
    ConversionError(String),
    #[error("Unexpected Error: {0}")]
    UnexpectedError(String),
    #[error("Unreachable code reached: {0}")]
    Unreachable(String),
    #[error("Failed to generate batch witness: {0}")]
    FailedToGenerateBatchWitness(#[source] ChainError),
    #[error("Missing blob for batch {0}")]
    MissingBlob(u64),
    #[error("Failed to create checkpoint: {0}")]
    FailedToCreateCheckpoint(String),
    #[error("Failed to process blobs: {0}")]
    ChainError(#[from] ChainError),
    #[error("Failed due to invalid fork choice: {0}")]
    InvalidForkChoice(#[from] InvalidForkChoice),
    #[error("Privileged transaction hash could not be computed")]
    InvalidPrivilegedTransaction,
}

#[derive(Debug, thiserror::Error)]
pub enum BlobEstimationError {
    #[error("Overflow error while estimating blob gas")]
    OverflowError,
    #[error("Failed to calculate blob gas due to invalid parameters")]
    CalculationError,
    #[error(
        "Blob gas estimation resulted in an infinite or undefined value. Outside valid or expected ranges"
    )]
    NonFiniteResult,
    #[error("{0}")]
    FakeExponentialError(#[from] FakeExponentialError),
}

#[derive(Debug, thiserror::Error)]
pub enum MetricsGathererError {
    #[error("MetricsGathererError: {0}")]
    MetricsError(#[from] ethrex_metrics::MetricsError),
    #[error("MetricsGatherer failed because of an EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("MetricsGatherer: {0}")]
    TryInto(String),
    #[error("Internal Error: {0}")]
    InternalError(#[from] GenServerError),
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutionCacheError {
    #[error("Failed because of io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed (de)serializing result: {0}")]
    Bincode(#[from] bincode::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionHandlerError {
    #[error("Internal Error: {0}")]
    InternalError(#[from] GenServerError),
}

#[derive(Debug, thiserror::Error)]
pub enum MonitorError {
    #[error("Failed because of io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to fetch {0:?} logs from {1}, {2}")]
    LogsSignatures(Vec<String>, Address, #[source] EthClientError),
    #[error("Failed to get batch by number {0}: {1}")]
    GetBatchByNumber(u64, #[source] RollupStoreError),
    #[error("Failed to get blocks by batch number {0}: {1}")]
    GetBlocksByBatch(u64, #[source] RollupStoreError),
    #[error("Batch {0} not found in the rollup store")]
    BatchNotFound(u64),
    #[error("Failed to get block by number {0}, {1}")]
    GetBlockByNumber(u64, #[source] StoreError),
    #[error("Block {0} not found in the store")]
    BlockNotFound(u64),
    #[error("Internal Error: {0}")]
    InternalError(#[from] GenServerError),
    #[error("Failed to get logs topics {0}")]
    LogsTopics(usize),
    #[error("Failed to get logs data from {0}")]
    LogsData(usize),
    #[error("Failed to get area chunks")]
    Chunks,
    #[error("Failed to get latest block")]
    GetLatestBlock,
    #[error("Failed to get latest batch")]
    GetLatestBatch,
    #[error("Failed to get latest verified batch")]
    GetLatestVerifiedBatch,
    #[error("Failed to get commited batch")]
    GetLatestCommittedBatch,
    #[error("Failed to get last L1 block fetched")]
    GetLastFetchedL1,
    #[error("Failed to get pending privileged transactions")]
    GetPendingPrivilegedTx,
    #[error("Failed to get transaction pool")]
    TxPoolError,
    #[error("Failed to encode calldata: {0}")]
    CalldataEncodeError(#[from] CalldataEncodeError),
    #[error("Failed to parse privileged transaction")]
    PrivilegedTxParseError,
    #[error("Failure in rpc call: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("Failed to get receipt for transaction")]
    ReceiptError,
    #[error("Expected transaction to have logs")]
    NoLogs,
    #[error("Expected items in the table")]
    NoItemsInTable,
    #[error("RPC List can't be empty")]
    RPCListEmpty,
    #[error("Error converting batch window")]
    BatchWindow,
    #[error("Error while parsing private key")]
    DecodingError(String),
    #[error("Error parsing secret key")]
    FromHexError(#[from] hex::FromHexError),
}
