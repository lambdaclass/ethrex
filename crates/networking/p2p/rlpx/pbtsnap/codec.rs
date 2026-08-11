//! `pbtsnap/1` message encoding and decoding.
//!
//! The same shape as `rlpx/snap/codec.rs`: an `Encoder::encode_field` chain
//! into a snappy-compressed body, decoded by the mirror-image
//! `Decoder::decode_field` chain terminated by `finish()`, which is what
//! rejects a message carrying extra fields.

use super::messages::{GetPbtLeafRange, PbtLeaf, PbtLeafRange};
use crate::rlpx::{
    message::RLPxMessage,
    utils::{snappy_compress, snappy_decompress},
};
use bytes::BufMut;
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::{RLPDecodeError, RLPEncodeError},
    structs::{Decoder, Encoder},
};

/// `pbtsnap/1` message codes.
///
/// Four slots, two implemented. `0x02`/`0x03` are earmarked for a healing pair
/// (`GetPbtNodes`/`PbtNodes`, fetching node preimages by bit-path) that v1 does
/// not need — a stale pivot restarts the download instead. They are reserved
/// rather than left unallocated so that adding healing later does not shift the
/// offset of any capability stacked above this one.
pub mod codes {
    pub const GET_PBT_LEAF_RANGE: u8 = 0x00;
    pub const PBT_LEAF_RANGE: u8 = 0x01;
    /// Reserved: `GetPbtNodes`. Unassigned in `pbtsnap/1`.
    pub const RESERVED_GET_PBT_NODES: u8 = 0x02;
    /// Reserved: `PbtNodes`. Unassigned in `pbtsnap/1`.
    pub const RESERVED_PBT_NODES: u8 = 0x03;
}

impl RLPxMessage for GetPbtLeafRange {
    const CODE: u8 = codes::GET_PBT_LEAF_RANGE;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.root_hash)
            .encode_field(&self.origin)
            .encode_field(&self.limit)
            .encode_field(&self.response_bytes)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder) = decoder.decode_field("request-id")?;
        let (root_hash, decoder) = decoder.decode_field("rootHash")?;
        let (origin, decoder) = decoder.decode_field("origin")?;
        let (limit, decoder) = decoder.decode_field("limit")?;
        let (response_bytes, decoder) = decoder.decode_field("responseBytes")?;
        decoder.finish()?;

        Ok(Self {
            id,
            root_hash,
            origin,
            limit,
            response_bytes,
        })
    }
}

impl RLPxMessage for PbtLeafRange {
    const CODE: u8 = codes::PBT_LEAF_RANGE;

    fn encode(&self, buf: &mut dyn BufMut) -> Result<(), RLPEncodeError> {
        let mut encoded_data = vec![];
        Encoder::new(&mut encoded_data)
            .encode_field(&self.id)
            .encode_field(&self.leaves)
            .encode_field(&self.left_proof)
            .encode_field(&self.right_proof)
            .finish();

        let msg_data = snappy_compress(encoded_data)?;
        buf.put_slice(&msg_data);
        Ok(())
    }

    fn decode(msg_data: &[u8]) -> Result<Self, RLPDecodeError> {
        let decompressed_data = snappy_decompress(msg_data)?;
        let decoder = Decoder::new(&decompressed_data)?;
        let (id, decoder) = decoder.decode_field("request-id")?;
        let (leaves, decoder) = decoder.decode_field("leaves")?;
        let (left_proof, decoder) = decoder.decode_field("leftProof")?;
        let (right_proof, decoder) = decoder.decode_field("rightProof")?;
        decoder.finish()?;

        Ok(Self {
            id,
            leaves,
            left_proof,
            right_proof,
        })
    }
}

impl RLPEncode for PbtLeaf {
    fn encode(&self, buf: &mut dyn BufMut) {
        Encoder::new(buf)
            .encode_field(&self.key)
            .encode_field(&self.value)
            .finish();
    }
}

