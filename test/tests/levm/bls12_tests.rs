#![allow(clippy::indexing_slicing)]
#![allow(clippy::unwrap_used)]

use bytes::Bytes;
use ethrex_common::types::Fork;
use ethrex_crypto::{Crypto, NATIVE_BLS_BACKEND, NativeCrypto};
use ethrex_levm::precompiles::bls12_pairing_check;

#[test]
fn pairing_infinity() {
    let zero = Bytes::copy_from_slice(&[0_u8; 32]);

    // This is a calldata that pairing check returns 0
    // This is from https://eips.ethereum.org/assets/eip-2537/pairing_check_bls.json
    // test "bls_pairing_non-degeneracy"
    let mut calldata = hex::decode("0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e100000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be").unwrap();
    let calldata_bytes = Bytes::from(calldata.clone());
    let mut remaining_gas = 10000000;

    let result = bls12_pairing_check(
        &calldata_bytes,
        &mut remaining_gas,
        Fork::Cancun,
        &NativeCrypto,
    );
    assert_eq!(result.unwrap(), zero);

    // Now we add a pair were one point is infinity, the result must not change

    // This represent a G1 infinity point
    calldata.extend_from_slice(&[0u8; 128]);

    // This a valid calldata from the test "bls_pairing_e(G1,-G2)=e(-G1,G2)" of the same link
    let valid_calldata = hex::decode("0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e100000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e100000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000d1b3cc2c7027888be51d9ef691d77bcb679afda66c73f17f9ee3837a55024f78c71363275a75d75d86bab79f74782aa0000000000000000000000000000000013fa4d4a0ad8b1ce186ed5061789213d993923066dddaf1040bc3ff59f825c78df74f2d75467e25e0f55f8a00fa030ed").unwrap();
    // We add only the first G2 point of the calldata
    calldata.extend_from_slice(valid_calldata.get(128..384).unwrap());

    let calldata_bytes = Bytes::from(calldata.clone());

    let result = bls12_pairing_check(
        &calldata_bytes,
        &mut remaining_gas,
        Fork::Cancun,
        &NativeCrypto,
    );

    assert_eq!(result.unwrap(), zero);
}

// ── blst backend (host BLS12-381 / EIP-2537) ─────────────────────────────────
//
// `NativeCrypto` routes BLS12-381 through the blst backend (the `blst` feature,
// on by default; the canonical host/L1 implementation). The portable `bls12_381`
// reference now lives in `ethrex-guest-program` for the zkVM guests, so blst's
// numerical agreement with it is covered by the EIP-2537 execution-spec
// state-test vectors. The tests below exercise the host backend directly.

/// Guard: the host BLS12-381 tests must run against blst. With the `blst`
/// feature off, `NativeCrypto`'s BLS ops return an error, which would make
/// `blst_rejects_invalid_inputs` pass vacuously (an error is still `is_err()`).
/// Fail loudly instead of silently testing nothing.
#[test]
// `NATIVE_BLS_BACKEND` is a cfg-derived const; the constant assertion *is* the guard.
#[allow(clippy::assertions_on_constants)]
fn native_backend_is_active() {
    assert!(
        NATIVE_BLS_BACKEND,
        "blst feature is off; the BLS12-381 host tests are not exercising blst"
    );
}

const INF1: ([u8; 48], [u8; 48]) = ([0u8; 48], [0u8; 48]);

fn scalar(k: u64) -> [u8; 32] {
    let mut s = [0u8; 32];
    s[24..32].copy_from_slice(&k.to_be_bytes());
    s
}

#[test]
fn blst_rejects_invalid_inputs() {
    // Non-canonical field element (>= modulus).
    let bad: [u8; 48] = [0xff; 48];
    assert!(NativeCrypto.bls12_381_g1_add((bad, bad), INF1).is_err());
    // Point not on the curve.
    let off_curve = ([1u8; 48], [1u8; 48]);
    assert!(NativeCrypto.bls12_381_g1_add(off_curve, INF1).is_err());
    assert!(
        NativeCrypto
            .bls12_381_g1_msm(&[(off_curve, scalar(1))])
            .is_err()
    );
}
