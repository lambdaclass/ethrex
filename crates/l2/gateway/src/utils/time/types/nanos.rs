use std::{
    num::ParseIntError,
    ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Sub, SubAssign},
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::Duration;
use crate::utils::time::global_clock;

/// Nanos since unix epoch, good till 2554 I think
#[derive(Copy, Clone, Debug, Default, Serialize, Deserialize)]
#[repr(C)]
pub struct Nanos(pub u64);

impl Nanos {
    pub const MAX: Nanos = Nanos(u64::MAX);
    pub const ZERO: Nanos = Nanos(0);

    pub const fn from_secs(s: u64) -> Self {
        Nanos(s * 1_000_000_000)
    }

    pub fn from_secs_f64(s: f64) -> Self {
        Nanos((s * 1_000_000_000.0).round() as u64)
    }

    pub fn from_millis_f64(s: f64) -> Self {
        Nanos((s * 1_000_000.0).round() as u64)
    }

    pub const fn from_millis(s: u64) -> Self {
        Nanos(s * 1_000_000)
    }

    pub const fn from_micros(s: u64) -> Self {
        Nanos(s * 1_000)
    }

    pub const fn from_mins(s: u64) -> Self {
        Nanos(s * 60 * 1_000_000_000)
    }

    pub const fn from_hours(s: u64) -> Self {
        Nanos::from_mins(s * 60)
    }

    pub fn from_rfc3339(datetime_str: &str) -> Option<Self> {
        match chrono::DateTime::parse_from_rfc3339(datetime_str)
            .map(|d| d.timestamp_nanos_opt().map(Self::from))
        {
            Ok(Some(n)) => Some(n),
            Ok(None) => {
                warn!("timestamp out of nanoseconds reach, using ingestion time");
                None
            }
            Err(e) => {
                warn!("Couldn't parse timestamp {}: {e}", datetime_str);
                None
            }
        }
    }

    pub fn as_secs(&self) -> f64 {
        self.0 as f64 / 1_000_000_000.0
    }

    pub fn as_millis(&self) -> f64 {
        self.0 as f64 / 1_000_000.0
    }

    pub fn as_millis_u64(&self) -> u64 {
        self.0 / 1_000_000
    }

    pub fn as_micros(&self) -> f64 {
        self.0 as f64 / 1_000.0
    }

    pub fn now() -> Self {
        SystemTime::now().into()
    }

    pub fn saturating_sub(self, rhs: Nanos) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }

    pub fn elapsed(&self) -> Self {
        let curt = Self::now();
        Nanos(curt.0 - self.0)
    }
}

impl From<Nanos> for chrono::DateTime<Utc> {
    fn from(value: Nanos) -> Self {
        chrono::DateTime::from_timestamp_nanos(value.0 as i64)
    }
}

impl std::fmt::Display for Nanos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if *self < Nanos::from_micros(1) {
            write!(f, "{}ns", self.0)
        } else if *self < Nanos::from_millis(1) {
            write!(f, "{}μs", self.0 as f64 / 1000.0)
        } else if *self < Nanos::from_secs(1) {
            write!(f, "{}ms", (self.0 / 1000) as f64 / 1000.0)
        } else if *self < Nanos::from_mins(1) {
            write!(f, "{}s", (self.0 / 1_000_000) as f64 / 1000.0)
        } else if *self < Nanos::from_hours(1) {
            let min = self.0 / Nanos::from_mins(1).0;
            let s = *self - Nanos::from_mins(min);
            write!(f, "{}m:{}", min, s)
        } else if *self < Nanos::from_hours(24) {
            let hours = self.0 / Nanos::from_hours(1).0;
            let min = *self - Nanos::from_hours(hours);
            write!(f, "{}h:{}", hours, min)
        } else {
            write!(f, "{}", chrono::DateTime::<Utc>::from(*self))
        }
    }
}

impl From<Nanos> for u64 {
    fn from(value: Nanos) -> Self {
        value.0
    }
}

impl Add for Nanos {
    type Output = Nanos;

    fn add(self, rhs: Nanos) -> Nanos {
        Nanos(self.0.wrapping_add(rhs.0))
    }
}

impl AddAssign for Nanos {
    fn add_assign(&mut self, rhs: Nanos) {
        *self = *self + rhs;
    }
}

impl Sub for Nanos {
    type Output = Nanos;

    fn sub(self, rhs: Nanos) -> Nanos {
        Nanos(self.0.wrapping_sub(rhs.0))
    }
}

impl SubAssign for Nanos {
    fn sub_assign(&mut self, rhs: Nanos) {
        *self = *self - rhs;
    }
}

impl Sub<u64> for Nanos {
    type Output = Nanos;

    fn sub(self, rhs: u64) -> Nanos {
        Nanos(self.0.wrapping_sub(rhs))
    }
}

impl SubAssign<u64> for Nanos {
    fn sub_assign(&mut self, rhs: u64) {
        *self = *self - rhs;
    }
}

impl Mul<u32> for Nanos {
    type Output = Nanos;

    fn mul(self, rhs: u32) -> Nanos {
        Nanos(self.0 * rhs as u64)
    }
}

impl Mul<i32> for Nanos {
    type Output = Nanos;

