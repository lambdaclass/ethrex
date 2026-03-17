use std::{cell::RefCell, rc::Rc};

use crate::{
    constants::{MEMORY_EXPANSION_QUOTIENT, WORD_SIZE_IN_BYTES_U64, WORD_SIZE_IN_BYTES_USIZE},
    errors::{ExceptionalHalt, InternalError, VMError},
};
use ExceptionalHalt::OutOfBounds;
use bytes::Bytes;
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
        self.ensure_buffer_capacity(new_memory_size, &mut buffer);

        Ok(())
    }

    /// Ensures the underlying buffer has enough capacity for `new_memory_size` bytes
    /// from `current_base`. Called with an already-borrowed buffer to avoid redundant
    /// RefCell borrow checks.
    #[inline(always)]
    fn ensure_buffer_capacity(&self, new_memory_size: usize, buffer: &mut Vec<u8>) {
        #[allow(clippy::arithmetic_side_effects)]
        let real_new_memory_size = new_memory_size + self.current_base;

        if real_new_memory_size > buffer.len() {
            // when resizing, avoid really small resizes.
            let new_size = real_new_memory_size.next_multiple_of(64);
            buffer.resize(new_size, 0);
        }
    }

    /// Load `size` bytes from the given offset. Returning a Bytes.
    #[inline]
    pub fn load_range(&mut self, offset: usize, size: usize) -> Result<Bytes, VMError> {
        if size == 0 {
            return Ok(Bytes::new());
        }

        let new_size = offset.checked_add(size).ok_or(OutOfBounds)?;

        let new_memory_size = new_size
            .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
            .ok_or(OutOfBounds)?;

        let true_offset = offset.wrapping_add(self.current_base);

        // Single borrow: resize (if needed) + read in one scope.
        let mut buf = self.buffer.borrow_mut();

        if new_memory_size > self.len {
            self.len = new_memory_size;
            self.ensure_buffer_capacity(new_memory_size, &mut buf);
        }

        // SAFETY: ensure_buffer_capacity guarantees bounds are correct.
        #[allow(unsafe_code)]
        unsafe {
            Ok(Bytes::copy_from_slice(buf.get_unchecked(
                true_offset..(true_offset.wrapping_add(size)),
            )))
        }
    }

    /// Load N bytes from the given offset.
    #[inline(always)]
    pub fn load_range_const<const N: usize>(&mut self, offset: usize) -> Result<[u8; N], VMError> {
        let new_size = offset.checked_add(N).ok_or(OutOfBounds)?;

        let new_memory_size = new_size
            .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
            .ok_or(OutOfBounds)?;

        // Single borrow: resize (if needed) + read in one scope.
        let mut buf = self.buffer.borrow_mut();

        if new_memory_size > self.len {
            self.len = new_memory_size;
            self.ensure_buffer_capacity(new_memory_size, &mut buf);
        }

        let true_offset = offset.wrapping_add(self.current_base);

        // SAFETY: ensure_buffer_capacity guarantees bounds are correct.
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

    /// Stores the given data at the given offset.
    #[inline(always)]
    pub fn store_data(&mut self, offset: usize, data: &[u8]) -> Result<(), VMError> {
        if data.is_empty() {
            return Ok(());
        }
        let new_size = offset.checked_add(data.len()).ok_or(OutOfBounds)?;

        let new_memory_size = new_size
            .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
            .ok_or(OutOfBounds)?;

        let real_offset = self.current_base.wrapping_add(offset);
        let real_data_size = data.len();

        // Single borrow: resize (if needed) + write in one scope.
        let mut buffer = self.buffer.borrow_mut();

        if new_memory_size > self.len {
            self.len = new_memory_size;
            self.ensure_buffer_capacity(new_memory_size, &mut buffer);
        }

        // SAFETY: ensure_buffer_capacity guarantees bounds are correct.
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

    /// Stores data and zero-pads up to total_size at the given offset.
    #[inline(always)]
    pub fn store_data_zero_padded(
        &mut self,
        offset: usize,
        data: &[u8],
        total_size: usize,
    ) -> Result<(), VMError> {
        if total_size == 0 {
            return Ok(());
        }

        let new_size = offset.checked_add(total_size).ok_or(OutOfBounds)?;

        let new_memory_size = new_size
            .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
            .ok_or(OutOfBounds)?;

        // Single borrow: resize + copy + zero-pad all in one scope.
        let mut buffer = self.buffer.borrow_mut();

        if new_memory_size > self.len {
            self.len = new_memory_size;
            self.ensure_buffer_capacity(new_memory_size, &mut buffer);
        }

        let copy_size = data.len().min(total_size);
        if copy_size > 0 {
            let real_offset = self.current_base.wrapping_add(offset);
            // SAFETY: ensure_buffer_capacity guarantees bounds are correct.
            #[allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
            #[allow(unsafe_code)]
            unsafe {
                std::ptr::copy_nonoverlapping(
                    data.get_unchecked(..copy_size).as_ptr(),
                    buffer
                        .get_unchecked_mut(real_offset..(real_offset + copy_size))
                        .as_mut_ptr(),
                    copy_size,
                );
            }
        }

        #[allow(clippy::arithmetic_side_effects)]
        if copy_size < total_size {
            // SAFETY: copy_size < total_size and offset + total_size didn't overflow (checked above),
            // so offset + copy_size cannot overflow.
            let zero_offset = offset.wrapping_add(copy_size);
            let zero_size = total_size - copy_size;
            let real_offset = self.current_base.wrapping_add(zero_offset);

            // SAFETY: ensure_buffer_capacity guarantees bounds are correct.
            #[expect(unsafe_code)]
            unsafe {
                buffer
                    .get_unchecked_mut(real_offset..real_offset.wrapping_add(zero_size))
                    .fill(0);
            }
        }

        Ok(())
    }

    /// Stores a word at the given offset, resizing memory if needed.
    #[inline(always)]
    pub fn store_word(&mut self, offset: usize, word: U256) -> Result<(), VMError> {
        let new_size: usize = offset
            .checked_add(WORD_SIZE_IN_BYTES_USIZE)
            .ok_or(OutOfBounds)?;

        let new_memory_size = new_size
            .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
            .ok_or(OutOfBounds)?;

        let data = u256_to_big_endian(word);
        let real_offset = self.current_base.wrapping_add(offset);

        // Single borrow: resize (if needed) + write in one scope.
        let mut buffer = self.buffer.borrow_mut();

        if new_memory_size > self.len {
            self.len = new_memory_size;
            self.ensure_buffer_capacity(new_memory_size, &mut buffer);
        }

        // SAFETY: ensure_buffer_capacity guarantees bounds are correct.
        #[allow(clippy::arithmetic_side_effects)]
        #[allow(unsafe_code)]
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                buffer
                    .get_unchecked_mut(real_offset..(real_offset + WORD_SIZE_IN_BYTES_USIZE))
                    .as_mut_ptr(),
                WORD_SIZE_IN_BYTES_USIZE,
            );
        }

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

        let new_memory_size = to_offset
            .max(from_offset)
            .checked_add(size)
            .ok_or(InternalError::Overflow)?
            .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
            .ok_or(OutOfBounds)?;

        let true_from_offset = from_offset
            .checked_add(self.current_base)
            .ok_or(OutOfBounds)?;

        let true_to_offset = to_offset
            .checked_add(self.current_base)
            .ok_or(OutOfBounds)?;

        // Single borrow: resize (if needed) + copy in one scope.
        let mut buffer = self.buffer.borrow_mut();

        if new_memory_size > self.len {
            self.len = new_memory_size;
            self.ensure_buffer_capacity(new_memory_size, &mut buffer);
        }

        buffer.copy_within(
            true_from_offset
                ..(true_from_offset
                    .checked_add(size)
                    .ok_or(InternalError::Overflow)?),
            true_to_offset,
        );

        Ok(())
    }

    #[inline(always)]
    pub fn store_zeros(&mut self, offset: usize, size: usize) -> Result<(), VMError> {
        if size == 0 {
            return Ok(());
        }

        let new_size = offset.checked_add(size).ok_or(OutOfBounds)?;

        let new_memory_size = new_size
            .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
            .ok_or(OutOfBounds)?;

        let real_offset = self.current_base.wrapping_add(offset);

        // Single borrow: resize (if needed) + zero-fill in one scope.
        let mut buffer = self.buffer.borrow_mut();

        if new_memory_size > self.len {
            self.len = new_memory_size;
            self.ensure_buffer_capacity(new_memory_size, &mut buffer);
        }

        // SAFETY: ensure_buffer_capacity guarantees bounds are correct.
        #[expect(unsafe_code)]
        unsafe {
            buffer
                .get_unchecked_mut(real_offset..(real_offset.wrapping_add(size)))
                .fill(0);
        }

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
