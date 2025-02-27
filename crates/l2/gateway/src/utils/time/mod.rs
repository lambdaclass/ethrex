pub mod repeater;
pub mod timer;
pub mod types;

use std::sync::OnceLock;

pub use repeater::*;
use serde::{Deserialize, Serialize};
pub use timer::*;
pub use types::*;
pub type Clock = quanta::Clock;

static GLOBAL_NANOS_FOR_100: OnceLock<u64> = OnceLock::new();
static GLOBAL_CLOCK: OnceLock<Clock> = OnceLock::new();

#[inline]
fn global_clock() -> &'static Clock {
    GLOBAL_CLOCK.get_or_init(Clock::new)
}

#[inline]
fn nanos_for_100() -> u64 {
    *GLOBAL_NANOS_FOR_100.get_or_init(|| global_clock().delta_as_nanos(0, 100))
}

/// Returns a high-precision timestamp counter:
/// - On x86: uses rdtscp (synchronized cycle counter)
/// - On ARM64: uses CNTVCT_EL0 (virtual timer counter)
/// - On other platforms: falls back to quanta's fastest timestamper
///
/// Performance: ~6-9ns on x86/ARM vs ~20ns for SystemTime::now
fn rdtscp() -> u64 {
    #[cfg(all(target_arch = "x86_64", not(target_arch = "wasm32")))]
    #[allow(clippy::uninit_assumed_init, invalid_value)]
    {
        let mut c = unsafe { std::mem::MaybeUninit::uninit().assume_init() };
        let o = unsafe { ::core::arch::x86_64::__rdtscp(&mut c) };
        o | ((c & 0x0000F000) as u64) << 50
    }
    #[cfg(all(target_arch = "aarch64", not(target_arch = "wasm32")))]
    {
        // CNTVCT_EL0 is a virtual counter that runs at a fixed frequency
        // and is accessible from userspace
        let value: u64;
        unsafe { core::arch::asm!("mrs {}, cntvct_el0", out(reg) value) };
        value
    }
    #[cfg(any(
        not(any(target_arch = "x86_64", target_arch = "aarch64")),
        target_arch = "wasm32"
    ))]
    {
        global_clock().raw()
    }
}

// TODO: move all of this to a more suited place
pub mod utils;

pub use types::{duration::Duration, instant::Instant, nanos::Nanos};
pub use utils::{vsync, vsync_with_cancel};

use crate::utils::communication::messages::InternalMessage;

/// A Timestamp that should be used for internal latency/performance tracking.
///
/// Should be instantiated at the very first time that a message arrives from the
/// network on the box. It takes a "real" timestamp, i.e. nanos since unix epoch,
/// and links it to the current value of the rdtscp counter.
/// When reporting internal latency/performance, the rdtscp counter alone should be used.
/// When creating an "exiting" message, i.e. a message leaving this box over the network,
/// the "real" time + linked rdtscp counter can be used to generate an approximate outgoing
/// "real" timestamp. This is approximate, but 2x faster than Nanos::now() which has
/// the same performance as SystemTime::now.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
pub struct IngestionTime {
    real: Nanos,
    internal: Instant,
}

impl IngestionTime {
    #[inline]
    pub fn now() -> Self {
        let real = Nanos::now();
        let internal = Instant::now();
        Self { real, internal }
    }

    #[inline]
    pub fn internal(&self) -> &Instant {
        &self.internal
    }

    #[inline]
    pub fn real(&self) -> Nanos {
        self.real
    }

    #[inline]
    pub fn to_msg<T>(&self, data: T) -> InternalMessage<T> {
        InternalMessage::new(*self, data)
    }
}

impl From<&IngestionTime> for Instant {
    #[inline]
    fn from(value: &IngestionTime) -> Self {
        value.internal
    }
}

impl From<&IngestionTime> for Nanos {
    #[inline]
    fn from(value: &IngestionTime) -> Self {
        value.real
    }
}

impl From<IngestionTime> for Instant {
    #[inline]
    fn from(value: IngestionTime) -> Self {
        value.internal
    }
}

impl From<IngestionTime> for Nanos {
    #[inline]
    fn from(value: IngestionTime) -> Self {
        value.real
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BlockSyncTimers {
    pub total: Timer,
    pub execution: Timer,
    pub execute_txs: Timer,
    pub validate: Timer,
    pub take_bundle: Timer,
    pub state_root: Timer,
    pub caches: Timer,
    pub state_changes: Timer,
    pub trie_updates: Timer,
    pub header_write: Timer,
    pub db_commit: Timer,
}
impl Default for BlockSyncTimers {
    fn default() -> Self {
        Self {
            total: Timer::new("BlockSync-total"),
            execution: Timer::new("BlockSync-execution"),
            execute_txs: Timer::new("BlockSync-execute_txs"),
            validate: Timer::new("BlockSync-validate"),
            take_bundle: Timer::new("BlockSync-take_bundle"),
            state_root: Timer::new("BlockSync-state_root"),
            caches: Timer::new("BlockSync-caches"),
            state_changes: Timer::new("BlockSync-state_changes"),
            trie_updates: Timer::new("BlockSync-trie_updates"),
            header_write: Timer::new("BlockSync-header_write"),
            db_commit: Timer::new("BlockSync-db_commit"),
        }
    }
}
