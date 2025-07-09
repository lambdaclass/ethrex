use std::{cell::RefCell, rc::Rc};

use crate::{
    constants::{MEMORY_EXPANSION_QUOTIENT, WORD_SIZE_IN_BYTES_USIZE},
    errors::{ExceptionalHalt, InternalError, VMError},
};
use ExceptionalHalt::OutOfBounds;
use ExceptionalHalt::OutOfGas;
use ethrex_common::{U256, utils::u256_from_big_endian_const};

/// A cheaply clonable callframe-shared memory buffer.
///
/// When a new callframe is created a RC clone of this memory is made, with the current base offset at the length of the buffer at that time.
#[derive(Debug, Clone)]
pub struct MemoryV2 {
    buffer: Rc<RefCell<Vec<u8>>>,
    current_base: usize,
    len_gas: usize,
}

#[allow(clippy::unwrap_used)]
impl MemoryV2 {
    #[inline]
    pub fn new(current_base: usize) -> Self {
        Self {
            buffer: Rc::new(RefCell::new(Vec::new())),
            current_base,
            len_gas: 0,
        }
    }

    #[inline]
    pub fn next_memory(&self) -> MemoryV2 {
        let mut mem = self.clone();
        mem.current_base = mem.buffer.borrow().len();
        mem.len_gas = 0;
        mem
    }

    /// Cleans the memory from base onwards, this should be used in callframes when handling returns. On the callframe that is about to be dropped.
    pub fn clean_from_base(&self) {
        self.buffer.borrow_mut().truncate(self.current_base);
    }

