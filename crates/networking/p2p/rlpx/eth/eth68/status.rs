use crate::rlpx::{
    error::PeerConnectionError,
    eth::status::StatusMessage,
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use ethrex_common::{
    U256,
    types::{BlockHash, ForkId},
};
use ethrex_storage::Store;
use librlp::{Header, RlpBuf, RlpDecode, RlpEncode, RlpError};

#[derive(Debug, Clone)]
pub struct StatusMessage68 {
    pub(crate) eth_version: u8,
    pub(crate) network_id: u64,
    pub(crate) total_difficulty: U256,
    pub(crate) block_hash: BlockHash,
    pub(crate) genesis: BlockHash,
    pub(crate) fork_id: ForkId,
}

impl RLPxMessage for StatusMessage68 {
    const CODE: u8 = 0x00;
    fn encode(&self, buf: &mut Vec<u8>) -> Result<(), snap::Error> {
        let mut rlp_buf = RlpBuf::new();
        rlp_buf.list(|buf| {
            self.eth_version.encode(buf);
            self.network_id.encode(buf);
            self.total_difficulty.encode(buf);
            self.block_hash.encode(buf);
            self.genesis.encode(buf);
            self.fork_id.encode(buf);
        });
        let msg_data = snappy_compress(rlp_buf.finish())?;
        buf.extend_from_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RlpError> {
        let decompressed_data =
            snappy_decompress(msg_data).map_err(|e| RlpError::Custom(e.to_string().into()))?;
        let mut buf = decompressed_data.as_slice();
        let header = Header::decode(&mut buf)?;
        if !header.list {
            return Err(RlpError::UnexpectedString);
        }
        let mut payload = &buf[..header.payload_length];
        let eth_version = u32::decode(&mut payload)?;

        if eth_version != 68 {
            return Err(RlpError::Custom(
                format!(
                    "incompatible protocol: Received message is encoded in eth version {} when negotiated eth version was 68",
                    eth_version
                )
                .into(),
            ));
        }

        let network_id = u64::decode(&mut payload)?;
        let total_difficulty = U256::decode(&mut payload)?;
        let block_hash = BlockHash::decode(&mut payload)?;
        let genesis = BlockHash::decode(&mut payload)?;
        let fork_id = ForkId::decode(&mut payload)?;
        // Implementations must ignore any additional list elements

        Ok(Self {
            eth_version: eth_version as u8,
            network_id,
            total_difficulty,
            block_hash,
            genesis,
            fork_id,
        })
    }
}

impl StatusMessage68 {
    pub async fn new(storage: &Store) -> Result<Self, PeerConnectionError> {
        let chain_config = storage.get_chain_config();
        let total_difficulty =
            U256::from(chain_config.terminal_total_difficulty.unwrap_or_default());
        let network_id = chain_config.chain_id;

        // These blocks must always be available
        let genesis_header = storage
            .get_block_header(0)?
            .ok_or(PeerConnectionError::NotFound("Genesis Block".to_string()))?;
        let lastest_block = storage.get_latest_block_number().await?;
        let block_header =
            storage
                .get_block_header(lastest_block)?
                .ok_or(PeerConnectionError::NotFound(format!(
                    "Block {lastest_block}"
                )))?;

        let genesis = genesis_header.hash();
        let lastest_block_hash = block_header.hash();
        let fork_id = ForkId::new(
            chain_config,
            genesis_header,
            block_header.timestamp,
            lastest_block,
        );

        Ok(StatusMessage68 {
            eth_version: 68,
            network_id,
            total_difficulty,
            block_hash: lastest_block_hash,
            genesis,
            fork_id,
        })
    }
}

impl StatusMessage for StatusMessage68 {
    fn get_network_id(&self) -> u64 {
        self.network_id
    }

    fn get_eth_version(&self) -> u8 {
        self.eth_version
    }

    fn get_fork_id(&self) -> ForkId {
        self.fork_id.clone()
    }

    fn get_genesis(&self) -> BlockHash {
        self.genesis
    }
}
