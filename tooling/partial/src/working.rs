//! The keys the node keeps proven at the anchor: pins plus keys requests read, bounded by
//! a cap with least-recently-used eviction. Only keys live here; values live in the store.

use std::collections::{BTreeMap, BTreeSet};

use ethrex_common::{Address, U256};

use crate::state::Key;

/// Default cap on the keys kept, pins included.
pub const WORKING_SET_CAP: usize = 1024;

/// Pins plus learned keys.
///
/// Invariants: closed under `Slot(a, _) => Account(a)`; pins and learned keys are
/// disjoint; `len() <= max(cap, pins)`. Only [`crate::state::State`] mutates it.
#[derive(Clone, Debug)]
pub struct WorkingSet {
    cap: usize,
    /// Watched accounts and configured contract accounts and slots.
    configured: BTreeSet<Key>,
    /// Keys read by the last seeding, minus configured ones.
    seeded: BTreeSet<Key>,
    /// Key to the tick of its last use. Ordered so an account's slots form one range.
    learned: BTreeMap<Key, u64>,
    /// Eviction order, oldest first. An account sorts before its slots at equal ticks.
    by_age: BTreeSet<(u64, Key)>,
    /// The next use's tick. Each `touch` is one use.
    tick: u64,
    membership: u64,
    recency: u64,
}

impl WorkingSet {
    /// A set holding `configured` pins and `remembered` keys, most recently used first.
    /// Remembered keys beyond capacity are dropped from the end.
    pub fn new(
        cap: usize,
        configured: impl IntoIterator<Item = Key>,
        remembered: Vec<Key>,
    ) -> Self {
        let mut set = Self {
            cap,
            configured: closed(configured),
            seeded: BTreeSet::new(),
            learned: BTreeMap::new(),
            by_age: BTreeSet::new(),
            tick: 0,
            membership: 0,
            recency: 0,
        };
        for key in remembered.into_iter().rev() {
            set.touch_one(key);
            set.tick = set.tick.wrapping_add(1);
        }
        set.evict();
        set
    }

    /// Whether `key` is pinned or learned.
    pub fn contains(&self, key: Key) -> bool {
        self.is_pin(key) || self.learned.contains_key(&key)
    }

    /// Pinned keys: configured ones first, then seeded ones.
    pub fn pins(&self) -> impl Iterator<Item = Key> + '_ {
        self.configured.iter().chain(&self.seeded).copied()
    }

    /// Learned keys, most recently used first.
    pub fn learned(&self) -> impl Iterator<Item = Key> + '_ {
        self.by_age.iter().rev().map(|(_, key)| *key)
    }

    /// Pins plus learned keys.
    pub fn len(&self) -> usize {
        self.configured.len() + self.seeded.len() + self.learned.len()
    }

    /// Whether the set holds no key.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of pinned keys.
    pub fn pin_count(&self) -> usize {
        self.configured.len() + self.seeded.len()
    }

    /// A count that changes whenever a key is learned or dropped.
    pub const fn membership(&self) -> u64 {
        self.membership
    }

    /// A count that changes whenever a use moves an already learned key ahead.
    pub const fn recency(&self) -> u64 {
        self.recency
    }

    /// The learned keys to persist, most recently used first: what [`WorkingSet::new`]
    /// takes back as `remembered`.
    pub fn remembered(&self) -> Vec<Key> {
        self.learned().collect()
    }

    /// Records one use of `keys`, and of the account of each slot. Pins are left alone.
    /// Returns the keys evicted to stay within the cap.
    pub(crate) fn touch(&mut self, keys: impl IntoIterator<Item = Key>) -> Vec<Key> {
        self.tick = self.tick.wrapping_add(1);
        for key in keys {
            self.touch_one(key);
        }
        self.evict()
    }

    /// Replaces the seeded pins. Keys no longer seeded become learned as just used.
    /// Returns the keys evicted to stay within the cap.
    pub(crate) fn set_seeded(&mut self, keys: BTreeSet<Key>) -> Vec<Key> {
        let seeded: BTreeSet<Key> = closed(keys)
            .into_iter()
            .filter(|key| !self.configured.contains(key))
            .collect();
        if seeded == self.seeded {
            return Vec::new();
        }
        let dropped = std::mem::replace(&mut self.seeded, seeded);
        for key in &self.seeded {
            if let Some(tick) = self.learned.remove(key) {
                self.by_age.remove(&(tick, *key));
                self.membership = self.membership.wrapping_add(1);
            }
        }
        self.touch(dropped)
    }

    fn is_pin(&self, key: Key) -> bool {
        self.configured.contains(&key) || self.seeded.contains(&key)
    }

    fn touch_one(&mut self, key: Key) {
        if let Key::Slot(address, _) = key {
            self.stamp(Key::Account(address));
        }
        self.stamp(key);
    }

    fn stamp(&mut self, key: Key) {
        if self.is_pin(key) {
            return;
        }
        let tick = self.tick;
        match self.learned.insert(key, tick) {
            None => self.membership = self.membership.wrapping_add(1),
            Some(old) if old != tick => {
                self.by_age.remove(&(old, key));
                self.recency = self.recency.wrapping_add(1);
            }
            Some(_) => {}
        }
        self.by_age.insert((tick, key));
    }

    /// Drops the least recently used keys until the set fits. Evicting an account evicts
    /// its learned slots, which closure keeps no fresher than the account.
    fn evict(&mut self) -> Vec<Key> {
        let mut evicted = Vec::new();
        while self.len() > self.cap {
            let Some((_, oldest)) = self.by_age.pop_first() else {
                break;
            };
            self.learned.remove(&oldest);
            self.membership = self.membership.wrapping_add(1);
            evicted.push(oldest);
            if let Key::Account(address) = oldest {
                let slots: Vec<(Key, u64)> = self
                    .learned
                    .range(slot_range(address))
                    .map(|(key, tick)| (*key, *tick))
                    .collect();
                for (key, tick) in slots {
                    self.learned.remove(&key);
                    self.by_age.remove(&(tick, key));
                    evicted.push(key);
                }
            }
        }
        evicted
    }
}

