// Parallelism strategy: each JSON file holds many fixtures (typically dozens to
// hundreds). We iterate fixtures serially within the test fn; datatest-stable
// parallelises across files using libtest. Tune concurrency with RUST_TEST_THREADS.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use ef_tests_engine::{EngineFixtureFile, FixtureFailure, RunOptions, render_summary, run_fixture};
use regex::Regex;

// Aggregate fixture counters across all `engine_runner` calls. libtest reports
// one test per JSON file, but each file holds many fixtures; these atomics
// surface the inner totals in a final line printed at process exit.
static F_PASSED: AtomicUsize = AtomicUsize::new(0);
static F_FAILED: AtomicUsize = AtomicUsize::new(0);
static F_SKIPPED: AtomicUsize = AtomicUsize::new(0);
static F_FILTERED: AtomicUsize = AtomicUsize::new(0);
static F_XFAIL_UPSTREAM: AtomicUsize = AtomicUsize::new(0);

#[ctor::dtor]
fn print_fixture_summary() {
    let p = F_PASSED.load(Ordering::Relaxed);
    let f = F_FAILED.load(Ordering::Relaxed);
    let s = F_SKIPPED.load(Ordering::Relaxed);
    let fi = F_FILTERED.load(Ordering::Relaxed);
    let x = F_XFAIL_UPSTREAM.load(Ordering::Relaxed);
    let total = p + f + s + fi + x;
    if total == 0 {
        return;
    }

    // Mirror libtest's coloring rules: respect NO_COLOR, CARGO_TERM_COLOR, and TTY.
    let color = {
        use std::io::IsTerminal;
        let force = std::env::var("CARGO_TERM_COLOR").ok();
        match force.as_deref() {
            Some("always") => true,
            Some("never") => false,
            _ => std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal(),
        }
    };
    let (g, r, y, c, b, z) = if color {
        (
            "\x1b[32m", "\x1b[31m", "\x1b[33m", "\x1b[36m", "\x1b[1m", "\x1b[0m",
        )
    } else {
        ("", "", "", "", "", "")
    };
    let verdict = if f > 0 {
        format!("{r}{b}FAILED{z}")
    } else {
        format!("{g}{b}ok{z}")
    };
    eprintln!(
        "\nfixture result: {verdict}. {p} {g}passed{z}; {f} {r}failed{z}; \
         {s} {y}skipped{z}; {fi} {c}filtered{z}; {x} {y}expected-fail (upstream-broken){z}; \
         {total} total",
    );
}

// Path patterns: if any pattern is contained in the file path, the file is skipped
// entirely. Entries are filled in once real failures are classified (Task 4.8).
const SKIP_PATTERNS: &[&str] = &[];

/// Fixtures the pinned release ships with expectations the EELS spec at the
/// same tag rejects — see `upstream_broken_fixtures.txt` for the per-class
/// analysis and citations. Strict on both sides: a listed fixture that passes
/// is a stale entry and fails the suite; one that fails outside the documented
/// signature is a real regression and fails the suite.
static UPSTREAM_BROKEN: OnceLock<HashSet<&'static str>> = OnceLock::new();

fn upstream_broken() -> &'static HashSet<&'static str> {
    UPSTREAM_BROKEN.get_or_init(|| {
        include_str!("upstream_broken_fixtures.txt")
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect()
    })
}

/// The only failure shapes `upstream_broken_fixtures.txt` documents: an
/// expected-`VALID` payload that ethrex rejects for the header inconsistency
/// the broken fill baked in (stale `blobGasUsed`, or a block access list over
/// the EIP-7928 `gas_limit / 2000` item cap).
fn is_documented_upstream_breakage(e: &FixtureFailure) -> bool {
    match e {
        FixtureFailure::WrongStatus {
            expected,
            got,
            validation_error: Some(ve),
            ..
        } => {
            expected == "VALID"
                && got == "INVALID"
                && (ve.contains("Blob gas used doesn't match value in header")
                    || ve.contains("Block access list exceeds gas limit"))
        }
        _ => false,
    }
}

static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn runtime() -> &'static tokio::runtime::Runtime {
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime init failed")
    })
}

/// Optional fixture-name regex (mirrors hive's `--sim.limit`).
/// Set via `ETHREX_ENGINE_LIMIT=<regex>` to run only fixtures whose name matches.
static LIMIT: OnceLock<Option<Regex>> = OnceLock::new();

fn limit() -> Option<&'static Regex> {
    LIMIT
        .get_or_init(|| {
            std::env::var("ETHREX_ENGINE_LIMIT").ok().map(|pat| {
                Regex::new(&pat)
                    .unwrap_or_else(|e| panic!("ETHREX_ENGINE_LIMIT='{pat}' invalid regex: {e}"))
            })
        })
        .as_ref()
}

fn engine_runner(path: &Path) -> datatest_stable::Result<()> {
    // Skip entire file if path matches any skip pattern.
    let path_str = path.to_string_lossy();
    for pat in SKIP_PATTERNS {
        if path_str.contains(pat) {
            return Ok(());
        }
    }

    let raw = std::fs::read_to_string(path)?;
    let fixtures: EngineFixtureFile = serde_json::from_str(&raw)?;

    let opts = RunOptions::from_env();
    let limit = limit();

    let mut total = 0usize;
    let mut passed = 0usize;
    let mut skipped = 0usize;
    let mut filtered = 0usize;
    let mut xfail_upstream = 0usize;
    let mut failures: Vec<(String, String)> = Vec::new();

    for (name, fixture) in &fixtures {
        if let Some(re) = limit
            && !re.is_match(name)
        {
            filtered += 1;
            continue;
        }
        total += 1;
        let result = runtime().block_on(run_fixture(name, fixture, &opts));
        let listed_broken = upstream_broken().contains(name.as_str());
        match result {
            Ok(()) if listed_broken => failures.push((
                name.clone(),
                "listed in upstream_broken_fixtures.txt but passed; stale entry, remove it"
                    .to_string(),
            )),
            Ok(()) => passed += 1,
            Err(e) if listed_broken && is_documented_upstream_breakage(&e) => xfail_upstream += 1,
            Err(e) if listed_broken => failures.push((
                name.clone(),
                format!(
                    "listed in upstream_broken_fixtures.txt but failed outside the documented signature: {e}"
                ),
            )),
            Err(e) if e.is_skip() => skipped += 1,
            Err(e) => failures.push((name.clone(), e.to_string())),
        }
    }

    F_PASSED.fetch_add(passed, Ordering::Relaxed);
    F_SKIPPED.fetch_add(skipped, Ordering::Relaxed);
    F_FILTERED.fetch_add(filtered, Ordering::Relaxed);
    F_XFAIL_UPSTREAM.fetch_add(xfail_upstream, Ordering::Relaxed);
    F_FAILED.fetch_add(failures.len(), Ordering::Relaxed);

    if failures.is_empty() {
        return Ok(());
    }

    let mut report = String::new();
    for (name, failure) in &failures {
        writeln!(report, "FAIL {name}: {failure}").expect("write to String is infallible");
    }
    report.push('\n');
    report.push_str(&render_summary(total, passed, skipped, failures.len()));
    if xfail_upstream > 0 {
        let _ = write!(report, " ({xfail_upstream} expected-fail upstream-broken)");
    }
    Err(report.into())
}

datatest_stable::harness!(engine_runner, "vectors/eest/", r".*\.json$");
