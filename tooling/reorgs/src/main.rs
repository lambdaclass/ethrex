use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use ethrex::{cli::Options, initializers::init_tracing};
use ethrex_l2_rpc::signer::{LocalSigner, Signer};
use tokio::sync::Mutex;
use tracing::warn;

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
        .unwrap_or_else(|| {
            warn!("No binary path provided, using default");
            "../../target/debug/ethrex".parse().unwrap()
        });

    // run_test(&cmd_path, test_one_block_reorg_and_back).await;
    run_test(&cmd_path, test_many_blocks_reorg).await;
}

async fn run_test<F, Fut>(cmd_path: &Path, test_fn: F)
where
    F: Fn(Arc<Mutex<Simulator>>) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let simulator = Arc::new(Mutex::new(Simulator::new(cmd_path.to_path_buf())));

    // Run in another task to clean up properly on panic
    let result = tokio::spawn(test_fn(simulator.clone())).await;

    simulator.lock_owned().await.stop();
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    if result.is_err() {
        eprintln!("Test panicked");
        std::process::exit(1);
    }
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

    // Make an ETH transfer in the base chain
    // NOTE: we do this to ensure both blocks are different
    node1
        .send_eth_transfer(&signer, recipient, transfer_amount)
        .await;

    // Advance the base chain with multiple blocks only known to node1
    for _ in 0..10 {
        base_chain = node1.build_payload(base_chain).await;
        node1.notify_new_payload(&base_chain).await;
        node1.update_forkchoice(&base_chain).await;
    }

    // Sanity check: balance hasn't changed
    let same_balance = node0.get_balance(recipient).await;
    assert_eq!(same_balance, initial_balance);

    // Advance the side chain with one more block and another ETH transfer
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
