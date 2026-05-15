use crate::runner::FixtureFailure;
use std::fmt::Write as _;

pub fn render_failures(failures: &[(String, FixtureFailure)]) -> String {
    let mut out = String::new();
    for (name, failure) in failures {
        writeln!(out, "FAIL {name}: {failure}").expect("write to String is infallible");
    }
    out
}

pub fn render_summary(total: usize, passed: usize, skipped: usize, failed: usize) -> String {
    format!(
        "=== {total} fixtures: {passed} passed, {skipped} skipped, {failed} failed ===",
    )
}
