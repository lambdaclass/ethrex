use ethereum_types::U256;
use hex::FromHexError;
use keccak_hash::H256;
use std::{
    collections::BTreeMap,
    fmt::{self, Debug},
    sync::{Arc, Mutex, Weak},
};

/// Converts a big endian slice to a u256, faster than `u256::from_big_endian`.
pub fn u256_from_big_endian(slice: &[u8]) -> U256 {
    let mut padded = [0u8; 32];
    padded[32 - slice.len()..32].copy_from_slice(slice);

    let mut ret = [0; 4];

    let mut u64_bytes = [0u8; 8];
    for i in 0..4 {
        u64_bytes.copy_from_slice(&padded[8 * i..(8 * i + 8)]);
        ret[4 - i - 1] = u64::from_be_bytes(u64_bytes);
    }

    U256(ret)
}

/// Converts a constant big endian slice to a u256, faster than `u256::from_big_endian` and `u256_from_big_endian`.
///
/// Note: N should not exceed 32.
pub fn u256_from_big_endian_const<const N: usize>(slice: [u8; N]) -> U256 {
    const { assert!(N <= 32, "N must be less or equal to 32") };

    let mut padded = [0u8; 32];
    padded[32 - N..32].copy_from_slice(&slice);

    let mut ret = [0u64; 4];

    let mut u64_bytes = [0u8; 8];
    for i in 0..4 {
        u64_bytes.copy_from_slice(&padded[8 * i..(8 * i + 8)]);
        ret[4 - i - 1] = u64::from_be_bytes(u64_bytes);
    }

    U256(ret)
}

/// Converts a U256 to a big endian slice.
#[inline(always)]
pub fn u256_to_big_endian(value: U256) -> [u8; 32] {
    let mut bytes = [0u8; 32];

    for i in 0..4 {
        let u64_be = value.0[4 - i - 1].to_be_bytes();
        bytes[8 * i..(8 * i + 8)].copy_from_slice(&u64_be);
    }

    bytes
}

#[inline(always)]
pub fn u256_to_h256(value: U256) -> H256 {
    H256(u256_to_big_endian(value))
}

pub fn decode_hex(hex: &str) -> Result<Vec<u8>, FromHexError> {
    let trimmed = hex.strip_prefix("0x").unwrap_or(hex);
    hex::decode(trimmed)
}

#[derive(Default)]
pub struct EventHandlerSet<T> {
    handlers: Arc<Mutex<BTreeMap<usize, Box<dyn 'static + Send + Sync + Fn(&T)>>>>,
}

impl<T> EventHandlerSet<T> {
    pub fn add(&self, f: impl 'static + Send + Sync + Fn(&T)) -> EventHandle<T> {
        let mut handlers = self.handlers.lock().expect("poisoned mutex");
        let index = handlers
            .last_entry()
            .map(|x| *x.key() + 1)
            .unwrap_or_default();

        handlers.insert(index, Box::new(f));
        EventHandle {
            handlers: Arc::downgrade(&self.handlers),
            index,
        }
    }

    pub fn send(&self, value: &T) {
        for (_, handler) in self.handlers.lock().expect("poisoned mutex").iter() {
            handler(value);
        }
    }
}

impl<T> Debug for EventHandlerSet<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventHandlerSet").finish_non_exhaustive()
    }
}

pub struct EventHandle<T> {
    handlers: Weak<Mutex<BTreeMap<usize, Box<dyn 'static + Send + Sync + Fn(&T)>>>>,
    index: usize,
}

impl<T> Debug for EventHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventHandle").finish_non_exhaustive()
    }
}

impl<T> Drop for EventHandle<T> {
    fn drop(&mut self) {
        if let Some(handlers) = self.handlers.upgrade() {
            _ = handlers.lock().expect("poisoned mutex").remove(&self.index);
        }
    }
}

#[cfg(test)]
mod test {
    use ethereum_types::U256;

    use crate::utils::u256_to_big_endian;

    #[test]
    fn u256_to_big_endian_test() {
        let a = u256_to_big_endian(U256::one());
        let b = U256::one().to_big_endian();
        assert_eq!(a, b);

        let a = u256_to_big_endian(U256::max_value());
        let b = U256::max_value().to_big_endian();
        assert_eq!(a, b);
    }
}
