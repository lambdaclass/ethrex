use crate::{
    constants::{MEMORY_EXPANSION_QUOTIENT, WORD_SIZE_IN_BYTES_USIZE},
    errors::{InternalError, OutOfGasError, VMError},
};
use ethrex_core::U256;

pub type Memory = Vec<u8>;

pub fn try_resize(memory: &mut Memory, unchecked_new_size: usize) -> Result<(), VMError> {
    if unchecked_new_size == 0 || unchecked_new_size <= memory.len() {
        return Ok(());
    }

    let new_size = unchecked_new_size
        .checked_next_multiple_of(WORD_SIZE_IN_BYTES_USIZE)
        .ok_or(VMError::OutOfOffset)?;

    if new_size > memory.len() {
        let additional_size = new_size.checked_sub(memory.len()).ok_or(VMError::Internal(
            InternalError::ArithmeticOperationUnderflow,
        ))?;
        memory
            .try_reserve(additional_size)
            .map_err(|_err| VMError::MemorySizeOverflow)?;
        memory.resize(new_size, 0);
    }

    Ok(())
}

pub fn load_word(memory: &mut Memory, offset: usize) -> Result<U256, VMError> {
    load_range(memory, offset, WORD_SIZE_IN_BYTES_USIZE).map(U256::from_big_endian)
}

pub fn load_range(memory: &mut Memory, offset: usize, size: usize) -> Result<&[u8], VMError> {
    if size == 0 {
        return Ok(&[]);
    }

    try_resize(
        memory,
        offset.checked_add(size).ok_or(VMError::OutOfOffset)?,
    )?;

    memory
        .get(offset..offset.checked_add(size).ok_or(VMError::OutOfOffset)?)
        .ok_or(VMError::OutOfOffset)
}

pub fn try_store_word(memory: &mut Memory, offset: usize, word: U256) -> Result<(), VMError> {
    try_resize(
        memory,
        offset
            .checked_add(WORD_SIZE_IN_BYTES_USIZE)
            .ok_or(VMError::OutOfOffset)?,
    )?;
    let mut word_bytes = [0u8; WORD_SIZE_IN_BYTES_USIZE];
    word.to_big_endian(&mut word_bytes);
    try_store(memory, &word_bytes, offset, WORD_SIZE_IN_BYTES_USIZE)
}

pub fn try_store_data(memory: &mut Memory, offset: usize, data: &[u8]) -> Result<(), VMError> {
    try_resize(
        memory,
        offset.checked_add(data.len()).ok_or(VMError::OutOfOffset)?,
    )?;
    try_store(memory, data, offset, data.len())
}

pub fn try_store_range(
    memory: &mut Memory,
    offset: usize,
    size: usize,
    data: &[u8],
) -> Result<(), VMError> {
    try_resize(
        memory,
        offset.checked_add(size).ok_or(VMError::OutOfOffset)?,
    )?;
    try_store(memory, data, offset, size)
}

fn try_store(
    memory: &mut Memory,
    data: &[u8],
    at_offset: usize,
    data_size: usize,
) -> Result<(), VMError> {
    if data_size == 0 {
        return Ok(());
    }

    for (byte_to_store, memory_slot) in data.iter().zip(
        memory
            .get_mut(
                at_offset
                    ..at_offset
                        .checked_add(data_size)
                        .ok_or(VMError::OutOfOffset)?,
            )
            .ok_or(VMError::OutOfOffset)?
            .iter_mut(),
    ) {
        *memory_slot = *byte_to_store;
    }
    Ok(())
}

pub fn try_copy_within(
    memory: &mut Memory,
    from_offset: usize,
    to_offset: usize,
    size: usize,
) -> Result<(), VMError> {
    if size == 0 {
        return Ok(());
    }

    try_resize(
        memory,
        to_offset.checked_add(size).ok_or(VMError::OutOfOffset)?,
    )?;

    let mut temporary_buffer = vec![0u8; size];
    for i in 0..size {
        if let Some(temporary_buffer_byte) = temporary_buffer.get_mut(i) {
            *temporary_buffer_byte = *memory
                .get(from_offset.checked_add(i).ok_or(VMError::OutOfOffset)?)
                .unwrap_or(&0u8);
        }
    }

    for i in 0..size {
        if let Some(memory_byte) =
            memory.get_mut(to_offset.checked_add(i).ok_or(VMError::OutOfOffset)?)
        {
            *memory_byte = *temporary_buffer.get(i).unwrap_or(&0u8);
        }
    }

    Ok(())
}

pub fn access_cost(
    new_memory_size: usize,
    current_memory_size: usize,
    static_cost: U256,
    dynamic_base_cost: U256,
) -> Result<U256, VMError> {
    let minimum_word_size = new_memory_size
        .checked_add(
            WORD_SIZE_IN_BYTES_USIZE
                .checked_sub(1)
                .ok_or(InternalError::ArithmeticOperationUnderflow)?,
        )
        .ok_or(OutOfGasError::MemoryExpansionCostOverflow)?
        / WORD_SIZE_IN_BYTES_USIZE;

    let static_gas = static_cost;
    let dynamic_cost = dynamic_base_cost
        .checked_mul(minimum_word_size.into())
        .ok_or(OutOfGasError::MemoryExpansionCostOverflow)?
        .checked_add(expansion_cost(new_memory_size, current_memory_size)?.into())
        .ok_or(OutOfGasError::MemoryExpansionCostOverflow)?;

    Ok(static_gas
        .checked_add(dynamic_cost)
        .ok_or(OutOfGasError::GasCostOverflow)?)
}

/// When a memory expansion is triggered, only the additional bytes of memory
/// must be paid for.
pub fn expansion_cost(
    new_memory_size: usize,
    current_memory_size: usize,
) -> Result<usize, VMError> {
    let cost = if new_memory_size <= current_memory_size {
        0
    } else {
        cost(new_memory_size)?
            .checked_sub(cost(current_memory_size)?)
            .ok_or(InternalError::ArithmeticOperationUnderflow)?
    };
    Ok(cost)
}

/// The total cost for a given memory size.
fn cost(memory_size: usize) -> Result<usize, VMError> {
    let memory_size_word = memory_size
        .checked_add(
            WORD_SIZE_IN_BYTES_USIZE
                .checked_sub(1)
                .ok_or(InternalError::ArithmeticOperationUnderflow)?,
        )
        .ok_or(OutOfGasError::MemoryExpansionCostOverflow)?
        / WORD_SIZE_IN_BYTES_USIZE;

    Ok(memory_size_word
        .checked_pow(2)
        .ok_or(OutOfGasError::MemoryExpansionCostOverflow)?
        .checked_div(MEMORY_EXPANSION_QUOTIENT)
        .ok_or(OutOfGasError::MemoryExpansionCostOverflow)?
        .checked_add(
            3usize
                .checked_mul(memory_size_word)
                .ok_or(OutOfGasError::MemoryExpansionCostOverflow)?,
        )
        .ok_or(OutOfGasError::MemoryExpansionCostOverflow)?)
}
