#![allow(clippy::indexing_slicing)]
#![allow(clippy::unwrap_used)]

use bytes::Bytes;
use ethrex_common::types::Fork;
use ethrex_crypto::NativeCrypto;
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

// ── blst backend differential tests ─────────────────────────────────────────
//
// `NativeCrypto` routes BLS12-381 through the blst backend (the `blst` feature,
// on by default). These tests assert it agrees byte-for-byte with the portable
// `bls12_381` trait-default implementation (`Ref` below), which already passes
// the Ethereum execution-spec EIP-2537 vectors. Inputs are the EIP-2537
// generators and points derived from them.

use ethrex_crypto::Crypto;
use hex_literal::hex;

/// Reference implementation: empty impl uses the portable (zkcrypto) defaults.
#[derive(Debug)]
struct Ref;
impl Crypto for Ref {}

// EIP-2537 generators (unpadded, 48-byte big-endian coordinates).
const G1X: [u8; 48] = hex!(
    "17f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb"
);
const G1Y: [u8; 48] = hex!(
    "08b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1"
);
const G2X0: [u8; 48] = hex!(
    "024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb8"
);
const G2X1: [u8; 48] = hex!(
    "13e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e"
);
const G2Y0: [u8; 48] = hex!(
    "0ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801"
);
const G2Y1: [u8; 48] = hex!(
    "0606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be"
);

const INF1: ([u8; 48], [u8; 48]) = ([0u8; 48], [0u8; 48]);
const INF2: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]) = ([0u8; 48], [0u8; 48], [0u8; 48], [0u8; 48]);

fn g1() -> ([u8; 48], [u8; 48]) {
    (G1X, G1Y)
}
fn g2() -> ([u8; 48], [u8; 48], [u8; 48], [u8; 48]) {
    (G2X0, G2X1, G2Y0, G2Y1)
}
fn scalar(k: u64) -> [u8; 32] {
    let mut s = [0u8; 32];
    s[24..32].copy_from_slice(&k.to_be_bytes());
    s
}
// Subgroup order q minus one, big-endian. `(q-1) * G == -G`.
const Q_MINUS_1: [u8; 32] =
    hex!("73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000");

// Split a 96-byte G1 encoding into its (x, y) coordinate halves.
fn split_g1(p: [u8; 96]) -> ([u8; 48], [u8; 48]) {
    (p[0..48].try_into().unwrap(), p[48..96].try_into().unwrap())
}

#[test]
fn blst_matches_reference_g1_add() {
    for (a, b) in [(g1(), g1()), (g1(), INF1), (INF1, g1())] {
        assert_eq!(
            NativeCrypto.bls12_381_g1_add(a, b).unwrap(),
            Ref.bls12_381_g1_add(a, b).unwrap(),
        );
    }
}

#[test]
fn blst_matches_reference_g2_add() {
    for (a, b) in [(g2(), g2()), (g2(), INF2), (INF2, g2())] {
        assert_eq!(
            NativeCrypto.bls12_381_g2_add(a, b).unwrap().as_slice(),
            Ref.bls12_381_g2_add(a, b).unwrap().as_slice(),
        );
    }
}

#[test]
fn blst_matches_reference_g1_msm() {
    // 2G derived through the blst backend; a valid on-curve subgroup point.
    let two_g = split_g1(NativeCrypto.bls12_381_g1_add(g1(), g1()).unwrap());
    let single = vec![(g1(), scalar(7))];
    let multi = vec![(g1(), scalar(3)), (two_g, scalar(5))];
    let zero_scalar = vec![(g1(), scalar(0))];
    for pairs in [single, multi, zero_scalar] {
        assert_eq!(
            NativeCrypto.bls12_381_g1_msm(&pairs).unwrap(),
            Ref.bls12_381_g1_msm(&pairs).unwrap(),
        );
    }
}

#[test]
fn blst_matches_reference_g2_msm() {
    let pairs = vec![(g2(), scalar(4))];
    assert_eq!(
        NativeCrypto.bls12_381_g2_msm(&pairs).unwrap().as_slice(),
        Ref.bls12_381_g2_msm(&pairs).unwrap().as_slice(),
    );
}

#[test]
fn blst_matches_reference_pairing() {
    // e(G, H) != 1  →  false
    let single = vec![(g1(), g2())];
    assert_eq!(
        NativeCrypto.bls12_381_pairing_check(&single).unwrap(),
        Ref.bls12_381_pairing_check(&single).unwrap(),
    );

    // e(G, H) * e(-G, H) == 1  →  true
    let neg_g = split_g1(NativeCrypto.bls12_381_g1_msm(&[(g1(), Q_MINUS_1)]).unwrap());
    let bilinear = vec![(g1(), g2()), (neg_g, g2())];
    assert!(NativeCrypto.bls12_381_pairing_check(&bilinear).unwrap());
    assert_eq!(
        NativeCrypto.bls12_381_pairing_check(&bilinear).unwrap(),
        Ref.bls12_381_pairing_check(&bilinear).unwrap(),
    );

    // A pair with a point at infinity is a no-op.
    let with_inf = vec![(g1(), g2()), (INF1, g2())];
    assert_eq!(
        NativeCrypto.bls12_381_pairing_check(&with_inf).unwrap(),
        Ref.bls12_381_pairing_check(&with_inf).unwrap(),
    );
}

#[test]
fn blst_matches_reference_map_to_curve() {
    for k in [1u64, 42, 1000, 0x9876_5432] {
        let mut fp = [0u8; 48];
        fp[40..48].copy_from_slice(&k.to_be_bytes());
        assert_eq!(
            NativeCrypto.bls12_381_fp_to_g1(&fp).unwrap().as_slice(),
            Ref.bls12_381_fp_to_g1(&fp).unwrap().as_slice(),
        );
        assert_eq!(
            NativeCrypto
                .bls12_381_fp2_to_g2((fp, fp))
                .unwrap()
                .as_slice(),
            Ref.bls12_381_fp2_to_g2((fp, fp)).unwrap().as_slice(),
        );
    }
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
