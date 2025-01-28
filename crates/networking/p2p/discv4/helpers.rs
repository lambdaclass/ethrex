use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn get_msg_expiration_from_seconds(seconds: u64) -> u64 {
    (SystemTime::now() + Duration::from_secs(seconds))
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn is_msg_expired(expiration: u64) -> bool {
    // this cast to a signed integer is needed as the rlp decoder doesn't take into account the sign
    // otherwise if a msg contains a negative expiration, it would pass since as it would wrap around the u64.
    (expiration as i64) < (current_unix_time() as i64)
}

pub fn elapsed_time_since(unix_timestamp: u64) -> u64 {
    let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(unix_timestamp);
    SystemTime::now()
        .duration_since(time)
        .unwrap_or_default()
        .as_secs()
}

pub fn current_unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
