use std::fmt::Display;
use std::fmt::Write as _;

/// Render one `FAIL <name>: <reason>` line per failure. Generic over the
/// reason so callers can pass either a [`crate::runner::FixtureFailure`] or a
/// message they synthesised themselves.
pub fn render_failures<E: Display>(failures: &[(String, E)]) -> String {
    let mut out = String::new();
    for (name, failure) in failures {
        writeln!(out, "FAIL {name}: {failure}").expect("write to String is infallible");
    }
    out
}

pub fn render_summary(total: usize, passed: usize, skipped: usize, failed: usize) -> String {
    format!("=== {total} fixtures: {passed} passed, {skipped} skipped, {failed} failed ===",)
}
