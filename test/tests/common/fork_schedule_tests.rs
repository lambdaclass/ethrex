//! A post-merge fork schedule with a gap is reported rather than resolved
//! silently.
//!
//! Fork rules resolve from the fork ordinal, so scheduling a fork activates the
//! rules of every fork below it. Checks keyed on an individual fork's own
//! activation timestamp do not follow, which makes a gap in the schedule
//! diverge from itself: `ChainConfig::is_amsterdam_activated` stays false while
//! `get_fork` already returns a fork at or above Amsterdam.

use ethrex_common::types::{ChainConfig, Fork};

/// A schedule with every post-merge fork set, Verkle excluded as ever.
fn full_schedule() -> ChainConfig {
    ChainConfig {
        shanghai_time: Some(0),
        cancun_time: Some(0),
        prague_time: Some(0),
        osaka_time: Some(0),
        amsterdam_time: Some(1_000),
        hegota_time: Some(2_000),
        ..Default::default()
    }
}

#[test]
fn a_complete_schedule_reports_no_gap() {
    assert!(
        full_schedule().unscheduled_predecessor_forks().is_empty(),
        "a fully scheduled chain must not be flagged"
    );
}

#[test]
fn an_unset_verkle_is_not_a_gap() {
    // Verkle is a placeholder that no real schedule sets, so it must never be
    // reported even though later forks are scheduled.
    let config = full_schedule();
    assert_eq!(config.verkle_time, None);
    assert!(config.unscheduled_predecessor_forks().is_empty());
}

#[test]
fn hegota_without_amsterdam_is_reported() {
    let config = ChainConfig {
        amsterdam_time: None,
        ..full_schedule()
    };
    assert_eq!(config.unscheduled_predecessor_forks(), vec!["Amsterdam"]);
}

/// The gap is exactly the condition under which the ordinal and the timestamp
/// field disagree. Asserted against `get_fork`/`is_amsterdam_activated` directly
/// rather than through the reporting helper, so this cannot pass by construction.
#[test]
fn the_reported_gap_is_where_the_ordinal_and_the_field_disagree() {
    let config = ChainConfig {
        amsterdam_time: None,
        ..full_schedule()
    };
    let post_hegota = 3_000;

    assert!(config.get_fork(post_hegota) >= Fork::Amsterdam);
    assert!(
        !config.is_amsterdam_activated(post_hegota),
        "the field-keyed check is inactive at a timestamp whose ordinal is past Amsterdam"
    );
    assert!(!config.unscheduled_predecessor_forks().is_empty());
}

#[test]
fn several_gaps_are_reported_in_activation_order() {
    let config = ChainConfig {
        cancun_time: None,
        osaka_time: None,
        ..full_schedule()
    };
    assert_eq!(
        config.unscheduled_predecessor_forks(),
        vec!["Cancun", "Osaka"]
    );
}

/// A chain that has simply not reached later forks yet has no gap: the unset
/// fields are all above the last scheduled one.
#[test]
fn trailing_unset_forks_are_not_a_gap() {
    let config = ChainConfig {
        shanghai_time: Some(0),
        cancun_time: Some(0),
        ..Default::default()
    };
    assert!(config.unscheduled_predecessor_forks().is_empty());
}

#[test]
fn a_pre_merge_chain_reports_no_gap() {
    assert!(
        ChainConfig::default()
            .unscheduled_predecessor_forks()
            .is_empty()
    );
}

#[test]
fn display_config_warns_about_a_gap() {
    let config = ChainConfig {
        amsterdam_time: None,
        ..full_schedule()
    };
    let shown = config.display_config();
    assert!(shown.contains("WARNING"), "no warning in:\n{shown}");
    assert!(shown.contains("Amsterdam"), "gap not named in:\n{shown}");

    assert!(
        !full_schedule().display_config().contains("WARNING"),
        "a complete schedule must not warn"
    );
}