impl RLPDecode for PbtLeaf {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (key, decoder) = decoder.decode_field("key")?;
        let (value, decoder) = decoder.decode_field("value")?;
        Ok((Self { key, value }, decoder.finish()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rlpx::message::{
        BASED_CAPABILITY_SLOT_COUNT, EthCapVersion, Message, PBTSNAP_CAPABILITY_SLOT_COUNT,
    };
    use bytes::Bytes;
    use ethrex_common::H256;

    const ETH_VERSIONS: [EthCapVersion; 4] = [
        EthCapVersion::V68,
        EthCapVersion::V69,
        EthCapVersion::V70,
        EthCapVersion::V71,
    ];

    fn request() -> GetPbtLeafRange {
        GetPbtLeafRange {
            id: 7,
            root_hash: H256::repeat_byte(0xab),
            // A 34-byte account-zone key and a 66-byte storage-zone key: the
            // two lengths the tree actually uses, in one message, because the
            // field is length-agnostic and a fixed-width type would have
            // compiled fine against only one of them.
            origin: Bytes::from(vec![0u8; 34]),
            limit: Bytes::from(vec![0xff; 66]),
            response_bytes: 512 * 1024,
        }
    }

    fn response() -> PbtLeafRange {
        PbtLeafRange {
            id: 7,
            leaves: vec![
                PbtLeaf {
                    key: Bytes::from(vec![0u8; 34]),
                    value: H256::repeat_byte(1),
                },
                PbtLeaf {
                    key: Bytes::from(vec![0xffu8; 66]),
                    value: H256::zero(),
                },
            ],
            left_proof: vec![Bytes::from(vec![0x01, 0x00, 0x02]), Bytes::from(vec![0x00])],
            right_proof: vec![Bytes::from(vec![0x01])],
        }
    }

    fn round_trip<M: RLPxMessage + PartialEq + std::fmt::Debug>(message: &M) {
        let mut buf = vec![];
        message.encode(&mut buf).expect("encode");
        assert_eq!(&M::decode(&buf).expect("decode"), message);
    }

    #[test]
    fn messages_round_trip_through_rlp() {
        round_trip(&request());
        round_trip(&response());
    }

    #[test]
    fn the_empty_boundary_sentinels_survive_the_round_trip() {
        // An empty `origin` means "from the first leaf" and an empty `limit`
        // means "no upper bound". Both are load-bearing sentinels, and RLP has
        // more than one way to be empty, so a codec that lost the distinction
        // between an empty string and a missing field would turn a whole-zone
        // request into something else.
        round_trip(&GetPbtLeafRange {
            origin: Bytes::new(),
            limit: Bytes::new(),
            ..request()
        });
        // An empty range with empty proofs is the legal answer to "nothing at
        // or after the origin", so it must survive too.
        round_trip(&PbtLeafRange {
            id: 3,
            leaves: vec![],
            left_proof: vec![Bytes::from(vec![0x00, 0x01])],
            right_proof: vec![],
        });
    }

    #[test]
    fn a_response_with_a_trailing_field_is_rejected() {
        // `Decoder::finish` is the only thing standing between a peer and an
        // extra field that a future version might give meaning to. Encode a
        // five-field body where the message has four.
        let mut body = vec![];
        Encoder::new(&mut body)
            .encode_field(&7u64)
            .encode_field(&Vec::<PbtLeaf>::new())
            .encode_field(&Vec::<Bytes>::new())
            .encode_field(&Vec::<Bytes>::new())
            .encode_field(&99u64)
            .finish();
        let compressed = snappy_compress(body).expect("compress");
        assert!(PbtLeafRange::decode(&compressed).is_err());
    }

    #[test]
    fn a_request_missing_a_field_is_rejected() {
        let mut body = vec![];
        Encoder::new(&mut body)
            .encode_field(&7u64)
            .encode_field(&H256::repeat_byte(0xab))
            .encode_field(&Bytes::new())
            .encode_field(&Bytes::new())
            .finish();
        let compressed = snappy_compress(body).expect("compress");
        assert!(GetPbtLeafRange::decode(&compressed).is_err());
    }

    #[test]
    fn a_leaf_value_that_is_not_32_bytes_is_rejected() {
        // The value is the tree's leaf value and is fixed-width by
        // construction; a short one would silently zero-extend under a looser
        // type and change the leaf's hash.
        let mut leaf = vec![];
        Encoder::new(&mut leaf)
            .encode_field(&Bytes::from(vec![0u8; 34]))
            .encode_field(&Bytes::from(vec![0u8; 31]))
            .finish();
        assert!(PbtLeaf::decode(&leaf).is_err());
    }

    #[test]
    fn pbtsnap_offsets_sit_above_based_for_every_eth_version() {
        for version in ETH_VERSIONS {
            assert_eq!(
                version.pbtsnap_capability_offset(),
                version.based_capability_offset() + BASED_CAPABILITY_SLOT_COUNT,
                "pbtsnap must be stacked directly above based",
            );
            assert!(version.pbtsnap_capability_offset() > version.based_capability_offset());
            assert!(version.based_capability_offset() > version.snap_capability_offset());
        }
    }

    /// The offset assertion above is the *only* guard against pbtsnap
    /// shadowing `based`: with the `l2` feature off, the based decode arm is
    /// `#[cfg]`-ed out, so a pbtsnap offset that collided with based would
    /// still route every id to pbtsnap and every other test would pass.
    #[cfg(feature = "l2")]
    #[test]
    fn the_based_slot_count_covers_every_based_message_code() {
        use crate::rlpx::l2::messages::{BatchSealed, NewBlock};
        // `const` blocks, not plain assertions: both sides are compile-time
        // constants, so a runtime `assert!` is folded away and guards nothing.
        // This way a collision fails the build.
        const { assert!(NewBlock::CODE < BASED_CAPABILITY_SLOT_COUNT) };
        const { assert!(BatchSealed::CODE < BASED_CAPABILITY_SLOT_COUNT) };
    }

    #[test]
    fn the_reserved_slots_are_accounted_for() {
        // Two implemented plus two reserved. The count is what a capability
        // stacked above this one would offset by, so it must include the
        // reserved pair or a later healing protocol would collide.
        assert_eq!(PBTSNAP_CAPABILITY_SLOT_COUNT, 4);
        const { assert!(codes::RESERVED_PBT_NODES < PBTSNAP_CAPABILITY_SLOT_COUNT) };
    }

    #[test]
    fn the_existing_offset_ladder_is_unchanged() {
        // The regression this guards: adding a capability above `based` must
        // not move `eth`, `snap` or `based` by a single code, or every existing
        // peer on the wire misreads us. These are the values on the branch
        // before pbtsnap existed.
        let expected = [
            (EthCapVersion::V68, 0x10, 0x21, 0x30),
            (EthCapVersion::V69, 0x10, 0x22, 0x31),
            (EthCapVersion::V70, 0x10, 0x22, 0x31),
            (EthCapVersion::V71, 0x10, 0x24, 0x33),
        ];
        for (version, eth, snap, based) in expected {
            assert_eq!(version.eth_capability_offset(), eth);
            assert_eq!(version.snap_capability_offset(), snap);
            assert_eq!(version.based_capability_offset(), based);
        }
    }

    #[test]
    fn the_decode_ladder_routes_pbtsnap_ids_to_pbtsnap_messages() {
        for version in ETH_VERSIONS {
            let offset = version.pbtsnap_capability_offset();

            let mut buf = vec![];
            request().encode(&mut buf).expect("encode");
            let decoded = Message::decode(offset + GetPbtLeafRange::CODE, &buf, version)
                .expect("GetPbtLeafRange must decode at its ladder id");
            assert!(matches!(decoded, Message::GetPbtLeafRange(msg) if msg == request()));

            let mut buf = vec![];
            response().encode(&mut buf).expect("encode");
            let decoded = Message::decode(offset + PbtLeafRange::CODE, &buf, version)
                .expect("PbtLeafRange must decode at its ladder id");
            assert!(matches!(decoded, Message::PbtLeafRange(msg) if msg == response()));
        }
    }

    #[test]
    fn the_reserved_codes_are_not_decodable() {
        // A peer that speaks a future `pbtsnap` with healing must not have its
        // healing messages silently mis-decoded as something we do understand.
        for version in ETH_VERSIONS {
            let offset = version.pbtsnap_capability_offset();
            let mut buf = vec![];
            response().encode(&mut buf).expect("encode");
            for reserved in [codes::RESERVED_GET_PBT_NODES, codes::RESERVED_PBT_NODES] {
                assert!(
                    Message::decode(offset + reserved, &buf, version).is_err(),
                    "reserved code {reserved:#x} must not decode",
                );
            }
        }
    }

    #[test]
    fn a_pbtsnap_body_is_not_decodable_at_a_snap_or_based_id() {
        // The ladder must partition, not overlap. If a pbtsnap body decoded at
        // a `snap` id we would be reading one capability's bytes as another's.
        for version in ETH_VERSIONS {
            let mut buf = vec![];
            request().encode(&mut buf).expect("encode");
            // `snap` id 0x00 is GetAccountRange: five fields, but three of
            // them are fixed-width hashes where ours are variable strings.
            let as_snap = Message::decode(version.snap_capability_offset(), &buf, version);
            assert!(
                as_snap.is_err(),
                "a GetPbtLeafRange body must not decode as GetAccountRange",
            );
        }
    }

    #[test]
    fn both_messages_carry_a_request_id() {
        // The connection layer matches responses to in-flight requests by this
        // id; returning `None` would strand every pbtsnap response.
        assert_eq!(Message::GetPbtLeafRange(request()).request_id(), Some(7));
        assert_eq!(Message::PbtLeafRange(response()).request_id(), Some(7));
    }

    #[test]
    fn message_codes_match_the_ladder_offsets() {
        for version in ETH_VERSIONS {
            let offset = version.pbtsnap_capability_offset();
            assert_eq!(
                Message::GetPbtLeafRange(request()).code(version),
                offset + GetPbtLeafRange::CODE,
            );
            assert_eq!(
                Message::PbtLeafRange(response()).code(version),
                offset + PbtLeafRange::CODE,
            );
        }
    }
}
