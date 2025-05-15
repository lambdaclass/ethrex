use ethrex_common::types::Genesis;
use std::fs::{self, read_dir};
pub fn main() {
    let genesis_files = read_dir("../test_data").unwrap();
    for file in genesis_files {
        let file = file.unwrap();
        let path = file.path();
        let file_name = path.file_name().unwrap();
        let is_genesis_file = file_name.to_string_lossy().contains("genesis")
            && file_name.to_string_lossy().contains(".json");
        if is_genesis_file {
            println!(
                "Formating genesis file: {}",
                path.file_name().unwrap().to_string_lossy()
            );
            let genesis_file = fs::read(&path).unwrap();
            let current_genesis: Genesis = serde_json::from_slice(&genesis_file).expect(&format!(
                "File {} is not a valid genesis json",
                path.to_string_lossy()
            ));
            current_genesis.write_as_json(&path).unwrap();
        }
    }
}
