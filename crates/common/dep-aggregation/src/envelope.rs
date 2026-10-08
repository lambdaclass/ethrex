//! The recursive proof envelope of the deployed EIP-8288 prototype
//! (`eip8288PrototypeTime`).
//!
//! The header's `stark_proof` carries the dependency list next to the proof, framed
//! with little-endian `u32` lengths:
//!
//! ```text
//! "NLR3" || u32 count || count x 96-byte triple || u32 payload_len || payload
//! ```
//!
//! The triples must be canonical (31 zero bytes, then a known scheme) and strictly
//! ascending, so the list is exactly the block's sorted, deduplicated dependency
//! set. An empty set has exactly one encoding, the 12-byte envelope with no
//! triples and no payload; a non-empty set must carry a payload, which is a leanVM
//! mixed recursive proof over those triples.
//!
//! The block commits to the set separately, as `keccak256` over the concatenated
//! triples (`BlockBody::prototype_block_deps_hash`), and rule 1 checks that against
//! the body before this module runs.

use ethrex_common::types::{DEPENDENCY_SCHEME_LEANSTARK, DependencyTriple};

use crate::{AggregateError, MAX_RECURSIVE_STARK_PROOF_BYTES, check_proof_length};

/// The envelope's magic bytes.
pub const ENVELOPE_MAGIC: &[u8; 4] = b"NLR3";
/// The most dependencies one envelope, and so one block, may carry.
pub const MAX_ENVELOPE_DEPENDENCIES: usize = 256;
/// The most leanSTARK dependencies one envelope may carry.
pub const MAX_ENVELOPE_LEANSTARK_DEPENDENCIES: usize = 16;
/// The largest payload: the proof bound minus the framing and a full dependency list.
pub const MAX_ENVELOPE_PAYLOAD_BYTES: usize =
    MAX_RECURSIVE_STARK_PROOF_BYTES - 12 - 96 * MAX_ENVELOPE_DEPENDENCIES;

/// The only valid envelope for an empty dependency set.
pub const EMPTY_ENVELOPE: [u8; 12] = *b"NLR3\0\0\0\0\0\0\0\0";

/// A decoded envelope: the dependency list it carries, and the proof payload.
#[derive(Debug, PartialEq, Eq)]
pub struct Envelope<'a> {
    pub dependencies: Vec<DependencyTriple>,
    pub payload: &'a [u8],
}

/// Decode an envelope, enforcing every framing rule. Any violation is
/// [`AggregateError::ProofMalformed`].
pub fn decode_envelope(proof: &[u8]) -> Result<Envelope<'_>, AggregateError> {
    check_proof_length(proof)?;
    let mut reader = Reader { bytes: proof };
    if reader.take(4)? != ENVELOPE_MAGIC {
        return Err(AggregateError::ProofMalformed);
    }
    let count = reader.length()?;
    if count > MAX_ENVELOPE_DEPENDENCIES {
        return Err(AggregateError::ProofMalformed);
    }
    let mut dependencies = Vec::with_capacity(count);
    let mut previous: Option<&[u8]> = None;
    for _ in 0..count {
        let encoded = reader.take(96)?;
        if previous.is_some_and(|previous| previous >= encoded) {
            return Err(AggregateError::ProofMalformed);
        }
        previous = Some(encoded);
        dependencies
            .push(DependencyTriple::from_bytes(encoded).ok_or(AggregateError::ProofMalformed)?);
    }
    let leanstark_count = dependencies
        .iter()
        .filter(|triple| triple.scheme == DEPENDENCY_SCHEME_LEANSTARK)
        .count();
    if leanstark_count > MAX_ENVELOPE_LEANSTARK_DEPENDENCIES {
        return Err(AggregateError::ProofMalformed);
    }
    let payload_len = reader.length()?;
    let payload = reader.take(payload_len)?;
    if !reader.bytes.is_empty()
        || payload.len() > MAX_ENVELOPE_PAYLOAD_BYTES
        || payload.is_empty() != dependencies.is_empty()
    {
        return Err(AggregateError::ProofMalformed);
    }
    Ok(Envelope {
        dependencies,
        payload,
    })
}

