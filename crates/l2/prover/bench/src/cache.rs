use std::{
    fs::File,
    io::{BufReader, BufWriter},
};

use zkvm_interface::io::ProgramInput;

pub fn load_cache(block_number: usize) -> Result<ProgramInput, String> {
    let file_name = format!("cache_{}.json", block_number);
    let file = BufReader::new(File::open(file_name).map_err(|err| err.to_string())?);
    serde_json::from_reader(file).map_err(|err| err.to_string())
}

pub fn write_cache(cache: &ProgramInput) -> Result<(), String> {
    let file_name = format!("cache_{}.json", cache.block.header.number);
    let file = BufWriter::new(File::create(file_name).map_err(|err| err.to_string())?);
    serde_json::to_writer(file, cache).map_err(|err| err.to_string())
}
