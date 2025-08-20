use std::{cell::RefCell, rc::Rc};

use crate::{
    constants::{MEMORY_EXPANSION_QUOTIENT, WORD_SIZE_IN_BYTES_U64, WORD_SIZE_IN_BYTES_USIZE},
    errors::{ExceptionalHalt, InternalError, VMError},
};
use ExceptionalHalt::OutOfBounds;
use ethrex_common::{
    U256,
    utils::{u256_from_big_endian_const, u256_to_big_endian},
};

/// A cheaply clonable callframe-shared memory buffer.
///
/// When a new callframe is created a RC clone of this memory is made, with the current base offset at the length of the buffer at that time.
#[derive(Debug, Clone)]
pub struct Memory {
    pub buffer: Rc<RefCell<Vec<u8>>>,
    pub len: usize,
    current_base: usize,
}

impl Memory {
    #[inline]
    pub fn new() -> Self {
        Self {
            buffer: Rc::new(RefCell::new(Vec::new())),
            len: 0,
            current_base: 0,
        }
    }

    /// Gets the Memory for the next children callframe.
    #[inline]
    pub fn next_memory(&self) -> Memory {
        let mut mem = self.clone();
        mem.current_base = mem.buffer.borrow().len();
        mem.len = 0;
        mem
    }

    /// Cleans the memory from base onwards, this must be used in callframes when handling returns.
    ///
    /// On the callframe that is about to be dropped.
    #[inline]
    pub fn clean_from_base(&self) {
        #[expect(unsafe_code)]
        unsafe {
            self.buffer
                .borrow_mut()
                .get_unchecked_mut(self.current_base..(self.current_base.wrapping_add(self.len)))
                .fill(0);
        }
    }

    /// Returns the len of the current memory, from the current base.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Resizes the from the current base to fit the memory specified at new_memory_size.
    ///
    /// Note: new_memory_size is increased to the next 32 byte multiple.
    #[inline(always)]
    pub fn resize(&mut self, new_memory_size: usize) -> Result<(), VMError> {
        if new_memory_size == 0 {
            return Ok(());
        }

        let new_memory_size = new_memory_size
            .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
            .ok_or(OutOfBounds)?;

        let current_len = self.len();

        if new_memory_size <= current_len {
            return Ok(());
        }

        self.len = new_memory_size;

        let mut buffer = self.buffer.borrow_mut();

        #[allow(clippy::arithmetic_side_effects)]
        let real_new_memory_size = new_memory_size + self.current_base;

        if real_new_memory_size > buffer.len() {
            // when resizing, resize by allocating entire pages instead of small memory sizes.
            let new_size = real_new_memory_size.next_multiple_of(4096);
            buffer.resize(new_size, 0);
        }

        Ok(())
    }

    /// Load `size` bytes from the given offset.
    #[inline]
    pub fn load_range(&mut self, offset: usize, size: usize) -> Result<Vec<u8>, VMError> {
        if size == 0 {
            return Ok(Vec::new());
        }

        let new_size = offset.checked_add(size).ok_or(OutOfBounds)?;
        self.resize(new_size)?;

        let true_offset = offset.wrapping_add(self.current_base);

        let buf = self.buffer.borrow();

        // SAFETY: resize already makes sure bounds are correct.
        #[allow(unsafe_code)]
        unsafe {
            Ok(buf
                .get_unchecked(true_offset..(true_offset.wrapping_add(size)))
                .to_vec())
        }
    }

    /// Load N bytes from the given offset.
    #[inline(always)]
    pub fn load_range_const<const N: usize>(&mut self, offset: usize) -> Result<[u8; N], VMError> {
        let new_size = offset.checked_add(N).ok_or(OutOfBounds)?;
        self.resize(new_size)?;

        let true_offset = offset.checked_add(self.current_base).ok_or(OutOfBounds)?;

        let buf = self.buffer.borrow();
        // SAFETY: resize already makes sure bounds are correct.
        #[allow(unsafe_code)]
        unsafe {
            Ok(*buf
                .get_unchecked(true_offset..(true_offset.wrapping_add(N)))
                .as_ptr()
                .cast::<[u8; N]>())
        }
    }

    /// Load a word from at the given offset.
    #[inline(always)]
    pub fn load_word(&mut self, offset: usize) -> Result<U256, VMError> {
        let value: [u8; 32] = self.load_range_const(offset)?;
        Ok(u256_from_big_endian_const(value))
    }