/// EIP-8288 rule 2 on the prototype schedule: the header's proof must be an
/// envelope carrying exactly `expected` (the block's sorted, deduplicated
/// dependencies), and its payload must verify as a mixed recursive proof over them.
pub fn verify_envelope(proof: &[u8], expected: &[DependencyTriple]) -> Result<(), AggregateError> {
    let envelope = decode_envelope(proof)?;
    if envelope.dependencies != expected {
        return Err(AggregateError::ClaimsMismatch(format!(
            "envelope carries {} dependencies, block declares {}",
            envelope.dependencies.len(),
            expected.len()
        )));
    }
    if envelope.dependencies.is_empty() {
        return Ok(());
    }
    verify_mixed_payload(&envelope.dependencies, envelope.payload)
}

#[cfg(feature = "leanvm")]
fn verify_mixed_payload(
    dependencies: &[DependencyTriple],
    payload: &[u8],
) -> Result<(), AggregateError> {
    crate::leanvm::verify_mixed_proof(dependencies, payload)
}

#[cfg(not(feature = "leanvm"))]
fn verify_mixed_payload(
    _dependencies: &[DependencyTriple],
    _payload: &[u8],
) -> Result<(), AggregateError> {
    Err(AggregateError::NoBackend)
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], AggregateError> {
        if n > self.bytes.len() {
            return Err(AggregateError::ProofMalformed);
        }
        let (taken, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Ok(taken)
    }

    fn length(&mut self) -> Result<usize, AggregateError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| AggregateError::ProofMalformed)?;
        usize::try_from(u32::from_le_bytes(bytes)).map_err(|_| AggregateError::ProofMalformed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::H256;

    fn triple(scheme: u8, data: u8) -> DependencyTriple {
        DependencyTriple {
            scheme,
            data_hash: H256::repeat_byte(data),
            verification_key_hash: H256::repeat_byte(0xaa),
        }
    }

    fn envelope(triples: &[DependencyTriple], payload: &[u8]) -> Vec<u8> {
        let mut out = ENVELOPE_MAGIC.to_vec();
        out.extend_from_slice(&u32::try_from(triples.len()).unwrap().to_le_bytes());
        for t in triples {
            out.extend_from_slice(&t.encode());
        }
        out.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn the_empty_set_has_exactly_one_encoding() {
        assert_eq!(envelope(&[], &[]), EMPTY_ENVELOPE);
        assert_eq!(verify_envelope(&EMPTY_ENVELOPE, &[]), Ok(()));
        // No envelope at all is not the empty set.
        assert_eq!(
            verify_envelope(&[], &[]),
            Err(AggregateError::ProofMalformed)
        );
        // A payload with no triples, and trailing bytes, are both malformed.
        assert!(verify_envelope(&envelope(&[], &[1]), &[]).is_err());
        let mut trailing = EMPTY_ENVELOPE.to_vec();
        trailing.push(0);
        assert!(verify_envelope(&trailing, &[]).is_err());
    }

    #[test]
    fn triples_must_be_strictly_ascending_and_match_the_block() {
        let a = triple(0x10, 1);
        let b = triple(0x10, 2);
        assert!(decode_envelope(&envelope(&[a, b], &[1])).is_ok());
        assert!(decode_envelope(&envelope(&[b, a], &[1])).is_err());
        assert!(decode_envelope(&envelope(&[a, a], &[1])).is_err());
        // Triples present but no payload is malformed.
        assert!(decode_envelope(&envelope(&[a], &[])).is_err());
        // A well-framed envelope for a different set is a claims mismatch.
        assert!(matches!(
            verify_envelope(&envelope(&[a], &[1]), &[b]),
            Err(AggregateError::ClaimsMismatch(_))
        ));
    }

    #[test]
    fn unknown_schemes_and_bad_magic_are_rejected() {
        let mut bad_scheme = envelope(&[triple(0x10, 1)], &[1]);
        // The first triple follows the magic and the count; byte 31 is its scheme.
        bad_scheme[8 + 31] = 0x12;
        assert!(decode_envelope(&bad_scheme).is_err());
        let mut bad_magic = EMPTY_ENVELOPE;
        bad_magic[3] = b'2';
        assert!(decode_envelope(&bad_magic).is_err());
    }
}
