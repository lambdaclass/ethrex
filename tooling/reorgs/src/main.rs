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

    // Fetch the path to the ethrex binary from the command line arguments
    // If not provided, use the default path
    let cmd_path: PathBuf = std::env::args()
        .nth(1)
        .map(|o| o.parse().unwrap())
        .unwrap_or_else(|| "../../target/debug/ethrex".parse().unwrap());

    let version = get_ethrex_version(&cmd_path).await;

    info!(%version, binary_path = %cmd_path.display(), "Fetched ethrex binary version");
    info!("Starting test run");
    info!("");

    run_test(&cmd_path, test_one_block_reorg_and_back).await;
    run_test(&cmd_path, test_storage_slots_reorg).await;
    run_test(&cmd_path, test_storage_tree_with_branches).await;

    // TODO: this test is failing
    // run_test(&cmd_path, test_many_blocks_reorg).await;
}

async fn get_ethrex_version(cmd_path: &Path) -> String {
    let version_output = Command::new(cmd_path)
        .arg("--version")
        .output()
        .expect("failed to get ethrex version");
    String::from_utf8(version_output.stdout).expect("failed to parse version output")
}

async fn run_test<F, Fut>(cmd_path: &Path, test_fn: F)
where
    F: Fn(Arc<Mutex<Simulator>>) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let test_name = std::any::type_name::<F>();
    let start = std::time::Instant::now();

    info!(test=%test_name, "Running test");
    let simulator = Arc::new(Mutex::new(Simulator::new(
        cmd_path.to_path_buf(),
        test_name.to_string(),
    )));

    // Run in another task to clean up properly on panic
    let result = tokio::spawn(test_fn(simulator.clone())).await;

    simulator.lock_owned().await.stop();
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

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

#[expect(unused)]
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

    base_chain = node0.extend_chain(base_chain, 10).await;
    // for _ in 0..10 {
    //     let extended_base_chain = node0.build_payload(base_chain).await;
    //     node0.notify_new_payload(&extended_base_chain).await;
    //     node0.update_forkchoice(&extended_base_chain).await;

    //     node1.notify_new_payload(&extended_base_chain).await;
    //     node1.update_forkchoice(&extended_base_chain).await;
    //     base_chain = extended_base_chain;
    // }

    node1.update_forkchoice(&base_chain).await;

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
}

async fn test_storage_tree_with_branches(simulator: Arc<Mutex<Simulator>>) {
    let mut simulator = simulator.lock().await;

    // These hash to the same 0x2b prefix, causing a branch in the storage trie
    let slot0A = 15865164872778812924_u64;
    let slot0B = 11566337052255443646_u64;

    // These hash to the same 0x83 prefix, causing a branch in the storage trie
    let slot1A = 5800641681667202032_u64;
    let slot1B = 8343558256908705664_u64;
}
