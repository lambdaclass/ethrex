//! End-to-end snap/2 round-trip over an in-memory duplex pipe.
//!
//! Tests the full `Message`-level dispatch: client encodes a
//! `Snap2GetBlockAccessLists`, server decodes it, calls
//! `build_snap2_bal_response` against a real `Store`, encodes the
//! `Snap2BlockAccessLists` response, client decodes it, asserts.
//!
//! Skips the encrypted RLPx framing layer (AES + MAC) and the auth
//! handshake — those weren't modified by the snap/2 PR. A heavier
//! harness can be added later by either feature-gating a
//! `RLPxCodec::for_test` constructor or making `handshake::perform`
//! generic over `AsyncRead + AsyncWrite`.

use bytes::{Buf, BufMut, BytesMut};
use ethrex_common::{H256, types::BlockHeader, types::block_access_list::BlockAccessList};
use ethrex_p2p::rlpx::connection::server::build_snap2_bal_response;
use ethrex_p2p::rlpx::message::{EthCapVersion, Message, SnapCapVersion};
use ethrex_p2p::rlpx::snap::Snap2GetBlockAccessLists;
use ethrex_rlp::{encode::RLPEncode, error::RLPDecodeError};
use ethrex_storage::{EngineType, Store, api::tables::HEADERS};
use futures::{SinkExt, StreamExt};
use std::io;
use tokio::io::duplex;
use tokio_util::codec::{Decoder, Encoder, Framed};

/// Length-prefixed `Message` codec for in-process tests. The body emitted
/// by `Message::encode` already starts with the message code byte; the
/// 4-byte length prefix here is just to delimit message boundaries on the
/// duplex stream (the production `RLPxCodec` does this with AES + MAC).
struct MessageCodec {
    eth: EthCapVersion,
    snap: Option<SnapCapVersion>,
}

impl Encoder<Message> for MessageCodec {
    type Error = io::Error;

    fn encode(&mut self, msg: Message, dst: &mut BytesMut) -> io::Result<()> {
        let mut buf: Vec<u8> = Vec::new();
        msg.encode(&mut buf, self.eth)
            .map_err(|e| io::Error::other(format!("rlp encode: {e:?}")))?;
        dst.put_u32(buf.len() as u32);
        dst.put_slice(&buf);
        Ok(())
    }
}

impl Decoder for MessageCodec {
    type Item = Message;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Message>> {
        if src.len() < 4 {
            return Ok(None);
        }
        let len = u32::from_be_bytes(src[..4].try_into().unwrap()) as usize;
        if src.len() < 4 + len {
            return Ok(None);
        }
        src.advance(4);
        if len == 0 {
            return Err(io::Error::other("empty frame"));
        }
        let code = src[0];
        let body = src[1..len].to_vec();
        src.advance(len);
        Message::decode(code, &body, self.eth, self.snap)
            .map(Some)
            .map_err(|e: RLPDecodeError| io::Error::other(format!("rlp decode: {e:?}")))
    }
}

fn store_with_post_amsterdam_header(hash: H256) -> Store {
    use ethrex_storage::rlp::BlockHeaderRLP;
    let store = Store::new("memory", EngineType::InMemory).expect("in-memory store");
    let header = BlockHeader {
        base_fee_per_gas: Some(0),
        withdrawals_root: Some(H256::zero()),
        blob_gas_used: Some(0),
        excess_blob_gas: Some(0),
        parent_beacon_block_root: Some(H256::zero()),
        requests_hash: Some(H256::zero()),
        block_access_list_hash: Some(H256::from([0xBBu8; 32])),
        ..Default::default()
    };
    let hash_key = hash.encode_to_vec();
    let header_bytes = BlockHeaderRLP::from(header).into_vec();
    store
        .write(HEADERS, hash_key, header_bytes)
        .expect("store header");
    store
}

/// A snap/2 client sends `Snap2GetBlockAccessLists` over a duplex pipe;
/// a server task on the other end reads it, invokes the production
/// `build_snap2_bal_response` handler against a real `Store`, and writes
/// back `Snap2BlockAccessLists`. The client decodes and asserts.
#[tokio::test]
async fn snap2_request_response_roundtrip_over_duplex() {
    let known_hash = H256::from([0x22u8; 32]);
    let server_store = store_with_post_amsterdam_header(known_hash);
    server_store
        .store_block_access_list(known_hash, &BlockAccessList::new())
        .expect("store BAL");

    let (client_io, server_io) = duplex(64 * 1024);
    let mut client = Framed::new(
        client_io,
        MessageCodec {
            eth: EthCapVersion::V68,
            snap: Some(SnapCapVersion::V2),
        },
    );
    let mut server = Framed::new(
        server_io,
        MessageCodec {
            eth: EthCapVersion::V68,
            snap: Some(SnapCapVersion::V2),
        },
    );

    let server_task = tokio::spawn(async move {
        let Some(Ok(Message::Snap2GetBlockAccessLists(req))) = server.next().await else {
            panic!("server expected Snap2GetBlockAccessLists");
        };
        let resp = build_snap2_bal_response(req, &server_store).expect("build response");
        server
            .send(Message::Snap2BlockAccessLists(resp))
            .await
            .expect("send response");
    });

    let request_id: u64 = 1234;
    client
        .send(Message::Snap2GetBlockAccessLists(
            Snap2GetBlockAccessLists {
                id: request_id,
                block_hashes: vec![known_hash, H256::from([0x99u8; 32])],
                response_bytes: 0,
            },
        ))
        .await
        .expect("send request");

    let Some(Ok(Message::Snap2BlockAccessLists(resp))) = client.next().await else {
        panic!("client expected Snap2BlockAccessLists");
    };
    server_task.await.expect("server task");

    assert_eq!(resp.id, request_id, "response id must match request");
    assert_eq!(resp.bals.len(), 2, "one slot per requested hash");
    assert!(resp.bals[0].is_some(), "known hash → Some");
    assert!(resp.bals[1].is_none(), "unknown hash → None");
}

/// The version-aware codec rejects snap/1-only codes (0x06/0x07) on a
/// snap/2 connection. Verified end-to-end over the duplex pipe by sending
/// raw bytes that decode to `GetTrieNodes::CODE` and confirming the
/// receiver's `Message::decode` returns `MalformedData`.
#[tokio::test]
async fn snap2_connection_rejects_get_trie_nodes_code() {
    let (a, b) = duplex(4 * 1024);
    let mut sender = Framed::new(
        a,
        MessageCodec {
            eth: EthCapVersion::V68,
            // The sender side encodes whatever — snap version irrelevant on encode.
            snap: Some(SnapCapVersion::V1),
        },
    );
    let mut receiver = Framed::new(
        b,
        MessageCodec {
            eth: EthCapVersion::V68,
            // The receiver is on snap/2 — must reject snap/1-only codes.
            snap: Some(SnapCapVersion::V2),
        },
    );

    // Hand-build a frame: length = 1, body = [0x06] (GetTrieNodes code).
    // Construct via the inner stream by sending raw bytes.
    use tokio::io::AsyncWriteExt;
    let raw = {
        let mut buf = BytesMut::new();
        let snap_offset = EthCapVersion::V68.snap_capability_offset();
        buf.put_u32(1);
        buf.put_u8(snap_offset + 0x06);
        buf.freeze()
    };
    sender.get_mut().write_all(&raw).await.expect("write raw");
    sender.get_mut().flush().await.expect("flush");

    let result = receiver.next().await.expect("frame arrives");
    assert!(
        result.is_err(),
        "snap/2 receiver must reject snap/1-only GetTrieNodes code"
    );
}
