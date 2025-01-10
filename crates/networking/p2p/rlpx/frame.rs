use aes::{
    cipher::{BlockEncrypt as _, KeyInit as _, StreamCipher as _},
    Aes256Enc,
};
use bytes::{Buf, BytesMut};
use ethrex_core::H128;
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode as _};
use sha3::Digest as _;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_stream::StreamExt;
use tokio_util::codec::{Decoder, Encoder, Framed};

use super::{connection::Established, error::RLPxError, message as rlpx};

// max RLPx Message size
// Taken from https://github.com/ethereum/go-ethereum/blob/82e963e5c981e36dc4b607dd0685c64cf4aabea8/p2p/rlpx/rlpx.go#L152
const MAX_MESSAGE_SIZE: usize = 0xFFFFFF;

pub(crate) struct FrameAdaptor<S> {
    pub(crate) framed: Framed<S, RLPxCodec>,
}

impl<S: AsyncWrite + AsyncRead + std::marker::Unpin> FrameAdaptor<S> {
    pub fn new(stream: S) -> FrameAdaptor<S> {
        FrameAdaptor {
            framed: Framed::new(stream, RLPxCodec::new()),
        }
    }

    pub(crate) async fn read(&mut self) -> Result<rlpx::Message, RLPxError> {
        if let Some(message) = self.framed.next().await {
            message
        } else {
            Err(RLPxError::Disconnect())
        }
    }

    #[allow(dead_code)]
    pub(crate) async fn write(
        &mut self,
        frame_data: Vec<u8>,
        state: &mut Established,
    ) -> Result<(), RLPxError> {
        write(frame_data, state, self.framed.get_mut()).await
    }

    pub(crate) fn stream(&mut self) -> &mut S {
        self.framed.get_mut()
    }
}

pub(crate) struct RLPxCodec {
    state: Established,
}

impl RLPxCodec {
    fn new() -> Self {
        let mac_key = ethrex_core::H256::random();
        let ingress_aes = <super::connection::Aes256Ctr64BE as aes::cipher::KeyIvInit>::new(
            &mac_key.0.into(),
            &[0; 16].into(),
        );
        Self {
            state: Established {
                remote_node_id: ethrex_core::H512::random(),
                mac_key,
                ingress_mac: sha3::Keccak256::default(),
                egress_mac: sha3::Keccak256::default(),
                ingress_aes: ingress_aes.clone(),
                egress_aes: ingress_aes,
            },
        }
    }
}

impl RLPxCodec {
    pub(crate) fn set_state(&mut self, state: Established) {
        self.state = state
    }

    pub(crate) fn get_state(&mut self) -> Established {
        self.state.clone()
    }
}

impl Decoder for RLPxCodec {
    type Item = rlpx::Message;

    type Error = RLPxError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let mac_aes_cipher = Aes256Enc::new_from_slice(&self.state.mac_key.0)?;

        // Receive the message's frame header
        if src.len() < 32 {
            // Not enough data to read the frame header.
            return Ok(None);
        }
        let mut frame_header = [0; 32];
        frame_header.copy_from_slice(&src[..32]);

        // Both are padded to the block's size (16 bytes)
        let (header_ciphertext, header_mac) = frame_header.split_at_mut(16);

