use serde::Serialize;

use ethrex_common::Address;
use ethrex_common::types::{Transaction, TxKind};
use ethrex_prover::backend::ZiskBackend;

use crate::cache::{cache_to_program_input, load_cache};

#[derive(Serialize)]
struct CurationRow {
    file: String,
    block: u64,
    size_bytes: u64,
    gas_used: u64,
    tx_count: usize,
    precompile_txs: usize,
    // Present only when `--ziskemu` is passed.
    #[serde(skip_serializing_if = "Option::is_none")]
    air_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    air_precompiles: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    air_memory: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    air_ram: Option<u64>,
}

/// True if `address` falls in the reserved precompile range `0x01..=0x0a`
/// (first 19 bytes zero, last byte in `1..=10`). Used only to flag
/// precompile-heavy blocks during curation, not as an exhaustive
/// fork-accurate precompile check.
fn is_precompile_address(address: &Address) -> bool {
    let bytes = address.as_bytes();
    bytes[..19].iter().all(|&b| b == 0) && (1..=10).contains(&bytes[19])
}

fn count_precompile_txs(transactions: &[Transaction]) -> usize {
    transactions
        .iter()
        .filter(|tx| matches!(tx.to(), TxKind::Call(addr) if is_precompile_address(&addr)))
        .count()
}

/// Builds one `CurationRow` for `path`, optionally running the ziskemu
/// profiled pass. Isolated in its own function so a single bad/incompatible
/// cache file can be logged and skipped by the caller instead of aborting
/// the whole directory scan.
fn curate_one(
    backend: &ZiskBackend,
    path: &std::path::Path,
    name: &str,
    ziskemu: bool,
) -> eyre::Result<CurationRow> {
    let size_bytes = std::fs::metadata(path)?.len();
    let path_str = path
        .to_str()
        .ok_or_else(|| eyre::eyre!("non-utf8 path: {}", path.display()))?;
    let cache = load_cache(path_str)?;
    let first_block = cache
        .blocks
        .first()
        .ok_or_else(|| eyre::eyre!("cache has no blocks"))?;
    let block = first_block.header.number;
    let gas_used = first_block.header.gas_used;
    let tx_count = first_block.body.transactions.len();
    let precompile_txs = count_precompile_txs(&first_block.body.transactions);

    let (air_total, air_precompiles, air_memory, air_ram) = if ziskemu {
        let input = cache_to_program_input(cache)?;
        match backend.execute_profiled(input) {
            Ok(z) => (
                Some(z.total),
                Some(z.precompiles),
                Some(z.memory),
                Some(z.ram_usage),
            ),
            Err(e) => {
                eprintln!("{name}: ziskemu execution failed: {e}");
                (None, None, None, None)
            }
        }
    } else {
        (None, None, None, None)
    };

    Ok(CurationRow {
        file: name.to_string(),
        block,
        size_bytes,
        gas_used,
        tx_count,
        precompile_txs,
        air_total,
        air_precompiles,
        air_memory,
        air_ram,
    })
}

/// Scans `cache_dir` for ethrex-replay `cache_mainnet_*` caches (both `.json`
/// and gzipped `.json.gz`; `load_cache` decodes either) and writes a per-block
/// metric table to `out` as pretty JSON. Non-mainnet caches (e.g. polygon/amoy)
/// are skipped. With `ziskemu`, also runs each block through
/// `ZiskBackend::execute_profiled` and records the AIR-cost breakdown.
///
/// Errors if the scan produces no rows, so pointing `curate` at a directory
/// with no matching caches fails loudly instead of writing an empty report.
pub fn run_curate(cache_dir: &str, out: &str, ziskemu: bool) -> eyre::Result<()> {
    let backend = ZiskBackend::new();
    let mut rows = Vec::new();
    let mut skipped = 0usize;

    for entry in std::fs::read_dir(cache_dir)? {
        let path = entry?.path();
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        if !name.starts_with("cache_mainnet_")
            || !(name.ends_with(".json") || name.ends_with(".json.gz"))
        {
            continue;
        }

        match curate_one(&backend, &path, &name, ziskemu) {
            Ok(row) => {
                println!("scanned block {}", row.block);
                rows.push(row);
            }
            Err(e) => {
                eprintln!("{name}: skipping (failed to scan): {e}");
                skipped += 1;
            }
        }
    }

    if rows.is_empty() {
        eyre::bail!(
            "no cache_mainnet_* caches scanned in {cache_dir} ({skipped} skipped); \
             expected ethrex-replay `cache_mainnet_*.json` or `.json.gz` files"
        );
    }

    std::fs::write(out, serde_json::to_string_pretty(&rows)?)?;
    println!("wrote {out} ({} rows, {skipped} skipped)", rows.len());
    Ok(())
}
