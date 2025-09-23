use std::sync::{Arc, RwLock};

use crate::rlpx::{
    error::RLPxError,
    message::{self as rlpx, EthCapVersion},
    utils::ecdh_xchng,
};

use super::handshake::{LocalState, RemoteState};
use aes::{
    Aes256Enc,
    cipher::{BlockEncrypt as _, KeyInit as _, KeyIvInit, StreamCipher as _},
};
use bytes::{Buf, BytesMut};
use ethrex_common::{H128, H256};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode as _};
use sha3::{Digest as _, Keccak256};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Decoder, Encoder, Framed};

// max RLPx Message size
// Taken from https://github.com/ethereum/go-ethereum/blob/82e963e5c981e36dc4b607dd0685c64cf4aabea8/p2p/rlpx/rlpx.go#L152
const MAX_MESSAGE_SIZE: u32 = 0xFFFFFF;
const RLPX_HEADER_SIZE: usize = 32;
const RLPX_MAC_SIZE: usize = 16;
const AES_BLOCK_SIZE: usize = 16;

type Aes256Ctr64BE = ctr::Ctr64BE<aes::Aes256>;

pub struct RLPxCodec {
    pub(crate) mac_key: H256,
    pub(crate) ingress_mac: Keccak256,
    pub(crate) egress_mac: Keccak256,
    pub(crate) ingress_aes: Aes256Ctr64BE,
    pub(crate) egress_aes: Aes256Ctr64BE,
    pub(crate) eth_version: Arc<RwLock<EthCapVersion>>,
}

impl RLPxCodec {
    pub(crate) fn new(
        local_state: &LocalState,
        remote_state: &RemoteState,
        hashed_nonces: [u8; 32],
        eth_version: Arc<RwLock<EthCapVersion>>,
    ) -> Result<Self, RLPxError> {
        let ephemeral_key_secret = ecdh_xchng(
            &local_state.ephemeral_key,
            &remote_state.ephemeral_key,
        )
        .map_err(|error| {
            RLPxError::CryptographyError(format!("Invalid generated ephemeral key secret: {error}"))
        })?;

        // shared-secret = keccak256(ephemeral-key || keccak256(nonce || initiator-nonce))
        let shared_secret =
            Keccak256::digest([ephemeral_key_secret, hashed_nonces].concat()).into();
        // aes-secret = keccak256(ephemeral-key || shared-secret)
        let aes_key =
            H256(Keccak256::digest([ephemeral_key_secret, shared_secret].concat()).into());
        // mac-secret = keccak256(ephemeral-key || aes-secret)
        let mac_key = H256(Keccak256::digest([ephemeral_key_secret, aes_key.0].concat()).into());

        // egress-mac = keccak256.init((mac-secret ^ remote-nonce) || auth)
        let egress_mac = Keccak256::default()
            .chain_update(mac_key ^ remote_state.nonce)
            .chain_update(&local_state.init_message);

        // ingress-mac = keccak256.init((mac-secret ^ initiator-nonce) || ack)
        let ingress_mac = Keccak256::default()
            .chain_update(mac_key ^ local_state.nonce)
            .chain_update(&remote_state.init_message);

        let ingress_aes =
            <Aes256Ctr64BE as KeyIvInit>::new(&aes_key.0.into(), &[0; AES_BLOCK_SIZE].into());
        let egress_aes = ingress_aes.clone();
        Ok(Self {
            mac_key,
            ingress_mac,
            egress_mac,
            ingress_aes,
            egress_aes,
            eth_version,
        })
    }
}

// Manual implementation as Aes256Ctr64BE does not implement Debug traits
impl std::fmt::Debug for RLPxCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RLPxCodec")
            .field("mac_key", &self.mac_key)
            .field("ingress_mac", &self.ingress_mac)
            .field("egress_mac", &self.egress_mac)
            .field("ingress_aes", &"Aes256Ctr64BE")
            .field("egress_aes", &"Aes256Ctr64BE")
            .field("eth_version", &self.eth_version)
            .finish()
    }
}

impl Decoder for RLPxCodec {
    type Item = rlpx::Message;

    type Error = RLPxError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let mac_aes_cipher = Aes256Enc::new_from_slice(&self.mac_key.0)?;

        // Receive the message's frame header
        if src.len() < RLPX_HEADER_SIZE {
            // Not enough data to read the frame header.
            return Ok(None);
        }
        let mut frame_header = [0; RLPX_HEADER_SIZE];
        frame_header.copy_from_slice(&src[..RLPX_HEADER_SIZE]);

