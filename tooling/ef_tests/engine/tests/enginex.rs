// `blockchain_test_engine_x` fixtures, run the way hive's enginex simulator runs
// them: the fixtures of one pre-allocation group share a single client, and each
// starts with a forkchoice update back to the group's genesis, so every fixture's
// chain is a new branch off the same genesis on a client that has already built
// the earlier ones. Groups run in parallel; a group's fixtures run in order. A
// group whose fixtures configure the chain differently (EIP-8253's account list)
// gets one client per configuration.
//
// Vectors live under `vectors_x/eest/` (`make enginex-vectors`): the fixture
// subtrees plus `pre_alloc/<preHash>.json`. Without them nothing runs, as with
// the datatest harness and `vectors/eest/`. `ETHREX_ENGINE_LIMIT` and
// `ETHREX_ENGINE_STRICT_EXCEPTIONS` work as for the `all` target.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Mutex, OnceLock};

use ef_tests_engine::{
    EngineXFixtureFile, EngineXGroup, FixtureFailure, PreAllocGroup, RunOptions, render_failures,
    run_engine_x_fixture,
};
use rayon::prelude::*;
use regex::Regex;
use serde::Deserialize;

const VECTORS_DIR: &str = "vectors_x/eest";
const PRE_ALLOC_DIR: &str = "pre_alloc";

static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn runtime() -> &'static tokio::runtime::Runtime {
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime init failed")
    })
}

/// The fields the index pass needs from each fixture: its pre-allocation group and
/// the chain configuration its client starts with.
#[derive(Deserialize)]
struct GroupRef {
    #[serde(rename = "preHash")]
    pre_hash: String,
    #[serde(default)]
    config: ClientConfig,
}

/// The part of a fixture's chain configuration a client is started with that the
/// pre-allocation hash does not cover: EIP-8253's account list, which each fixture
/// names itself. Fixtures of one pre-allocation group can name different lists, so
/// a client is shared only by the fixtures that agree on it.
#[derive(Deserialize, Default, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ClientConfig {
    #[serde(rename = "zeroNonceStorageAccounts", default)]
    zero_nonce_storage_accounts: Vec<String>,
}

impl ClientConfig {
    fn of(fixture_config: &serde_json::Value) -> Self {
        serde_json::from_value(fixture_config.clone()).unwrap_or_default()
    }
}

/// The fixtures of one pre-allocation group that share a client configuration, by
/// the files that hold them.
struct GroupPlan {
    pre_hash: String,
    client_config: ClientConfig,
    files: Vec<PathBuf>,
    fixtures: usize,
}

#[derive(Default)]
struct Outcome {
    passed: usize,
    skipped: usize,
    failures: Vec<(String, FixtureFailure)>,
}

fn main() -> ExitCode {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(VECTORS_DIR);
    if !root.is_dir() {
        eprintln!("no engine_x vectors at {}, nothing to run", root.display());
        return ExitCode::SUCCESS;
    }
    let limit = std::env::var("ETHREX_ENGINE_LIMIT").ok().map(|pat| {
        Regex::new(&pat)
            .unwrap_or_else(|e| panic!("ETHREX_ENGINE_LIMIT='{pat}' invalid regex: {e}"))
    });
    let opts = RunOptions::from_env();

    let mut files = Vec::new();
    collect_fixture_files(&root, &mut files);
    files.sort();
    let (plans, filtered) = plan_groups(&files, limit.as_ref());

    let queue = Mutex::new(plans);
    let totals = Mutex::new(Outcome::default());
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let Some(plan) = queue.lock().expect("queue lock").pop() else {
                        break;
                    };
                    let outcome =
                        runtime().block_on(run_group(&root, &plan, limit.as_ref(), &opts));
                    let mut totals = totals.lock().expect("totals lock");
                    totals.passed += outcome.passed;
                    totals.skipped += outcome.skipped;
                    totals.failures.extend(outcome.failures);
                }
            });
        }
    });

    let mut totals = totals.into_inner().expect("totals lock");
    totals.failures.sort_by(|a, b| a.0.cmp(&b.0));
    if !totals.failures.is_empty() {
        eprintln!("{}", render_failures(&totals.failures));
    }
    let failed = totals.failures.len();
    let total = totals.passed + failed + totals.skipped + filtered;
    let verdict = if failed > 0 { "FAILED" } else { "ok" };
    eprintln!(
        "\nengine_x fixture result: {verdict}. {} passed; {failed} failed; {} skipped; \
         {filtered} filtered; {total} total",
        totals.passed, totals.skipped,
    );
    if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Every fixture file under `dir`, leaving out the pre-allocation groups.
