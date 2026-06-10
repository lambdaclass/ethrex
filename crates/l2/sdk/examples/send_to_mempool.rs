//! Submits signed EIP-1559 eth-transfers to a node's mempool without waiting
//! for inclusion. Used to give `testing_buildBlockV1` real transactions to
//! build a non-empty block (and a non-trivial Block Access List) from.
//!
//! Usage:
//!   cargo run -p ethrex-sdk --example send_to_mempool -- <RPC_URL> <PRIVATE_KEY> <COUNT>

use ethereum_types::{H160, U256};
use ethrex_common::types::TxType;
use ethrex_l2_rpc::signer::{LocalSigner, Signer};
use ethrex_l2_sdk::{build_generic_tx, send_generic_transaction};
use ethrex_rpc::clients::{EthClient, Overrides};
use ethrex_rpc::types::block_identifier::{BlockIdentifier, BlockTag};
use secp256k1::SecretKey;
use std::env;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    let url = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "http://localhost:8545".into());
    let pkey = args.get(2).ok_or("private key (0x-prefixed) required as arg 2")?;
    let count: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);

    let client = EthClient::new(url.parse()?)?;
    let chain_id = client.get_chain_id().await?.as_u64();

    let key_bytes = hex::decode(pkey.strip_prefix("0x").unwrap_or(pkey))?;
    let signer: Signer = LocalSigner::new(SecretKey::from_slice(&key_bytes)?).into();
    let from = signer.address();

    let start_nonce = client
        .get_nonce(from, BlockIdentifier::Tag(BlockTag::Pending))
        .await?;

    for i in 0..count {
        let dst = H160::random();
        let tx = build_generic_tx(
            &client,
            TxType::EIP1559,
            dst,
            from,
            Default::default(),
            Overrides {
                chain_id: Some(chain_id),
                value: Some(U256::from(1_000_000_000_000u64)),
                nonce: Some(start_nonce + i),
                max_fee_per_gas: Some(i64::MAX as u64),
                max_priority_fee_per_gas: Some(10),
                gas_limit: Some(21_000),
                ..Default::default()
            },
        )
        .await?;
        let hash = send_generic_transaction(&client, tx, &signer).await?;
        println!("sent nonce {} -> {dst:#x} : {hash:#x}", start_nonce + i);
    }
    println!("submitted {count} tx(s) from {from:#x} (chain_id {chain_id})");
    Ok(())
}
