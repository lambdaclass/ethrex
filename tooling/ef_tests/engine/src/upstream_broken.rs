//! Fixtures the pinned release ships with expectations the spec at the same tag
//! rejects, and the rules that keep the list honest.
//!
//! The list itself lives in `tests/upstream_broken_fixtures.txt`, next to the
//! harness that enforces it; it is included from here so that the parsing, the
//! failure signature it documents, and the test that checks it has not rotted
//! all sit together.

use std::collections::HashSet;
#[cfg(test)]
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::runner::FixtureFailure;

static UPSTREAM_BROKEN: OnceLock<HashSet<&'static str>> = OnceLock::new();

/// Names of the fixtures whose expected verdict contradicts the spec at the
/// pinned tag. The harness is strict in both directions: a listed fixture that
/// passes is a stale entry and fails the suite, and one that fails outside the
/// documented signature is a real regression and fails the suite.
pub fn upstream_broken() -> &'static HashSet<&'static str> {
    UPSTREAM_BROKEN.get_or_init(|| {
        include_str!("../tests/upstream_broken_fixtures.txt")
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect()
    })
}

/// The only failure shapes `upstream_broken_fixtures.txt` documents: an
/// expected-`VALID` payload that ethrex rejects for the header inconsistency
/// the broken fill baked in — a stale `blobGasUsed`, or a block access list
/// over the EIP-7928 `gas_limit / 2000` item cap.
pub fn is_documented_upstream_breakage(failure: &FixtureFailure) -> bool {
    match failure {
        FixtureFailure::WrongStatus {
            expected,
            got,
            validation_error: Some(validation_error),
            ..
        } => {
            expected == "VALID"
                && got == "INVALID"
                && (validation_error.contains("Blob gas used doesn't match value in header")
                    || validation_error.contains("Block access list exceeds gas limit"))
        }
        _ => false,
    }
}

/// The fixture file a given fixture name lives in, relative to the vectors root.
/// Only the coverage test below needs this; the harness itself matches by name.
///
/// EEST names a fixture after the test that produced it, and lays the bundle
/// out to match: `tests/<dirs>/test_<module>.py::test_<case>[<id>]` becomes
/// `for_<fork>/<dirs>/<module>/<case>.json`. Returns `None` for a name that
/// does not follow that shape.
#[cfg(test)]
fn fixture_file_for(name: &str, subtree: &str) -> Option<PathBuf> {
    let (path, case) = name.split_once("::test_")?;
    let dirs = path.strip_prefix("tests/")?;
    let (dirs, module) = dirs.rsplit_once("/test_")?;
    let module = module.strip_suffix(".py")?;
    let case = case.split_once('[')?.0;
    Some(
        Path::new(subtree)
            .join(dirs)
            .join(module)
            .join(format!("{case}.json")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EngineFixtureFile;

    /// Every name in `upstream_broken_fixtures.txt` must match a fixture the
    /// pinned bundle actually ships. A renamed or misspelled entry matches
    /// nothing: it never fires the stale-entry check, and a list that has rotted
    /// into inertness this way is indistinguishable from a healthy one in the
    /// suite's own output.
    #[test]
    fn every_entry_names_a_fixture_the_bundle_ships() {
        let vectors = Path::new(env!("CARGO_MANIFEST_DIR")).join("vectors/eest");
        if !vectors.join("for_bogota").is_dir() {
            // No bundle on disk to check against; `make test` downloads it first.
            return;
        }

        let mut unmatched = Vec::new();
        for name in upstream_broken() {
            let Some(relative) = fixture_file_for(name, "for_bogota") else {
                unmatched.push(format!("{name} (name does not follow the EEST layout)"));
                continue;
            };
            let path = vectors.join(&relative);
            let matched = std::fs::read_to_string(&path)
                .ok()
                .and_then(|raw| serde_json::from_str::<EngineFixtureFile>(&raw).ok())
                .is_some_and(|fixtures| fixtures.contains_key(*name));
            if !matched {
                unmatched.push(format!("{name} (looked in {})", relative.display()));
            }
        }

        assert!(
            unmatched.is_empty(),
            "{} entries in upstream_broken_fixtures.txt match no fixture in the pinned bundle; \
             they can never fire and must be re-derived or removed:\n{}",
            unmatched.len(),
            unmatched.join("\n"),
        );
    }
}
