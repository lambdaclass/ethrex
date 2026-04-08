/// BSC-specific eth protocol extensions.
///
/// After the standard eth status exchange, BSC peers send an `UpgradeStatusMsg`
/// (message code 0x0b on the eth sub-protocol). Failing to handle this message
/// causes BSC peers to disconnect.
///
/// Reference: https://github.com/bnb-chain/bsc/blob/master/eth/protocols/eth/protocol.go
use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use bytes::BufMut;
use ethrex_rlp::{
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};

/// BSC `UpgradeStatusMsg` — message code 0x0b on the eth sub-protocol.
///
/// Sent by BSC peers immediately after the eth status exchange.
///
/// Wire format (snappy-compressed RLP):
/// ```text
/// UpgradeStatusPacket {
///     Extension UpgradeStatusExtension
/// }
/// UpgradeStatusExtension {
///     DisablePeerTxBroadcast bool
/// }
/// ```
/// This encodes as `[[disable_peer_tx_broadcast]]` in RLP.
///
/// Reference: `eth/protocols/eth/protocol.go` lines 102-110,
///            `eth/protocols/eth/handshake.go` lines 92-96.
#[derive(Debug, Clone, Default)]
pub struct UpgradeStatusMsg {
    /// Whether the sender has disabled peer transaction broadcast.
    pub disable_peer_tx_broadcast: bool,
}

impl RLPxMessage for UpgradeStatusMsg {
    const CODE: u8 = 0x0b;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        // Encode the inner UpgradeStatusExtension list: [disable_peer_tx_broadcast]
        let mut extension_buf = vec![];
        Encoder::new(&mut extension_buf)
            .encode_field(&self.disable_peer_tx_broadcast)
            .finish();

        // Encode the outer UpgradeStatusPacket list: [[disable_peer_tx_broadcast]]
        // using encode_raw to embed the already-RLP-encoded extension as a nested list.
        let mut outer_buf = vec![];
        Encoder::new(&mut outer_buf)
            .encode_raw(&extension_buf)
            .finish();

        let msg_data = snappy_compress(outer_buf)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed = snappy_decompress(msg_data)?;

        // Outer list: UpgradeStatusPacket
        let outer_decoder = Decoder::new(&decompressed)?;

        // Inner list: UpgradeStatusExtension
        // decode_field here will decode the next RLP item as the extension struct.
        // Since UpgradeStatusExtension is itself a list [bool], we decode it via
        // a nested Decoder.
        let (extension_bytes, _outer_decoder) = outer_decoder.get_encoded_item()?;

        // Decode the inner list [disable_peer_tx_broadcast]
        let inner_decoder = Decoder::new(&extension_bytes)?;
        let (disable_peer_tx_broadcast, _inner_decoder) =
            inner_decoder.decode_field::<bool>("disablePeerTxBroadcast")?;

        Ok(Self {
            disable_peer_tx_broadcast,
        })
    }
}
