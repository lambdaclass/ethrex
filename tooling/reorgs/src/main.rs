use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use ethrex::{cli::Options, initializers::init_tracing};
use ethrex_common::U256;
use ethrex_l2_rpc::signer::{LocalSigner, Signer};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::simulator::Simulator;

mod simulator;

#[tokio::main]
async fn main() {
    // Setup logging
    init_tracing(&Options::default_l1());

    // Argument parsing:
    //   cargo run                            -> run all scenarios (legacy default path)
    //   cargo run -- <binary_path>           -> run all scenarios with given binary
    //   cargo run -- --scenario <name>       -> run one named scenario (default binary path)
    //   cargo run -- --scenario <name> <bin> -> run one named scenario with given binary
    let args: Vec<String> = std::env::args().skip(1).collect();

    let (scenario_filter, cmd_path) = parse_args(&args);

    let version = get_ethrex_version(&cmd_path).await;

    info!(%version, binary_path = %cmd_path.display(), "Fetched ethrex binary version");
    if let Some(ref filter) = scenario_filter {
        info!(scenario = %filter, "Running single scenario");
    } else {
        info!("Starting test run (all scenarios)");
    }
    info!("");

    let all_scenarios: &[(&str, ScenarioFn)] = &[
        ("no_reorgs_full_sync_smoke_test", |s| {
            Box::pin(no_reorgs_full_sync_smoke_test(s))
        }),
        ("test_reorg_back_to_base", |s| {
            Box::pin(test_reorg_back_to_base(s))
        }),
        ("test_chain_split", |s| Box::pin(test_chain_split(s))),
        ("test_one_block_reorg_and_back", |s| {
            Box::pin(test_one_block_reorg_and_back(s))
        }),
        ("test_reorg_back_to_base_with_common_ancestor", |s| {
            Box::pin(test_reorg_back_to_base_with_common_ancestor(s))
        }),
        ("test_storage_slots_reorg", |s| {
            Box::pin(test_storage_slots_reorg(s))
        }),
        ("test_many_blocks_reorg", |s| {
            Box::pin(test_many_blocks_reorg(s))
        }),
        ("deep_reorg_beyond_128", |s| {
            Box::pin(test_deep_reorg_beyond_128(s))
        }),
    ];

    for (name, scenario_fn) in all_scenarios {
        if let Some(ref filter) = scenario_filter
            && *name != filter.as_str()
        {
            continue;
        }
        run_test_dyn(&cmd_path, name, *scenario_fn).await;
    }
}

/// Parse CLI args into an optional scenario filter and a binary path.
///
/// Supported invocations:
///   (none)                          -> all scenarios, default binary
///   <binary_path>                   -> all scenarios, given binary (arg contains '/')
///   <scenario_name>                 -> one scenario, default binary (no '/' in arg)
///   --scenario <name>               -> one scenario, default binary
///   --scenario <name> <binary_path> -> one scenario, given binary
fn parse_args(args: &[String]) -> (Option<String>, PathBuf) {
    let default_bin: PathBuf = "../../target/debug/ethrex".parse().unwrap();

    let mut scenario: Option<String> = None;
    let mut bin: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--scenario" {
            i += 1;
            if i < args.len() {
                scenario = Some(args[i].clone());
            }
        } else if args[i].contains('/') || args[i].starts_with('.') {
            // Looks like a file path -- treat as binary location.
            bin = Some(args[i].parse().unwrap());
        } else {
            // Plain word without path separator -- treat as scenario name.
            scenario = Some(args[i].clone());
        }
        i += 1;
    }

    (scenario, bin.unwrap_or(default_bin))
}

async fn get_ethrex_version(cmd_path: &Path) -> String {
    let version_output = Command::new(cmd_path)
        .arg("--version")
        .output()
        .expect("failed to get ethrex version");
    String::from_utf8(version_output.stdout).expect("failed to parse version output")
}

type ScenarioFn =
    fn(Arc<Mutex<Simulator>>) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

