// Parallelism strategy: each JSON file holds many fixtures (typically dozens to
// hundreds). We iterate fixtures serially within the test fn; datatest-stable
// parallelises across files using libtest. Tune concurrency with RUST_TEST_THREADS.

use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use ef_tests_engine::{
    EngineFixtureFile, FixtureFailure, RunOptions, render_failures, render_summary, run_fixture,
};
use regex::Regex;

// Aggregate fixture counters across all `engine_runner` calls. libtest reports
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
         {s} {y}skipped{z}; {fi} {c}filtered{z}; {total} total",
    );
}

// Path patterns: if any pattern is contained in the file path, every fixture in
// that file is counted as skipped and the reason is printed. A skip here means
// the fixture cannot be run at all, never that it is expected to fail.
const SKIP_PATTERNS: &[(&str, &str)] = &[(
    "for_bogota/bogota/eip7805_focil/",
    "tests-focil@v0.1.0 predates EIP-8282 (Builder Execution Requests, EIP-7773 \
     Glamsterdam): its pre-state omits the builder deposit/exit predeploys, so every \
     block fails EIP-8282's empty-code rule before the inclusion-list check runs. \
     Supplying the predeploys is not an option either; they move the genesis state \
     root, and the fixtures commit to a genesis hash computed without them. Re-enable \
     when execution-specs publishes a FOCIL fixture release filled against an \
     Amsterdam that carries EIP-8282.",
)];

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
    let raw = std::fs::read_to_string(path)?;
    let fixtures: EngineFixtureFile = serde_json::from_str(&raw)?;

    // Skip the whole file if its path matches a skip pattern, counting its
    // fixtures so the totals stay honest about what was not run.
    let path_str = path.to_string_lossy();
    if let Some((pat, reason)) = SKIP_PATTERNS.iter().find(|(pat, _)| path_str.contains(pat)) {
        F_SKIPPED.fetch_add(fixtures.len(), Ordering::Relaxed);
        eprintln!(
            "skipping {} fixture(s) under {pat}: {reason}",
            fixtures.len()
        );
        return Ok(());
    }

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

datatest_stable::harness!(engine_runner, "vectors/eest/", r".*\.json$");
