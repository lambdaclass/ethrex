use std::{
    collections::VecDeque,
    sync::{Arc, RwLock},
};

use crate::rlpx::{
    error::PeerConnectionError,
    message::{self as rlpx, EthCapVersion, SnapCapVersion},
    utils::ecdh_xchng,
};

use super::handshake::{LocalState, RemoteState};
use aes::{
    Aes256Enc,
    cipher::{BlockEncrypt as _, KeyInit as _, KeyIvInit, StreamCipher as _},
};
use bytes::{Buf, BytesMut};
use ethrex_common::{
    H128, H256,
    utils::{keccak, truncate_array},
};
use ethrex_crypto::keccak::{Keccak256, keccak_hash};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode as _};
use rustc_hash::FxHashSet;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Decoder, Encoder, Framed};

// max RLPx Message size
// Taken from https://github.com/ethereum/go-ethereum/blob/82e963e5c981e36dc4b607dd0685c64cf4aabea8/p2p/rlpx/rlpx.go#L152
const MAX_MESSAGE_SIZE: u32 = 0xFFFFFF;

type Aes256Ctr64BE = ctr::Ctr64BE<aes::Aes256>;

/// A request id is forgotten once this many newer requests were sent: requests are answered
/// or time out long before that many more go out on one connection.
const MAX_AWAITED_RESPONSES: usize = 1024;

/// Ids of the requests sent on this connection whose responses are matched by id
/// ([`rlpx::Message::routed_request_id`]) and have not arrived yet. The encoder records an id
/// when it sends the request and the decoder consumes it when the response arrives.
#[derive(Default)]
struct AwaitedResponses {
    ids: FxHashSet<u64>,
    order: VecDeque<u64>,
}

impl AwaitedResponses {
    fn insert(&mut self, id: u64) {
        if self.ids.insert(id) {
            self.order.push_back(id);
            if self.order.len() > MAX_AWAITED_RESPONSES
                && let Some(oldest) = self.order.pop_front()
            {
                self.ids.remove(&oldest);
            }
        }
    }

    /// Consumes `id`, returning whether a response with it was awaited.
    fn take(&mut self, id: u64) -> bool {
        self.ids.remove(&id)
    }
}

pub struct RLPxCodec {
    pub(crate) mac_key: H256,
    pub(crate) ingress_mac: Keccak256,
    pub(crate) egress_mac: Keccak256,
    pub(crate) ingress_aes: Aes256Ctr64BE,
    pub(crate) egress_aes: Aes256Ctr64BE,
    pub(crate) eth_version: Arc<RwLock<EthCapVersion>>,
    pub(crate) snap_version: Arc<RwLock<Option<SnapCapVersion>>>,
    awaited_responses: AwaitedResponses,
}

impl RLPxCodec {
    pub(crate) fn new(
        local_state: &LocalState,
        remote_state: &RemoteState,
        hashed_nonces: [u8; 32],
        eth_version: Arc<RwLock<EthCapVersion>>,
        snap_version: Arc<RwLock<Option<SnapCapVersion>>>,
    ) -> Result<Self, PeerConnectionError> {
        let ephemeral_key_secret =
            ecdh_xchng(&local_state.ephemeral_key, &remote_state.ephemeral_key).map_err(
                |error| {
                    PeerConnectionError::CryptographyError(format!(
                        "Invalid generated ephemeral key secret: {error}"
                    ))
                },
            )?;

        // shared-secret = keccak256(ephemeral-key || keccak256(nonce || initiator-nonce))
        let shared_secret = keccak_hash([ephemeral_key_secret, hashed_nonces].concat());
        // aes-secret = keccak256(ephemeral-key || shared-secret)
        let aes_key = keccak([ephemeral_key_secret, shared_secret].concat());
        // mac-secret = keccak256(ephemeral-key || aes-secret)
        let mac_key = keccak([ephemeral_key_secret, aes_key.0].concat());

        // egress-mac = keccak256.init((mac-secret ^ remote-nonce) || auth)
        let egress_mac = Keccak256::default()
            .update(mac_key ^ remote_state.nonce)
            .update(&local_state.init_message);

        // ingress-mac = keccak256.init((mac-secret ^ initiator-nonce) || ack)
        let ingress_mac = Keccak256::default()
            .update(mac_key ^ local_state.nonce)
            .update(&remote_state.init_message);

        let ingress_aes = <Aes256Ctr64BE as KeyIvInit>::new(&aes_key.0.into(), &[0; 16].into());
        let egress_aes = ingress_aes.clone();
        Ok(Self {
            mac_key,
            ingress_mac,
            egress_mac,
            ingress_aes,
            egress_aes,
            eth_version,
            snap_version,
            awaited_responses: AwaitedResponses::default(),
        })
    }
}