        // Both are padded to the block's size (16 bytes)
        let (header_ciphertext, header_mac) =
            frame_header.split_at_mut_checked(16).ok_or_else(|| {
                RLPxError::CryptographyError("Invalid frame header length".to_owned())
            })?;

        // Validate MAC header
        // header-mac-seed = aes(mac-secret, keccak256.digest(egress-mac)[:16]) ^ header-ciphertext
        let header_mac_seed = {
            let mac_digest: [u8; 16] = self
                .ingress_mac
                .clone()
                .finalize()
                .get(..16)
                .ok_or_else(|| RLPxError::CryptographyError("Invalid mac digest".to_owned()))?
                .try_into()
                .map_err(|_| RLPxError::CryptographyError("Invalid mac digest".to_owned()))?;
            let mut seed = mac_digest.into();
            mac_aes_cipher.encrypt_block(&mut seed);
            (H128(seed.into())
                ^ H128(header_ciphertext.try_into().map_err(|_| {
                    RLPxError::CryptographyError("Invalid header ciphertext length".to_owned())
                })?))
            .0
        };

        // ingress-mac = keccak256.update(ingress-mac, header-mac-seed)
        // Use temporary value as it can be discarded if the buffer does not contain yet the full message
        let mut temp_ingress_mac = self.ingress_mac.clone();
        temp_ingress_mac.update(header_mac_seed);

        // header-mac = keccak256.digest(egress-mac)[:16]
        let expected_header_mac = H128(
            temp_ingress_mac
                .clone()
                .finalize()
                .get(..16)
                .ok_or_else(|| RLPxError::CryptographyError("Invalid header mac".to_owned()))?
                .try_into()
                .map_err(|_| RLPxError::CryptographyError("Invalid header mac".to_owned()))?,
        );

        if header_mac != expected_header_mac.0 {
            return Err(RLPxError::InvalidMessageFrame(
                "Mismatched header mac".to_string(),
            ));
        }

        let header_text = header_ciphertext;
        // Use temporary value as it can be discarded if the buffer does not contain yet the full message
        let mut temp_ingress_aes = self.ingress_aes.clone();
        temp_ingress_aes.try_apply_keystream(header_text)?;

        if header_text.len() < 3 {
            return Err(RLPxError::CryptographyError(
                "Invalid header text length".to_owned(),
            ));
        }

        let frame_size = u32::from_be_bytes([0, header_text[0], header_text[1], header_text[2]]);

        let padded_size = frame_size.next_multiple_of(AES_BLOCK_SIZE as u32);

        // Check that the size is not too large to avoid a denial of
        // service attack where the server runs out of memory.
        if padded_size > MAX_MESSAGE_SIZE {
            return Err(RLPxError::InvalidMessageLength());
        }

        let total_message_size = RLPX_HEADER_SIZE + padded_size as usize + RLPX_MAC_SIZE;

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
            .ok_or_else(|| RLPxError::CryptographyError("Invalid frame data length".to_owned()))?
            .to_vec();

        src.advance(total_message_size);

        // The buffer contains the full message and will be consumed; update the ingress_mac and aes values
        self.ingress_mac = temp_ingress_mac;
        self.ingress_aes = temp_ingress_aes;

        let (frame_ciphertext, frame_mac) = frame_data
            .split_at_mut_checked(padded_size as usize)
            .ok_or_else(|| {
            RLPxError::CryptographyError("Invalid frame data length".to_owned())
        })?;

        // check MAC
        self.ingress_mac.update(&frame_ciphertext);
        let frame_mac_seed = {
            let mac_digest: [u8; 16] = self
                .ingress_mac
                .clone()
                .finalize()
                .get(..16)
                .ok_or_else(|| RLPxError::CryptographyError("Invalid mac digest".to_owned()))?
                .try_into()
                .map_err(|_| RLPxError::CryptographyError("Invalid mac digest".to_owned()))?;
            let mut seed = mac_digest.into();
            mac_aes_cipher.encrypt_block(&mut seed);
            (H128(seed.into()) ^ H128(mac_digest)).0
        };
        self.ingress_mac.update(frame_mac_seed);

        let expected_frame_mac: [u8; 16] = self
            .ingress_mac
            .clone()
            .finalize()
            .get(..16)
            .ok_or_else(|| RLPxError::CryptographyError("Invalid frame mac".to_owned()))?
            .try_into()
            .map_err(|_| RLPxError::CryptographyError("Invalid frame mac".to_owned()))?;

