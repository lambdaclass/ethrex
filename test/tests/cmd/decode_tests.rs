use ethrex::decode::chain_file;
use ethrex_common::H256;
use std::{fs::File, path::PathBuf, str::FromStr as _};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

#[test]
fn decode_chain_file() {
    let file = File::open(workspace_root().join("fixtures/blockchain/chain.rlp"))
        .expect("Failed to open chain file");
    let blocks = chain_file(file).expect("Failed to decode chain file");
    assert_eq!(20, blocks.len(), "There should be 20 blocks in chain file");
    assert_eq!(
        1,
        blocks.first().unwrap().header.number,
        "first block should be number 1"
    );
    // Just checking some block hashes.
    // May add more asserts in the future.
    assert_eq!(
        H256::from_str("0xac5c61edb087a51279674fe01d5c1f65eac3fd8597f9bea215058e745df8088e")
            .unwrap(),
        blocks.first().unwrap().hash(),
        "First block hash does not match"
    );
    assert_eq!(
        H256::from_str("0xa111ce2477e1dd45173ba93cac819e62947e62a63a7d561b6f4825fb31c22645")
            .unwrap(),
        blocks.get(1).unwrap().hash(),
        "Second block hash does not match"
    );
    assert_eq!(
        H256::from_str("0x8f64c4436f7213cfdf02cfb9f45d012f1774dfb329b8803de5e7479b11586902")
            .unwrap(),
        blocks.get(19).unwrap().hash(),
        "Last block hash does not match"
    );
}