// Manual implementation as Aes256Ctr64BE does not implement Debug traits
impl std::fmt::Debug for RLPxCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RLPxCodec")
            .field("mac_key", &self.mac_key)
            .field("ingress_mac", &"ingress_mac")
            .field("egress_mac", &"egress_mac")
            .field("ingress_aes", &"Aes256Ctr64BE")
            .field("egress_aes", &"Aes256Ctr64BE")
            .field("eth_version", &self.eth_version)
            .field("snap_version", &self.snap_version)
            .finish()
    }
}

impl Decoder for RLPxCodec {
    type Item = rlpx::Message;

    type Error = PeerConnectionError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let mac_aes_cipher = Aes256Enc::new_from_slice(&self.mac_key.0)?;

        // Receive the message's frame header
        if src.len() < 32 {
            // Not enough data to read the frame header.
            return Ok(None);
        }
        let mut frame_header = [0; 32];
        frame_header.copy_from_slice(&src[..32]);

        // Both are padded to the block's size (16 bytes)
        let (header_ciphertext, header_mac) =
            frame_header.split_at_mut_checked(16).ok_or_else(|| {
                PeerConnectionError::CryptographyError("Invalid frame header length".to_owned())
            })?;

        // Validate MAC header
        // header-mac-seed = aes(mac-secret, keccak256.digest(egress-mac)[:16]) ^ header-ciphertext
        let header_mac_seed = {
            let mac_digest: [u8; 16] = truncate_array(self.ingress_mac.clone().finalize());
            let mut seed = mac_digest.into();
            mac_aes_cipher.encrypt_block(&mut seed);
            (H128(seed.into())
                ^ H128(header_ciphertext.try_into().map_err(|_| {
                    PeerConnectionError::CryptographyError(
                        "Invalid header ciphertext length".to_owned(),
                    )
                })?))
            .0
        };

        // ingress-mac = keccak256.update(ingress-mac, header-mac-seed)
        // Use temporary value as it can be discarded if the buffer does not contain yet the full message
        let mut temp_ingress_mac = self.ingress_mac.clone();
        temp_ingress_mac.update(header_mac_seed);

        // header-mac = keccak256.digest(egress-mac)[:16]
        let expected_header_mac = H128(truncate_array(temp_ingress_mac.clone().finalize()));

        if header_mac != expected_header_mac.0 {
            return Err(PeerConnectionError::InvalidMessageFrame(
                "Mismatched header mac".to_string(),
            ));
        }

        let header_text = header_ciphertext;
        // Use temporary value as it can be discarded if the buffer does not contain yet the full message
        let mut temp_ingress_aes = self.ingress_aes.clone();
        temp_ingress_aes.try_apply_keystream(header_text)?;

        if header_text.len() < 3 {
            return Err(PeerConnectionError::CryptographyError(
                "Invalid header text length".to_owned(),
            ));
        }

        let frame_size = u32::from_be_bytes([0, header_text[0], header_text[1], header_text[2]]);

        let padded_size = frame_size.next_multiple_of(16);

        // Check that the size is not too large to avoid a denial of
        // service attack where the server runs out of memory.
        if padded_size > MAX_MESSAGE_SIZE {
            return Err(PeerConnectionError::InvalidMessageLength);
        }

        let total_message_size = (32 + padded_size + 16) as usize;

        if src.len() < total_message_size {
            // The full string has not yet arrived.
            //
            // We reserve more space in the buffer. This is not strictly
            // necessary, but is a good idea performance-wise.
            src.reserve(total_message_size - src.len());

            // We inform the Framed that we need more bytes to form the next
            // frame.
            return Ok(None);
        }

        // Use advance to modify src such that it no longer contains
        // this frame.
        let mut frame_data = src
            .get(32..total_message_size)
            .ok_or_else(|| {
                PeerConnectionError::CryptographyError("Invalid frame data length".to_owned())
            })?
            .to_vec();
        src.advance(total_message_size);

        // The buffer contains the full message and will be consumed; update the ingress_mac and aes values
        self.ingress_mac = temp_ingress_mac.clone();
        self.ingress_aes = temp_ingress_aes;

        let (frame_ciphertext, frame_mac) = frame_data
            .split_at_mut_checked(padded_size as usize)
            .ok_or_else(|| {
            PeerConnectionError::CryptographyError("Invalid frame data length".to_owned())
        })?;

