use serde::Serialize;

use crate::report::Report;

#[derive(Debug, Serialize)]
pub struct WorkloadDelta {
    pub name: String,
    pub baseline_total: u64,
    pub head_total: u64,
    pub pct: f64,
    pub regressed: bool,
    /// Set when this delta is forced to `regressed` for a reason other than
    /// crossing `threshold_pct` on the AIR-cost delta: the head workload
    /// failed (`guest_output_ok == false`) or the workload is present in the
    /// baseline but missing entirely from the head report. `note` explains
    /// which.
    pub failed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
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
        // A failed head workload (guest_output_ok == false) always reports
        // air_cost.total == 0, which would otherwise compute as a huge
        // *improvement* and let a fully broken run (e.g. built without
        // `--features zisk-elf`) pass the regression gate silently. Force
        // it to a regression regardless of the pct math.
        let (regressed, failed, note) = if !hw.guest_output_ok {
            (
                true,
                true,
                Some("workload failed in head run (guest_output_ok=false)".to_string()),
            )
        } else {
            (pct > threshold_pct, false, None)
        };
        any_regression |= regressed;
        deltas.push(WorkloadDelta {
            name: hw.name.clone(),
            baseline_total: base,
            head_total: cur,
            pct,
            regressed,
            failed,
            note,
        });
    }

    // A baseline workload that's simply absent from the head report (e.g.
    // dropped from the manifest, or its build step erroring out before the
    // report was even populated) must not pass silently either.
    for bw in &baseline.workloads {
        if head.workloads.iter().any(|hw| hw.name == bw.name) {
            continue;
        }
        any_regression = true;
        deltas.push(WorkloadDelta {
            name: bw.name.clone(),
            baseline_total: bw.air_cost.total,
            head_total: 0,
            pct: 0.0,
            regressed: true,
            failed: true,
            note: Some("workload missing from head report".to_string()),
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
        match &d.note {
            Some(note) => println!(
                "{:<40} {:>12} -> {:>12}  {:+.2}%  {}  ({note})",
                d.name, d.baseline_total, d.head_total, d.pct, flag
            ),
            None => println!(
                "{:<40} {:>12} -> {:>12}  {:+.2}%  {}",
                d.name, d.baseline_total, d.head_total, d.pct, flag
            ),
        }
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

    /// A workload that failed during `run` — always lands with
    /// `air_cost.total == 0` and `guest_output_ok: false`.
    fn wl_failed(name: &str) -> WorkloadResult {
        let mut w = wl(name, 0);
        w.guest_output_ok = false;
        w
    }

    fn rep(items: Vec<WorkloadResult>) -> Report {
        Report {
            meta: Meta {
                zisk_version: "v1.0.0-alpha".into(),
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
        assert!(!a.failed);
        let b = res.deltas.iter().find(|d| d.name == "b").unwrap();
        assert!(!b.regressed);
        assert!(!b.failed);
    }

    #[test]
    fn no_regression_when_within_threshold() {
        let base = rep(vec![wl("a", 100)]);
        let head = rep(vec![wl("a", 101)]);
        assert!(!compare_reports(&base, &head, 3.0).any_regression);
    }

    /// A failed head workload always has `air_cost.total == 0`, which
    /// against a nonzero baseline computes as a ~-100% "improvement" under
    /// the pure pct math. That must not let a broken run pass the gate: a
    /// failed workload is always a regression.
    #[test]
    fn failed_head_workload_is_a_regression_regardless_of_pct() {
        let base = rep(vec![wl("a", 100)]);
        let head = rep(vec![wl_failed("a")]);
        let res = compare_reports(&base, &head, 3.0);
        assert!(res.any_regression);
        let a = res.deltas.iter().find(|d| d.name == "a").unwrap();
        assert!(a.regressed);
        assert!(a.failed);
        assert_eq!(a.head_total, 0);
        assert!(a.note.is_some());
    }

    /// A workload present in the baseline but absent from the head report
    /// (e.g. dropped from the manifest, or its build step erroring out
    /// before the report was populated) must also count as a regression,
    /// not be silently dropped from the comparison.
    #[test]
    fn workload_missing_from_head_is_a_regression() {
        let base = rep(vec![wl("a", 100), wl("b", 100)]);
        let head = rep(vec![wl("a", 100)]); // "b" is entirely absent from head
        let res = compare_reports(&base, &head, 3.0);
        assert!(res.any_regression);
        let b = res.deltas.iter().find(|d| d.name == "b").unwrap();
        assert!(b.regressed);
        assert!(b.failed);
        assert_eq!(b.baseline_total, 100);
        assert_eq!(b.head_total, 0);
        assert!(b.note.is_some());
    }
}
