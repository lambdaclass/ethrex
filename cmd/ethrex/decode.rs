use anyhow::Error;
use bytes::Bytes;
use ethrex_common::types::Block;
use ethrex_rlp::decode::RLPDecode as _;
use std::{
    fs::File,
    io::{BufReader, Read as _},
};
pub fn jwtsecret_file(file: &mut File) -> Bytes {
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .expect("Failed to read jwt secret file");
    contents = contents
        .strip_prefix("0x")
        .unwrap_or(&contents)
        .trim_end_matches('\n')
        .to_string();
    hex::decode(contents)
        .expect("Secret should be hex encoded")
        .into()
}
pub fn chain_file(file: File) -> Result<Vec<Block>, Error> {
    let mut chain_rlp_reader = BufReader::new(file);
    let mut buf = vec![];
    chain_rlp_reader.read_to_end(&mut buf)?;
    let mut buf = buf.as_slice();
    let mut blocks = Vec::new();
    while !buf.is_empty() {
        let (item, rest) = Block::decode_unfinished(buf)?;
        blocks.push(item);
        buf = rest;
    }
    Ok(blocks)
}
