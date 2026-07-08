use serde::Serialize;

use crate::report::Report;

#[derive(Debug, Serialize)]
pub struct WorkloadDelta {
    pub name: String,
    pub baseline_total: u64,
    pub head_total: u64,
    pub pct: f64,
    pub regressed: bool,
}

#[derive(Debug, Serialize)]
pub struct CompareResult {
    pub deltas: Vec<WorkloadDelta>,
    pub any_regression: bool,
}

pub fn compare_reports(baseline: &Report, head: &Report, threshold_pct: f64) -> CompareResult {
    let mut deltas = Vec::new();
    let mut any_regression = false;
    for hw in &head.workloads {
        let Some(bw) = baseline.workloads.iter().find(|b| b.name == hw.name) else {
            continue;
        };
        let base = bw.air_cost.total;
        let cur = hw.air_cost.total;
        let pct = if base == 0 {
            0.0
        } else {
            (cur as f64 - base as f64) / base as f64 * 100.0
        };
        let regressed = pct > threshold_pct;
        any_regression |= regressed;
        deltas.push(WorkloadDelta {
            name: hw.name.clone(),
            baseline_total: base,
            head_total: cur,
            pct,
            regressed,
        });
    }
    CompareResult {
        deltas,
        any_regression,
    }
}

pub fn run_compare(
    baseline: &str,
    head: &str,
    threshold_pct: f64,
    out: Option<&str>,
) -> eyre::Result<i32> {
    let base = Report::read_json(baseline)?;
    let head = Report::read_json(head)?;
    let result = compare_reports(&base, &head, threshold_pct);
    for d in &result.deltas {
        let flag = if d.regressed { "REGRESSION" } else { "ok" };
        println!(
            "{:<40} {:>12} -> {:>12}  {:+.2}%  {}",
            d.name, d.baseline_total, d.head_total, d.pct, flag
        );
    }
    if let Some(path) = out {
        std::fs::write(path, serde_json::to_string_pretty(&result)?)?;
    }
    Ok(if result.any_regression { 1 } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{AirCost, Meta, Report, WorkloadResult};

    fn wl(name: &str, total: u64) -> WorkloadResult {
        WorkloadResult {
            name: name.into(),
            r#type: "micro".into(),
            category: None,
            gas: None,
            air_cost: AirCost {
                total,
                ..Default::default()
            },
            steps: total,
            zkvm_ram_bytes: 0,
            guest_output_ok: true,
        }
    }
    fn rep(items: Vec<WorkloadResult>) -> Report {
        Report {
            meta: Meta {
                zisk_version: "v0.16.1".into(),
                guest_elf_sha256: "x".into(),
                generated_by: "t".into(),
                git_commit: None,
            },
            workloads: items,
        }
    }

    #[test]
    fn flags_regression_over_threshold() {
        let base = rep(vec![wl("a", 100), wl("b", 100)]);
        let head = rep(vec![wl("a", 105), wl("b", 101)]); // a: +5%, b: +1%
        let res = compare_reports(&base, &head, 3.0);
        assert!(res.any_regression);
        let a = res.deltas.iter().find(|d| d.name == "a").unwrap();
        assert!(a.regressed);
        let b = res.deltas.iter().find(|d| d.name == "b").unwrap();
        assert!(!b.regressed);
    }

    #[test]
    fn no_regression_when_within_threshold() {
        let base = rep(vec![wl("a", 100)]);
        let head = rep(vec![wl("a", 101)]);
        assert!(!compare_reports(&base, &head, 3.0).any_regression);
    }
}