    fn mul(self, rhs: i32) -> Nanos {
        Nanos(self.0 * rhs as u64)
    }
}

impl Mul<Nanos> for u32 {
    type Output = Nanos;

    fn mul(self, rhs: Nanos) -> Nanos {
        rhs * self
    }
}
impl Mul<Nanos> for i32 {
    type Output = Nanos;

    fn mul(self, rhs: Nanos) -> Nanos {
        rhs * self
    }
}

impl MulAssign<u32> for Nanos {
    fn mul_assign(&mut self, rhs: u32) {
        *self = *self * rhs;
    }
}

impl Div<u32> for Nanos {
    type Output = Nanos;

    fn div(self, rhs: u32) -> Nanos {
        Nanos(self.0 / rhs as u64)
    }
}
impl Div<usize> for Nanos {
    type Output = Nanos;

    fn div(self, rhs: usize) -> Nanos {
        Nanos(self.0 / rhs as u64)
    }
}

impl DivAssign<u32> for Nanos {
    fn div_assign(&mut self, rhs: u32) {
        *self = *self / rhs;
    }
}
impl Mul<u64> for Nanos {
    type Output = Nanos;

    fn mul(self, rhs: u64) -> Nanos {
        Nanos(self.0 * rhs)
    }
}

impl Mul<Nanos> for u64 {
    type Output = Nanos;

    fn mul(self, rhs: Nanos) -> Nanos {
        rhs * self
    }
}

impl Mul<Nanos> for Nanos {
    type Output = Nanos;

    fn mul(self, rhs: Nanos) -> Nanos {
        Nanos(rhs.0 * self.0)
    }
}

impl MulAssign<u64> for Nanos {
    fn mul_assign(&mut self, rhs: u64) {
        *self = *self * rhs;
    }
}

impl Div<u64> for Nanos {
    type Output = Nanos;

    fn div(self, rhs: u64) -> Nanos {
        Nanos(self.0 / rhs)
    }
}

impl DivAssign<u64> for Nanos {
    fn div_assign(&mut self, rhs: u64) {
        *self = *self / rhs;
    }
}

impl Div<Nanos> for Nanos {
    type Output = Nanos;

    fn div(self, rhs: Nanos) -> Nanos {
        Nanos(self.0 / rhs.0)
    }
}

impl DivAssign<Nanos> for Nanos {
    fn div_assign(&mut self, rhs: Nanos) {
        self.0 /= rhs.0
    }
}

impl PartialEq for Nanos {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for Nanos {}

impl PartialOrd for Nanos {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Nanos {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl std::iter::Sum for Nanos {
    fn sum<I>(iter: I) -> Self
    where
        I: Iterator<Item = Self>,
    {
        Nanos(iter.map(|v| v.0).sum())
    }
}
impl<'a> std::iter::Sum<&'a Self> for Nanos {
    fn sum<I>(iter: I) -> Self
    where
        I: Iterator<Item = &'a Self>,
    {
        Nanos(iter.map(|v| v.0).sum())
    }
}

impl From<u64> for Nanos {
    fn from(value: u64) -> Self {
        Nanos(value)
    }
}
impl From<u128> for Nanos {
    fn from(value: u128) -> Self {
        Nanos(value as u64)
    }
}
impl From<u32> for Nanos {
    fn from(value: u32) -> Self {
        Nanos(value as u64)
    }
}
impl From<i64> for Nanos {
    fn from(value: i64) -> Self {
        Nanos(value as u64)
    }
}
impl From<i32> for Nanos {
    fn from(value: i32) -> Self {
        Nanos(value as u64)
    }
}

impl From<Nanos> for i64 {
    fn from(val: Nanos) -> Self {
        val.0 as i64
    }
}

impl From<SystemTime> for Nanos {
    fn from(value: SystemTime) -> Self {
        Nanos(unsafe {
            value
                .duration_since(UNIX_EPOCH)
                .unwrap_unchecked()
                .as_nanos() as u64
                - TIME_OFFSET_NANOS.load(Ordering::Relaxed)
        })
    }
}

impl FromStr for Nanos {
    type Err = ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.contains("ns") {
            s.parse::<u64>().map(Nanos)
        } else {
            let len = s.len();
            s[..len - 2].parse::<u64>().map(Nanos)
        }
    }
}

impl From<Duration> for Nanos {
    fn from(value: Duration) -> Self {
        Nanos(global_clock().delta_as_nanos(0, value.0))
    }
}

impl From<Nanos> for std::time::Duration {
    fn from(value: Nanos) -> Self {
        std::time::Duration::from_nanos(value.0)
    }
}

/// Sets a spoof 'now' time - this adds an offset to the reported system time in order to allow
/// time travel.
static TIME_OFFSET_NANOS: AtomicU64 = AtomicU64::new(0);
pub fn set_time_delta(duration_since_epoch: std::time::Duration) {
    let since_epoch_now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    assert!(since_epoch_now > duration_since_epoch);
    let delta_nanos = (since_epoch_now.as_nanos() - duration_since_epoch.as_nanos()) as u64;
    TIME_OFFSET_NANOS.store(delta_nanos, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde() {
        assert_eq!("1", serde_json::to_string(&Nanos(1)).unwrap());
    }
}
