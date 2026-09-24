use ethrex_common::H256;
use ethrex_p2p::discv5::session::{create_id_signature, derive_session_keys, verify_id_signature};
use hex_literal::hex;
use secp256k1::{PublicKey, SecretKey};

#[test]
fn derivation_matches_vector() {
    let ephemeral_key = SecretKey::from_byte_array(&hex!(
        "fb757dc581730490a1d7a00deea65e9b1936924caaea8f44d476014856b68736"
    ))
    .unwrap();
    let dest_pubkey = PublicKey::from_slice(&hex!(
        "0317931e6e0840220642f230037d285d122bc59063221ef3226b1f403ddc69ca91"
    ))
    .unwrap();
    let node_id_a = H256::from_slice(&hex!(
        "aaaa8419e9f49d0083561b48287df592939a8d19947d8c0ef88f2a4856a69fbb"
    ));
    let node_id_b = H256::from_slice(&hex!(
        "bbbb9d047f0488c0b5a93c1c3f2d8bafc7c8ff337024a55434a0d0555de64db9"
    ));
    let challenge_data = hex!(
        "000000000000000000000000000000006469736376350001010102030405060708090a0b0c00180102030405060708090a0b0c0d0e0f100000000000000000"
    );

    let session = derive_session_keys(
        &ephemeral_key,
        &dest_pubkey,
        &node_id_a,
        &node_id_b,
        &challenge_data,
        true, // initiator
    );
    assert_eq!(
        session.outbound_key,
        hex!("dccc82d81bd610f4f76d3ebe97a40571")
    );
    assert_eq!(
        session.inbound_key,
        hex!("ac74bb8773749920b0d3a8881c173ec5")
    );
}

#[test]
fn id_signature_matches_vector() {
    let static_key = SecretKey::from_byte_array(&hex!(
        "fb757dc581730490a1d7a00deea65e9b1936924caaea8f44d476014856b68736"
    ))
    .unwrap();
    let challenge_data = hex!(
        "000000000000000000000000000000006469736376350001010102030405060708090a0b0c00180102030405060708090a0b0c0d0e0f100000000000000000"
    );
    let ephemeral_pubkey =
        hex!("039961e4c2356d61bedb83052c115d311acb3a96f5777296dcf297351130266231");
    let node_id_b = H256::from_slice(&hex!(
        "bbbb9d047f0488c0b5a93c1c3f2d8bafc7c8ff337024a55434a0d0555de64db9"
    ));

    let signature =
        create_id_signature(&static_key, &challenge_data, &ephemeral_pubkey, &node_id_b);
    assert_eq!(
        signature.serialize_compact(),
        hex!(
            "94852a1e2318c4e5e9d422c98eaf19d1d90d876b29cd06ca7cb7546d0fff7b484fe86c09a064fe72bdbef73ba8e9c34df0cd2b53e9d65528c2c7f336d5dfc6e6"
        )
    );

    // Verify the signature
    let src_pubkey = static_key.public_key(secp256k1::SECP256K1);
    assert!(verify_id_signature(
        &src_pubkey,
        &challenge_data,
        &ephemeral_pubkey,
        &node_id_b,
        &signature
    ));
}
