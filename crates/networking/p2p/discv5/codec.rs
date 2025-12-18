use crate::discv5::messages::{Packet, PacketCodecError};
use crate::discv5::session::Session;

use bytes::BytesMut;
use ethrex_common::H256;
use rand::{Rng, RngCore, thread_rng};
use tokio_util::codec::{Decoder, Encoder};

#[derive(Debug)]
pub struct Discv5Codec {
    /// Local node id, used to decode incoming Packets
    local_node_id: H256,
    /// Outgoing message count, used for nonce generation as per the spec.
    counter: u32,
    session: Option<Session>,
}

impl Discv5Codec {
    pub fn new(dest_id: H256) -> Self {
        Self {
            local_node_id: dest_id,
            counter: 0,
            session: None,
        }
    }

    pub fn with_session(dest_id: H256, session: Session) -> Self {
        Self {
            local_node_id: dest_id,
            counter: 0,
            session: Some(session),
        }
    }

    pub fn set_session(&mut self, session: Session) {
        self.session = Some(session);
    }

    /// Generates a 96-bit AES-GCM nonce
    /// ## Spec Recommendation
    /// Encode the current outgoing message count into the first 32 bits of the nonce and fill the remaining 64 bits with random data generated
    /// by a cryptographically secure random number generator.
    pub fn next_nonce<R: RngCore>(&mut self, rng: &mut R) -> [u8; 12] {
        let counter = self.counter;
        self.counter = self.counter.wrapping_add(1);

        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&counter.to_be_bytes());
        rng.fill_bytes(&mut nonce[4..]);
        nonce
    }
}

impl Decoder for Discv5Codec {
    type Item = Packet;
    type Error = PacketCodecError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if !buf.is_empty() {
            let key: &[u8] = match &self.session {
                Some(session) => session.inbound_key(),
                None => &[],
            };
            Ok(Some(Packet::decode(
                &self.local_node_id,
                key,
                &buf.split_to(buf.len()),
            )?))
        } else {
            Ok(None)
        }
    }
}

impl Encoder<Packet> for Discv5Codec {
    type Error = PacketCodecError;

    fn encode(&mut self, packet: Packet, buf: &mut BytesMut) -> Result<(), Self::Error> {
        let mut rng = thread_rng();
        let masking_iv: u128 = rng.r#gen();
        let nonce = self.next_nonce(&mut rng);
        // TODO:
        // - We need to receive remote node dest_id in order to be able to obtain session data (also used for encoding later)
        //   Probably use a Packet wrapper struct that includes it.
        // - With dest_id, we fetch Session data from peer_table
        //   If no session is present, or WhoAreYou, we just use a random key
        // - We need to save the message by nonce, as it can be used to identify dest_id from a future WhoAreYou incoming messages
        //
        // key isnt needed in WHOAREYOU packets
        let key = match (&packet, &mut self.session) {
            (Packet::WhoAreYou(_), _) => &[][..],
            (_, Some(session)) => session.outbound_key(),
            (_, None) => return Err(PacketCodecError::SessionNotEstablished),
        };
        // FIX: we have to use remote dest_id here instead of self.local_node_id
        packet.encode(buf, masking_iv, &nonce, &self.local_node_id, key)
    }
}
