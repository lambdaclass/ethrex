use std::ops::{Add, AddAssign, Sub};

use serde::{Deserialize, Serialize};

use super::{Duration, Nanos};
use crate::utils::time::{global_clock, nanos_for_100, rdtscp};
// Socket is in the top 2 bits, rdtscp counter in lower 62
#[derive(Copy, Clone, Debug, Default, Serialize, Deserialize)]
#[repr(C)]
pub struct Instant(pub u64);
impl Instant {
    pub const MAX: Self = Self(u64::MAX);
    pub const ZERO: Self = Self(0);

    #[inline]
    pub fn now() -> Self {
        Instant(rdtscp())
    }

    #[inline]
    fn remove_socket(self) -> Self {
        Instant(self.0 & 0x3fffffffffffffff)
    }

    #[inline]
    pub fn socket(&self) -> u64 {
        self.0 & 0xc000000000000000
    }

    #[inline]
    pub fn same_socket(&self, other: &Self) -> bool {
        self.socket() == other.socket()
    }

    #[inline]
    pub fn elapsed(&self) -> Duration {
        let curt = Instant::now();
        curt.wrapping_sub(self)
    }

    #[inline]
    pub fn as_delta_nanos(&self) -> Nanos {
        Nanos(global_clock().delta_as_nanos(0, self.remove_socket().0))
    }

    #[inline]
    pub fn wrapping_sub(&self, other: &Instant) -> Duration {
        Duration(self.0.wrapping_sub(other.0))
    }

    #[inline]
    pub fn saturating_sub(&self, other: &Instant) -> Duration {
        Duration(self.0.saturating_sub(other.0))
    }
}

impl PartialEq for Instant {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for Instant {}

impl PartialOrd for Instant {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Instant {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl Sub for Instant {
    type Output = Duration;

    fn sub(self, rhs: Instant) -> Duration {
        Duration(self.0.saturating_sub(rhs.0))
    }
}

impl Sub<Nanos> for Instant {
    type Output = Instant;

    fn sub(self, rhs: Nanos) -> Instant {
        Instant(self.0.saturating_sub(rhs.0) / nanos_for_100() * 100)
    }
}

impl Add<Nanos> for Instant {
    type Output = Instant;

    fn add(self, rhs: Nanos) -> Self::Output {
        Instant(self.0 + rhs.0 / nanos_for_100() * 100)
    }
}

impl Add<Duration> for Instant {
    type Output = Instant;

    fn add(self, rhs: Duration) -> Self::Output {
        Instant(self.0 + rhs.0)
    }
}

impl AddAssign<Duration> for Instant {
    fn add_assign(&mut self, rhs: Duration) {
        self.0 += rhs.0
    }
}