/// Run a scenario function identified by a display name.
async fn run_test_dyn(cmd_path: &Path, test_name: &str, test_fn: ScenarioFn) {
    let start = std::time::Instant::now();

    info!(test=%test_name, "Running test");
    let simulator = Arc::new(Mutex::new(Simulator::new(
        cmd_path.to_path_buf(),
        test_name.to_string(),
    )));

    // Run in another task to clean up properly on panic
    let result = tokio::spawn(test_fn(simulator.clone())).await;

    simulator.lock_owned().await.stop().await;

    match result {
        Ok(_) => info!(test=%test_name, elapsed=?start.elapsed(), "test completed successfully"),
        Err(err) if err.is_panic() => {
            error!(test=%test_name, %err, "test panicked");
            std::process::exit(1);
        }
        Err(err) => {
            warn!(test=%test_name, %err, "test task was cancelled");
        }
    }
    // Add a blank line after each test for readability
    info!("");
}

async fn no_reorgs_full_sync_smoke_test(simulator: Arc<Mutex<Simulator>>) {
    let mut simulator = simulator.lock().await;

    // Start two ethrex nodes
    let node0 = simulator.start_node().await;
    let node1 = simulator.start_node().await;

    // Create a chain and extend it with a few empty blocks
    let base_chain = node0.extend_chain(simulator.get_base_chain(), 10).await;

    // Try to fully sync node1 (which is a peer of node0)
    node1.update_forkchoice(&base_chain).await;
}

async fn test_reorg_back_to_base(simulator: Arc<Mutex<Simulator>>) {
    let mut simulator = simulator.lock().await;

    // Start two ethrex nodes
    let node0 = simulator.start_node().await; // Test_node
    let node1 = simulator.start_node().await;

    let base_chain = simulator.get_base_chain();
    let side_chain = base_chain.fork();
    // Create a chain and extend it with a few empty blocks
    let base_chain = node1.extend_chain(base_chain, 10).await;

    // Create another chain (a fork of the first one) and extend it with a few empty blocks
    let _ = node0.extend_chain(side_chain, 10).await;

    // Try to fully sync node0 to base chain
    node0.update_forkchoice(&base_chain).await;
}

async fn test_reorg_back_to_base_with_common_ancestor(simulator: Arc<Mutex<Simulator>>) {
    let mut simulator = simulator.lock().await;

    // Start two ethrex nodes
    let node0 = simulator.start_node().await; // Test_node
    let node1 = simulator.start_node().await;

    let base_chain = simulator.get_base_chain();
    // Create a chain and extend it with a few empty blocks
    let base_chain = node1.extend_chain(base_chain, 10).await;

    // Update node0 to the base chain
    node0.update_forkchoice(&base_chain).await;

    let side_chain = base_chain.fork();
    // Extend the base chain
    let base_chain = node1.extend_chain(base_chain, 10).await;
    // Create another chain (a fork of the first one) and extend it with a few empty blocks
    let _ = node0.extend_chain(side_chain, 10).await;

    // Try to fully sync node0 to base chain
    node0.update_forkchoice(&base_chain).await;
}

async fn test_chain_split(simulator: Arc<Mutex<Simulator>>) {
    let mut simulator = simulator.lock().await;

    // Start three ethrex nodes
    let node0 = simulator.start_node().await; // Test_node
    let node1 = simulator.start_node().await;
    let node2 = simulator.start_node().await;

    let base_chain = simulator.get_base_chain();
    let side_chain = base_chain.fork();
    // Create a chain and extend it with a few empty blocks
    let base_chain = node1.extend_chain(base_chain, 10).await;

    // Create another chain (a fork of the first one) and extend it with a few empty blocks
    let _ = node2.extend_chain(side_chain, 10).await;

    // Try to fully sync node0 to base chain (which is a peer of node1 and node2)
    // It will ask peer1 sometimes and peer2 others, it has to work with both
    // So if one fails, this test will fail 50% of the time
    node0.update_forkchoice(&base_chain).await;
}

