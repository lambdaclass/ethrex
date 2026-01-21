use ethrex_crypto::keccak::keccak_hash;
use ethrex_p2p::discovery::is_discv4_packet;

#[test]
fn test_is_discv4_packet_valid() {
    // Create a fake discv4 packet with valid hash
    let mut packet = vec![0u8; 100];
    // Fill the "rest of packet" (after hash) with some data
    for (i, byte) in packet[32..].iter_mut().enumerate() {
        *byte = i as u8;
    }
    // Compute hash and put at the beginning
    let hash = keccak_hash(&packet[32..]);
    packet[0..32].copy_from_slice(&hash);

    assert!(is_discv4_packet(&packet));
}

#[test]
fn test_is_discv4_packet_corrupted_hash() {
    // Create a fake discv4 packet with valid hash
    let mut packet = vec![0u8; 100];
    for (i, byte) in packet[32..].iter_mut().enumerate() {
        *byte = i as u8;
    }
    let hash = keccak_hash(&packet[32..]);
    packet[0..32].copy_from_slice(&hash);

    // Corrupt the hash
    packet[0] ^= 0xFF;
    assert!(!is_discv4_packet(&packet));
}

#[test]
fn test_is_discv4_packet_too_short() {
    let packet = vec![0u8; 50]; // Less than minimum size (98 bytes)
    assert!(!is_discv4_packet(&packet));
}

#[test]
fn test_is_discv4_packet_exactly_minimum_size() {
    // Create a packet with exactly the minimum size (98 bytes)
    let mut packet = vec![0u8; 98];
    for (i, byte) in packet[32..].iter_mut().enumerate() {
        *byte = i as u8;
    }
    let hash = keccak_hash(&packet[32..]);
    packet[0..32].copy_from_slice(&hash);

    assert!(is_discv4_packet(&packet));
}

#[test]
fn test_is_discv4_packet_random_data() {
    // Random data that doesn't have a valid hash should not be detected as discv4
    let packet = vec![42u8; 200];
    assert!(!is_discv4_packet(&packet));
}
