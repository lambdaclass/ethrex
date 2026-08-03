//! Tests for `validate_requests_hash` — specifically the BSC (Parlia) rule.
//!
//! On BSC, the `requests_hash` header field is NOT the EIP-7685 requests hash.
//! Per BEP-675, bsc-geth repurposes it as an opaque MEV block-source tag
//! (`[11 zero bytes][version byte][20-byte builder address]`) on blocks built
//! from an external builder's bid; locally-built blocks keep the empty-requests
//! hash. Parlia's `VerifyRequests` is a no-op — the only consensus rule is that
//! the field is present post-Prague. ethrex must therefore NOT recompute and
//! compare it against block contents on BSC.

use ethrex_common::{types::BlockHeader, types::ChainConfig, validate_requests_hash, H256};

fn bsc_prague_config() -> ChainConfig {
    ChainConfig {
        chain_id: 56,
        prague_time: Some(0),
        ..Default::default()
    }
}

/// A real BEP-675 MEV tag observed on BSC mainnet block 111,436,224:
/// version 0x01 (SendBid) + builder 0x487e5dfe70119c1b320b8219b190a6fa95a5bb48.
fn mev_tag() -> H256 {
    let mut b = [0u8; 32];
    b[11] = 0x01;
    b[12..].copy_from_slice(
        &hex::decode("487e5dfe70119c1b320b8219b190a6fa95a5bb48").expect("valid hex"),
    );
    H256(b)
}

#[test]
fn bsc_accepts_bep675_mev_tagged_requests_hash() {
    // MEV-tagged requests_hash must be accepted even though the extracted
    // requests are empty (BSC extracts none) — it must not be recomputed and
    // compared, which would expect sha256("") and reject the block.
    let header = BlockHeader {
        timestamp: 1_784_705_930,
        requests_hash: Some(mev_tag()),
        ..Default::default()
    };
    assert!(validate_requests_hash(&header, &bsc_prague_config(), &[]).is_ok());
}

#[test]
fn bsc_accepts_empty_requests_hash() {
    // Locally-built BSC blocks keep sha256("") — must also be accepted.
    let header = BlockHeader {
        timestamp: 1_784_705_930,
        requests_hash: Some(H256::from_slice(
            &hex::decode("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
                .expect("valid hex"),
        )),
        ..Default::default()
    };
    assert!(validate_requests_hash(&header, &bsc_prague_config(), &[]).is_ok());
}

#[test]
fn bsc_rejects_missing_requests_hash_post_prague() {
    // Post-Prague the field must be present; absence is invalid (Parlia rule).
    let header = BlockHeader {
        timestamp: 1_784_705_930,
        requests_hash: None,
        ..Default::default()
    };
    assert!(validate_requests_hash(&header, &bsc_prague_config(), &[]).is_err());
}