async fn test_one_block_reorg_and_back(simulator: Arc<Mutex<Simulator>>) {
    let mut simulator = simulator.lock().await;
    let signer: Signer = LocalSigner::new(
        "941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e"
            .parse()
            .unwrap(),
    )
    .into();
    // Some random address
    let recipient = "941e103320615d394a55708be13e45994c7d93b0".parse().unwrap();
    let transfer_amount = 1000000;

    let node0 = simulator.start_node().await;
    let node1 = simulator.start_node().await;

    // Create a chain with a few empty blocks
    let mut base_chain = simulator.get_base_chain();
    for _ in 0..10 {
        let extended_base_chain = node0.build_payload(base_chain).await;
        node0.notify_new_payload(&extended_base_chain).await;
        node0.update_forkchoice(&extended_base_chain).await;

        node1.notify_new_payload(&extended_base_chain).await;
        node1.update_forkchoice(&extended_base_chain).await;
        base_chain = extended_base_chain;
    }

    let initial_balance = node0.get_balance(recipient).await;

    // Fork the chain
    let side_chain = base_chain.fork();

    // Mine a new block in the base chain
    let base_chain = node0.build_payload(base_chain).await;
    node0.notify_new_payload(&base_chain).await;
    node0.update_forkchoice(&base_chain).await;

    // Mine a new block in the base chain (but don't announce it yet)
    let extended_base_chain = node0.build_payload(base_chain).await;

    // In parallel, mine a block in the side chain, with an ETH transfer
    node1
        .send_eth_transfer(&signer, recipient, transfer_amount)
        .await;

    let side_chain = node1.build_payload(side_chain).await;
    node1.notify_new_payload(&side_chain).await;
    node1.update_forkchoice(&side_chain).await;

    // Sanity check: balance hasn't changed
    let same_balance = node0.get_balance(recipient).await;
    assert_eq!(same_balance, initial_balance);

    // Notify the first node of the side chain block, it should reorg
    node0.notify_new_payload(&side_chain).await;
    node0.update_forkchoice(&side_chain).await;

    // Check the transfer has been processed
    let new_balance = node0.get_balance(recipient).await;
    assert_eq!(new_balance, initial_balance + transfer_amount);

    // Finally, move to the extended base chain, it should reorg back
    node0.notify_new_payload(&extended_base_chain).await;
    node0.update_forkchoice(&extended_base_chain).await;

    // Check the transfer has been reverted
    let new_balance = node0.get_balance(recipient).await;
    assert_eq!(new_balance, initial_balance);
}

async fn test_many_blocks_reorg(simulator: Arc<Mutex<Simulator>>) {
    let mut simulator = simulator.lock().await;
    let signer: Signer = LocalSigner::new(
        "941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e"
            .parse()
            .unwrap(),
    )
    .into();
    // Some random address
    let recipient = "941e103320615d394a55708be13e45994c7d93b0".parse().unwrap();
    let transfer_amount = 1000000;

    let node0 = simulator.start_node().await;
    let node1 = simulator.start_node().await;

    // Create a chain with a few empty blocks
    let mut base_chain = simulator.get_base_chain();
    for _ in 0..10 {
        let extended_base_chain = node0.build_payload(base_chain).await;
        node0.notify_new_payload(&extended_base_chain).await;
        node0.update_forkchoice(&extended_base_chain).await;

        node1.notify_new_payload(&extended_base_chain).await;
        node1.update_forkchoice(&extended_base_chain).await;
        base_chain = extended_base_chain;
    }

    let initial_balance = node0.get_balance(recipient).await;

    // Fork the chain
    let mut side_chain = base_chain.fork();

    // Create a side chain with multiple blocks only known to node0
    for _ in 0..10 {
        side_chain = node0.build_payload(side_chain).await;
        node0.notify_new_payload(&side_chain).await;
        node0.update_forkchoice(&side_chain).await;
    }

    // Sanity check: balance hasn't changed
    let same_balance = node0.get_balance(recipient).await;
    assert_eq!(same_balance, initial_balance);

    // Advance the base chain with multiple blocks only known to node1
    for _ in 0..10 {
        base_chain = node1.build_payload(base_chain).await;
        node1.notify_new_payload(&base_chain).await;
        node1.update_forkchoice(&base_chain).await;
    }

    // Sanity check: balance hasn't changed
    let same_balance = node0.get_balance(recipient).await;
    assert_eq!(same_balance, initial_balance);

    // Advance the side chain with one more block and an ETH transfer
    node1
        .send_eth_transfer(&signer, recipient, transfer_amount)
        .await;
    base_chain = node1.build_payload(base_chain).await;
    node1.notify_new_payload(&base_chain).await;
    node1.update_forkchoice(&base_chain).await;

    // Bring node0 again to the base chain, it should reorg
    node0.notify_new_payload(&base_chain).await;
    node0.update_forkchoice(&base_chain).await;

    // Check the transfer has been processed
    let new_balance = node0.get_balance(recipient).await;
    assert_eq!(new_balance, initial_balance + transfer_amount);
}

