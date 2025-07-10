use crate::{
    constants::{MEMORY_EXPANSION_QUOTIENT, WORD_SIZE_IN_BYTES_USIZE},
    errors::{ExceptionalHalt, InternalError, VMError},
};
use tracing::info;
use ExceptionalHalt::OutOfBounds;
use ExceptionalHalt::OutOfGas;
use ethrex_common::{U256, utils::u256_from_big_endian};

/// Memory of the EVM, a volatile byte array.
pub type Memory = Vec<u8>;

pub fn try_resize(memory: &mut Memory, unchecked_new_size: usize) -> Result<(), VMError> {
    if unchecked_new_size == 0 || unchecked_new_size <= memory.len() {
        return Ok(());
    }

    let new_size = unchecked_new_size
        .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
        .ok_or(OutOfBounds)?;

    if new_size > memory.len() {
        let additional_size = new_size
            .checked_sub(memory.len())
            .ok_or(InternalError::Underflow)?;
        memory
            .try_reserve(additional_size)
            .map_err(|_err| InternalError::MemorySizeOverflow)?;
        memory.resize(new_size, 0);
    }

    Ok(())
}

pub fn load_word(memory: &mut Memory, offset: U256) -> Result<U256, VMError> {
    load_range(memory, offset, WORD_SIZE_IN_BYTES_USIZE).map(u256_from_big_endian)
}

pub fn load_range(memory: &mut Memory, offset: U256, size: usize) -> Result<&[u8], VMError> {
    if size == 0 {
        return Ok(&[]);
    }

    let offset: usize = offset
        .try_into()
        .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;

    try_resize(memory, offset.checked_add(size).ok_or(OutOfBounds)?)?;

    memory
        .get(offset..offset.checked_add(size).ok_or(OutOfBounds)?)
        .ok_or(OutOfBounds.into())
}

pub fn try_store_word(memory: &mut Memory, offset: U256, word: U256) -> Result<(), VMError> {
    let new_size: usize = offset
        .checked_add(WORD_SIZE_IN_BYTES_USIZE.into())
        .ok_or(OutOfBounds)?
        .try_into()
        .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;

    try_resize(memory, new_size)?;
    try_store(
        memory,
        &word.to_big_endian(),
        offset,
        WORD_SIZE_IN_BYTES_USIZE,
    )
}

pub fn try_store_data(memory: &mut Memory, offset: U256, data: &[u8]) -> Result<(), VMError> {
    let new_size = offset
        .checked_add(data.len().into())
        .ok_or(OutOfBounds)?
        .try_into()
        .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
    try_resize(memory, new_size)?;
    try_store(memory, data, offset, data.len())
}

pub fn try_store_range(
    memory: &mut Memory,
    offset: U256,
    size: usize,
    data: &[u8],
) -> Result<(), VMError> {
    if size == 0 {
        return Ok(());
    }

    let new_size = offset
        .checked_add(size.into())
        .ok_or(OutOfBounds)?
        .try_into()
        .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;
    try_resize(memory, new_size)?;
    try_store(memory, data, offset, size)
}

fn try_store(
    memory: &mut Memory,
    data: &[u8],
    at_offset: U256,
    data_size: usize,
) -> Result<(), VMError> {
    if data_size == 0 {
        return Ok(());
    }

    let at_offset: usize = at_offset
        .try_into()
        .map_err(|_err| ExceptionalHalt::VeryLargeNumber)?;

    for (byte_to_store, memory_slot) in data.iter().zip(
        memory
            .get_mut(
                at_offset
                    ..at_offset
                        .checked_add(data_size)
                        .ok_or(InternalError::Overflow)?,
            )
            .ok_or(OutOfBounds)?
            .iter_mut(),
    ) {
        *memory_slot = *byte_to_store;
    }
    Ok(())
}

pub fn try_copy_within(
    memory: &mut Memory,
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
    try_resize(
        memory,
        to_offset
            .max(from_offset)
            .checked_add(size)
            .ok_or(InternalError::Overflow)?,
    )?;

    let mut temporary_buffer = vec![0u8; size];
    for i in 0..size {
        if let Some(temporary_buffer_byte) = temporary_buffer.get_mut(i) {
            *temporary_buffer_byte = *memory
                .get(from_offset.checked_add(i).ok_or(InternalError::Overflow)?)
                .unwrap_or(&0u8);
        }
    }

    for i in 0..size {
        if let Some(memory_byte) = memory.get_mut(to_offset.checked_add(i).ok_or(OutOfBounds)?) {
            *memory_byte = *temporary_buffer.get(i).unwrap_or(&0u8);
        }
    }

    Ok(())
}

/// When a memory expansion is triggered, only the additional bytes of memory
/// must be paid for.
pub fn expansion_cost(new_memory_size: usize, current_memory_size: usize) -> Result<u64, VMError> {
    info!("Calculating expansion cost: 
        - Current expansion cost: {} (for memzise{})
        - Previous expansion cost: {} (for memzise{})",
        cost(new_memory_size)?, new_memory_size,
        cost(current_memory_size)?, current_memory_size,
    );
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
    // memory_size_word = memory_size + (32 - 1)/ 32
    // gas_cost = memory_size_word ** 2 / 512 + 3 * memory_size_word
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
    Ok(((offset + size + 31)/ 32) * 32)

}

