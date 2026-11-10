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
    let secret = hex::decode(contents).expect("Secret should be hex encoded");
    // The Engine API spec fixes the secret at 256 bits. A shorter one, down to an
    // empty file, would be accepted as an HMAC key that is weak or empty.
    assert!(
        secret.len() == JWT_SECRET_LEN,
        "JWT secret must be {JWT_SECRET_LEN} bytes ({} hex characters), got {} bytes",
        JWT_SECRET_LEN * 2,
        secret.len()
    );
    secret.into()
}

/// Length in bytes of the Engine API JWT secret.
pub const JWT_SECRET_LEN: usize = 32;
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
