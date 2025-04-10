use chrono::{Utc, TimeDelta};

pub fn get_msg_expiration_from_seconds(seconds: u32) -> u64 {
    let delta = TimeDelta::try_seconds(seconds.into());
    (Utc::now() + delta.unwrap_or_default()).timestamp().try_into().unwrap_or(1 << 63)
}

pub fn is_msg_expired(expiration: u64) -> bool {
    // this cast to a signed integer is needed as the rlp decoder doesn't take into account the sign
    // otherwise if a msg contains a negative expiration, it would pass since as it would wrap around the u64.
    (expiration as i64) < (current_unix_time() as i64)
}

pub fn elapsed_time_since(unix_timestamp: u64) -> u64 {
    current_unix_time().saturating_sub(unix_timestamp)
}

pub fn current_unix_time() -> u64 {
    Utc::now().timestamp().try_into().unwrap_or(1 << 63)
}
