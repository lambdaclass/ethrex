//! Output formatters — markdown summary table and per-iteration CSV.

use crate::cli::{Transport, Workload};
use crate::stats;
use crate::workloads::IterationRecord;
use eyre::Result;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug)]
pub struct Summary {
    pub workload: Workload,
    pub transport: Transport,
    pub bytes_req: usize,
    pub bytes_resp: usize,
    pub ms_median: f64,
    pub ms_p99: f64,
    pub n: usize,
}

pub fn aggregate(records: &[IterationRecord]) -> Vec<Summary> {
    let mut groups: BTreeMap<(Workload, Transport), Vec<&IterationRecord>> = BTreeMap::new();
    for rec in records {
        groups
            .entry((rec.workload, rec.transport))
            .or_default()
            .push(rec);
    }

    let mut summaries = Vec::new();
    for ((workload, transport), recs) in groups {
        let bytes_req = recs[0].bytes_sent;
        let bytes_resp = recs[0].bytes_received;
        let mut times: Vec<u128> = recs.iter().map(|r| r.wall_time_us).collect();
        times.sort_unstable();
        let med = stats::median(&times) as f64 / 1000.0;
        let p99 = stats::p99(&times) as f64 / 1000.0;
        summaries.push(Summary {
            workload,
            transport,
            bytes_req,
            bytes_resp,
            ms_median: med,
            ms_p99: p99,
            n: recs.len(),
        });
    }
    summaries
}

pub fn print_markdown(summaries: &[Summary]) {
    println!();
    println!("| workload | transport | bytes_req | bytes_resp | ms_median | ms_p99 | n |");
    println!("|---|---|---|---|---|---|---|");
    for s in summaries {
        println!(
            "| {:?} | {:?} | {} | {} | {:.2} | {:.2} | {} |",
            s.workload, s.transport, s.bytes_req, s.bytes_resp, s.ms_median, s.ms_p99, s.n
        );
    }
    println!();
}

pub fn write_csv(records: &[IterationRecord], path: &Path) -> Result<()> {
    let mut wtr = csv::Writer::from_path(path)?;
    wtr.write_record([
        "workload",
        "transport",
        "iteration",
        "bytes_sent",
        "bytes_received",
        "wall_time_us",
        "http_status",
    ])?;
    for r in records {
        wtr.write_record([
            format!("{:?}", r.workload),
            format!("{:?}", r.transport),
            r.iteration.to_string(),
            r.bytes_sent.to_string(),
            r.bytes_received.to_string(),
            r.wall_time_us.to_string(),
            r.http_status.to_string(),
        ])?;
    }
    wtr.flush()?;
    Ok(())
}
