use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn get_expiration(seconds: u64) -> u64 {
    (SystemTime::now() + Duration::from_secs(seconds))
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn is_expired(expiration: u64) -> bool {
    // this cast to a signed integer is needed as the rlp decoder doesn't take into account the sign
    // otherwise a potential negative expiration would pass since it would take 2^64.
    (expiration as i64)
        < SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
}

pub fn time_since_in_hs(time: u64) -> u64 {
    let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(time);
    SystemTime::now()
        .duration_since(time)
        .unwrap_or_default()
        .as_secs()
        / 3600
}

pub fn current_unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
