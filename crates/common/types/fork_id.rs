use crc32fast::Hasher;
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

use ethereum_types::H32;

use super::{BlockHash, BlockNumber, ChainConfig};

#[derive(Debug, PartialEq)]
pub struct ForkId {
    fork_hash: H32,
    fork_next: BlockNumber,
}

impl ForkId {
    pub fn new(
        chain_config: ChainConfig,
        genesis_hash: BlockHash,
        head_timestamp: u64,
        head_block_number: u64,
    ) -> Self {
        let (block_number_based_forks, timestamp_based_forks) = chain_config.gather_forks();
        let mut fork_next;
        let mut hasher = Hasher::new();
        // Calculate the starting checksum from the genesis hash
        hasher.update(genesis_hash.as_bytes());

        // Update the checksum with the block number based forks
        fork_next = update_checksum(block_number_based_forks, &mut hasher, head_block_number);
        if fork_next > 0 {
            let fork_hash = H32::from_slice(&hasher.finalize().to_be_bytes());
            return Self {
                fork_hash,
                fork_next,
            };
        }
        // Update the checksum with the timestamp based forks
        fork_next = update_checksum(timestamp_based_forks, &mut hasher, head_timestamp);

        let fork_hash = hasher.finalize();
        let fork_hash = H32::from_slice(&fork_hash.to_be_bytes());
        Self {
            fork_hash,
            fork_next,
        }
    }
}

fn update_checksum(forks: Vec<Option<u64>>, hasher: &mut Hasher, head: u64) -> u64 {
    let mut last_included = 0;

    for activation in forks.into_iter().flatten() {
        if activation <= head {
            if activation != last_included {
                hasher.update(&activation.to_be_bytes());
                last_included = activation;
            }
        } else {
            // fork_next found
            return activation;
        }
    }
    0
}

impl RLPEncode for ForkId {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.fork_hash)
            .encode_field(&self.fork_next)
            .finish();
    }
}

impl RLPDecode for ForkId {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (fork_hash, decoder) = decoder.decode_field("forkHash")?;
        let (fork_next, decoder) = decoder.decode_field("forkNext")?;
        let remaining = decoder.finish()?;
        let fork_id = ForkId {
            fork_hash,
            fork_next,
        };
        Ok((fork_id, remaining))
    }
}

#[cfg(test)]
mod tests {

    use std::str::FromStr;

    use hex_literal::hex;

    use super::*;

    #[test]
    fn encode_fork_id() {
        let fork = ForkId {
            fork_hash: H32::zero(),
            fork_next: 0,
        };
        let expected = hex!("c6840000000080");
        assert_eq!(fork.encode_to_vec(), expected);
    }
    #[test]
    fn encode_fork_id2() {
        let fork = ForkId {
            fork_hash: H32::from_str("0xdeadbeef").unwrap(),
            fork_next: u64::from_str_radix("baddcafe", 16).unwrap(),
        };
        let expected = hex!("ca84deadbeef84baddcafe");
        assert_eq!(fork.encode_to_vec(), expected);
    }
    #[test]
    fn encode_fork_id3() {
        let fork = ForkId {
            fork_hash: H32::from_low_u64_le(u32::MAX.into()),
            fork_next: u64::MAX,
        };
        let expected = hex!("ce84ffffffff88ffffffffffffffff");
        assert_eq!(fork.encode_to_vec(), expected);
    }
}
