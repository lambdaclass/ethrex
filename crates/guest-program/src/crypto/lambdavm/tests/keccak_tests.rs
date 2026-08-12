//! Host-side tests for the Keccak-256 sponge (`keccak256_with_permute`),
//! driving it with the trusted `keccak` crate's f1600 permutation and
//! cross-checking against ethrex's reference `keccak_hash`.

use super::super::*;

/// Cross-check our sponge body against the trusted `keccak` crate's f1600.
fn check_keccak(input: &[u8]) {
    let got = keccak256_with_permute(input, keccak::f1600);
    let want = keccak_hash(input);
    assert_eq!(
        got,
        want,
        "keccak256 mismatch for {}-byte input",
        input.len()
    );
}

/// Cross-check our sponge against a hardcoded vector from the Ethereum spec.
fn check_keccak_kat(input: &[u8], expected_hex: &str) {
    let expected: Vec<u8> = (0..expected_hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&expected_hex[i..i + 2], 16).unwrap())
        .collect();
    let got = keccak256_with_permute(input, keccak::f1600);
    assert_eq!(
        got.as_ref(),
        expected.as_slice(),
        "KAT mismatch for {}-byte input",
        input.len()
    );
}

#[test]
fn keccak_sponge_matches_trusted_permutation() {
    // Empty input.
    check_keccak(&[]);
    // One byte.
    check_keccak(&[0xab]);
    // 135 bytes — RATE-1: padding lands on byte 135 (0x01) and byte 135 is
    // also the last byte (0x80), so both bits land on the same byte: 0x81.
    check_keccak(&[0x5a; 135]);
    // Exactly RATE (136): fills one full block, final block is all-padding.
    check_keccak(&[0x3c; 136]);
    // RATE+1: one full block + one-byte remainder.
    check_keccak(&[0x7e; 137]);
    // Multi-block: ~1.5 × RATE (200 bytes), deterministic pattern.
    let long: Vec<u8> = (0u8..200).collect();
    check_keccak(&long);
    // 2 × RATE (272 bytes): two full absorb blocks + all-padding final block.
    check_keccak(&[0xaa; 272]);
    // 2 × RATE - 1 (271 bytes): two full absorbs + one-byte remainder.
    check_keccak(&[0xbb; 271]);
}

#[test]
fn keccak_sponge_known_answer_vectors() {
    // Vectors from the Ethereum Yellow Paper / EIP-155. These use Keccak-256
    // (0x01 padding), NOT SHA3-256 (0x06 padding). Any sponge framing bug
    // (wrong rate, wrong padding byte, wrong lane endianness) breaks these
    // even if the differential test above passes.

    // keccak256("") = c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470
    check_keccak_kat(
        b"",
        "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
    );
    // keccak256("abc")
    check_keccak_kat(
        b"abc",
        "4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45",
    );
    // keccak256("The quick brown fox jumps over the lazy dog")
    check_keccak_kat(
        b"The quick brown fox jumps over the lazy dog",
        "4d741b6f1eb29cb2a9b9911c82f56fa8d73b04959d3d9d222895df6c0b28aa15",
    );
}