        // check MAC
        self.ingress_mac.update(&frame_ciphertext);
        let frame_mac_seed = {
            let mac_digest: [u8; 16] = truncate_array(self.ingress_mac.clone().finalize());
            let mut seed = mac_digest.into();
            mac_aes_cipher.encrypt_block(&mut seed);
            (H128(seed.into()) ^ H128(mac_digest)).0
        };
        self.ingress_mac.update(frame_mac_seed);
        let expected_frame_mac: [u8; 16] = truncate_array(self.ingress_mac.clone().finalize());

        if frame_mac != expected_frame_mac {
            return Err(PeerConnectionError::InvalidMessageFrame(
                "Mismatched frame mac".to_string(),
            ));
        }

        // decrypt frame
        self.ingress_aes.try_apply_keystream(frame_ciphertext)?;

        let (frame_data, _padding) = frame_ciphertext
            .split_at_checked(frame_size as usize)
            .ok_or_else(|| {
                PeerConnectionError::CryptographyError("Invalid frame size".to_owned())
            })?;

        let (msg_id, msg_data): (u8, _) = RLPDecode::decode_unfinished(frame_data)?;
        let eth_ver = *self
            .eth_version
            .read()
            .map_err(|err| PeerConnectionError::InternalError(err.to_string()))?;
        let snap_ver = *self
            .snap_version
            .read()
            .map_err(|err| PeerConnectionError::InternalError(err.to_string()))?;
        // The connection matches a response to its request only after decoding it, and
        // decoding can take tens of times the message's size in memory (a one-byte storage
        // key becomes a 32-byte `U256`). A response to nothing this connection asked for is
        // dropped after reading its id.
        if let Some(id) = rlpx::Message::routed_response_id(msg_id, msg_data, eth_ver, snap_ver)?
            && !self.awaited_responses.take(id)
        {
            return Err(PeerConnectionError::ExpectedRequestId(format!(
                "unrequested response with message id {msg_id} and request id {id}"
            )));
        }
        Ok(Some(rlpx::Message::decode(
            msg_id, msg_data, eth_ver, snap_ver,
        )?))
    }

    fn framed<S: AsyncRead + AsyncWrite + Sized>(self, io: S) -> Framed<S, Self>
    where
        Self: Sized,
    {
        Framed::new(io, self)
    }
}

impl Encoder<rlpx::Message> for RLPxCodec {
    type Error = PeerConnectionError;

