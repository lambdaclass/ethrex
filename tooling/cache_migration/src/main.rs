use clap::Parser;
use ethrex_replay::cache::{load_cache, Cache};
use ethrex_replay::run::get_input;
use eyre::{Result, WrapErr};
use rkyv::rancor::Error;
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::PathBuf;
use zkvm_interface::io::{JSONProgramInput, ProgramInput};

// Import the Cache type from ethrex-replay
// You might need to adjust this import based on your project structure

// #[derive(Parser, Debug)]
// #[command(
//     author,
//     version,
//     about,
//     long_about = "Converts cache JSON files to rkyv binary format"
// )]
// struct Args {
//     /// Path to the input JSON cache file
//     #[arg(short, long)]
//     input: PathBuf,

//     /// Path where to save the rkyv-serialized output
//     #[arg(short, long)]
//     output: PathBuf,
// }

fn main() -> Result<()> {
    let cache = load_cache("cache_mainnet_23218216.json")?;
    let input = get_input(cache)?;
    // let bytes = bincode::serialize(&input)?;
    // let bytes = rkyv::to_bytes::<Error>(&input)?;
    let mut file = File::create("cache.bin")?;
    file.write_all(&bytes)?;
    println!("Serialized input size: {} bytes", bytes.len());
    Ok(())
}
