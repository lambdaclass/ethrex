use ethrex_p2p::rlpx::utils::{compress_pubkey, decompress_pubkey, ecdh_xchng};
use rand::rngs::OsRng;
use secp256k1::SecretKey;

#[test]
fn ecdh_xchng_smoke_test() {
    let a_sk = SecretKey::new(&mut OsRng);
    let b_sk = SecretKey::new(&mut OsRng);

    let a_sk_b_pk = ecdh_xchng(&a_sk, &b_sk.public_key(secp256k1::SECP256K1)).unwrap();
    let b_sk_a_pk = ecdh_xchng(&b_sk, &a_sk.public_key(secp256k1::SECP256K1)).unwrap();

    // The shared secrets should be the same.
    // The operation done is:
    //   a_sk * b_pk = a * (b * G) = b * (a * G) = b_sk * a_pk
    assert_eq!(a_sk_b_pk, b_sk_a_pk);
}

#[test]
fn compress_pubkey_decompress_pubkey_smoke_test() {
    let sk = SecretKey::new(&mut OsRng);
    let pk = sk.public_key(secp256k1::SECP256K1);
    let id = decompress_pubkey(&pk);
    let _pk2 = compress_pubkey(id).unwrap();
}