fn collect_fixture_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("read entry of {}: {e}", dir.display()))
            .path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != PRE_ALLOC_DIR) {
                collect_fixture_files(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "json") {
            out.push(path);
        }
    }
}

/// Index the fixtures by pre-allocation group. Returns the groups, largest last
/// so the workers pop them first and the longest groups do not run at the end,
/// and how many fixtures `limit` filtered out.
fn plan_groups(files: &[PathBuf], limit: Option<&Regex>) -> (Vec<GroupPlan>, usize) {
    let indexed: Vec<(PathBuf, BTreeMap<String, GroupRef>)> = files
        .par_iter()
        .map(|file| {
            let raw =
                std::fs::read(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
            let refs = serde_json::from_slice(&raw)
                .unwrap_or_else(|e| panic!("parse {}: {e}", file.display()));
            (file.clone(), refs)
        })
        .collect();

    let mut groups: BTreeMap<(String, ClientConfig), GroupPlan> = BTreeMap::new();
    let mut filtered = 0;
    for (file, refs) in indexed {
        for (name, group_ref) in refs {
            if limit.is_some_and(|re| !re.is_match(&name)) {
                filtered += 1;
                continue;
            }
            let plan = groups
                .entry((group_ref.pre_hash.clone(), group_ref.config.clone()))
                .or_insert_with(|| GroupPlan {
                    pre_hash: group_ref.pre_hash,
                    client_config: group_ref.config,
                    files: Vec::new(),
                    fixtures: 0,
                });
            if plan.files.last() != Some(&file) {
                plan.files.push(file.clone());
            }
            plan.fixtures += 1;
        }
    }
    let mut plans: Vec<GroupPlan> = groups.into_values().collect();
    plans.sort_by_key(|plan| plan.fixtures);
    (plans, filtered)
}

/// Run every fixture of one group, in file and name order, on one client.
async fn run_group(
    root: &Path,
    plan: &GroupPlan,
    limit: Option<&Regex>,
    opts: &RunOptions,
) -> Outcome {
    let mut outcome = Outcome::default();
    let group_path = root
        .join(PRE_ALLOC_DIR)
        .join(format!("{}.json", plan.pre_hash));
    let group: PreAllocGroup = match std::fs::read(&group_path)
        .map_err(|e| e.to_string())
        .and_then(|raw| serde_json::from_slice(&raw).map_err(|e| e.to_string()))
    {
        Ok(group) => group,
        Err(e) => {
            outcome.failures.push((
                format!("pre-allocation group {}", plan.pre_hash),
                FixtureFailure::FixtureParse(format!("{}: {e}", group_path.display())),
            ));
            return outcome;
        }
    };

    let mut client: Option<EngineXGroup> = None;
    for file in &plan.files {
        let fixtures: EngineXFixtureFile = match std::fs::read(file)
            .map_err(|e| e.to_string())
            .and_then(|raw| serde_json::from_slice(&raw).map_err(|e| e.to_string()))
        {
            Ok(fixtures) => fixtures,
            Err(e) => {
                outcome
                    .failures
                    .push((file.display().to_string(), FixtureFailure::FixtureParse(e)));
                continue;
            }
        };
        for (name, fixture) in &fixtures {
            if fixture.pre_hash != plan.pre_hash
                || ClientConfig::of(&fixture.config) != plan.client_config
                || limit.is_some_and(|re| !re.is_match(name))
            {
                continue;
            }
            let client = match &mut client {
                Some(client) => client,
                None => match Box::pin(EngineXGroup::new(&group, fixture, opts)).await {
                    Ok(started) => client.insert(started),
                    Err(e) => {
                        // Every fixture of the group needs this client.
                        outcome.failures.push((name.clone(), e));
                        return outcome;
                    }
                },
            };
            match Box::pin(run_engine_x_fixture(name, fixture, client, opts)).await {
                Ok(()) => outcome.passed += 1,
                Err(e) if e.is_skip() => outcome.skipped += 1,
                Err(e) => outcome.failures.push((name.clone(), e)),
            }
        }
    }
    outcome
}
