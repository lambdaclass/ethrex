//! Bitmap types for tracking cell modifications in the grid trie.
//!
//! TouchMap tracks which cells (nibbles 0-15) were accessed during processing.
//! AfterMap tracks which cells have children after modifications.

/// 16-bit bitmap tracking which children (nibbles 0-15) were touched/accessed.
/// Each bit position corresponds to a nibble value (0-15).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TouchMap(pub u16);

impl TouchMap {
    /// Create a new empty TouchMap
    pub const fn new() -> Self {
        Self(0)
    }

    /// Set the bit for the given nibble (0-15)
    #[inline]
    pub fn set(&mut self, nibble: u8) {
        debug_assert!(nibble < 16, "nibble must be 0-15");
        self.0 |= 1 << nibble;
    }

    /// Clear the bit for the given nibble (0-15)
    #[inline]
    pub fn clear(&mut self, nibble: u8) {
        debug_assert!(nibble < 16, "nibble must be 0-15");
        self.0 &= !(1 << nibble);
    }

    /// Check if the bit for the given nibble is set
    #[inline]
    pub fn is_set(&self, nibble: u8) -> bool {
        debug_assert!(nibble < 16, "nibble must be 0-15");
        (self.0 >> nibble) & 1 == 1
    }

    /// Count the number of set bits
    #[inline]
    pub fn count(&self) -> u32 {
        self.0.count_ones()
    }

    /// Check if no bits are set
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }

    /// Reset all bits to zero
    #[inline]
    pub fn reset(&mut self) {
        self.0 = 0;
    }

    /// Iterate over all set nibble positions
    pub fn iter_set(&self) -> impl Iterator<Item = u8> + '_ {
        (0u8..16).filter(move |&i| self.is_set(i))
    }

    /// Get the first set nibble position, if any
    #[inline]
    pub fn first_set(&self) -> Option<u8> {
        if self.0 == 0 {
            None
        } else {
            Some(self.0.trailing_zeros() as u8)
        }
    }

    /// Combine with another TouchMap using OR
    #[inline]
    pub fn union(&mut self, other: TouchMap) {
        self.0 |= other.0;
    }

    /// Combine with another TouchMap using AND
    #[inline]
    pub fn intersect(&mut self, other: TouchMap) {
        self.0 &= other.0;
    }
}

/// AfterMap tracks which cells have children after modifications.
/// Same structure as TouchMap but with different semantic meaning.
pub type AfterMap = TouchMap;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_touch_map_set_and_get() {
        let mut map = TouchMap::new();
        assert!(map.is_empty());

        map.set(0);
        map.set(5);
        map.set(15);

        assert!(map.is_set(0));
        assert!(!map.is_set(1));
        assert!(map.is_set(5));
        assert!(map.is_set(15));
        assert_eq!(map.count(), 3);
    }

    #[test]
    fn test_touch_map_clear() {
        let mut map = TouchMap::new();
        map.set(5);
        map.set(10);
        assert_eq!(map.count(), 2);

        map.clear(5);
        assert!(!map.is_set(5));
        assert!(map.is_set(10));
        assert_eq!(map.count(), 1);
    }

    #[test]
    fn test_touch_map_iter_set() {
        let mut map = TouchMap::new();
        map.set(1);
        map.set(4);
        map.set(9);

        let set_nibbles: Vec<u8> = map.iter_set().collect();
        assert_eq!(set_nibbles, vec![1, 4, 9]);
    }

    #[test]
    fn test_touch_map_first_set() {
        let mut map = TouchMap::new();
        assert_eq!(map.first_set(), None);

        map.set(7);
        map.set(3);
        assert_eq!(map.first_set(), Some(3));
    }

    #[test]
    fn test_touch_map_union_intersect() {
        let mut map1 = TouchMap::new();
        map1.set(1);
        map1.set(2);

        let mut map2 = TouchMap::new();
        map2.set(2);
        map2.set(3);

        let mut union = map1;
        union.union(map2);
        assert!(union.is_set(1));
        assert!(union.is_set(2));
        assert!(union.is_set(3));

        let mut intersect = map1;
        intersect.intersect(map2);
        assert!(!intersect.is_set(1));
        assert!(intersect.is_set(2));
        assert!(!intersect.is_set(3));
    }
}