        // Validate MAC header
        // header-mac-seed = aes(mac-secret, keccak256.digest(egress-mac)[:16]) ^ header-ciphertext
        let header_mac_seed = {
            let mac_digest: [u8; 16] =
                self.state.ingress_mac.clone().finalize()[..16]
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
        self.state.ingress_mac.update(header_mac_seed);

        // header-mac = keccak256.digest(egress-mac)[:16]
        let expected_header_mac = H128(
            self.state.ingress_mac.clone().finalize()[..16]
                .try_into()
                .map_err(|_| RLPxError::CryptographyError("Invalid header mac".to_owned()))?,
        );

        assert_eq!(header_mac, expected_header_mac.0);

        let header_text = header_ciphertext;
        self.state.ingress_aes.apply_keystream(header_text);

        // header-data = [capability-id, context-id]
        // Both are unused, and always zero
        assert_eq!(&header_text[3..6], &(0_u8, 0_u8).encode_to_vec());

        let frame_size: usize =
            u32::from_be_bytes([0, header_text[0], header_text[1], header_text[2]])
                .try_into()
                .map_err(|_| RLPxError::CryptographyError("Invalid frame size".to_owned()))?;

        let padded_size = frame_size.next_multiple_of(16);

        // Check that the size is not too large to avoid a denial of
        // service attack where the server runs out of memory.
        if padded_size > MAX_MESSAGE_SIZE {
            return Err(RLPxError::InvalidMessageLength());
        }

        if src.len() < 32 + padded_size {
            // The full string has not yet arrived.
            //
            // We reserve more space in the buffer. This is not strictly
            // necessary, but is a good idea performance-wise.
            src.reserve(32 + padded_size - src.len());

            // We inform the Framed that we need more bytes to form the next
            // frame.
            return Ok(None);
        }

        // Use advance to modify src such that it no longer contains
        // this frame.
        let mut frame_data = src[32..32 + padded_size + 16].to_vec();
        src.advance(32 + padded_size + 16);

        let (frame_ciphertext, frame_mac) = frame_data.split_at_mut(padded_size);

        // check MAC
        #[allow(clippy::needless_borrows_for_generic_args)]
        self.state.ingress_mac.update(&frame_ciphertext);
        let frame_mac_seed = {
            let mac_digest: [u8; 16] =
                self.state.ingress_mac.clone().finalize()[..16]
                    .try_into()
                    .map_err(|_| RLPxError::CryptographyError("Invalid mac digest".to_owned()))?;
            let mut seed = mac_digest.into();
            mac_aes_cipher.encrypt_block(&mut seed);
            (H128(seed.into()) ^ H128(mac_digest)).0
        };
        self.state.ingress_mac.update(frame_mac_seed);
        let expected_frame_mac: [u8; 16] = self.state.ingress_mac.clone().finalize()[..16]
            .try_into()
            .map_err(|_| RLPxError::CryptographyError("Invalid frame mac".to_owned()))?;

        assert_eq!(frame_mac, expected_frame_mac);

        // decrypt frame
        self.state.ingress_aes.apply_keystream(frame_ciphertext);

        let (frame_data, _padding) = frame_ciphertext.split_at(frame_size);

        let (msg_id, msg_data): (u8, _) = RLPDecode::decode_unfinished(frame_data)?;
        Ok(Some(rlpx::Message::decode(msg_id, msg_data)?))
    }

    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.decode(buf)? {
            Some(frame) => Ok(Some(frame)),
            None => {
                if buf.is_empty() {
                    Ok(None)
                } else {
                    Err(
                        std::io::Error::new(std::io::ErrorKind::Other, "bytes remaining on stream")
                            .into(),
                    )
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

    fn encode(&mut self, _item: rlpx::Message, _dst: &mut BytesMut) -> Result<(), Self::Error> {
        todo!()
    }
}

pub(crate) async fn write<S: AsyncWrite + std::marker::Unpin>(
    mut frame_data: Vec<u8>,
    state: &mut Established,
    stream: &mut S,
) -> Result<(), RLPxError> {
    let mac_aes_cipher = Aes256Enc::new_from_slice(&state.mac_key.0)?;

    // header = frame-size || header-data || header-padding
    let mut header = Vec::with_capacity(32);
    let frame_size = frame_data.len().to_be_bytes();
    header.extend_from_slice(&frame_size[5..8]);

    // header-data = [capability-id, context-id]  (both always zero)
    let header_data = (0_u8, 0_u8);
    header_data.encode(&mut header);

    header.resize(16, 0);
    state.egress_aes.apply_keystream(&mut header[..16]);

    let header_mac_seed =
        {
            let mac_digest: [u8; 16] = state.egress_mac.clone().finalize()[..16]
                .try_into()
                .map_err(|_| RLPxError::CryptographyError("Invalid mac digest".to_owned()))?;
            let mut seed = mac_digest.into();
            mac_aes_cipher.encrypt_block(&mut seed);
            H128(seed.into())
                ^ H128(header[..16].try_into().map_err(|_| {
                    RLPxError::CryptographyError("Invalid header length".to_owned())
                })?)
        };
    state.egress_mac.update(header_mac_seed);
    let header_mac = state.egress_mac.clone().finalize();
    header.extend_from_slice(&header_mac[..16]);

    // Write header
    stream.write_all(&header).await?;

    // Pad to next multiple of 16
    frame_data.resize(frame_data.len().next_multiple_of(16), 0);
    state.egress_aes.apply_keystream(&mut frame_data);
    let frame_ciphertext = frame_data;

    // Send frame
    stream.write_all(&frame_ciphertext).await?;

    // Compute frame-mac
    state.egress_mac.update(&frame_ciphertext);

    // frame-mac-seed = aes(mac-secret, keccak256.digest(egress-mac)[:16]) ^ keccak256.digest(egress-mac)[:16]
    let frame_mac_seed = {
        let mac_digest: [u8; 16] = state.egress_mac.clone().finalize()[..16]
            .try_into()
            .map_err(|_| RLPxError::CryptographyError("Invalid mac digest".to_owned()))?;
        let mut seed = mac_digest.into();
        mac_aes_cipher.encrypt_block(&mut seed);
        (H128(seed.into()) ^ H128(mac_digest)).0
    };
    state.egress_mac.update(frame_mac_seed);
    let frame_mac = state.egress_mac.clone().finalize();
    // Send frame-mac
    stream.write_all(&frame_mac[..16]).await?;
    Ok(())
}

#[allow(dead_code)]
pub(crate) async fn read_old<S: AsyncRead + std::marker::Unpin>(
    state: &mut Established,
    stream: &mut S,
) -> Result<Vec<u8>, RLPxError> {
    let mac_aes_cipher = Aes256Enc::new_from_slice(&state.mac_key.0)?;

    // Receive the message's frame header
    let mut frame_header = [0; 32];
    stream.read_exact(&mut frame_header).await?;
    // Both are padded to the block's size (16 bytes)
    let (header_ciphertext, header_mac) = frame_header.split_at_mut(16);

    // Validate MAC header
    // header-mac-seed = aes(mac-secret, keccak256.digest(egress-mac)[:16]) ^ header-ciphertext
    let header_mac_seed = {
        let mac_digest: [u8; 16] = state.ingress_mac.clone().finalize()[..16]
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
    state.ingress_mac.update(header_mac_seed);

    // header-mac = keccak256.digest(egress-mac)[:16]
    let expected_header_mac = H128(
        state.ingress_mac.clone().finalize()[..16]
            .try_into()
            .map_err(|_| RLPxError::CryptographyError("Invalid header mac".to_owned()))?,
    );

    assert_eq!(header_mac, expected_header_mac.0);

    let header_text = header_ciphertext;
    state.ingress_aes.apply_keystream(header_text);

    // header-data = [capability-id, context-id]
    // Both are unused, and always zero
    assert_eq!(&header_text[3..6], &(0_u8, 0_u8).encode_to_vec());

    let frame_size: usize = u32::from_be_bytes([0, header_text[0], header_text[1], header_text[2]])
        .try_into()
        .map_err(|_| RLPxError::CryptographyError("Invalid frame size".to_owned()))?;
    // Receive the hello message
    let padded_size = frame_size.next_multiple_of(16);
    let mut frame_data = vec![0; padded_size + 16];
    stream.read_exact(&mut frame_data).await?;
    let (frame_ciphertext, frame_mac) = frame_data.split_at_mut(padded_size);

    // check MAC
    #[allow(clippy::needless_borrows_for_generic_args)]
    state.ingress_mac.update(&frame_ciphertext);
    let frame_mac_seed = {
        let mac_digest: [u8; 16] = state.ingress_mac.clone().finalize()[..16]
            .try_into()
            .map_err(|_| RLPxError::CryptographyError("Invalid mac digest".to_owned()))?;
        let mut seed = mac_digest.into();
        mac_aes_cipher.encrypt_block(&mut seed);
        (H128(seed.into()) ^ H128(mac_digest)).0
    };
    state.ingress_mac.update(frame_mac_seed);
    let expected_frame_mac: [u8; 16] = state.ingress_mac.clone().finalize()[..16]
        .try_into()
        .map_err(|_| RLPxError::CryptographyError("Invalid frame mac".to_owned()))?;

    assert_eq!(frame_mac, expected_frame_mac);

    // decrypt frame
    state.ingress_aes.apply_keystream(frame_ciphertext);

    let (frame_data, _padding) = frame_ciphertext.split_at(frame_size);

    Ok(frame_data.to_vec())
}
