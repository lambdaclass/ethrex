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
            buffer: Rc::new(RefCell::new(Vec::with_capacity(4096))),
            current_base,
            len_gas: 0,
        }
    }

    pub fn next_memory(&self) -> MemoryV2 {
        let mut mem = self.clone();
        mem.current_base = mem.buffer.borrow().len();
        mem
    }

    /// Cleans the memory from base onwards, this should be used in callframes when handling returns. On the callframe that is about to be dropped.
    pub fn clean_from_base(&self) {
        self.buffer.borrow_mut().truncate(self.current_base);
    }

    /// Returns the len of the current memory for gas, this differs from the actual allocated length due to optimizations.
    pub fn len(&self) -> usize {
        self.len_gas
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the len of the current memory, from the current base.
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

        if new_memory_size <= current_len {
            return Ok(());
        }

        #[expect(clippy::arithmetic_side_effects)]
        {
            self.len_gas += new_memory_size - current_len;
        }

        let mut buffer = self.buffer.borrow_mut();

        #[expect(clippy::arithmetic_side_effects)]
        // when resizing, resize by allocating entire pages instead of small memory sizes.
        let new_size =
            (buffer.len() + new_memory_size) + (4096 - ((buffer.len() + new_memory_size) % 4096));
        buffer.resize(new_size, 0);

        Ok(())
    }

    pub fn load_range(&mut self, offset: U256, size: usize) -> Result<Vec<u8>, VMError> {
        let offset: usize = offset
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;

        let new_size = offset.checked_add(size).unwrap();
        self.resize(new_size)?;

        let true_offset = offset.checked_add(self.current_base).unwrap();

        let buf = self.buffer.borrow();
        Ok(buf
            .get(true_offset..(true_offset.checked_add(size).unwrap()))
            .ok_or(OutOfBounds)?
            .to_vec())
    }

    pub fn load_range_const<const N: usize>(&mut self, offset: U256) -> Result<[u8; N], VMError> {
        let offset: usize = offset
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;

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

    pub fn load_word(&mut self, offset: U256) -> Result<U256, VMError> {
        let value: [u8; 32] = self.load_range_const(offset)?;
        Ok(u256_from_big_endian_const(value))
    }

    pub fn store(&self, data: &[u8], at_offset: U256, data_size: usize) -> Result<(), VMError> {
        if data_size == 0 {
            return Ok(());
        }

        let at_offset: usize = at_offset
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;

        let real_offset = self
            .current_base
            .checked_add(at_offset)
            .ok_or(OutOfBounds)?;

        let mut buffer = self.buffer.borrow_mut();

        #[allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
        buffer[real_offset..(real_offset + data_size)].copy_from_slice(&data[..data_size]);

        Ok(())
    }

    pub fn store_data(&mut self, offset: U256, data: &[u8]) -> Result<(), VMError> {
        let new_size = offset
            .checked_add(data.len().into())
            .ok_or(OutOfBounds)?
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
        self.resize(new_size)?;
        self.store(data, offset, data.len())
    }

    pub fn store_range(&mut self, offset: U256, size: usize, data: &[u8]) -> Result<(), VMError> {
        if size == 0 {
            return Ok(());
        }

        let new_size = offset
            .checked_add(size.into())
            .ok_or(OutOfBounds)?
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
        self.resize(new_size)?;
        self.store(data, offset, size)
    }

    pub fn store_word(&mut self, offset: U256, word: U256) -> Result<(), VMError> {
        let new_size: usize = offset
            .checked_add(WORD_SIZE_IN_BYTES_USIZE.into())
            .ok_or(OutOfBounds)?
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;

        self.resize(new_size)?;
        self.store(&word.to_big_endian(), offset, WORD_SIZE_IN_BYTES_USIZE)
    }

    pub fn copy_within(
        &mut self,
        from_offset: U256,
        to_offset: U256,
        size: usize,
    ) -> Result<(), VMError> {
        if size == 0 {
            return Ok(());
        }

        let from_offset: usize = from_offset
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
        let to_offset: usize = to_offset
            .try_into()
            .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;

        self.resize(
            to_offset
                .max(from_offset)
                .checked_add(size)
                .ok_or(InternalError::Overflow)?,
        )?;

        let mut temporary_buffer = vec![0u8; size];
        let true_from_offset = from_offset
            .checked_add(self.current_base)
            .ok_or(OutOfBounds)?;
        let mut buffer = self.buffer.borrow_mut();

        #[expect(clippy::indexing_slicing)]
        temporary_buffer[..size].copy_from_slice(
            &buffer[true_from_offset
                ..(true_from_offset
                    .checked_add(size)
                    .ok_or(InternalError::Overflow)?)],
        );

        let true_to_offset = to_offset
            .checked_add(self.current_base)
            .ok_or(OutOfBounds)?;

        #[expect(clippy::indexing_slicing)]
        buffer[true_to_offset
            ..(true_to_offset
                .checked_add(size)
                .ok_or(InternalError::Overflow)?)]
            .copy_from_slice(&temporary_buffer[..size]);

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

pub fn calculate_memory_size(offset: U256, size: usize) -> Result<usize, VMError> {
    if size == 0 {
        return Ok(0);
    }

    let offset: usize = offset.try_into().map_err(|_err| OutOfGas)?;

    offset
        .checked_add(size)
        .and_then(|sum| sum.checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE))
        .ok_or(OutOfBounds.into())
}