async fn test_storage_slots_reorg(simulator: Arc<Mutex<Simulator>>) {
    let mut simulator = simulator.lock().await;
    // Initcode for deploying a contract that receives two `bytes32` parameters and sets `storage[param0] = param1`
    let contract_deploy_bytecode = hex::decode("656020355f35555f526006601af3").unwrap().into();
    let signer: Signer = LocalSigner::new(
        "941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e"
            .parse()
            .unwrap(),
    )
    .into();

    let slot_key0 = U256::from(42);
    let slot_value0 = U256::from(1163);
    let slot_key1 = U256::from(25);
    let slot_value1 = U256::from(7474);

    let node0 = simulator.start_node().await;
    let node1 = simulator.start_node().await;

    // Create a chain with a few empty blocks
    let mut base_chain = simulator.get_base_chain();

    // Send a deploy tx for a contract which receives: `(bytes32 key, bytes32 value)` as parameters
    let contract_address = node0
        .send_contract_deploy(&signer, contract_deploy_bytecode)
        .await;

    for _ in 0..10 {
        let extended_base_chain = node0.build_payload(base_chain).await;
        node0.notify_new_payload(&extended_base_chain).await;
        node0.update_forkchoice(&extended_base_chain).await;

        node1.notify_new_payload(&extended_base_chain).await;
        node1.update_forkchoice(&extended_base_chain).await;
        base_chain = extended_base_chain;
    }

    // Sanity check: storage slots are initially empty
    let initial_value = node0.get_storage_at(contract_address, slot_key0).await;
    assert_eq!(initial_value, U256::zero());
    let initial_value = node0.get_storage_at(contract_address, slot_key1).await;
    assert_eq!(initial_value, U256::zero());

    // Fork the chain
    let mut side_chain = base_chain.fork();

    // Create a side chain with multiple blocks only known to node0
    for _ in 0..10 {
        side_chain = node0.build_payload(side_chain).await;
        node0.notify_new_payload(&side_chain).await;
        node0.update_forkchoice(&side_chain).await;
    }

    // Advance the base chain with multiple blocks only known to node1
    for _ in 0..10 {
        base_chain = node1.build_payload(base_chain).await;
        node1.notify_new_payload(&base_chain).await;
        node1.update_forkchoice(&base_chain).await;
    }

    // Set a storage slot in the contract in node0
    let calldata0 = [slot_key0.to_big_endian(), slot_value0.to_big_endian()]
        .concat()
        .into();
    node0.send_call(&signer, contract_address, calldata0).await;

    // Set another storage slot in the contract in node1
    let calldata1 = [slot_key1.to_big_endian(), slot_value1.to_big_endian()]
        .concat()
        .into();
    node1.send_call(&signer, contract_address, calldata1).await;

    // Build a block in the side chain
    side_chain = node0.build_payload(side_chain).await;
    node0.notify_new_payload(&side_chain).await;
    node0.update_forkchoice(&side_chain).await;

    // Build a block in the base chain
    base_chain = node1.build_payload(base_chain).await;
    node1.notify_new_payload(&base_chain).await;
    node1.update_forkchoice(&base_chain).await;

    // Assert the storage slots are as expected in both forks
    let value_slot0 = node0.get_storage_at(contract_address, slot_key0).await;
    assert_eq!(value_slot0, slot_value0);
    let value_slot1 = node0.get_storage_at(contract_address, slot_key1).await;
    assert_eq!(value_slot1, U256::zero());

    let value_slot0 = node1.get_storage_at(contract_address, slot_key0).await;
    assert_eq!(value_slot0, U256::zero());
    let value_slot1 = node1.get_storage_at(contract_address, slot_key1).await;
    assert_eq!(value_slot1, slot_value1);

    // Reorg the node0 to the base chain
    node0.notify_new_payload(&base_chain).await;
    node0.update_forkchoice(&base_chain).await;

    // Check the storage slots are as expected after the reorg
    let value_slot0 = node0.get_storage_at(contract_address, slot_key0).await;
    assert_eq!(value_slot0, U256::zero());
    let value_slot1 = node0.get_storage_at(contract_address, slot_key1).await;
    assert_eq!(value_slot1, slot_value1);
}

