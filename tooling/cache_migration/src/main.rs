use clap::Parser;
use ethrex_replay::cache::{load_cache, Cache};
use ethrex_replay::run::get_input;
use eyre::{Result, WrapErr};
use rkyv::rancor::Error;
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::PathBuf;
use zkvm_interface::io::ProgramInput;

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
    // // Initialize logging
    // let subscriber = FmtSubscriber::builder()
    //     .with_max_level(Level::INFO)
    //     .finish();
    // tracing::subscriber::set_global_default(subscriber)?;

    // // Parse command-line arguments
    // let args = Args::parse();

    // // Read the input JSON file
    // info!("Reading cache from {}", args.input.display());
    // let file = File::open(&args.input)
    //     .wrap_err_with(|| format!("Failed to open input file: {}", args.input.display()))?;
    // let reader = BufReader::new(file);

    // // Deserialize from JSON
    // let cache: Cache =
    //     serde_json::from_reader(reader).wrap_err("Failed to parse JSON cache file")?;

    // // Serialize with rkyv
    // info!("Serializing to rkyv format");
    // let mut serializer = AllocSerializer::<0>::default();
    // serializer
    //     .serialize_value(&cache)
    //     .wrap_err("Failed to serialize cache to rkyv format")?;
    // let bytes = serializer.into_serializer().into_inner();

    // // Write to output file
    // info!(
    //     "Writing rkyv data to {} ({} bytes)",
    //     args.output.display(),
    //     bytes.len()
    // );
    // let mut file = File::create(&args.output)
    //     .wrap_err_with(|| format!("Failed to create output file: {}", args.output.display()))?;
    // file.write_all(&bytes)
    //     .wrap_err("Failed to write rkyv data to output file")?;

    // info!("Migration complete");
    // Ok(())
    let cache = load_cache("cache_mainnet_23097991.json")?;
    let input = get_input(cache)?;
    let bytes = rkyv::to_bytes::<Error>(&input)?;
    // write it as raw bytes to cache.bin
    let mut file = File::create("cache.bin")?;
    file.write_all(&bytes)?;
    println!("Serialized input size: {} bytes", bytes.len());
    Ok(())
}
