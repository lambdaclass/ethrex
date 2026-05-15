// Parallelism strategy: each JSON file holds many fixtures (typically dozens to
// hundreds). We iterate fixtures serially within the test fn; datatest-stable
// parallelises across files using libtest. Tune concurrency with RUST_TEST_THREADS.

use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use ef_tests_hive::{
    EngineFixtureFile, FixtureFailure, RunOptions, render_failures, render_summary, run_fixture,
};
use regex::Regex;

// Aggregate fixture counters across all `hive_runner` calls. libtest reports
// one test per JSON file, but each file holds many fixtures; these atomics
// surface the inner totals in a final line printed at process exit.
static F_PASSED: AtomicUsize = AtomicUsize::new(0);
static F_FAILED: AtomicUsize = AtomicUsize::new(0);
static F_SKIPPED: AtomicUsize = AtomicUsize::new(0);
static F_FILTERED: AtomicUsize = AtomicUsize::new(0);

#[ctor::dtor]
fn print_fixture_summary() {
    let p = F_PASSED.load(Ordering::Relaxed);
    let f = F_FAILED.load(Ordering::Relaxed);
    let s = F_SKIPPED.load(Ordering::Relaxed);
    let fi = F_FILTERED.load(Ordering::Relaxed);
    let total = p + f + s + fi;
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
        ("\x1b[32m", "\x1b[31m", "\x1b[33m", "\x1b[36m", "\x1b[1m", "\x1b[0m")
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
         {s} {y}skipped{z}; {fi} {c}filtered{z}; {total} total",
    );
}

// Path patterns: if any pattern is contained in the file path, the file is skipped
// entirely. Entries are filled in once real failures are classified (Task 4.8).
const SKIP_PATTERNS: &[&str] = &[];

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
/// Set via `ETHREX_HIVE_LIMIT=<regex>` to run only fixtures whose name matches.
static LIMIT: OnceLock<Option<Regex>> = OnceLock::new();

fn limit() -> Option<&'static Regex> {
    LIMIT
        .get_or_init(|| {
            std::env::var("ETHREX_HIVE_LIMIT").ok().map(|pat| {
                Regex::new(&pat)
                    .unwrap_or_else(|e| panic!("ETHREX_HIVE_LIMIT='{pat}' invalid regex: {e}"))
            })
        })
        .as_ref()
}

fn hive_runner(path: &Path) -> datatest_stable::Result<()> {
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
    let mut failures: Vec<(String, FixtureFailure)> = Vec::new();

    for (name, fixture) in &fixtures {
        if let Some(re) = limit
            && !re.is_match(name)
        {
            filtered += 1;
            continue;
        }
        total += 1;
        let result = runtime().block_on(run_fixture(name, fixture, &opts));
        match result {
            Ok(()) => passed += 1,
            Err(e) if e.is_skip() => skipped += 1,
            Err(e) => failures.push((name.clone(), e)),
        }
    }

    F_PASSED.fetch_add(passed, Ordering::Relaxed);
    F_SKIPPED.fetch_add(skipped, Ordering::Relaxed);
    F_FILTERED.fetch_add(filtered, Ordering::Relaxed);
    F_FAILED.fetch_add(failures.len(), Ordering::Relaxed);

    if failures.is_empty() {
        return Ok(());
    }

    let mut report = render_failures(&failures);
    report.push('\n');
    report.push_str(&render_summary(total, passed, skipped, failures.len()));
    Err(report.into())
}

datatest_stable::harness!(hive_runner, "vectors/eest/", r".*\.json$");
