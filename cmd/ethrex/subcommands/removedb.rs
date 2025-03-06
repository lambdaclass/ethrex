use std::path::Path;

use tracing::{info, warn};

pub fn remove_db(data_dir: &str) {
    let path = Path::new(data_dir);
    if path.exists() {
        std::fs::remove_dir_all(path).expect("Failed to remove data directory");
        info!("Succesfully removed database at {}", data_dir);
    } else {
        warn!("Data directory does not exist: {}", data_dir);
    }
}
