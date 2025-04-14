use clap::Parser;
use ethrex::{
    cli::CLI,
    initializers::{
        get_local_p2p_node, get_network, get_signer, init_blockchain, init_metrics, init_rpc_api,
        init_store, init_tracing,
    },
    utils::{set_datadir, store_known_peers},
};
use ethrex_common::types::Block;
use ethrex_p2p::network::peer_table;
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::Store;
use std::{io::Write, path::PathBuf, time::Duration};
use tokio_util::task::TaskTracker;
use tracing::info;

/// Generates a `test.rlp` file for use by the prover during testing.
/// Place this in the `proposer/mod.rs` file,
/// specifically in the `start` function,
/// before calling `send_commitment()` to send the block commitment.
pub fn generate_rlp(
    up_to_block_number: u64,
    store: &Store,
) -> Result<(), Box<dyn std::error::Error>> {
    let up_to_block_number = store.get_latest_block_number()?.min(up_to_block_number);
    info!("Generating RLP up to block {}", up_to_block_number);
    // if store.get_latest_block_number()? >= up_to_block_number {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let file_name = "l2-test.rlp";

    path.push(file_name);

    let mut file = std::fs::File::create(path.to_str().unwrap())?;
    for i in 1..up_to_block_number {
        let body = store.get_block_body(i)?.unwrap();
        let header = store.get_block_header(i)?.unwrap();

        let block = Block::new(header, body);
        let vec = block.encode_to_vec();
        file.write_all(&vec)?;
    }

    info!("TEST RLP GENERATED AT: {path:?}");
    // }
    Ok(())
}

#[cfg(any(feature = "l2", feature = "based"))]
use ethrex::l2::L2Options;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let CLI { opts, command } = CLI::parse();

    init_tracing(&opts);

    if let Some(subcommand) = command {
        return subcommand.run(&opts).await;
    }

    let data_dir = set_datadir(&opts.datadir);

    let network = get_network(&opts);

    let store = init_store(&data_dir, &network).await;

    if let Err(_) = generate_rlp(100, &store) {
        panic!("ERROR GENERATING RLP")
    }

    panic!("STOP EXECUTION");

    let blockchain = init_blockchain(opts.evm, store.clone());

    let signer = get_signer(&data_dir);

    let local_p2p_node = get_local_p2p_node(&opts, &signer);

    let peer_table = peer_table(signer.clone());

    // TODO: Check every module starts properly.
    let tracker = TaskTracker::new();

    let cancel_token = tokio_util::sync::CancellationToken::new();

    init_rpc_api(
        &opts,
        #[cfg(any(feature = "l2", feature = "based"))]
        &L2Options::default(),
        &signer,
        peer_table.clone(),
        local_p2p_node,
        store.clone(),
        blockchain.clone(),
        cancel_token.clone(),
        tracker.clone(),
    );

    init_metrics(&opts, tracker.clone());

    cfg_if::cfg_if! {
        if #[cfg(feature = "dev")] {
            use ethrex::initializers::init_dev_network;

            init_dev_network(&opts, &store, tracker.clone());
        } else {
            use ethrex::initializers::init_network;

            if opts.p2p_enabled {
                init_network(
                    &opts,
                    &network,
                    &data_dir,
                    local_p2p_node,
                    signer,
                    peer_table.clone(),
                    store.clone(),
                    tracker.clone(),
                    blockchain.clone(),
                )
                .await;
            } else {
                info!("P2P is disabled");
            }
        }
    }

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Server shut down started...");
            let peers_file = PathBuf::from(data_dir + "/peers.json");
            info!("Storing known peers at {:?}...", peers_file);
            cancel_token.cancel();
            store_known_peers(peer_table, peers_file).await;
            tokio::time::sleep(Duration::from_secs(1)).await;
            info!("Server shutting down!");
        }
    }

    Ok(())
}
