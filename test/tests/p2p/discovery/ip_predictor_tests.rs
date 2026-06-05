use ethrex_common::H256;
use ethrex_p2p::discovery::IpPredictor;
use rustc_hash::FxHashSet;
use std::{net::IpAddr, time::Instant};

#[tokio::test]
async fn test_ip_voting_returns_winning_ip() {
    let mut predictor = IpPredictor::default();

    let new_ip: IpAddr = "203.0.113.50".parse().unwrap();
    let voter1 = H256::from_low_u64_be(1);
    let voter2 = H256::from_low_u64_be(2);
    let voter3 = H256::from_low_u64_be(3);

    assert_eq!(predictor.record_ip_vote(new_ip, voter1), None);
    assert_eq!(predictor.record_ip_vote(new_ip, voter2), None);
    // Third vote triggers round end, returns the winning IP
    assert_eq!(predictor.record_ip_vote(new_ip, voter3), Some(new_ip));
    assert!(predictor.ip_votes.is_empty());
}

#[tokio::test]
async fn test_ip_voting_same_peer_votes_once() {
    let mut predictor = IpPredictor::default();

    let new_ip: IpAddr = "203.0.113.50".parse().unwrap();
    let same_voter = H256::from_low_u64_be(1);

    predictor.record_ip_vote(new_ip, same_voter);
    predictor.record_ip_vote(new_ip, same_voter);
    predictor.record_ip_vote(new_ip, same_voter);

    assert_eq!(predictor.ip_votes.get(&new_ip).map(|v| v.len()), Some(1));
}

#[tokio::test]
async fn test_ip_voting_ignores_private_ips() {
    let mut predictor = IpPredictor::default();

    let voter1 = H256::from_low_u64_be(1);

    let private_ip: IpAddr = "192.168.1.100".parse().unwrap();
    predictor.record_ip_vote(private_ip, voter1);
    assert!(predictor.ip_votes.is_empty());

    let loopback: IpAddr = "127.0.0.1".parse().unwrap();
    predictor.record_ip_vote(loopback, voter1);
    assert!(predictor.ip_votes.is_empty());

    let public_ip: IpAddr = "203.0.113.50".parse().unwrap();
    predictor.record_ip_vote(public_ip, voter1);
    assert_eq!(predictor.ip_votes.get(&public_ip).map(|v| v.len()), Some(1));
}

#[tokio::test]
async fn test_ip_voting_split_votes_no_winner() {
    let mut predictor = IpPredictor::default();

    let ip1: IpAddr = "203.0.113.50".parse().unwrap();
    let ip2: IpAddr = "203.0.113.51".parse().unwrap();
    let voter1 = H256::from_low_u64_be(1);
    let voter2 = H256::from_low_u64_be(2);
    let voter3 = H256::from_low_u64_be(3);

    predictor.record_ip_vote(ip1, voter1);
    predictor.record_ip_vote(ip2, voter2);
    // ip1 has 2 votes, ip2 has 1 — ip1 wins but only has 2 < threshold 3
    assert_eq!(predictor.record_ip_vote(ip1, voter3), None);
    assert!(predictor.ip_votes.is_empty());
    assert!(predictor.first_ip_vote_round_completed);
}

#[tokio::test]
async fn test_ip_vote_cleanup() {
    let mut predictor = IpPredictor::default();

    let ip: IpAddr = "203.0.113.50".parse().unwrap();
    let voter1 = H256::from_low_u64_be(1);

    let mut voters = FxHashSet::default();
    voters.insert(voter1);
    predictor.ip_votes.insert(ip, voters);
    predictor.ip_vote_period_start = Some(Instant::now());
    assert_eq!(predictor.ip_votes.len(), 1);

    // check_timeout should retain votes (round hasn't timed out yet)
    assert_eq!(predictor.check_timeout(), None);
    assert_eq!(predictor.ip_votes.len(), 1);
    assert!(!predictor.first_ip_vote_round_completed);
}

#[tokio::test]
async fn test_discv4_pong_observation_feeds_predictor() {
    let mut predictor = IpPredictor::default();

    let public_ip: IpAddr = "203.0.113.42".parse().unwrap();
    let voter1 = H256::from_low_u64_be(1);
    let voter2 = H256::from_low_u64_be(2);
    let voter3 = H256::from_low_u64_be(3);

    assert_eq!(predictor.record_ip_vote(public_ip, voter1), None);
    assert_eq!(predictor.record_ip_vote(public_ip, voter2), None);
    // Third distinct voter triggers round completion
    assert_eq!(predictor.record_ip_vote(public_ip, voter3), Some(public_ip));
}
