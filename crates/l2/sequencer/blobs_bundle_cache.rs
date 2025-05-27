use std::{env::temp_dir, fs::File, path::PathBuf};

use bincode;
use ethrex_common::types::BlobsBundle;
use thiserror::Error;
use tracing::warn;

#[derive(Error, Debug)]
pub enum BlobsBundleCacheError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Bincode error: {0}")]
    Bincode(#[from] bincode::Error),
}

/// L1 committer will push blobs bundles into the cache so other components can retrieve them.
/// The cache is implemented with temporary files.
#[derive(Clone)]
pub struct BlobsBundleCache {
    tempdir: PathBuf,
}

impl Default for BlobsBundleCache {
    fn default() -> Self {
        Self {
            tempdir: temp_dir(),
        }
    }
}

impl BlobsBundleCache {
    pub fn push(
        &self,
        batch_number: u64,
        blobs_bundle: BlobsBundle,
    ) -> Result<(), BlobsBundleCacheError> {
        let filename = format!("blobs_bundle_batch_{batch_number}.ethrex");
        let file_path = self.tempdir.join(filename);
        let file = File::create(file_path)?;
        bincode::serialize_into(file, &blobs_bundle).map_err(BlobsBundleCacheError::from)
    }

    pub fn get(&self, batch_number: u64) -> Result<Option<BlobsBundle>, BlobsBundleCacheError> {
        let filename = format!("blobs_bundle_batch_{batch_number}.ethrex");
        File::open(self.tempdir.join(filename))
            .inspect_err(|err| warn!("{err}"))
            .ok()
            .map(|file| bincode::deserialize_from(file).map_err(BlobsBundleCacheError::from))
            .transpose()
    }
}
