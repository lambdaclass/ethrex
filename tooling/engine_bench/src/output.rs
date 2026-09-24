//! Output formatters — markdown summary table and per-iteration CSV.

use crate::cli::{ForkArg, Transport, Workload};
use crate::stats;
use crate::workloads::IterationRecord;
use eyre::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug)]
pub struct Summary {
    pub fork: ForkArg,
    pub workload: Workload,
    pub blobs_version: Option<u8>,
    pub transport: Transport,
    /// Median request/response sizes (constant in a healthy run).
    pub bytes_req: u128,
    pub bytes_resp: u128,
    pub ms_min: f64,
    pub ms_median: f64,
    pub ms_p99: f64,
    /// Distinct HTTP statuses seen, e.g. "200" or "200,404". A mixed set means
    /// the server did not answer uniformly — treat timings with suspicion.
    pub statuses: String,
    /// Median hit count for nullable-list responses (blobs, bodies).
    pub hits: Option<u128>,
    pub n: usize,
}

type GroupKey = (ForkArg, Workload, Option<u8>, Transport);

pub fn aggregate(records: &[IterationRecord]) -> Vec<Summary> {
    let mut groups: BTreeMap<GroupKey, Vec<&IterationRecord>> = BTreeMap::new();
    for rec in records {
        groups
            .entry((rec.fork, rec.workload, rec.blobs_version, rec.transport))
            .or_default()
            .push(rec);
    }

    let mut summaries = Vec::new();
    for ((fork, workload, blobs_version, transport), recs) in groups {
        let mut times: Vec<u128> = recs.iter().map(|r| r.wall_time_us).collect();
        times.sort_unstable();
        let mut req: Vec<u128> = recs.iter().map(|r| r.bytes_sent as u128).collect();
        req.sort_unstable();
        let mut resp: Vec<u128> = recs.iter().map(|r| r.bytes_received as u128).collect();
        resp.sort_unstable();
        let statuses: BTreeSet<u16> = recs.iter().map(|r| r.http_status).collect();
        let statuses = statuses
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let mut hit_counts: Vec<u128> = recs
            .iter()
            .filter_map(|r| r.hits.map(|h| h as u128))
            .collect();
        hit_counts.sort_unstable();
        let hits = (!hit_counts.is_empty()).then(|| stats::median(&hit_counts));

        summaries.push(Summary {
            fork,
            workload,
            blobs_version,
            transport,
            bytes_req: stats::median(&req),
            bytes_resp: stats::median(&resp),
            ms_min: times[0] as f64 / 1000.0,
            ms_median: stats::median(&times) as f64 / 1000.0,
            ms_p99: stats::p99(&times) as f64 / 1000.0,
            statuses,
            hits,
            n: recs.len(),
        });
    }
    summaries
}

fn workload_label(workload: Workload, blobs_version: Option<u8>) -> String {
    match blobs_version {
        Some(v) => format!("{workload:?}(v{v})"),
        None => format!("{workload:?}"),
    }
}

pub fn print_markdown(summaries: &[Summary]) {
    println!();
    println!(
        "| fork | workload | transport | bytes_req | bytes_resp | ms_min | ms_median | ms_p99 | hits | status | n |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    for s in summaries {
        let hits = s.hits.map_or("-".to_string(), |h| h.to_string());
        println!(
            "| {} | {} | {:?} | {} | {} | {:.2} | {:.2} | {:.2} | {} | {} | {} |",
            s.fork.path(),
            workload_label(s.workload, s.blobs_version),
            s.transport,
            s.bytes_req,
            s.bytes_resp,
            s.ms_min,
            s.ms_median,
            s.ms_p99,
            hits,
            s.statuses,
            s.n
        );
    }
    println!();
}

pub fn write_csv(records: &[IterationRecord], path: &Path) -> Result<()> {
    let mut wtr = csv::Writer::from_path(path)?;
    wtr.write_record([
        "fork",
        "workload",
        "blobs_version",
        "transport",
        "iteration",
        "bytes_sent",
        "bytes_received",
        "wall_time_us",
        "http_status",
        "hits",
    ])?;
    for r in records {
        wtr.write_record([
            r.fork.path().to_string(),
            format!("{:?}", r.workload),
            r.blobs_version.map_or(String::new(), |v| v.to_string()),
            format!("{:?}", r.transport),
            r.iteration.to_string(),
            r.bytes_sent.to_string(),
            r.bytes_received.to_string(),
            r.wall_time_us.to_string(),
            r.http_status.to_string(),
            r.hits.map_or(String::new(), |h| h.to_string()),
        ])?;
    }
    wtr.flush()?;
    Ok(())
}