fn closed(keys: impl IntoIterator<Item = Key>) -> BTreeSet<Key> {
    let mut set = BTreeSet::new();
    for key in keys {
        if let Key::Slot(address, _) = key {
            set.insert(Key::Account(address));
        }
        set.insert(key);
    }
    set
}

fn slot_range(address: Address) -> std::ops::RangeInclusive<Key> {
    Key::Slot(address, U256::zero())..=Key::Slot(address, U256::max_value())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(n: u8) -> Address {
        Address::from_low_u64_be(u64::from(n))
    }

    #[test]
    fn pins_are_never_evicted_and_slots_close_over_accounts() {
        let mut set = WorkingSet::new(3, [Key::Slot(addr(1), U256::one())], Vec::new());
        assert!(set.contains(Key::Account(addr(1))));
        assert!(set.contains(Key::Slot(addr(1), U256::one())));
        assert_eq!(set.pin_count(), 2);

        let evicted = set.touch([Key::Account(addr(2))]);
        assert!(evicted.is_empty());
        let evicted = set.touch([Key::Account(addr(3))]);
        assert_eq!(evicted, vec![Key::Account(addr(2))]);
        assert!(set.contains(Key::Account(addr(3))));
        assert!(set.contains(Key::Account(addr(1))));
    }

    #[test]
    fn evicting_an_account_takes_its_learned_slots() {
        let mut set = WorkingSet::new(3, Vec::<Key>::new(), Vec::new());
        set.touch([
            Key::Slot(addr(1), U256::one()),
            Key::Slot(addr(1), U256::from(2)),
        ]);
        assert_eq!(set.len(), 3);
        let evicted = set.touch([Key::Account(addr(9))]);
        assert_eq!(evicted.len(), 3);
        assert!(evicted.contains(&Key::Account(addr(1))));
        assert!(!set.contains(Key::Slot(addr(1), U256::one())));
        assert!(set.contains(Key::Account(addr(9))));
    }

    #[test]
    fn remembered_round_trips_most_recent_first() {
        let mut set = WorkingSet::new(10, Vec::<Key>::new(), Vec::new());
        set.touch([Key::Account(addr(1))]);
        set.touch([Key::Account(addr(2))]);
        let remembered = set.remembered();
        assert_eq!(
            remembered,
            vec![Key::Account(addr(2)), Key::Account(addr(1))]
        );
        let restored = WorkingSet::new(10, Vec::<Key>::new(), remembered.clone());
        assert_eq!(restored.remembered(), remembered);
    }

    #[test]
    fn seeded_keys_move_between_pins_and_learned() {
        let mut set = WorkingSet::new(10, [Key::Account(addr(1))], Vec::new());
        set.touch([Key::Account(addr(2))]);
        let evicted = set.set_seeded([Key::Account(addr(2)), Key::Account(addr(3))].into());
        assert!(evicted.is_empty());
        assert_eq!(set.pin_count(), 3);
        assert_eq!(set.learned().count(), 0);
        let evicted = set.set_seeded([Key::Account(addr(3))].into());
        assert!(evicted.is_empty());
        assert_eq!(
            set.learned().collect::<Vec<_>>(),
            vec![Key::Account(addr(2))]
        );
    }
}