    /// Returns the len of the current memory for gas, this differs from the actual allocated length due to optimizations.
    #[inline]
    pub fn len(&self) -> usize {
        self.len_gas
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the len of the current memory, from the current base.
    #[inline]
    fn current_len(&self) -> usize {
        // will never wrap
        self.buffer.borrow().len().wrapping_sub(self.current_base)
    }

    #[inline]
    pub fn resize(&mut self, new_memory_size: usize) -> Result<(), VMError> {
        if new_memory_size == 0 {
            return Ok(());
        }

        let current_len = self.current_len();

        #[allow(clippy::arithmetic_side_effects)]
        {
            if new_memory_size > self.len_gas {
                self.len_gas = new_memory_size
                    .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
                    .ok_or(OutOfBounds)?;
            }
        }

        if new_memory_size <= current_len {
            return Ok(());
        }

        let mut buffer = self.buffer.borrow_mut();

        #[expect(clippy::arithmetic_side_effects)]
        // when resizing, resize by allocating entire pages instead of small memory sizes.
        let new_size =
            (buffer.len() + new_memory_size) + (4096 - ((buffer.len() + new_memory_size) % 4096));
        buffer.reserve_exact(new_size);
        buffer.resize(new_size, 0);

        Ok(())
    }

    #[inline]
    pub fn load_range(&mut self, offset: usize, size: usize) -> Result<Vec<u8>, VMError> {
        let new_size = offset.checked_add(size).unwrap();
        self.resize(new_size)?;

        let true_offset = offset.checked_add(self.current_base).unwrap();

        let buf = self.buffer.borrow();
        Ok(buf
            .get(true_offset..(true_offset.checked_add(size).unwrap()))
            .ok_or(OutOfBounds)?
            .to_vec())
    }

    #[inline]
    pub fn load_range_const<const N: usize>(&mut self, offset: usize) -> Result<[u8; N], VMError> {
        let new_size = offset.checked_add(N).unwrap();
        self.resize(new_size)?;

        let true_offset = offset.checked_add(self.current_base).unwrap();

        let buf = self.buffer.borrow();
        Ok(buf
            .get(true_offset..(true_offset.checked_add(N).unwrap()))
            .unwrap()
            .try_into()
            .unwrap())
    }

    #[inline]
    pub fn load_word(&mut self, offset: usize) -> Result<U256, VMError> {
        let value: [u8; 32] = self.load_range_const(offset)?;
        Ok(u256_from_big_endian_const(value))
    }

    pub fn store(&self, data: &[u8], at_offset: usize, data_size: usize) -> Result<(), VMError> {
        if data_size == 0 {
            return Ok(());
        }

        let real_offset = self
            .current_base
            .checked_add(at_offset)
            .ok_or(OutOfBounds)?;

        let mut buffer = self.buffer.borrow_mut();

        let real_data_size = data_size.min(data.len());

        #[allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
        buffer[real_offset..(real_offset + real_data_size)]
            .copy_from_slice(&data[..real_data_size]);

        #[allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
        if real_data_size < data_size {
            buffer[(real_offset + real_data_size)..(real_offset + data_size)].fill(0);
        }

        Ok(())
    }

    #[inline]
    pub fn store_data(&mut self, offset: usize, data: &[u8]) -> Result<(), VMError> {
        let new_size = offset.checked_add(data.len()).ok_or(OutOfBounds)?;
        self.resize(new_size)?;
        self.store(data, offset, data.len())
    }

    #[inline]
    pub fn store_zeroes(&mut self, offset: usize, size: usize) -> Result<(), VMError> {
        let new_size = offset.checked_add(size).ok_or(OutOfBounds)?;
        self.resize(new_size)
    }

    #[inline]
    pub fn store_range(&mut self, offset: usize, size: usize, data: &[u8]) -> Result<(), VMError> {
        if size == 0 {
            return Ok(());
        }

        let new_size = offset.checked_add(size).ok_or(OutOfBounds)?;
        self.resize(new_size)?;
        self.store(data, offset, size)
    }

    #[inline]
    pub fn store_word(&mut self, offset: usize, word: U256) -> Result<(), VMError> {
        let new_size: usize = offset
            .checked_add(WORD_SIZE_IN_BYTES_USIZE)
            .ok_or(OutOfBounds)?;

        self.resize(new_size)?;
        if word != U256::zero() {
            self.store(&word.to_big_endian(), offset, WORD_SIZE_IN_BYTES_USIZE)?;
        }
        Ok(())
    }

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

impl Default for MemoryV2 {
    fn default() -> Self {
        Self::new(0)
    }
}

/// When a memory expansion is triggered, only the additional bytes of memory
/// must be paid for.
#[inline]
pub fn expansion_cost(new_memory_size: usize, current_memory_size: usize) -> Result<u64, VMError> {
    let cost = if new_memory_size <= current_memory_size {
        0
    } else {
        cost(new_memory_size)?
            .checked_sub(cost(current_memory_size)?)
            .ok_or(InternalError::Underflow)?
    };
    Ok(cost)
}

/// The total cost for a given memory size.
fn cost(memory_size: usize) -> Result<u64, VMError> {
    let memory_size_word = memory_size
        .checked_add(
            WORD_SIZE_IN_BYTES_USIZE
                .checked_sub(1)
                .ok_or(InternalError::Underflow)?,
        )
        .ok_or(OutOfGas)?
        / WORD_SIZE_IN_BYTES_USIZE;

    let gas_cost = (memory_size_word.checked_pow(2).ok_or(OutOfGas)? / MEMORY_EXPANSION_QUOTIENT)
        .checked_add(3usize.checked_mul(memory_size_word).ok_or(OutOfGas)?)
        .ok_or(OutOfGas)?;

    gas_cost
        .try_into()
        .map_err(|_| ExceptionalHalt::VeryLargeNumber.into())
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

    use crate::memory::MemoryV2;

    #[test]
    fn test_basic_store_data() {
        let mut mem = MemoryV2::new(0);

        mem.store_data(0, &[1, 2, 3, 4, 0, 0, 0, 0, 0, 0]).unwrap();

        assert_eq!(&mem.buffer.borrow()[0..10], &[1, 2, 3, 4, 0, 0, 0, 0, 0, 0]);
        assert_eq!(mem.len(), 32);
    }

    #[test]
    fn test_words() {
        let mut mem = MemoryV2::new(0);

        mem.store_word(0, U256::from(4)).unwrap();

        assert_eq!(mem.load_word(0).unwrap(), U256::from(4));
        assert_eq!(mem.len(), 32);
    }

    #[test]
    fn test_copy_word_within() {
        {
            let mut mem = MemoryV2::new(0);

            mem.store_word(0, U256::from(4)).unwrap();
            mem.copy_within(0, 32, 32).unwrap();

            assert_eq!(mem.load_word(32).unwrap(), U256::from(4));
            assert_eq!(mem.len(), 64);
        }

        {
            let mut mem = MemoryV2::new(0);

            mem.store_word(32, U256::from(4)).unwrap();
            mem.copy_within(32, 0, 32).unwrap();

            assert_eq!(mem.load_word(0).unwrap(), U256::from(4));
            assert_eq!(mem.len(), 64);
        }

        {
            let mut mem = MemoryV2::new(0);

            mem.store_word(0, U256::from(4)).unwrap();
            mem.copy_within(0, 0, 32).unwrap();

            assert_eq!(mem.load_word(0).unwrap(), U256::from(4));
            assert_eq!(mem.len(), 32);
        }

        {
            let mut mem = MemoryV2::new(0);

            mem.store_word(0, U256::from(4)).unwrap();
            mem.copy_within(32, 0, 32).unwrap();

            assert_eq!(mem.load_word(0).unwrap(), U256::zero());
            assert_eq!(mem.len(), 64);
        }
    }
}
