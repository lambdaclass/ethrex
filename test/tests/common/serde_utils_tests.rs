use std::time::Duration;

use ethrex_common::serde_utils::parse_duration;

#[test]
fn parse_duration_simple_integers() {
    assert_eq!(
        parse_duration("24h".to_string()),
        Some(Duration::from_secs(60 * 60 * 24))
    );
    assert_eq!(
        parse_duration("20m".to_string()),
        Some(Duration::from_secs(60 * 20))
    );
    assert_eq!(
        parse_duration("13s".to_string()),
        Some(Duration::from_secs(13))
    );
    assert_eq!(
        parse_duration("500ms".to_string()),
        Some(Duration::from_millis(500))
    );
    assert_eq!(
        parse_duration("900µs".to_string()),
        Some(Duration::from_micros(900))
    );
    assert_eq!(
        parse_duration("900us".to_string()),
        Some(Duration::from_micros(900))
    );
    assert_eq!(
        parse_duration("40ns".to_string()),
        Some(Duration::from_nanos(40))
    );
}

#[test]
fn parse_duration_mixed_integers() {
    assert_eq!(
        parse_duration("24h30m".to_string()),
        Some(Duration::from_secs(60 * 60 * 24 + 30 * 60))
    );
    assert_eq!(
        parse_duration("20m15s".to_string()),
        Some(Duration::from_secs(60 * 20 + 15))
    );
    assert_eq!(
        parse_duration("13s4ms".to_string()),
        Some(Duration::from_secs(13) + Duration::from_millis(4))
    );
    assert_eq!(
        parse_duration("500ms60µs".to_string()),
        Some(Duration::from_millis(500) + Duration::from_micros(60))
    );
    assert_eq!(
        parse_duration("900us21ns".to_string()),
        Some(Duration::from_micros(900) + Duration::from_nanos(21))
    );
}

#[test]
fn parse_duration_simple_with_decimals() {
    assert_eq!(
        parse_duration("1.5h".to_string()),
        Some(Duration::from_secs(60 * 90))
    );
    assert_eq!(
        parse_duration("0.5m".to_string()),
        Some(Duration::from_secs(30))
    );
    assert_eq!(
        parse_duration("4.5s".to_string()),
        Some(Duration::from_secs_f32(4.5))
    );
    assert_eq!(
        parse_duration("0.8ms".to_string()),
        Some(Duration::from_micros(800))
    );
    assert_eq!(
        parse_duration("0.95us".to_string()),
        Some(Duration::from_nanos(950))
    );
    // Rounded Up
    assert_eq!(
        parse_duration("0.75ns".to_string()),
        Some(Duration::from_nanos(1))
    );
}

#[test]
fn parse_duration_mixed_decimals() {
    assert_eq!(
        parse_duration("1.5h0.5m10s".to_string()),
        Some(Duration::from_secs(60 * 90 + 30 + 10))
    );
    assert_eq!(
        parse_duration("0.5m15s".to_string()),
        Some(Duration::from_secs(30 + 15))
    );
}
