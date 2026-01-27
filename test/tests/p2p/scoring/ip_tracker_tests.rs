use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use ethrex_common::H256;
use ethrex_p2p::scoring::{IpColocationTracker, IpPrefix};

fn make_peer_id(n: u8) -> H256 {
    H256([n; 32])
}

#[test]
fn test_ip_prefix_ipv4() {
    let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
    let prefix = IpPrefix::from_ip(ip);

    assert!(!prefix.is_ipv6());
    assert_eq!(prefix.display(), "192.168.1.0/24");

    // Same /24 should produce same prefix
    let ip2 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 200));
    let prefix2 = IpPrefix::from_ip(ip2);
    assert_eq!(prefix, prefix2);

    // Different /24 should produce different prefix
    let ip3 = IpAddr::V4(Ipv4Addr::new(192, 168, 2, 100));
    let prefix3 = IpPrefix::from_ip(ip3);
    assert_ne!(prefix, prefix3);
}

#[test]
fn test_ip_prefix_ipv6() {
    let ip = IpAddr::V6(Ipv6Addr::new(
        0x2001, 0x0db8, 0x85a3, 0x0000, 0x0000, 0x8a2e, 0x0370, 0x7334,
    ));
    let prefix = IpPrefix::from_ip(ip);

    assert!(prefix.is_ipv6());
}

#[test]
fn test_no_penalty_under_threshold() {
    let mut tracker = IpColocationTracker::new(3, 1.5);

    // Add 3 peers from same subnet (at threshold)
    for i in 0..3 {
        tracker.add_peer(make_peer_id(i), IpAddr::V4(Ipv4Addr::new(192, 168, 1, i)));
    }

    // No penalty at threshold
    assert_eq!(tracker.colocation_penalty(&make_peer_id(0)), 0.0);
}

#[test]
fn test_quadratic_penalty_above_threshold() {
    let mut tracker = IpColocationTracker::new(3, 1.0);

    // Add 5 peers from same subnet (2 over threshold)
    for i in 0..5 {
        tracker.add_peer(make_peer_id(i), IpAddr::V4(Ipv4Addr::new(192, 168, 1, i)));
    }

    // Penalty should be 1.0 * (5-3)^2 = 4.0
    let penalty = tracker.colocation_penalty(&make_peer_id(0));
    assert!(
        (penalty - 4.0).abs() < 0.01,
        "Expected 4.0, got {}",
        penalty
    );

    // Add more peers
    for i in 5..8 {
        tracker.add_peer(make_peer_id(i), IpAddr::V4(Ipv4Addr::new(192, 168, 1, i)));
    }

    // Penalty should be 1.0 * (8-3)^2 = 25.0
    let penalty = tracker.colocation_penalty(&make_peer_id(0));
    assert!(
        (penalty - 25.0).abs() < 0.01,
        "Expected 25.0, got {}",
        penalty
    );
}

#[test]
fn test_remove_peer() {
    let mut tracker = IpColocationTracker::new(2, 1.0);

    // Add 4 peers from same subnet
    for i in 0..4 {
        tracker.add_peer(make_peer_id(i), IpAddr::V4(Ipv4Addr::new(192, 168, 1, i)));
    }

    assert_eq!(tracker.peers_in_same_prefix(&make_peer_id(0)), 4);

    // Remove one peer
    tracker.remove_peer(&make_peer_id(0));

    // Remaining peers should now only see 3
    assert_eq!(tracker.peers_in_same_prefix(&make_peer_id(1)), 3);

    // Removed peer should have no penalty
    assert_eq!(tracker.colocation_penalty(&make_peer_id(0)), 0.0);
}

#[test]
fn test_different_subnets() {
    let mut tracker = IpColocationTracker::new(2, 1.0);

    // Add peers from different subnets
    tracker.add_peer(make_peer_id(0), IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
    tracker.add_peer(make_peer_id(1), IpAddr::V4(Ipv4Addr::new(192, 168, 2, 1)));
    tracker.add_peer(make_peer_id(2), IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));

    // Each peer is alone in their subnet
    assert_eq!(tracker.peers_in_same_prefix(&make_peer_id(0)), 1);
    assert_eq!(tracker.peers_in_same_prefix(&make_peer_id(1)), 1);
    assert_eq!(tracker.peers_in_same_prefix(&make_peer_id(2)), 1);

    // No penalties
    assert_eq!(tracker.colocation_penalty(&make_peer_id(0)), 0.0);
}

#[test]
fn test_congested_prefixes() {
    let mut tracker = IpColocationTracker::new(2, 1.0);

    // Add peers to create one congested subnet
    for i in 0..5 {
        tracker.add_peer(make_peer_id(i), IpAddr::V4(Ipv4Addr::new(192, 168, 1, i)));
    }

    // Add peers to a normal subnet
    tracker.add_peer(make_peer_id(10), IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
    tracker.add_peer(make_peer_id(11), IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)));

    let congested = tracker.congested_prefixes();
    assert_eq!(congested.len(), 1);
    assert_eq!(congested[0].1, 5); // 5 peers in congested subnet
}

#[test]
fn test_peer_ip_update() {
    let mut tracker = IpColocationTracker::new(2, 1.0);

    // Add peer to one subnet
    tracker.add_peer(make_peer_id(0), IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
    assert_eq!(tracker.peers_in_same_prefix(&make_peer_id(0)), 1);

    // Move peer to another subnet
    tracker.add_peer(make_peer_id(0), IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));

    // Peer should be in new subnet
    assert_eq!(tracker.peers_in_same_prefix(&make_peer_id(0)), 1);

    // Old prefix should be empty (tracked peers = 1 for new prefix)
    assert_eq!(tracker.prefix_count(), 1);
}
