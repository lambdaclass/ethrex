use ethrex_p2p::scoring::{FailureSeverity, RequestType};

#[test]
fn test_request_type_categorization() {
    assert!(RequestType::BlockHeaders.is_eth());
    assert!(RequestType::BlockBodies.is_eth());
    assert!(!RequestType::BlockHeaders.is_snap());

    assert!(RequestType::AccountRange.is_snap());
    assert!(RequestType::TrieNodes.is_snap());
    assert!(!RequestType::AccountRange.is_eth());
}

#[test]
fn test_failure_severity_ordering() {
    assert!(FailureSeverity::Low < FailureSeverity::Medium);
    assert!(FailureSeverity::Medium < FailureSeverity::High);
    assert!(FailureSeverity::High < FailureSeverity::Critical);
}

#[test]
fn test_failure_severity_penalties() {
    assert!(FailureSeverity::Low.penalty() > FailureSeverity::Medium.penalty());
    assert!(FailureSeverity::Medium.penalty() > FailureSeverity::High.penalty());
    assert!(FailureSeverity::High.penalty() > FailureSeverity::Critical.penalty());
}

#[test]
fn test_all_request_types() {
    let all = RequestType::all();
    assert_eq!(all.len(), 8);
}
