use std::{
    fmt,
    ops::{Bound, Deref, RangeBounds},
};

/// A single nibble.
///
/// Its representation is equivalent to that of an `u8`. It is safe to transmute it to/from `u8` as
/// long as its value is within `[0, 16)` (aka. the valid range of a nibble).
#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum Nibble {
    #[default]
    N0 = 0x00,
    N1 = 0x01,
    N2 = 0x02,
    N3 = 0x03,
    N4 = 0x04,
    N5 = 0x05,
    N6 = 0x06,
    N7 = 0x07,
    N8 = 0x08,
    N9 = 0x09,
    NA = 0x0A,
    NB = 0x0B,
    NC = 0x0C,
    ND = 0x0D,
    NE = 0x0E,
    NF = 0x0F,
}

impl fmt::Debug for Nibble {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        u8::fmt(&u8::from(*self), f)
    }
}

impl From<Nibble> for u8 {
    fn from(value: Nibble) -> Self {
        unsafe { std::mem::transmute::<Nibble, u8>(value) }
    }
}

impl TryFrom<u8> for Nibble {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value < 0x10 {
            Ok(unsafe { std::mem::transmute::<u8, Nibble>(value) })
        } else {
            Err(value)
        }
    }
}

/// A fixed-size nibble array.
///
/// The constant `N` is in bytes of capacity, not nibbles. The length will vary between `2 * N - 2`
/// and `2 * N` nibbles, depending on the half ends.
pub type NibbleArray<const N: usize> = Nibbles<[u8; N]>;
/// A readonly nibble slice (owned).
pub type NibbleBoxedSlice = Nibbles<Box<[u8]>>;
/// A readonly nibble slice (borrowed).
pub type NibbleSlice<'a> = Nibbles<&'a [u8]>;
/// A mutable nibble vec.
pub type NibbleVec = Nibbles<Vec<u8>>;

/// Wrapper around [`Nibbles`] made for trie paths.
///
/// This wrapper contains a flag to differentiate between partial (node) and complete (leaf) paths.
#[derive(Clone, Copy, Eq, Default, Hash, PartialEq)]
pub struct NibblesPath<T> {
    /// The path as a nibble string.
    pub inner: T,
    /// Whether the nibble string points to a leaf.
    pub is_leaf: bool,
}

// Note: `DerefMut` is not implemented because modifying paths to nodes does not make sense. If this
//   functionality was required, the correct way would be to extract the nibbles string from the
//   path, modify it accordingly, and finally creating a new path with the correct `is_leaf` value.
impl<T> Deref for NibblesPath<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// Wrapper around a container for nibble strings.
#[derive(Clone, Copy, Eq, Default, Hash, PartialEq)]
pub struct Nibbles<T> {
    /// The underlying nibble data.
    data: T,
    /// Whether the string's start and end are half-bytes.
    is_half: (bool, bool),
}

impl<T> Nibbles<T> {
    // TODO: Maybe add a realignment API.

    /// Create a nibble string from a compact bytes representation.
    ///
    /// This method does not allow for odd string lengths. Use [`Self::new_full`] for strings that
    /// start or end in half-bytes.
    pub fn new(data: T) -> Self {
        Self {
            data,
            is_half: (false, false),
        }
    }

    /// Create a nibble string from a compact bytes representation.
    ///
    /// This method supports strings that start or end in half-bytes.
    pub fn new_full(data: T, is_half: (bool, bool)) -> Self {
        Self { data, is_half }
    }

    /// Map the nibbles storage into a new one.
    ///
    /// Used for conversions between arrays, boxed slices, slices and vecs.
    pub fn map_data<U>(self, f: impl FnOnce(T) -> U) -> Nibbles<U> {
        Nibbles {
            data: f(self.data),
            is_half: self.is_half,
        }
    }

    /// Fallible version of [`Self::map_data`].
    pub fn try_map_data<U, E>(
        self,
        f: impl FnOnce(T) -> Result<U, (T, E)>,
    ) -> Result<Nibbles<U>, (Self, E)> {
        let Self { data, is_half } = self;
        match f(data) {
            Ok(data) => Ok(Nibbles { data, is_half }),
            Err((data, e)) => Err((Self { data, is_half }, e)),
        }
    }