    fn encode(&mut self, message: rlpx::Message, buffer: &mut BytesMut) -> Result<(), Self::Error> {
        if let Some(id) = message.routed_request_id() {
            self.awaited_responses.insert(id);
        }
        let mut frame_data = vec![];
        message.encode(
            &mut frame_data,
            *self
                .eth_version
                .read()
                .map_err(|err| PeerConnectionError::InternalError(err.to_string()))?,
        )?;

        // The frame header carries the size in 3 bytes, and peers reject frames above
        // MAX_MESSAGE_SIZE: refuse to send one rather than announce a truncated size.
        if frame_data.len() > MAX_MESSAGE_SIZE as usize {
            return Err(PeerConnectionError::InvalidMessageLength);
        }

        let mac_aes_cipher = Aes256Enc::new_from_slice(&self.mac_key.0)?;

        // header = frame-size || header-data || header-padding
        let mut header = Vec::with_capacity(32);
        let frame_size = frame_data.len().to_be_bytes();
        header.extend_from_slice(frame_size.get(5..8).ok_or_else(|| {
            PeerConnectionError::CryptographyError("Invalid frame size".to_owned())
        })?);

        // header-data = [capability-id, context-id]  (both always zero)
        let header_data = (0_u8, 0_u8);
        header_data.encode(&mut header);

        header.resize(16, 0);
        self.egress_aes
            .try_apply_keystream(header.get_mut(..16).ok_or_else(|| {
                PeerConnectionError::CryptographyError("Invalid header length".to_owned())
            })?)?;

        let header_mac_seed = {
            let mac_digest: [u8; 16] = truncate_array(self.egress_mac.clone().finalize());
            let mut seed = mac_digest.into();
            mac_aes_cipher.encrypt_block(&mut seed);
            let header_data = header
                .get(..16)
                .ok_or_else(|| {
                    PeerConnectionError::CryptographyError("Invalid header length".to_owned())
                })?
                .try_into()
                .map_err(|_| {
                    PeerConnectionError::CryptographyError("Invalid header length".to_owned())
                })?;
            H128(seed.into()) ^ H128(header_data)
        };
        self.egress_mac.update(header_mac_seed);
        let header_mac = self.egress_mac.clone().finalize();
        let header_mac_data: [u8; 16] = truncate_array(header_mac);
        header.extend_from_slice(&header_mac_data);

        // Write header
        buffer.extend_from_slice(&header);

        // Pad to next multiple of 16
        frame_data.resize(frame_data.len().next_multiple_of(16), 0);
        self.egress_aes.try_apply_keystream(&mut frame_data)?;
        let frame_ciphertext = frame_data;

        // Write frame
        buffer.extend_from_slice(&frame_ciphertext);

        // Compute frame-mac
        self.egress_mac.update(&frame_ciphertext);

        // frame-mac-seed = aes(mac-secret, keccak256.digest(egress-mac)[:16]) ^ keccak256.digest(egress-mac)[:16]
        let frame_mac_seed = {
            let mac_digest: [u8; 16] = truncate_array(self.egress_mac.clone().finalize());
            let mut seed = mac_digest.into();
            mac_aes_cipher.encrypt_block(&mut seed);
            (H128(seed.into()) ^ H128(mac_digest)).0
        };
        self.egress_mac.update(frame_mac_seed);
        let frame_mac = self.egress_mac.clone().finalize();

        // Write frame-mac
        buffer.extend_from_slice(&frame_mac[..16]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rlpx::{
        eth::{
            block_access_lists::{BlockAccessLists, GetBlockAccessLists},
            blocks::BlockHeaders,
        },
        message::{Message, RLPxMessage as _},
        snap::{Snap2BlockAccessLists, Snap2GetBlockAccessLists},
        utils::snappy_compress,
    };
    use ethrex_common::{
        H512,
        types::block_access_list::{AccountChanges, BlockAccessList},
    };
    use ethrex_rlp::structs::Encoder as RlpEncoder;
    use secp256k1::SecretKey;

    /// Both ends of one connection, speaking eth/71 and snap/2.
    fn codec_pair() -> (RLPxCodec, RLPxCodec) {
        let key_a = SecretKey::from_byte_array(&[1; 32]).unwrap();
        let key_b = SecretKey::from_byte_array(&[2; 32]).unwrap();
        let (nonce_a, nonce_b) = (H256([3; 32]), H256([4; 32]));
        let (init_a, init_b) = (vec![5; 8], vec![6; 8]);
        let hashed_nonces = keccak_hash([nonce_b.0, nonce_a.0].concat());
        let codec = |local_key: SecretKey,
                     local_nonce,
                     local_init: &[u8],
                     remote_key: SecretKey,
                     remote_nonce,
                     remote_init: &[u8]| {
            RLPxCodec::new(
                &LocalState {
                    nonce: local_nonce,
                    ephemeral_key: local_key,
                    init_message: local_init.to_vec(),
                },
                &RemoteState {
                    public_key: H512::zero(),
                    nonce: remote_nonce,
                    ephemeral_key: remote_key.public_key(secp256k1::SECP256K1),
                    init_message: remote_init.to_vec(),
                },
                hashed_nonces,
                Arc::new(RwLock::new(EthCapVersion::V71)),
                Arc::new(RwLock::new(Some(SnapCapVersion::V2))),
            )
            .unwrap()
        };
        (
            codec(key_a, nonce_a, &init_a, key_b, nonce_b, &init_b),
            codec(key_b, nonce_b, &init_b, key_a, nonce_a, &init_a),
        )
    }

    fn deliver(
        from: &mut RLPxCodec,
        to: &mut RLPxCodec,
        message: Message,
    ) -> Result<Option<Message>, PeerConnectionError> {
        let mut wire = BytesMut::new();
        from.encode(message, &mut wire).unwrap();
        to.decode(&mut wire)
    }

    fn bals() -> Vec<Option<BlockAccessList>> {
        let account = AccountChanges::new(ethrex_common::Address::from_low_u64_be(1));
        vec![Some(BlockAccessList::from_accounts(vec![account])), None]
    }

    /// A response SHALL be decoded only if this connection sent the request it answers, and
    /// only once.
    #[test]
    fn responses_are_decoded_only_for_requests_this_connection_sent() {
        let (mut a, mut b) = codec_pair();

        let unrequested = Message::BlockAccessLists(BlockAccessLists::new(7, bals()));
        assert!(matches!(
            deliver(&mut b, &mut a, unrequested),
            Err(PeerConnectionError::ExpectedRequestId(_))
        ));

        let request = Message::GetBlockAccessLists(GetBlockAccessLists {
            id: 7,
            block_hashes: vec![H256::zero(); 2],
        });
        assert!(matches!(
            deliver(&mut a, &mut b, request),
            Ok(Some(Message::GetBlockAccessLists(_)))
        ));
        let response = || Message::BlockAccessLists(BlockAccessLists::new(7, bals()));
        match deliver(&mut b, &mut a, response()) {
            Ok(Some(Message::BlockAccessLists(message))) => {
                assert_eq!(message.id, 7);
                assert_eq!(message.block_access_lists, bals());
            }
            other => panic!("the requested response must decode, got {other:?}"),
        }
        assert!(matches!(
            deliver(&mut b, &mut a, response()),
            Err(PeerConnectionError::ExpectedRequestId(_))
        ));
    }

    /// The snap/2 response takes the same path.
    #[test]
    fn snap2_responses_are_decoded_only_when_requested() {
        let (mut a, mut b) = codec_pair();
        let response = || {
            Message::Snap2BlockAccessLists(Snap2BlockAccessLists {
                id: 9,
                bals: bals(),
            })
        };
        assert!(matches!(
            deliver(&mut b, &mut a, response()),
            Err(PeerConnectionError::ExpectedRequestId(_))
        ));
        let request = Message::Snap2GetBlockAccessLists(Snap2GetBlockAccessLists {
            id: 9,
            block_hashes: vec![H256::zero(); 2],
            response_bytes: 2 * 1024 * 1024,
        });
        deliver(&mut a, &mut b, request).unwrap();
        assert!(matches!(
            deliver(&mut b, &mut a, response()),
            Ok(Some(Message::Snap2BlockAccessLists(_)))
        ));
    }

    /// Reading a response's id SHALL NOT decode its body: a body that does not decode still
    /// yields the id.
    #[test]
    fn a_response_id_is_read_without_decoding_the_body() {
        let mut encoded = Vec::new();
        RlpEncoder::new(&mut encoded)
            .encode_field(&7u64)
            .encode_field(&bytes::Bytes::from_static(
                b"not a list of block access lists",
            ))
            .finish();
        let data = snappy_compress(encoded).unwrap();
        let code = EthCapVersion::V71.eth_capability_offset() + BlockAccessLists::CODE;

        assert!(Message::decode(code, &data, EthCapVersion::V71, None).is_err());
        assert_eq!(
            Message::routed_response_id(code, &data, EthCapVersion::V71, None).unwrap(),
            Some(7)
        );
    }

    /// Only a response the negotiated capabilities can carry SHALL be read as a routed
    /// response: block access lists from eth/71, and snap/2 access lists only over snap/2.
    #[test]
    fn routed_responses_follow_the_negotiated_capabilities() {
        let mut encoded = Vec::new();
        RlpEncoder::new(&mut encoded)
            .encode_field(&7u64)
            .encode_field(&Vec::<u8>::new())
            .finish();
        let data = snappy_compress(encoded).unwrap();
        let id = |code: u8, eth: EthCapVersion, snap: Option<SnapCapVersion>| {
            Message::routed_response_id(code, &data, eth, snap).unwrap()
        };

        let headers = |eth: EthCapVersion| eth.eth_capability_offset() + BlockHeaders::CODE;
        assert_eq!(
            id(headers(EthCapVersion::V68), EthCapVersion::V68, None),
            Some(7)
        );

        let bals = |eth: EthCapVersion| eth.eth_capability_offset() + BlockAccessLists::CODE;
        assert_eq!(
            id(bals(EthCapVersion::V71), EthCapVersion::V71, None),
            Some(7)
        );
        assert_eq!(id(bals(EthCapVersion::V70), EthCapVersion::V70, None), None);

        let snap_bals = EthCapVersion::V71.snap_capability_offset() + Snap2BlockAccessLists::CODE;
        let v71 = EthCapVersion::V71;
        assert_eq!(id(snap_bals, v71, Some(SnapCapVersion::V2)), Some(7));
        assert_eq!(id(snap_bals, v71, Some(SnapCapVersion::V1)), None);
        assert_eq!(id(snap_bals, v71, None), None);
    }

    #[test]
    fn the_oldest_awaited_id_is_forgotten_past_the_limit() {
        let mut awaited = AwaitedResponses::default();
        for id in 0..=MAX_AWAITED_RESPONSES as u64 {
            awaited.insert(id);
        }
        assert!(!awaited.take(0));
        assert!(awaited.take(MAX_AWAITED_RESPONSES as u64));
        assert!(awaited.take(1));
    }
}
