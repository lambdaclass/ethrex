#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::as_conversions)]

use ethrex_common::{Address, U256};
use ethrex_l2_sdk::get_address_from_secret_key;
use ethrex_rpc::{EthClient, types::block_identifier::BlockIdentifier};

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
            get_address_from_secret_key(&secret_key).unwrap()
        })
        .collect::<Vec<_>>();

    test_state_block(&addresses, 0, 0).await;
    test_state_block(&addresses, 6, 50).await;
    test_state_block(&addresses, 11, 100).await;
    test_state_block(&addresses, 16, 150).await;
    test_state_block(&addresses, 21, addresses.len() as u64).await;
}

async fn test_state_block(addresses: &[Address], block_number: u64, rich_accounts: u64) {
    let client = connect().await;

    for (index, address) in addresses.iter().enumerate() {
        let balance = client
            .get_balance(*address, BlockIdentifier::Number(block_number))
            .await
            .expect("Error getting balance");
        if index < rich_accounts as usize {
            assert_eq!(
                balance,
                U256::from_dec_str("500000000000000000000000000").unwrap()
            );
        } else {
            assert_eq!(balance, U256::zero());
        }
    }
}

async fn connect() -> EthClient {
    let client = EthClient::new(ETH_RPC_URL).unwrap();

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
