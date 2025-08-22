use std::time::{Duration, Instant};

use clap::Parser;
use ethrex::{initializers::load_store, utils::{default_datadir, init_datadir}};
use tracing::info;
use tracing_subscriber::FmtSubscriber;

const MIN_BLOCKS_TO_KEEP: u64 = 128;

fn mseconds_to_readable(mut mseconds: u128) -> String {
    const DAY: u128 = 24 * HOUR;
    const HOUR: u128 = 60 * MINUTE;
    const MINUTE: u128 = 60 * SECOND;
    const SECOND: u128 = 1000 * MSECOND;
    const MSECOND: u128 = 1;
    let mut res = String::new();
    let mut apply_time_unit = |unit_in_ms: u128, unit_str: &str| {
        if mseconds > unit_in_ms {
            let amount_of_unit = mseconds / unit_in_ms;
            res.push_str(&format!("{amount_of_unit}{unit_str}"));
            mseconds -= unit_in_ms * amount_of_unit
        }
    };
    apply_time_unit(DAY, "d");
    apply_time_unit(HOUR, "h");
    apply_time_unit(MINUTE, "m");
    apply_time_unit(SECOND, "s");
    apply_time_unit(MSECOND, "ms");

    res
}

#[derive(Parser)]
struct Args {
    #[arg(
        long = "blocks-to-keep",
        value_name = "NUMBER",
        help = "Amount of blocks to keep",
        long_help = "Cannot be smaller than 128",
        default_value_t = MIN_BLOCKS_TO_KEEP,
    )]
    blocks_to_keep: u64,
    #[arg(
        long = "datadir",
        value_name = "DATABASE_DIRECTORY",
        default_value_t = default_datadir(),
        help = "Receives the name of the directory where the Database is located.",
        long_help = "If the datadir is the word `memory`, ethrex will use the `InMemory Engine`.",
        env = "ETHREX_DATADIR"
    )]
    pub datadir: String,
}

#[tokio::main]
pub async fn main() -> eyre::Result<()> {
    let args = Args::parse();
    tracing::subscriber::set_global_default(FmtSubscriber::new())
        .expect("setting default subscriber failed");
    if args.blocks_to_keep < MIN_BLOCKS_TO_KEEP {
        return Err(eyre::ErrReport::msg(
            format!("Must keep at least {MIN_BLOCKS_TO_KEEP} in store"),
        ));
    }
    let data_dir = init_datadir(&args.datadir);
    let store = load_store(&data_dir).await;
    let latest_number = store.get_latest_block_number().await?;
    if latest_number <= args.blocks_to_keep {
        return Err(eyre::ErrReport::msg(
            format!("Only have {latest_number} blocks in store, cannot prune"),
        ));
    }
    let last_block_to_prune = latest_number - args.blocks_to_keep;
    let prune_start = Instant::now();
    let mut last_show_progress = Instant::now();
    const SHOW_PROGRESS_INTERVAL: Duration = Duration::from_secs(5);
    for block_number in 0..last_block_to_prune {
        if last_show_progress.elapsed() > SHOW_PROGRESS_INTERVAL {
            last_show_progress = Instant::now();
            info!("Pruned {block_number} blocks, {}% done", (block_number * 100) / last_block_to_prune)
        }
        store.purge_block(block_number).await?;
    }
    info!("Succesfully purged {last_block_to_prune} blocks in {}", mseconds_to_readable(prune_start.elapsed().as_millis()));
    Ok(())
}