    /// Create an iterator over the nibble string.
    pub fn iter(&self) -> NibbleIter<&T> {
        NibbleIter::new_full(&self.data, self.is_half)
    }
}

impl<T> IntoIterator for Nibbles<T>
where
    T: AsRef<[u8]>,
{
    type Item = Nibble;
    type IntoIter = NibbleIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        NibbleIter::new_full(self.data, self.is_half)
    }
}

impl<T> fmt::Debug for Nibbles<T>
where
    T: AsRef<[u8]>,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        struct NibblesHex<'a, T>(&'a T, (bool, bool));
        impl<T> fmt::Debug for NibblesHex<'_, T>
        where
            T: AsRef<[u8]>,
        {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                for x in NibbleIter::new_full(self.0, self.1) {
                    write!(f, "{x:x?}")?;
                }
                Ok(())
            }
        }

        f.debug_tuple("Nibbles")
            .field(&NibblesHex(&self.data, self.is_half))
            .finish()
    }
}

/// A generic nibbles iterator.
#[derive(Clone, Copy, Debug)]
pub struct NibbleIter<T> {
    inner: T,
    is_half: (bool, bool),
    offset: usize,
}

impl<T> NibbleIter<T> {
    /// Create a new nibbles iterator from a compact bytes representation.
    ///
    /// This method does not allow for odd string lengths. Use [`Self::new_full`] for strings that
    /// start or end in half-bytes.
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            is_half: (false, false),
            offset: 0,
        }
    }

    /// Create a new nibbles iterator from a compact bytes representation.
    ///
    /// This method supports strings that start or end in half-bytes.
    pub fn new_full(inner: T, is_half: (bool, bool)) -> Self {
        Self {
            inner,
            is_half,
            offset: 0,
        }
    }

    // TODO: Prefix methods.

    /// Return the current iterator's offset.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Return a slice to the already consumed nibbles.
    pub fn consumed_slice<'a>(&'a self) -> NibbleSlice<'a>
    where
        T: AsRef<[u8]>,
    {
        let offset = self.offset + usize::from(self.is_half.0) + 1;
        NibbleSlice::new_full(
            &self.inner.as_ref()[..offset >> 1],
            (self.is_half.0, offset & 0x01 == 0),
        )
    }

    /// Return a slice to the remaining nibbles.
    pub fn remaining_slice<'a>(&'a self) -> NibbleSlice<'a>
    where
        T: AsRef<[u8]>,
    {
        let offset = self.offset + usize::from(self.is_half.0);
        NibbleSlice::new_full(
            &self.inner.as_ref()[offset >> 1..],
            (offset & 0x01 != 0, self.is_half.1),
        )
    }
}

