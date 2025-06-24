use crate::based::block_fetcher::BlockFetcherError;
use crate::based::state_updater::StateUpdaterError;
use crate::utils::error::UtilsError;
use crate::utils::prover::errors::SaveStateError;
use crate::utils::prover::proving_systems::ProverType;
use ethereum_types::FromStrRadixErr;
use ethrex_blockchain::error::{ChainError, InvalidForkChoice};
use ethrex_common::types::{BlobsBundleError, FakeExponentialError};
use ethrex_l2_common::deposits::DepositError;
use ethrex_l2_common::l1_messages::L1MessagingError;
use ethrex_l2_common::state_diff::StateDiffError;
use ethrex_l2_sdk::merkle_tree::MerkleError;
use ethrex_rpc::clients::EngineClientError;
use ethrex_rpc::clients::eth::errors::{CalldataEncodeError, EthClientError};
use ethrex_storage::error::StoreError;
use ethrex_storage_rollup::RollupStoreError;
use ethrex_vm::{EvmError, ProverDBError};
use spawned_concurrency::GenServerError;
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
    #[error("Spawned GenServer Error")]
    GenServerError(GenServerError),
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
    #[error("ProofCoordinator encountered a SaveStateError: {0}")]
    SaveStateError(#[from] SaveStateError),
    #[error("Failed to encode calldata: {0}")]
    CalldataEncodeError(#[from] CalldataEncodeError),
    #[error("Unexpected Error: {0}")]
    InternalError(String),
    #[error("ProofCoordinator failed when (de)serializing JSON: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("ProofCoordinator encountered a StateDiffError")]
    StateDiffError(#[from] StateDiffError),
    #[error("ProofCoordinator encountered a ExecutionCacheError")]
    ExecutionCacheError(#[from] ExecutionCacheError),
    #[error("ProofCoordinator encountered a BlobsBundleError: {0}")]
    BlobsBundleError(#[from] ethrex_common::types::BlobsBundleError),
    #[error("Failed to execute command: {0}")]
    ComandError(std::io::Error),
    #[error("ProofCoordinator failed failed because of a ProverDB error: {0}")]
    ProverDBError(#[from] ProverDBError),
    #[error("Missing blob for batch {0}")]
    MissingBlob(u64),
}

#[derive(Debug, thiserror::Error)]
pub enum ProofSenderError {
    #[error("Failed because of an EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("Failed to encode calldata: {0}")]
    CalldataEncodeError(#[from] CalldataEncodeError),
    #[error("Failed with a SaveStateError: {0}")]
    SaveStateError(#[from] SaveStateError),
    #[error("{0} proof is not present")]
    ProofNotPresent(ProverType),
    #[error("Unexpected Error: {0}")]
    InternalError(String),
    #[error("Failed to parse OnChainProposer response: {0}")]
    FailedToParseOnChainProposerResponse(String),
    #[error("Spawned GenServer Error")]
    GenServerError(GenServerError),
    #[error("Proof Sender failed because of a rollup store error: {0}")]
    RollUpStoreError(#[from] RollupStoreError),
    #[error("Proof Sender failed to estimate Aligned fee: {0}")]
    AlignedFeeEstimateError(String),
    #[error("Proof Sender failed to get nonce from batcher: {0}")]
    AlignedGetNonceError(String),
    #[error("Proof Sender failed to submit proof: {0}")]
    AlignedSubmitProofError(String),
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
    SaveStateError(#[from] SaveStateError),
    #[error("Failed to encode calldata: {0}")]
    CalldataEncodeError(#[from] CalldataEncodeError),
}

#[derive(Debug, thiserror::Error)]
pub enum BlockProducerError {
    #[error("Block Producer failed because of an EngineClient error: {0}")]
    EngineClientError(#[from] EngineClientError),
    #[error("Block Producer failed because of a ChainError error: {0}")]
    ChainError(#[from] ChainError),
    #[error("Block Producer failed because of a EvmError error: {0}")]
    EvmError(#[from] EvmError),
    #[error("Block Producer failed because of a ProverDB error: {0}")]
    ProverDBError(#[from] ProverDBError),
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
    #[error("Interval does not fit in u64")]
    TryIntoError(#[from] std::num::TryFromIntError),
    #[error("{0}")]
    Custom(String),
    #[error("Failed to parse withdrawal: {0}")]
    FailedToParseWithdrawal(#[from] UtilsError),
    #[error("Failed to encode AccountStateDiff: {0}")]
    FailedToEncodeAccountStateDiff(#[from] StateDiffError),
    #[error("Failed to get data from: {0}")]
    FailedToGetDataFrom(String),
    #[error("Spawned GenServer Error")]
    GenServerError(GenServerError),
}

#[derive(Debug, thiserror::Error)]
pub enum CommitterError {
    #[error("Committer failed because of an EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("Committer failed to  {0}")]
    FailedToParseLastCommittedBlock(#[from] FromStrRadixErr),
    #[error("Committer failed retrieve block from storage: {0}")]
    StoreError(#[from] StoreError),
    #[error("Committer failed retrieve block from rollup storage: {0}")]
    RollupStoreError(#[from] RollupStoreError),
    #[error("Committer failed because of an execution cache error")]
    ExecutionCache(#[from] ExecutionCacheError),
    #[error("Committer failed retrieve data from storage")]
    FailedToRetrieveDataFromStorage,
    #[error("Committer failed to generate blobs bundle: {0}")]
    FailedToGenerateBlobsBundle(#[from] BlobsBundleError),
    #[error("Committer failed to get information from storage")]
    FailedToGetInformationFromStorage(String),
    #[error("Committer failed to encode state diff: {0}")]
    FailedToEncodeStateDiff(#[from] StateDiffError),
    #[error("Committer failed to open Points file: {0}")]
    FailedToOpenPointsFile(#[from] std::io::Error),
    #[error("Committer failed to re-execute block: {0}")]
    FailedToReExecuteBlock(#[from] EvmError),
    #[error("Committer failed to send transaction: {0}")]
    FailedToSendCommitment(String),
    #[error("Committer failed to decode deposit hash")]
    FailedToDecodeDepositHash,
    #[error("Committer failed to merkelize: {0}")]
    FailedToMerkelize(#[from] MerkleError),
    #[error("Withdrawal transaction was invalid")]
    InvalidWithdrawalTransaction,
    #[error("Blob estimation failed: {0}")]
    BlobEstimationError(#[from] BlobEstimationError),
    #[error("length does not fit in u16")]
    TryIntoError(#[from] std::num::TryFromIntError),
    #[error("Failed to encode calldata: {0}")]
    CalldataEncodeError(#[from] CalldataEncodeError),
    #[error("Unexpected Error: {0}")]
    InternalError(String),
    #[error("Failed to get withdrawals: {0}")]
    FailedToGetWithdrawals(#[from] UtilsError),
    #[error("Deposit error: {0}")]
    DepositError(#[from] DepositError),
    #[error("L1Message error: {0}")]
    L1MessageError(#[from] L1MessagingError),
    #[error("Spawned GenServer Error")]
    GenServerError(GenServerError),
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
    #[error("Spawned GenServer Error")]
    GenServerError(GenServerError),
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
    #[error("Spawned GenServer Error")]
    GenServerError(GenServerError),
}