    /// Stores the given data and data size at the given offset.
    ///
    /// Internal use.
    #[inline(always)]
    fn store(&self, data: &[u8], at_offset: usize, data_size: usize) -> Result<(), VMError> {
        if data_size == 0 {
            return Ok(());
        }

        let real_offset = self.current_base.wrapping_add(at_offset);

        let mut buffer = self.buffer.borrow_mut();

        let real_data_size = data_size.min(data.len());

        // SAFETY: Used internally, resize always called before this function.
        #[allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
        #[allow(unsafe_code)]
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.get_unchecked(..real_data_size).as_ptr(),
                buffer
                    .get_unchecked_mut(real_offset..(real_offset + real_data_size))
                    .as_mut_ptr(),
                real_data_size,
            );
        }

        Ok(())
    }

    /// Stores the given data at the given offset.
    #[inline(always)]
    pub fn store_data(&mut self, offset: usize, data: &[u8]) -> Result<(), VMError> {
        if data.is_empty() {
            return Ok(());
        }
        let new_size = offset.checked_add(data.len()).ok_or(OutOfBounds)?;
        self.resize(new_size)?;
        self.store(data, offset, data.len())
    }

    /// Stores the given data and data size at the given offset.
    ///
    /// Resizes memory to fit the given data.
    #[inline(always)]
    pub fn store_range(&mut self, offset: usize, size: usize, data: &[u8]) -> Result<(), VMError> {
        if size == 0 {
            return Ok(());
        }

        let new_size = offset.checked_add(size).ok_or(OutOfBounds)?;
        self.resize(new_size)?;
        self.store(data, offset, size)
    }

    /// Stores a word at the given offset, resizing memory if needed.
    #[inline(always)]
    pub fn store_word(&mut self, offset: usize, word: U256) -> Result<(), VMError> {
        let new_size: usize = offset
            .checked_add(WORD_SIZE_IN_BYTES_USIZE)
            .ok_or(OutOfBounds)?;

        self.resize(new_size)?;
        self.store(&u256_to_big_endian(word), offset, WORD_SIZE_IN_BYTES_USIZE)?;
        Ok(())
    }

    /// Copies memory within 2 offsets. Like a memmove.
    ///
    /// Resizes if needed, because one can copy from "expanded memory", which is initialized with zeroes.
    pub fn copy_within(
        &mut self,
        from_offset: usize,
        to_offset: usize,
        size: usize,
    ) -> Result<(), VMError> {
        if size == 0 {
            return Ok(());
        }

        self.resize(
            to_offset
                .max(from_offset)
                .checked_add(size)
                .ok_or(InternalError::Overflow)?,
        )?;

        let true_from_offset = from_offset
            .checked_add(self.current_base)
            .ok_or(OutOfBounds)?;

        let true_to_offset = to_offset
            .checked_add(self.current_base)
            .ok_or(OutOfBounds)?;
        let mut buffer = self.buffer.borrow_mut();

        buffer.copy_within(
            true_from_offset
                ..(true_from_offset
                    .checked_add(size)
                    .ok_or(InternalError::Overflow)?),
            true_to_offset,
        );

        Ok(())
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

/// When a memory expansion is triggered, only the additional bytes of memory
/// must be paid for.
#[inline]
pub fn expansion_cost(new_memory_size: usize, current_memory_size: usize) -> Result<u64, VMError> {
    let cost = if new_memory_size <= current_memory_size {
        0
    } else {
        // We already know new_memory_size > current_memory_size,
        // and cost(x) > cost(y) where x > y, so cost should not underflow.
        cost(new_memory_size)?.wrapping_sub(cost(current_memory_size)?)
    };
    Ok(cost)
}

/// The total cost for a given memory size.
/// Gas cost should always be computed in u64
#[inline]
fn cost(memory_size: usize) -> Result<u64, VMError> {
    let memory_size = u64::try_from(memory_size).map_err(|_| InternalError::TypeConversion)?;

    // memory size measured in 32 byte words
    let words = memory_size.div_ceil(WORD_SIZE_IN_BYTES_U64);

    // Cost(words) ≈ floor(words^2 / q) + 3 * words
    // For this to overflow memory size in words should be 2^32, which is impossible.
    #[expect(clippy::arithmetic_side_effects)]
    let gas_cost = words * words / MEMORY_EXPANSION_QUOTIENT + 3 * words;

    Ok(gas_cost)
}

#[inline]
pub fn calculate_memory_size(offset: usize, size: usize) -> Result<usize, VMError> {
    if size == 0 {
        return Ok(0);
    }

    offset
        .checked_add(size)
        .and_then(|sum| sum.checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE))
        .ok_or(OutOfBounds.into())
}

#[cfg(test)]
mod test {
    #![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
    use ethrex_common::U256;

    use crate::memory::Memory;

    #[test]
    fn test_basic_store_data() {
        let mut mem = Memory::new();

        mem.store_data(0, &[1, 2, 3, 4, 0, 0, 0, 0, 0, 0]).unwrap();

        assert_eq!(&mem.buffer.borrow()[0..10], &[1, 2, 3, 4, 0, 0, 0, 0, 0, 0]);
        assert_eq!(mem.len(), 32);
    }

    #[test]
    fn test_words() {
        let mut mem = Memory::new();

        mem.store_word(0, U256::from(4)).unwrap();

        assert_eq!(mem.load_word(0).unwrap(), U256::from(4));
        assert_eq!(mem.len(), 32);
    }

    #[test]
    fn test_copy_word_within() {
        {
            let mut mem = Memory::new();

            mem.store_word(0, U256::from(4)).unwrap();
            mem.copy_within(0, 32, 32).unwrap();

            assert_eq!(mem.load_word(32).unwrap(), U256::from(4));
            assert_eq!(mem.len(), 64);
        }

        {
            let mut mem = Memory::new();

            mem.store_word(32, U256::from(4)).unwrap();
            mem.copy_within(32, 0, 32).unwrap();

            assert_eq!(mem.load_word(0).unwrap(), U256::from(4));
            assert_eq!(mem.len(), 64);
        }

        {
            let mut mem = Memory::new();

            mem.store_word(0, U256::from(4)).unwrap();
            mem.copy_within(0, 0, 32).unwrap();

            assert_eq!(mem.load_word(0).unwrap(), U256::from(4));
            assert_eq!(mem.len(), 32);
        }

        {
            let mut mem = Memory::new();

            mem.store_word(0, U256::from(4)).unwrap();
            mem.copy_within(32, 0, 32).unwrap();

            assert_eq!(mem.load_word(0).unwrap(), U256::zero());
            assert_eq!(mem.len(), 64);
        }
    }
}
