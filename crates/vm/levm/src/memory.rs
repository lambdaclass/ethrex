use crate::{
    constants::{MEMORY_EXPANSION_QUOTIENT, WORD_SIZE},
    errors::{InternalError, OutOfGasError, VMError},
};
use ethrex_core::U256;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Memory {
    pub data: Vec<u8>,
}

impl From<Vec<u8>> for Memory {
    fn from(data: Vec<u8>) -> Self {
        Memory { data }
    }
}

impl Memory {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    pub fn new_from_vec(data: Vec<u8>) -> Self {
        Self { data }
    }

    fn resize(&mut self, offset: usize) -> Result<(), VMError> {
        let new_offset = offset
            .checked_next_multiple_of(WORD_SIZE)
            .ok_or(VMError::Internal(
                InternalError::ArithmeticOperationOverflow,
            ))?;
        if new_offset > self.data.len() {
            // Expand the size
            let size_to_expand =
                new_offset
                    .checked_sub(self.data.len())
                    .ok_or(VMError::Internal(
                        InternalError::ArithmeticOperationUnderflow,
                    ))?;
            self.data
                .try_reserve(size_to_expand)
                .map_err(|_err| VMError::MemorySizeOverflow)?;

            // Fill the new space with zeros
            self.data.extend(std::iter::repeat(0).take(size_to_expand));
        }
        Ok(())
    }

    pub fn load(&mut self, offset: usize) -> Result<U256, VMError> {
        self.resize(offset.checked_add(WORD_SIZE).ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow, // MemoryLoadOutOfBounds?
        ))?)?;
        let value_bytes = self
            .data
            .get(
                offset
                    ..offset.checked_add(WORD_SIZE).ok_or(VMError::Internal(
                        InternalError::ArithmeticOperationOverflow, // MemoryLoadOutOfBounds?
                    ))?,
            )
            .ok_or(VMError::MemoryLoadOutOfBounds)?;

        Ok(U256::from_big_endian(value_bytes))
    }

    pub fn load_range(&mut self, offset: usize, size: usize) -> Result<Vec<u8>, VMError> {
        let size_to_load = offset.checked_add(size).ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;
        self.resize(size_to_load)?;
        self.data
            .get(offset..size_to_load)
            .ok_or(VMError::MemoryLoadOutOfBounds)
            .map(|slice| slice.to_vec())
    }

    pub fn store_bytes(&mut self, offset: usize, value: &[u8]) -> Result<(), VMError> {
        let len = value.len();
        let size_to_store = offset.checked_add(len).ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;
        self.resize(size_to_store)?;
        self.data
            .splice(offset..size_to_store, value.iter().copied());

        Ok(())
    }

    pub fn store_n_bytes(
        &mut self,
        offset: usize,
        value: &[u8],
        size: usize,
    ) -> Result<(), VMError> {
        let size_to_store = offset.checked_add(size).ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;
        self.resize(size_to_store)?;
        self.data
            .splice(offset..size_to_store, value.iter().copied());

        Ok(())
    }

    pub fn size(&self) -> U256 {
        U256::from(self.data.len())
    }

    pub fn copy(
        &mut self,
        src_offset: usize,
        dest_offset: usize,
        size: usize,
    ) -> Result<(), VMError> {
        let src_copy_size = src_offset.checked_add(size).ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;
        let dest_copy_size = dest_offset.checked_add(size).ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow,
        ))?;
        let max_size = std::cmp::max(src_copy_size, dest_copy_size);

        if max_size > self.data.len() {
            self.resize(max_size)?;
        }

        let mut temp = vec![0u8; size];

        temp.copy_from_slice(
            self.data
                .get(src_offset..src_copy_size)
                .ok_or(VMError::Internal(InternalError::SlicingError))?,
        );

        for i in 0..size {
            if let Some(temp_byte) = temp.get_mut(i) {
                *temp_byte = *self
                    .data
                    .get(
                        src_offset
                            .checked_add(i)
                            .ok_or(VMError::MemoryLoadOutOfBounds)?,
                    )
                    .unwrap_or(&0u8);
            }
        }

        for i in 0..size {
            if let Some(memory_byte) = self.data.get_mut(
                dest_offset
                    .checked_add(i)
                    .ok_or(VMError::MemoryLoadOutOfBounds)?,
            ) {
                *memory_byte = *temp.get(i).unwrap_or(&0u8);
            }
        }
        Ok(())
    }

    pub fn expansion_cost(&self, memory_byte_size: usize) -> Result<U256, OutOfGasError> {
        if memory_byte_size <= self.data.len() {
            return Ok(U256::zero());
        }

        let new_memory_size_word = memory_byte_size
            .checked_add(WORD_SIZE - 1)
            .ok_or(OutOfGasError::GasCostOverflow)?
            / WORD_SIZE;

        let new_memory_cost = new_memory_size_word
            .checked_mul(new_memory_size_word)
            .map(|square| square / MEMORY_EXPANSION_QUOTIENT)
            .and_then(|cost| cost.checked_add(new_memory_size_word.checked_mul(3)?))
            .ok_or(OutOfGasError::GasCostOverflow)?;

        let last_memory_size_word = self
            .data
            .len()
            .checked_add(WORD_SIZE - 1)
            .ok_or(OutOfGasError::GasCostOverflow)?
            / WORD_SIZE;

        let last_memory_cost = last_memory_size_word
            .checked_mul(last_memory_size_word)
            .map(|square| square / MEMORY_EXPANSION_QUOTIENT)
            .and_then(|cost| cost.checked_add(last_memory_size_word.checked_mul(3)?))
            .ok_or(OutOfGasError::GasCostOverflow)?;

        Ok((new_memory_cost
            .checked_sub(last_memory_cost)
            .ok_or(OutOfGasError::GasCostOverflow)?)
        .into())
    }
}