        if frame_mac != expected_frame_mac {
            return Err(RLPxError::InvalidMessageFrame(
                "Mismatched frame mac".to_string(),
            ));
        }

        // decrypt frame
        self.ingress_aes.try_apply_keystream(frame_ciphertext)?;

        let (frame_data, _padding) = frame_ciphertext
            .split_at_checked(frame_size as usize)
            .ok_or_else(|| RLPxError::CryptographyError("Invalid frame size".to_owned()))?;

        let (msg_id, msg_data): (u8, _) = RLPDecode::decode_unfinished(frame_data)?;
        Ok(Some(rlpx::Message::decode(
            msg_id,
            msg_data,
            *self
                .eth_version
                .read()
                .map_err(|err| RLPxError::InternalError(err.to_string()))?,
        )?))
    }

    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.decode(buf)? {
            Some(frame) => Ok(Some(frame)),
            None => {
                if buf.is_empty() {
                    Ok(None)
                } else {
                    Err(std::io::Error::other("bytes remaining on stream").into())
                }
            }
        }
    }

    fn framed<S: AsyncRead + AsyncWrite + Sized>(self, io: S) -> Framed<S, Self>
    where
        Self: Sized,
    {
        Framed::new(io, self)
    }
}

impl Encoder<rlpx::Message> for RLPxCodec {
    type Error = RLPxError;

    fn encode(&mut self, message: rlpx::Message, buffer: &mut BytesMut) -> Result<(), Self::Error> {
        let mut frame_data = vec![];
        message.encode(
            &mut frame_data,
            *self
                .eth_version
                .read()
                .map_err(|err| RLPxError::InternalError(err.to_string()))?,
        )?;

        let mac_aes_cipher = Aes256Enc::new_from_slice(&self.mac_key.0)?;

        // header = frame-size || header-data || header-padding
        let mut header = Vec::with_capacity(RLPX_HEADER_SIZE);
        let frame_size = frame_data.len().to_be_bytes();
        header.extend_from_slice(
            frame_size
                .get(5..8)
                .ok_or_else(|| RLPxError::CryptographyError("Invalid frame size".to_owned()))?,
        );

        // header-data = [capability-id, context-id]  (both always zero)
        let header_data = (0_u8, 0_u8);
        header_data.encode(&mut header);

        header.resize(AES_BLOCK_SIZE, 0);
        self.egress_aes
            .apply_keystream(&mut header[..AES_BLOCK_SIZE]);

        header.resize(16, 0);
        self.egress_aes.try_apply_keystream(
            header
                .get_mut(..16)
                .ok_or_else(|| RLPxError::CryptographyError("Invalid header length".to_owned()))?,
        )?;

        let header_mac_seed = {
            let mac_digest: [u8; 16] = self
                .egress_mac
                .clone()
                .finalize()
                .get(..16)
                .ok_or_else(|| RLPxError::CryptographyError("Invalid mac digest".to_owned()))?
                .try_into()
                .map_err(|_| RLPxError::CryptographyError("Invalid mac digest".to_owned()))?;
            let mut seed = mac_digest.into();
            mac_aes_cipher.encrypt_block(&mut seed);

            let header_data = header
                .get(..16)
                .ok_or_else(|| RLPxError::CryptographyError("Invalid header length".to_owned()))?
                .try_into()
                .map_err(|_| RLPxError::CryptographyError("Invalid header length".to_owned()))?;
            H128(seed.into()) ^ H128(header_data)
        };
        self.egress_mac.update(header_mac_seed);
        let header_mac = self.egress_mac.clone().finalize();
        let header_mac_data = header_mac
            .get(..16)
            .ok_or_else(|| RLPxError::CryptographyError("Invalid header mac".to_owned()))?;
        header.extend_from_slice(header_mac_data);

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
            let mac_digest: [u8; 16] = self
                .egress_mac
                .clone()
                .finalize()
                .get(..16)
                .ok_or_else(|| RLPxError::CryptographyError("Invalid mac digest".to_owned()))?
                .try_into()
                .map_err(|_| RLPxError::CryptographyError("Invalid mac digest".to_owned()))?;
            let mut seed = mac_digest.into();
            mac_aes_cipher.encrypt_block(&mut seed);
            (H128(seed.into()) ^ H128(mac_digest)).0
        };
        self.egress_mac.update(frame_mac_seed);
        let frame_mac = self.egress_mac.clone().finalize();

        // Write frame-mac
        buffer.extend_from_slice(&frame_mac[..RLPX_MAC_SIZE]);
        Ok(())
    }
}
