use crate::utils::error::UtilsError;
use crate::utils::prover::errors::SaveStateError;
use crate::utils::prover::proving_systems::ProverType;
use ethereum_types::FromStrRadixErr;
use ethrex_blockchain::error::{ChainError, InvalidForkChoice};
use ethrex_common::types::{BlobsBundleError, FakeExponentialError};
use ethrex_l2_sdk::merkle_tree::MerkleError;
use ethrex_rpc::clients::eth::errors::{CalldataEncodeError, EthClientError};
use ethrex_rpc::clients::EngineClientError;
use ethrex_storage::error::StoreError;
use ethrex_trie::TrieError;
use ethrex_vm::EvmError;
use tokio::task::JoinError;

#[derive(Debug, thiserror::Error)]
pub enum SequencerError {
    #[error("Failed to start L1Watcher: {0}")]
    L1WatcherError(#[from] L1WatcherError),
    #[error("Failed to start ProverServer: {0}")]
    ProverServerError(#[from] ProverServerError),
    #[error("Failed to start BlockProducer: {0}")]
    BlockProducerError(#[from] BlockProducerError),
    #[error("Failed to start Committer: {0}")]
    CommitterError(#[from] CommitterError),
    #[error("Failed to start ProofSender: {0}")]
    ProofSenderError(#[from] ProofSenderError),
    #[error("Failed to start MetricsGatherer: {0}")]
    MetricsGathererError(#[from] MetricsGathererError),
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
    #[error("{0}")]
    Custom(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ProverServerError {
    #[error("ProverServer connection failed: {0}")]
    ConnectionError(#[from] std::io::Error),
    #[error("ProverServer failed because of an EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("ProverServer failed to send transaction: {0}")]
    FailedToVerifyProofOnChain(String),
    #[error("ProverServer failed to access Store: {0}")]
    FailedAccessingStore(#[from] StoreError),
    #[error("ProverServer failed to retrieve block from storaga, data is None.")]
    StorageDataIsNone,
    #[error("ProverServer failed to create ProverInputs: {0}")]
    FailedToCreateProverInputs(#[from] EvmError),
    #[error("ProverServer JoinError: {0}")]
    JoinError(#[from] JoinError),
    #[error("ProverServer failed: {0}")]
    Custom(String),
    #[error("ProverServer failed to write to TcpStream: {0}")]
    WriteError(String),
    #[error("ProverServer failed to get data from Store: {0}")]
    ItemNotFoundInStore(String),
    #[error("ProverServer encountered a SaveStateError: {0}")]
    SaveStateError(#[from] SaveStateError),
    #[error("Failed to encode calldata: {0}")]
    CalldataEncodeError(#[from] CalldataEncodeError),
    #[error("Unexpected Error: {0}")]
    InternalError(String),
    #[error("ProverServer failed when (de)serializing JSON: {0}")]
    JsonError(#[from] serde_json::Error),
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
}

#[derive(Debug, thiserror::Error)]
pub enum CommitterError {
    #[error("Committer failed because of an EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
    #[error("Committer failed to  {0}")]
    FailedToParseLastCommittedBlock(#[from] FromStrRadixErr),
    #[error("Committer failed retrieve block from storage: {0}")]
    StoreError(#[from] StoreError),
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
}

#[derive(Debug, thiserror::Error)]
pub enum BlobEstimationError {
    #[error("Overflow error while estimating blob gas")]
    OverflowError,
    #[error("Failed to calculate blob gas due to invalid parameters")]
    CalculationError,
    #[error("Blob gas estimation resulted in an infinite or undefined value. Outside valid or expected ranges")]
    NonFiniteResult,
    #[error("{0}")]
    FakeExponentialError(#[from] FakeExponentialError),
}

#[derive(Debug, thiserror::Error)]
pub enum StateDiffError {
    #[error("StateDiff failed to deserialize: {0}")]
    FailedToDeserializeStateDiff(String),
    #[error("StateDiff failed to serialize: {0}")]
    FailedToSerializeStateDiff(String),
    #[error("StateDiff invalid account state diff type: {0}")]
    InvalidAccountStateDiffType(u8),
    #[error("StateDiff unsupported version: {0}")]
    UnsupportedVersion(u8),
    #[error("Both bytecode and bytecode hash are set")]
    BytecodeAndBytecodeHashSet,
    #[error("Empty account diff")]
    EmptyAccountDiff,
    #[error("The length of the vector is too big to fit in u16: {0}")]
    LengthTooBig(#[from] core::num::TryFromIntError),
    #[error("DB Error: {0}")]
    DbError(#[from] TrieError),
    #[error("Store Error: {0}")]
    StoreError(#[from] StoreError),
    #[error("New nonce is lower than the previous one")]
    FailedToCalculateNonce,
}

#[derive(Debug, thiserror::Error)]
pub enum MetricsGathererError {
    #[error("MetricsGathererError: {0}")]
    MetricsError(#[from] ethrex_metrics::MetricsError),
    #[error("MetricsGatherer failed because of an EthClient error: {0}")]
    EthClientError(#[from] EthClientError),
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutionCacheError {
    #[error("Failed because of io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed (de)serializing result: {0}")]
    Bincode(#[from] bincode::Error),
}
