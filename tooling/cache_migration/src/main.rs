use clap::Parser;
use ethrex_replay::cache::load_cache;
use ethrex_replay::run::get_input;
use eyre::{Result, WrapErr};
use rkyv::rancor::Error;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(
    name = "cache_migration",
    about,
    long_about = "Converts cache JSON files to rkyv binary format"
)]
struct Args {
    /// Path to directory containing cache_*.json files
    #[arg(short = 'd', long, default_value = ".")]
    directory: PathBuf,
}

fn main() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let args = Args::parse();
    let dir_path = &args.directory;

    if !dir_path.is_dir() {
        return Err(eyre::eyre!(
            "Provided path is not a directory: {}",
            dir_path.display()
        ));
    }

    // Find all cache_*.json files in the directory
    let entries = fs::read_dir(dir_path)?;
    let mut processed_count = 0;
    let mut failed_count = 0;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                if filename.starts_with("cache_") && filename.ends_with(".json") {
                    if let Err(e) = process_cache_file(&path) {
                        error!("Failed to process -- skipping {}: {}", path.display(), e);
                        failed_count += 1;
                        continue;
                    }
                    processed_count += 1;
                }
            }
        }
    }

    info!("Completed processing {} cache files", processed_count);
    info!("Failed to process {} cache files", failed_count);
    if processed_count == 0 {
        warn!("No cache_*.json files found in {}", dir_path.display());
    }

    Ok(())
}

fn process_cache_file(json_path: &Path) -> Result<()> {
    let filename = json_path
        .file_name()
        .and_then(|f| f.to_str())
        .ok_or_else(|| eyre::eyre!("Invalid filename"))?;

    let bin_filename = filename.replace(".json", ".bin");
    let bin_path = json_path.with_file_name(bin_filename);

    info!(
        "Processing {} -> {}",
        json_path.display(),
        bin_path.display()
    );

    let cache = load_cache(json_path.to_str().unwrap())
        .wrap_err_with(|| format!("Failed to load cache from: {}", json_path.display()))?;

    let input = get_input(cache)
        .wrap_err_with(|| "Failed to convert cache to program input".to_string())?;

    let bytes = rkyv::to_bytes::<Error>(&input).wrap_err("Failed to serialize with rkyv")?;

    let mut file = File::create(&bin_path)
        .wrap_err_with(|| format!("Failed to create output file: {}", bin_path.display()))?;
    file.write_all(&bytes)
        .wrap_err("Failed to write binary data")?;

    info!(
        "Serialized input {} size: {} bytes",
        json_path.display(),
        bytes.len()
    );

    Ok(())
}
