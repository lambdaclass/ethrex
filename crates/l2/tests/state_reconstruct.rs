#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::as_conversions)]

use ethrex_common::{Address, H160, U256};
use ethrex_l2_common::utils::get_address_from_secret_key;
use ethrex_rpc::{EthClient, types::block_identifier::BlockIdentifier};

use reqwest::Url;
use secp256k1::SecretKey;

const ETH_RPC_URL: &str = "http://localhost:1729";

use std::fs;

// This test verifies the correct reconstruction of the L2 state from data blobs.

// Test Data:
// - The test uses 5 pre-generated data blobs located under /fixtures/blobs/
// - Each blob contains a batch of blocks with specific deposit transactions:
//
// Blob Contents:
// 1. blob_1: Batch of a single empty block (block 1)
// 2. blob_2: Batch of blocks 2 through 6
// 3. blob_3: Batch of blocks 7 through 11
// 4. blob_4: Batch of blocks 12 through 16
// 5. blob_5: Batch of blocks 17 through 21
//
// - Each block contains exactly 10 deposit transactions
#[tokio::test]
async fn test_state_reconstruct() {
    let pks_path = std::env::var("PRIVATE_KEYS_PATH")
        .unwrap_or("../../fixtures/keys/private_keys_l1.txt".to_string());
    let pks = fs::read_to_string(&pks_path).unwrap();
    let private_keys: Vec<String> = pks
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .collect::<Vec<_>>();

    let addresses: Vec<Address> = private_keys
        .iter()
        .map(|pk| {
            let secret_key = pk
                .strip_prefix("0x")
                .unwrap_or(pk)
                .parse::<SecretKey>()
                .unwrap();
            get_address_from_secret_key(&secret_key.secret_bytes()).unwrap()
        })
        .collect::<Vec<_>>();

    // TODO: Historical state is not supported in the DB currently by the client.
    // This is due to the newest path-based trie implementation.
    // A potential fix would be to store the historical state in the DB through
    // diff layers. The commented tests below make no sense until then.
    //
    // test_state_block(&addresses, 0, 0).await;
    // test_state_block(&addresses, 6, 50).await;
    // test_state_block(&addresses, 11, 100).await;
    // test_state_block(&addresses, 16, 150).await;
    test_state_block(&addresses, 29, addresses.len() as u64).await;
}

async fn test_state_block(addresses: &[Address], block_number: u64, rich_accounts: u64) {
    let client = connect().await;

    for (index, address) in addresses.iter().enumerate() {
        let balance = client
            .get_balance(*address, BlockIdentifier::Number(block_number))
            .await
            .expect("Error getting balance");
        if index < rich_accounts as usize {
            // The bridge owner accept the ownership transfer, so the balance is not exactly 500000000000000000000000000
            if *address
                == H160::from_slice(
                    &hex::decode("4417092b70a3e5f10dc504d0947dd256b965fc62").unwrap(),
                )
            {
                assert!(balance > U256::zero(), "Bridge owner has zero balance");
                continue;
            }
            assert_eq!(
                balance,
                U256::from_dec_str("500000000000000000000000000").unwrap(),
                "Balance mismatch for address {address:#x} at block {block_number}. Expected 500000000000000000000000000, got {balance}"
            );
        } else {
            assert_eq!(
                balance,
                U256::zero(),
                "Balance should be zero for address {address:#x} at block {block_number}. Expected 0, got {balance}"
            );
        }
    }
}

async fn connect() -> EthClient {
    let client = EthClient::new(Url::parse(ETH_RPC_URL).unwrap()).unwrap();

    let mut retries = 0;
    while retries < 20 {
        match client.get_block_number().await {
            Ok(_) => return client,
            Err(_) => {
                println!("Couldn't get block number. Retries: {retries}");
                retries += 1;
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }

    panic!("Couldn't connect to the RPC server")
}
