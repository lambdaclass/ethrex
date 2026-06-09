use ethrex_common::H256;
use ethrex_p2p::rlpx::{connection::handshake::decode_ack_message, utils::decompress_pubkey};
use hex_literal::hex;
use secp256k1::SecretKey;
use std::str::FromStr;

#[test]
fn test_ack_decoding() {
    // This is the Ack₂ message from EIP-8.
    // https://github.com/ethereum/EIPs/blob/master/EIPS/eip-8.md
    let msg = hex!(
        "01ea0451958701280a56482929d3b0757da8f7fbe5286784beead59d95089c217c9b917788989470b0e330cc6e4fb383c0340ed85fab836ec9fb8a49672712aeabbdfd1e837c1ff4cace34311cd7f4de05d59279e3524ab26ef753a0095637ac88f2b499b9914b5f64e143eae548a1066e14cd2f4bd7f814c4652f11b254f8a2d0191e2f5546fae6055694aed14d906df79ad3b407d94692694e259191cde171ad542fc588fa2b7333313d82a9f887332f1dfc36cea03f831cb9a23fea05b33deb999e85489e645f6aab1872475d488d7bd6c7c120caf28dbfc5d6833888155ed69d34dbdc39c1f299be1057810f34fbe754d021bfca14dc989753d61c413d261934e1a9c67ee060a25eefb54e81a4d14baff922180c395d3f998d70f46f6b58306f969627ae364497e73fc27f6d17ae45a413d322cb8814276be6ddd13b885b201b943213656cde498fa0e9ddc8e0b8f8a53824fbd82254f3e2c17e8eaea009c38b4aa0a3f306e8797db43c25d68e86f262e564086f59a2fc60511c42abfb3057c247a8a8fe4fb3ccbadde17514b7ac8000cdb6a912778426260c47f38919a91f25f4b5ffb455d6aaaf150f7e5529c100ce62d6d92826a71778d809bdf60232ae21ce8a437eca8223f45ac37f6487452ce626f549b3b5fdee26afd2072e4bc75833c2464c805246155289f4"
    );
    let static_key_a = SecretKey::from_slice(&hex!(
        "49a7b37aa6f6645917e7b807e9d1c00d4fa71f18343b0d4122a4d2df64dd6fee"
    ))
    .unwrap();

    let expected_nonce_b =
        H256::from_str("559aead08264d5795d3909718cdd05abd49572e84fe55590eef31a88a08fdffd").unwrap();
    let expected_ephemeral_key_b = decompress_pubkey(
        &SecretKey::from_slice(&hex!(
            "e238eb8e04fee6511ab04c6dd3c89ce097b11f25d584863ac2b6d5b35b1847e4"
        ))
        .unwrap()
        .public_key(secp256k1::SECP256K1),
    );

    let ack = decode_ack_message(&static_key_a, &msg[2..], &msg[..2]).unwrap();

    assert_eq!(ack.ephemeral_pubkey, expected_ephemeral_key_b);
    assert_eq!(ack.nonce, expected_nonce_b);
    assert_eq!(ack.version, 4u8);
}
