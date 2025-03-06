use std::path::Path;

use clap::ArgMatches;
use tracing::warn;

use crate::{set_datadir, DEFAULT_DATADIR};

pub fn remove_db(matches: &ArgMatches) {
    let data_dir = matches
        .get_one::<String>("datadir")
        .map_or(set_datadir(DEFAULT_DATADIR), |datadir| set_datadir(datadir));
    remove_db_file(&data_dir);
}

pub fn remove_db_file(data_dir: &String) {
    let path = Path::new(data_dir);
    if path.exists() {
        std::fs::remove_dir_all(path).expect("Failed to remove data directory");
    } else {
        warn!("Data directory does not exist: {}", data_dir);
    }
}
