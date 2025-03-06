use std::{env::temp_dir, fs::File, path::PathBuf};

use ethrex_common::types::BlockHash;
use ethrex_storage::AccountUpdate;

use super::errors::ExecutionCacheError;

/// For now the result will only be account updates, in the future we can add other parameters as
/// they're needed.
pub type ExecutionResult = Vec<AccountUpdate>;

/// Proposer will push execution results into the cache so other components can retrieve them,
/// without having to re-execute. The cache is implemented with temporary files.
pub struct ExecutionCache {
    tempdir: PathBuf,
}

impl Default for ExecutionCache {
    fn default() -> Self {
        Self {
            tempdir: temp_dir(),
        }
    }
}

impl ExecutionCache {
    pub fn push(
        &self,
        block_hash: BlockHash,
        execution_result: ExecutionResult,
    ) -> Result<(), ExecutionCacheError> {
        let filename = format!("result_{block_hash:x}.ethrex");
        let file = File::create(self.tempdir.join(filename))?;
        bincode::serialize_into(file, &execution_result).map_err(ExecutionCacheError::from)
    }

    pub fn get(
        &self,
        block_hash: BlockHash,
    ) -> Result<Option<ExecutionResult>, ExecutionCacheError> {
        let filename = format!("result_{block_hash:x}.ethrex");
        File::open(self.tempdir.join(filename))
            .ok()
            .map(|file| bincode::deserialize_from(file).map_err(ExecutionCacheError::from))
            .transpose()
    }
}
