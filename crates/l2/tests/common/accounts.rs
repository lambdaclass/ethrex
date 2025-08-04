use std::sync::{LazyLock};

use secp256k1::SecretKey;
use tokio::sync::Mutex;

pub const PRIVATE_KEYS_FILE_PATH: &str = "../../fixtures/keys/private_keys_l1.txt";

static ACCOUNTS: LazyLock<Mutex<Vec<SecretKey>>> = LazyLock::new(|| {
    Mutex::new(
        std::fs::read_to_string(PRIVATE_KEYS_FILE_PATH)
            .unwrap()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.trim().to_string())
            .map(|hex| hex.trim_start_matches("0x").to_string())
            .map(|trimmed| hex::decode(trimmed).unwrap())
            .map(|decoded| SecretKey::from_slice(&decoded).unwrap())
            .collect(),
    )
});

pub async fn get_rich_account() -> SecretKey {
    let mut accounts = ACCOUNTS.lock().await;
    accounts.pop().expect("not enough rich accounts")
}
