#![allow(clippy::indexing_slicing)]
#![allow(clippy::unwrap_used)]

use bytes::Bytes;
use ethrex_levm::precompiles::bls12_pairing_check;
#[cfg(feature = "l2")]
use ethrex_levm::precompiles::p_256_verify;

#[test]
fn pairing_infinity() {
    let zero = Bytes::copy_from_slice(&[0_u8; 32]);

    // This is a calldata that pairing check returns 0
    // This is from https://eips.ethereum.org/assets/eip-2537/pairing_check_bls.json
    // test "bls_pairing_non-degeneracy"
    let mut calldata = hex::decode("0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e100000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be").unwrap();
    let calldata_bytes = Bytes::from(calldata.clone());
    let mut consumed_gas = 0;

    let result = bls12_pairing_check(&calldata_bytes, 10000000, &mut consumed_gas);
    assert_eq!(result.unwrap(), zero);

    // Now we add a pair were one point is infinity, the result must not change

    // This represent a G1 infinity point
    calldata.extend_from_slice(&[0u8; 128]);

    // This a valid calldata from the test "bls_pairing_e(G1,-G2)=e(-G1,G2)" of the same link
    let valid_calldata = hex::decode("0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e100000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e100000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb80000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e000000000000000000000000000000000d1b3cc2c7027888be51d9ef691d77bcb679afda66c73f17f9ee3837a55024f78c71363275a75d75d86bab79f74782aa0000000000000000000000000000000013fa4d4a0ad8b1ce186ed5061789213d993923066dddaf1040bc3ff59f825c78df74f2d75467e25e0f55f8a00fa030ed").unwrap();
    // We add only the first G2 point of the calldata
    calldata.extend_from_slice(valid_calldata.get(128..384).unwrap());

    let calldata_bytes = Bytes::from(calldata.clone());

    let result = bls12_pairing_check(&calldata_bytes, 10000000, &mut consumed_gas);

    assert_eq!(result.unwrap(), zero);
}

#[cfg(feature = "l2")]
use serde::Deserialize;

#[cfg(feature = "l2")]
use std::fs;

#[cfg(feature = "l2")]
#[derive(Debug, Deserialize)]
struct P256TestCase {
    input: String,
    expected: String,
    gas: u64,
    name: String,
}

#[cfg(feature = "l2")]
#[test]
fn p_256_verify_test() {
    // Taken from https://github.com/ulerdogan/go-ethereum/tree/ulerdogan-secp256r1.

    let json_data = fs::read_to_string("./tests/p_256_verify.json").unwrap();

    let tests: Vec<P256TestCase> = serde_json::from_str(&json_data).unwrap();

    for test in tests {
        let calldata = hex::decode(&test.input).unwrap();
        let calldata = Bytes::from(calldata);
        let mut consumed_gas = 0;
        let result = p_256_verify(&calldata, 10000, &mut consumed_gas).unwrap();
        let expected_result = Bytes::from(hex::decode(&test.expected).unwrap());
        assert_eq!(
            result, expected_result,
            "Result assertion failed on test: {}.",
            test.name
        );
        assert_eq!(
            consumed_gas, test.gas,
            "Gas assertion failed on test: {}.",
            test.name
        );
    }
}