impl<T> Iterator for NibbleIter<T>
where
    T: AsRef<[u8]>,
{
    type Item = Nibble;

    fn next(&mut self) -> Option<Self::Item> {
        let inner = self.inner.as_ref();
        let offset = self.offset + usize::from(self.is_half.0);

        if (offset + usize::from(self.is_half.1)) >> 1 >= inner.len() {
            return None;
        }

        inner.get(offset >> 1).map(|&byte| {
            self.offset += 1;
            let nibble_data = match offset & 0x01 {
                0 => byte >> 4,
                _ => byte & 0x0F,
            };
            unsafe { std::mem::transmute::<u8, Nibble>(nibble_data) }
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl<T> ExactSizeIterator for NibbleIter<T>
where
    T: AsRef<[u8]>,
{
    fn len(&self) -> usize {
        self.inner.as_ref().len() - self.offset
    }
}

pub trait NibblesRef {
    fn as_ref(&self) -> &[u8];

    fn is_empty(&self) -> bool;
    fn len(&self) -> usize;

    fn get(&self, index: usize) -> Option<Nibble>;
    fn get_range(&self, index: impl RangeBounds<usize>) -> NibbleSlice<'_>;
}

impl NibblesRef for &'_ [u8] {
    fn as_ref(&self) -> &[u8] {
        self
    }

    fn is_empty(&self) -> bool {
        (*self).is_empty()
    }

    fn len(&self) -> usize {
        2 * (*self).len()
    }

    fn get(&self, index: usize) -> Option<Nibble> {
        (*self).get(index >> 1).map(|&byte| unsafe {
            std::mem::transmute::<u8, Nibble>(if index & 0x01 == 0 {
                byte >> 4
            } else {
                byte & 0x0F
            })
        })
    }

    fn get_range(&self, index: impl RangeBounds<usize>) -> NibbleSlice<'_> {
        let (byte_range, is_half) = {
            let (start_offset, start_half) = match index.start_bound() {
                Bound::Included(&offset) => (Bound::Included(offset >> 1), offset & 0x01 != 0),
                Bound::Excluded(&offset) => {
                    let is_half = offset & 0x01 == 0;
                    (
                        if is_half {
                            Bound::Included(offset >> 1)
                        } else {
                            Bound::Excluded(offset >> 1)
                        },
                        is_half,
                    )
                }
                Bound::Unbounded => (Bound::Unbounded, false),
            };
            let (end_offset, end_half) = match index.end_bound() {
                Bound::Included(&offset) => (Bound::Included(offset >> 1), offset & 0x01 == 0),
                Bound::Excluded(&offset) => {
                    let is_half = offset & 0x01 == 0;
                    (
                        if is_half {
                            Bound::Excluded(offset >> 1)
                        } else {
                            Bound::Included(offset >> 1)
                        },
                        !is_half,
                    )
                }
                Bound::Unbounded => (Bound::Unbounded, false),
            };

            ((start_offset, end_offset), (start_half, end_half))
        };

        // TODO: Clamp range to bounds to allow partial ranges (ex. allow slicing 2..10 into "abcdef", returning "cdef"
        //   only).
        (*self)
            .get(byte_range)
            .map(|data| NibbleSlice::new_full(data, is_half))
            .unwrap_or_default()
    }
}

impl<T> NibblesRef for Nibbles<T>
where
    T: AsRef<[u8]>,
{
    fn as_ref(&self) -> &[u8] {
        self.data.as_ref()
    }

    fn is_empty(&self) -> bool {
        match self.data.as_ref().len() {
            0 => true,
            1 if self.is_half == (true, true) => true,
            _ => false,
        }
    }

    fn len(&self) -> usize {
        2 * self.data.as_ref().len() - usize::from(self.is_half.0) - usize::from(self.is_half.1)
    }

    fn get(&self, index: usize) -> Option<Nibble> {
        let offset = index + usize::from(self.is_half.0);
        self.data.as_ref().get(offset >> 1).map(|&byte| unsafe {
            std::mem::transmute::<u8, Nibble>(if offset & 0x01 == 0 {
                byte >> 4
            } else {
                byte & 0x0F
            })
        })
    }

    fn get_range(&self, index: impl RangeBounds<usize>) -> NibbleSlice<'_> {
        let (byte_range, is_half) = {
            let (start_offset, start_half) = match index.start_bound() {
                Bound::Included(&offset) => {
                    let offset = offset + usize::from(self.is_half.0);
                    (Bound::Included(offset >> 1), offset & 0x01 != 0)
                }
                Bound::Excluded(&offset) => {
                    let offset = offset + usize::from(self.is_half.0);
                    let is_half = offset & 0x01 == 0;
                    (
                        if is_half {
                            Bound::Included(offset >> 1)
                        } else {
                            Bound::Excluded(offset >> 1)
                        },
                        is_half,
                    )
                }
                Bound::Unbounded => (Bound::Unbounded, self.is_half.0),
            };
            let (end_offset, end_half) = match index.end_bound() {
                Bound::Included(&offset) => {
                    let offset = offset + usize::from(self.is_half.0);
                    (Bound::Included(offset >> 1), offset & 0x01 == 0)
                }
                Bound::Excluded(&offset) => {
                    let offset = offset + usize::from(self.is_half.0);
                    let is_half = offset & 0x01 == 0;
                    (
                        if is_half {
                            Bound::Excluded(offset >> 1)
                        } else {
                            Bound::Included(offset >> 1)
                        },
                        !is_half,
                    )
                }
                Bound::Unbounded => (Bound::Unbounded, self.is_half.1),
            };

            ((start_offset, end_offset), (start_half, end_half))
        };

        // TODO: Clamp range to bounds to allow partial ranges (ex. allow slicing 2..10 into "abcdef", returning "cdef"
        //   only).
        self.data
            .as_ref()
            .get(byte_range)
            .map(|data| NibbleSlice::new_full(data, is_half))
            .unwrap_or_default()
    }
}

pub trait NibblesMut {
    /// Push a single nibble into the string.
    fn push(&mut self, value: Nibble);
    /// Pop the last nibble of the string.
    fn pop(&mut self) -> Option<Nibble>;

    /// Insert a single nibble into the string at a given position.
    ///
    /// Everything to the right is shifted by one nibble.
    fn insert(&mut self, index: usize, value: Nibble);
    /// Remove a single nibble from the string at a given position.
    ///
    /// Everything to the right is shifted by one nibble.
    fn remove(&mut self, index: usize) -> Option<Nibble>;

    /// Extend the string from an iterator of nibbles.
    fn extend(&mut self, iter: impl IntoIterator<Item = Nibble>);
    /// Extend the string from a collection of nibble pairs.
    ///
    /// This method does not allow extending an odd number of nibbles. Use
    /// [`NibblesMut::extend_from_nibbles`] or [`NibblesMut::extend`] instead.
    fn extend_from_bytes(&mut self, other: impl AsRef<[u8]>);
    /// Extend the string from another nibble string.
    ///
    /// This is more efficient than using [`NibblesMut::extend`].
    fn extend_from_nibbles(&mut self, other: NibbleSlice);
}

impl NibblesMut for NibbleVec {
    fn push(&mut self, value: Nibble) {
        self.is_half.1 = !self.is_half.1;
        if self.is_half.1 {
            self.data.push(u8::from(value) << 4);
        } else {
            let value_ref = self.data.last_mut().unwrap();
            *value_ref &= 0xF0;
            *value_ref |= u8::from(value);
        }
    }

    fn pop(&mut self) -> Option<Nibble> {
        self.is_half.1 = !self.is_half.1;
        let value = if self.is_half.1 {
            *self.data.last()? & 0x0F
        } else {
            self.data.pop()? >> 4
        };

        if self.data.len() == 1 && self.is_half == (true, true) {
            self.data.clear();
            self.is_half = (false, false);
        }

        Some(unsafe { std::mem::transmute::<u8, Nibble>(value) })
    }

    fn insert(&mut self, index: usize, value: Nibble) {
        // TODO: Check index for out of bounds.

        let offset = index + usize::from(self.is_half.0);
        if !self.is_half.1 {
            self.data.push(0);
        }

        self::algorithm::shr4(&mut self.data[offset >> 1..]);
        self.is_half.1 = !self.is_half.1;

        if offset & 0x01 == 0 {
            self.data[offset >> 1] |= u8::from(value) << 4;
        } else {
            self.data[offset >> 1] <<= 4;
            self.data[offset >> 1] |= u8::from(value);
        }
    }

    fn remove(&mut self, index: usize) -> Option<Nibble> {
        // TODO: Check index for out of bounds.

        let offset = index + usize::from(self.is_half.0);

        let byte = self.data[offset >> 1];
        self::algorithm::shl4(&mut self.data[offset >> 1..]);

        self.is_half.1 = !self.is_half.1;
        if !self.is_half.1 {
            self.data.pop();
        }

        let nibble = if offset & 0x01 == 0 {
            byte >> 4
        } else {
            self.data[offset >> 1] &= 0x0F;
            self.data[offset >> 1] |= byte & 0xF0;

            byte & 0x0F
        };

        if self.data.len() == 1 && self.is_half == (true, true) {
            self.data.clear();
            self.is_half = (false, false);
        }

        Some(unsafe { std::mem::transmute::<u8, Nibble>(nibble) })
    }

    fn extend(&mut self, iter: impl IntoIterator<Item = Nibble>) {
        for nibble in iter {
            self.push(nibble);
        }
    }

    fn extend_from_bytes(&mut self, other: impl AsRef<[u8]>) {
        let len = self.data.len();
        self.data.extend_from_slice(other.as_ref());

        if self.is_half.1 {
            let byte = self.data[len - 1];
            self::algorithm::shl4(&mut self.data[len - 1..]);

            self.data[len - 1] &= 0x0F;
            self.data[len - 1] |= byte & 0xF0;
        }
    }

    fn extend_from_nibbles(&mut self, other: NibbleSlice) {
        match (self.is_half.1, other.is_half.0) {
            (false, false) => {
                self.data.extend_from_slice(other.data);
                self.is_half.1 = other.is_half.1;
            }
            (false, true) => {
                let len = self.data.len();
                self.data.extend_from_slice(other.data);
                self::algorithm::shl4(&mut self.data[len..]);
                self.is_half.1 = !other.is_half.1;
            }
            (true, false) => {
                let len = self.data.len();
                let byte = self.data[len - 1];
                self.data.extend_from_slice(other.data);
                self::algorithm::shl4(&mut self.data[len - 1..]);
                self.data[len - 1] &= 0x0F;
                self.data[len - 1] |= byte & 0xF0;
                self.is_half.1 = !other.is_half.1;
            }
            (true, true) => {
                if !other.data.is_empty() {
                    let len = self.data.len();
                    self.data.extend_from_slice(&other.data[1..]);
                    self.data[len - 1] &= 0xF0;
                    self.data[len - 1] |= other.data[0] & 0x0F;
                    self.is_half.1 = other.is_half.1;
                }
            }
        }
    }
}

/// Miscellaneous algorithms for nibble strings.
pub mod algorithm {
    use super::*;
    use std::cmp::Ordering;

    /// Utility to shift left a buffer by a single nibble.
    pub(super) fn shl4(mut data: &mut [u8]) {
        // TODO: Write shortcuts for when
        //   - lhs.count() >= size_of::<u64>() -> 8x speedup
        //   - lhs.count() >= size_of::<u32>() -> 4x speedup
        //   - lhs.count() >= size_of::<u16>() -> 2x speedup
        //   - lhs.count() >= size_of::<u8>()  -> 1x speedup
        //
        // TODO: Consider SIMD shortcuts
        //   - lhs.count() >= 64 (AVX512)    -> 64x speedup
        //   - lhs.count() >= 32 (AVX2)      -> 32x speedup
        //   - lhs.count() >= 32 (AVX, NEON) -> 16x speedup

        while let Some((first, rest)) = data.split_first_mut() {
            *first <<= 4;
            if let Some(&next) = rest.first() {
                *first |= next >> 4;
            }

            data = rest;
        }
    }

    /// Utility to shift right a buffer by a single nibble.
    pub(super) fn shr4(mut data: &mut [u8]) {
        // TODO: Write shortcuts for when
        //   - lhs.count() >= size_of::<u64>() -> 16x speedup
        //   - lhs.count() >= size_of::<u32>() ->  8x speedup
        //   - lhs.count() >= size_of::<u16>() ->  4x speedup
        //   - lhs.count() >= size_of::<u8>()  ->  2x speedup
        //
        // TODO: Consider SIMD shortcuts
        //   - lhs.count() >= 64 (AVX512)    -> 128x speedup
        //   - lhs.count() >= 32 (AVX2)      ->  64x speedup
        //   - lhs.count() >= 32 (AVX, NEON) ->  32x speedup

        while let Some((last, rest)) = data.split_last_mut() {
            *last >>= 4;
            if let Some(&next) = rest.last() {
                *last |= next << 4;
            }

            data = rest;
        }
    }

    /// Utility to compare nibble prefixes that returns:
    ///   - Length of common prefix.
    ///   - Ordering of the next nibble after the prefix.
    pub fn prefix_cmp(
        mut lhs: impl Iterator<Item = Nibble>,
        mut rhs: impl Iterator<Item = Nibble>,
    ) -> (usize, Ordering) {
        // TODO: Write shortcuts for when
        //   - lhs.count() >= size_of::<u64>() -> 16x speedup
        //   - lhs.count() >= size_of::<u32>() ->  8x speedup
        //   - lhs.count() >= size_of::<u16>() ->  4x speedup
        //   - lhs.count() >= size_of::<u8>()  ->  2x speedup
        //
        // TODO: Consider SIMD shortcuts
        //   - lhs.count() >= 64 (AVX512)    -> 128x speedup
        //   - lhs.count() >= 32 (AVX2)      ->  64x speedup
        //   - lhs.count() >= 32 (AVX, NEON) ->  32x speedup

        let mut prefix_len = 0;
        loop {
            match (lhs.next(), rhs.next()) {
                (Some(lhs), Some(rhs)) => {
                    if lhs == rhs {
                        prefix_len += 1;
                    } else {
                        break (prefix_len, lhs.cmp(&rhs));
                    }
                }
                (Some(_), None) => break (prefix_len, Ordering::Greater),
                (None, Some(_)) => break (prefix_len, Ordering::Less),
                (None, None) => break (prefix_len, Ordering::Equal),
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn nibble_iter() {
        let data = b"\x01\x23\x45\x67\x89\xAB\xCD\xEF";
        assert_eq!(
            NibbleIter::new_full(data, (false, false))
                .map(|x| format!("{x:x?}"))
                .collect::<String>(),
            "0123456789abcdef"
        );

        let mut iter = NibbleIter::new_full(data, (false, false));
        let mut index = 0;
        loop {
            assert_eq!(
                iter.consumed_slice().into_iter().collect::<Vec<_>>(),
                NibbleIter::new_full(data, (false, false))
                    .take(index)
                    .collect::<Vec<_>>(),
            );
            assert_eq!(
                iter.remaining_slice().into_iter().collect::<Vec<_>>(),
                NibbleIter::new_full(data, (false, false))
                    .skip(index)
                    .collect::<Vec<_>>(),
            );

            if iter.next().is_none() {
                break;
            }
            index += 1;
        }
    }

    #[test]
    fn nibble_iter_half_start() {
        let data = b"\x01\x23\x45\x67\x89\xAB\xCD\xEF";
        assert_eq!(
            NibbleIter::new_full(data, (true, false))
                .map(|x| format!("{x:x?}"))
                .collect::<String>(),
            "123456789abcdef"
        );

        let mut iter = NibbleIter::new_full(data, (false, false));
        let mut index = 0;
        loop {
            assert_eq!(
                iter.consumed_slice().into_iter().collect::<Vec<_>>(),
                NibbleIter::new_full(data, (false, false))
                    .take(index)
                    .collect::<Vec<_>>(),
            );
            assert_eq!(
                iter.remaining_slice().into_iter().collect::<Vec<_>>(),
                NibbleIter::new_full(data, (false, false))
                    .skip(index)
                    .collect::<Vec<_>>(),
            );

            if iter.next().is_none() {
                break;
            }
            index += 1;
        }
    }

    #[test]
    fn nibble_iter_half_end() {
        let data = b"\x01\x23\x45\x67\x89\xAB\xCD\xEF";
        assert_eq!(
            NibbleIter::new_full(data, (false, true))
                .map(|x| format!("{x:x?}"))
                .collect::<String>(),
            "0123456789abcde"
        );

        let mut iter = NibbleIter::new_full(data, (false, false));
        let mut index = 0;
        loop {
            assert_eq!(
                iter.consumed_slice().into_iter().collect::<Vec<_>>(),
                NibbleIter::new_full(data, (false, false))
                    .take(index)
                    .collect::<Vec<_>>(),
            );
            assert_eq!(
                iter.remaining_slice().into_iter().collect::<Vec<_>>(),
                NibbleIter::new_full(data, (false, false))
                    .skip(index)
                    .collect::<Vec<_>>(),
            );

            if iter.next().is_none() {
                break;
            }
            index += 1;
        }
    }

    #[test]
    fn nibble_iter_half_start_end() {
        let data = b"\x01\x23\x45\x67\x89\xAB\xCD\xEF";
        assert_eq!(
            NibbleIter::new_full(data, (true, true))
                .map(|x| format!("{x:x?}"))
                .collect::<String>(),
            "123456789abcde"
        );
        let mut iter = NibbleIter::new_full(data, (false, false));
        let mut index = 0;
        loop {
            assert_eq!(
                iter.consumed_slice().into_iter().collect::<Vec<_>>(),
                NibbleIter::new_full(data, (false, false))
                    .take(index)
                    .collect::<Vec<_>>(),
            );
            assert_eq!(
                iter.remaining_slice().into_iter().collect::<Vec<_>>(),
                NibbleIter::new_full(data, (false, false))
                    .skip(index)
                    .collect::<Vec<_>>(),
            );

            if iter.next().is_none() {
                break;
            }
            index += 1;
        }
    }
}