/// Verifies that ethrex accepts a reorg deeper than the legacy 128-block cap.
///
/// `build_payload` issues an FCU on every block so the canonical chain
/// advances with each call. The original version of this test built a single
/// 200-block chain from genesis and then FCUd to its tip; at that point the
/// tip was *already* canonical, so the cap check inside `apply_fork_choice`
/// saw `reorg_depth = 0` and never exercised the lifted limit.
///
/// To actually trigger the cap check, this version builds two parallel
/// chains:
///
///   1. Chain A: 150 blocks from genesis. Each `build_payload` FCUs the new
///      tip canonical, so after the loop `latest_canonical_block_number = 150`.
///   2. Chain B: 200 blocks, also from genesis (a sibling of A). The first
///      `build_payload` on B issues `FCU(head = genesis)`, which lands in
///      `apply_fork_choice` with `latest = 150`, `canonical_link_height = 0`,
///      hence `reorg_depth = 150`. The old `REORG_DEPTH_LIMIT = 128` would
///      have returned `TooDeepReorg`; PR 4's finality-bounded ceiling
///      (`latest - finalized_number = 150`) accepts it.
async fn test_deep_reorg_beyond_128(simulator: Arc<Mutex<Simulator>>) {
    let mut simulator = simulator.lock().await;
    let node = simulator.start_node().await;
    let base_chain = simulator.get_base_chain();

    // Phase 1: build chain A and make it canonical.
    let mut chain_a = base_chain.fork();
    let chain_a_len: usize = std::env::var("DEEP_REORG_CHAIN_A_LEN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(150);
    info!(chain_a_len, "Building chain A (canonical baseline)");
    for i in 0..chain_a_len {
        chain_a = node.build_payload(chain_a).await;
        node.notify_new_payload(&chain_a).await;
        if i % 50 == 49 {
            info!(block = i + 1, "Chain A progress");
        }
    }
    info!(latest = chain_a_len, "Chain A tip is canonical");

    // Phase 2: build chain B from genesis. The first build_payload triggers
    // the cap-check at reorg_depth = 150 (latest_A - link_to_genesis = 150).
    // Pre-PR-4 this would return TooDeepReorg{ limit: 128 } and the test
    // would panic on the build_payload assertion.
    let mut chain_b = base_chain.fork();
    info!("Building chain B (200 blocks, forks at genesis -- first FCU is the 150-deep reorg)");
    for i in 0..200usize {
        chain_b = node.build_payload(chain_b).await;
        node.notify_new_payload(&chain_b).await;
        if i % 50 == 49 {
            info!(block = i + 1, "Chain B progress");
        }
    }

    info!(
        canonical_baseline = 150,
        side_chain_len = chain_b.len() - 1,
        "Deep reorg beyond legacy 128-block cap accepted -- test passed"
    );
}
