use anyhow::Error;
use bytes::Bytes;
use ethrex_core::types::{Block, Genesis};
use ethrex_rlp::decode::RLPDecode as _;
use std::{
    fs::File,
    io::{BufReader, Read as _},
};
pub fn jwtsecret_file(file: &mut File) -> Bytes {
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .expect("Failed to read jwt secret file");
    if contents[0..2] == *"0x" {
        contents = contents[2..contents.len()].to_string();
    }
    hex::decode(contents)
        .expect("Secret should be hex encoded")
        .into()
}
pub fn chain_file(file: File) -> Result<Vec<Block>, Error> {
    let mut chain_rlp_reader = BufReader::new(file);
    let mut buf = vec![];
    chain_rlp_reader.read_to_end(&mut buf)?;
    let mut blocks = Vec::new();
    while !buf.is_empty() {
        let (item, rest) = Block::decode_unfinished(&buf)?;
        blocks.push(item);
        buf = rest.to_vec();
    }
    Ok(blocks)
}

pub fn genesis_file(file: File) -> Result<Genesis, serde_json::Error> {
    let genesis_reader = BufReader::new(file);
    serde_json::from_reader(genesis_reader)
}

#[cfg(test)]
mod tests {
    use crate::decode::chain_file;
    use ethrex_core::H256;
    use std::{fs::File, str::FromStr as _};

    #[test]
    fn decode_chain_file() {
        let file = File::open("../../test_data/chain.rlp").expect("Failed to open chain file");
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
}
