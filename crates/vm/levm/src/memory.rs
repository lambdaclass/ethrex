#![warn(clippy::arithmetic_side_effects)]

use crate::{
    constants::{MEMORY_EXPANSION_QUOTIENT, WORD_SIZE_IN_BYTES_USIZE},
    errors::{ExceptionalHalt, VMError},
};
use ethrex_common::U256;
use std::{
    alloc::{self, Layout},
    array,
    cell::UnsafeCell,
    collections::BTreeMap,
    fmt,
    mem::{self, MaybeUninit},
    ops::Range,
    ptr,
    rc::{Rc, Weak},
    slice,
};

#[derive(Debug)]
struct MemoryAllocatorImpl {
    buffer: *mut u8,
    len: usize,

    tracker: BTreeMap<(usize, usize), Weak<CowSliceRef>>,

    #[cfg(debug_assertions)]
    memory_stack: Vec<usize>,
}

#[derive(Debug)]
struct MemoryAllocator(UnsafeCell<MemoryAllocatorImpl>);

impl MemoryAllocator {
    pub fn new_memory(self: Rc<Self>, offset: usize) -> Memory {
        #[cfg(debug_assertions)]
        {
            let state = unsafe { &mut *self.0.get() };
            assert!(
                state
                    .memory_stack
                    .last()
                    .is_none_or(|&last_offset| offset >= last_offset),
                "attempt to create a child memory from a non-last memory",
            );

            state.memory_stack.push(offset);
        }

        Memory {
            allocator: self,
            range: offset..offset,
        }
    }

    pub fn as_slice(&self, offset: usize, len: usize) -> &[u8] {
        let state = unsafe { &*self.0.get() };
        if state.buffer.is_null() && len == 0 {
            return &[];
        }

        debug_assert!(
            offset + len <= state.len,
            "memory as_slice operation is out of bounds",
        );

        #[expect(unsafe_code, reason = "bounds have already been checked in debug mode")]
        unsafe {
            slice::from_raw_parts(state.buffer.wrapping_add(offset), len)
        }
    }

    pub fn read(&self, offset: usize, buffer: &mut [u8]) {
        let state = unsafe { &*self.0.get() };
        debug_assert!(
            offset + buffer.len() <= state.len,
            "memory read operation is out of bounds",
        );
        debug_assert!(
            state
                .buffer
                .wrapping_add(offset)
                .addr()
                .abs_diff(buffer.as_ptr().addr())
                >= buffer.len(),
            "memory read operation has overlapping source and target buffers",
        );

        #[expect(unsafe_code, reason = "bounds have already been checked in debug mode")]
        unsafe {
            ptr::copy_nonoverlapping(
                state.buffer.wrapping_add(offset),
                buffer.as_mut_ptr(),
                buffer.len(),
            );
        }
    }

    pub fn write(&self, offset: usize, buffer: &[u8]) {
        let state = unsafe { &*self.0.get() };
        debug_assert!(
            offset + buffer.len() <= state.len,
            "memory write operation is out of bounds",
        );
        debug_assert!(
            state
                .buffer
                .wrapping_add(offset)
                .addr()
                .abs_diff(buffer.as_ptr().addr())
                >= buffer.len(),
            "memory write operation has overlapping source and target buffers",
        );

        #[expect(unsafe_code, reason = "bounds have already been checked in debug mode")]
        unsafe {
            ptr::copy_nonoverlapping(
                buffer.as_ptr(),
                state.buffer.wrapping_add(offset),
                buffer.len(),
            );
        }
    }

    pub fn copy(&self, source: usize, target: usize, len: usize) {
        let state = unsafe { &*self.0.get() };
        debug_assert!(
            source + len <= state.len,
            "memory copy operation source is out of bounds",
        );
        debug_assert!(
            target + len <= state.len,
            "memory copy operation target is out of bounds",
        );

        #[expect(unsafe_code, reason = "bounds have already been checked in debug mode")]
        unsafe {
            ptr::copy(
                state.buffer.wrapping_add(source),
                state.buffer.wrapping_add(target),
                len,
            );
        }
    }

    pub fn copy_nonoverlapping(&self, source: usize, target: usize, len: usize) {
        let state = unsafe { &*self.0.get() };
        debug_assert!(
            source + len <= state.len,
            "memory copy operation source is out of bounds",
        );
        debug_assert!(
            target + len <= state.len,
            "memory copy operation target is out of bounds",
        );
        debug_assert!(
            source.abs_diff(target) >= len,
            "memory copy operation has overlapping source and target buffers",
        );

        #[expect(unsafe_code, reason = "bounds have already been checked in debug mode")]
        unsafe {
            ptr::copy_nonoverlapping(
                state.buffer.wrapping_add(source),
                state.buffer.wrapping_add(target),
                len,
            );
        }
    }

    pub fn fill_zeros(&self, offset: usize, len: usize) {
        let state = unsafe { &*self.0.get() };
        debug_assert!(
            offset + len <= state.len,
            "memory zero-fill operation is out of bounds",
        );

        #[expect(unsafe_code, reason = "bounds have already been checked in debug mode")]
        unsafe {
            ptr::write_bytes(state.buffer.wrapping_add(offset), 0, len);
        }
    }

    pub fn maybe_grow(&self, len: usize) {
        let state = unsafe { &mut *self.0.get() };
        if len > state.len {
            state.len = len.next_multiple_of(4096);
            state.buffer = unsafe { alloc::realloc(state.buffer, Layout::new::<u8>(), state.len) };
        }
    }

    #[cfg(debug_assertions)]
    #[expect(
        clippy::arithmetic_side_effects,
        clippy::expect_used,
        reason = "debug-only method"
    )]
    pub fn check_memory_growth(&self, memory_offset: usize, len: usize) {
        let state = unsafe { &*self.0.get() };

        // Find the memory offset in the stack.
        let stack_offset = state
            .memory_stack
            .binary_search(&memory_offset)
            .expect("requested memory not in memory stack");

        // Ensure that either:
        //   - It's the last memory in the stack (which can grow indefinitely).
        //   - It has space to grow before the next memory.
        assert!(
            state
                .memory_stack
                .get(stack_offset + 1)
                .is_none_or(|&memory_offset| len <= memory_offset),
            "attempt to grow a memory into another memory's buffer space",
        );
    }

    #[cfg(debug_assertions)]
    pub fn drop_memory(&self, memory_offset: usize) {
        let state = unsafe { &mut *self.0.get() };

        // Find the memory offset in the stack.
        let stack_offset = state
            .memory_stack
            .binary_search(&memory_offset)
            .expect("requested memory not in memory stack");

        // Remove the memory from the tracked list.
        state.memory_stack.remove(stack_offset);
    }
}

impl Default for MemoryAllocator {
    fn default() -> Self {
        Self(UnsafeCell::new(MemoryAllocatorImpl {
            buffer: ptr::null_mut(),
            len: 0,

            tracker: BTreeMap::new(),

            #[cfg(debug_assertions)]
            memory_stack: Vec::new(),
        }))
    }
}

pub struct Memory {
    allocator: Rc<MemoryAllocator>,
    range: Range<usize>,
}

impl Memory {
    pub fn make_child(&self) -> Self {
        self.allocator.clone().new_memory(self.range.end)
    }

    pub fn is_empty(&self) -> bool {
        self.range.start == self.range.end
    }

    #[expect(
        clippy::arithmetic_side_effects,
        reason = "end is never less than start"
    )]
    pub fn len(&self) -> usize {
        self.range.end - self.range.start
    }

    // TODO: Remove once slices are properly implemented.
    pub fn grow_to(&mut self, len: usize) {
        self.maybe_grow(self.range.start + len, self.range.start + len);
    }

    pub fn as_slice(&mut self, offset: usize, len: usize) -> &[u8] {
        if len == 0 {
            return &[];
        }

        // Compute allocator buffer range.
        let range = {
            let offset = self.range.start + offset;
            offset..offset + len
        };

        // Grow memory if necessary, zero-filling as needed.
        self.maybe_grow(range.end, range.end);

        self.allocator.as_slice(range.start, len)
    }

    // TODO: try_as_slice().

    // TODO: get_slice().
    // TODO: try_get_slice().
    // TODO: get_slice_uninit().
    // TODO: try_get_slice_uninit().

    pub fn read(&mut self, offset: usize, buffer: &mut [u8]) {
        if buffer.len() == 0 {
            return;
        }

        // Compute allocator buffer range.
        let range = {
            let offset = self.range.start + offset;
            offset..offset + buffer.len()
        };

        // Grow memory if necessary, zero-filling as needed until `offset`.
        self.maybe_grow(range.end, range.end);

        // Copy the data.
        self.allocator.read(range.start, buffer);
    }

    pub fn try_read(&self, offset: usize, buffer: &mut [u8]) -> Result<(), ()> {
        if buffer.len() == 0 {
            return Ok(());
        }

        // Compute allocator buffer range.
        let range = {
            let offset = self.range.start + offset;
            offset..offset + buffer.len()
        };

        // Check memory bounds.
        if range.end > self.range.end {
            return Err(());
        }

        // Copy the data.
        self.allocator.read(range.start, buffer);

        Ok(())
    }

    pub fn read_u8(&mut self, offset: usize) -> u8 {
        let mut value = 0u8;
        self.read(offset, array::from_mut(&mut value));
        value
    }

    pub fn try_read_u8(&self, offset: usize) -> Result<u8, ()> {
        let mut value = 0u8;
        self.try_read(offset, array::from_mut(&mut value))?;
        Ok(value)
    }

    pub fn read_u256(&mut self, offset: usize) -> U256 {
        let mut value = MaybeUninit::<U256>::uninit();
        #[expect(
            unsafe_code,
            reason = "write-only buffer, all states are valid, reduced alignment requirements"
        )]
        let buffer = unsafe { slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), 32) };

        self.read(offset, buffer);
        buffer.reverse();

        #[expect(unsafe_code, reason = "already initialized")]
        unsafe {
            value.assume_init()
        }
    }

    pub fn try_read_u256(&self, offset: usize) -> Result<U256, ()> {
        let mut value = MaybeUninit::<U256>::uninit();
        #[expect(
            unsafe_code,
            reason = "write-only buffer, all states are valid, reduced alignment requirements"
        )]
        let buffer = unsafe { slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), 32) };

        self.try_read(offset, buffer)?;
        buffer.reverse();

        #[expect(unsafe_code, reason = "already initialized")]
        Ok(unsafe { value.assume_init() })
    }

    pub fn write(&mut self, offset: usize, buffer: &[u8]) {
        if buffer.len() == 0 {
            return;
        }

        // Compute allocator buffer range.
        let range = {
            let offset = self.range.start + offset;
            offset..offset + buffer.len()
        };

        // Grow memory if necessary, zero-filling as needed until `offset`.
        self.maybe_grow(range.end, range.start);

        // Copy the data.
        self.allocator.write(range.start, buffer);
    }

    pub fn try_write(&mut self, offset: usize, buffer: &[u8]) -> Result<(), ()> {
        if buffer.len() == 0 {
            return Ok(());
        }

        // Compute allocator buffer range.
        let range = {
            let offset = self.range.start + offset;
            offset..offset + buffer.len()
        };

        // Check memory bounds.
        if range.end > self.range.end {
            return Err(());
        }

        // Copy the data.
        self.allocator.write(range.start, buffer);

        Ok(())
    }

    pub fn write_u8(&mut self, offset: usize, value: u8) {
        self.write(offset, array::from_ref(&value));
    }

    pub fn try_write_u8(&mut self, offset: usize, value: u8) -> Result<(), ()> {
        self.try_write(offset, array::from_ref(&value))
    }

    pub fn write_u256(&mut self, offset: usize, mut value: U256) {
        #[expect(unsafe_code, reason = "reduced alignment requirements")]
        let buffer = unsafe { mem::transmute::<&mut [u64; 4], &mut [u8; 32]>(&mut value.0) };
        buffer.reverse();

        self.write(offset, buffer);
    }

    pub fn try_write_u256(&mut self, offset: usize, mut value: U256) -> Result<(), ()> {
        #[expect(unsafe_code, reason = "reduced alignment requirements")]
        let buffer = unsafe { mem::transmute::<&mut [u64; 4], &mut [u8; 32]>(&mut value.0) };
        buffer.reverse();

        self.try_write(offset, buffer)
    }

    pub fn copy(&mut self, source: usize, target: usize, len: usize) {
        if len == 0 {
            return;
        }

        let source = {
            let offset = self.range.start + source;
            offset..offset + len
        };
        let target = {
            let offset = self.range.start + target;
            offset..offset + len
        };

        // Grow memory if necessary, zero-filling as needed until `offset`.
        self.maybe_grow(source.end, source.end);
        self.maybe_grow(target.end, target.start);

        // Copy the data from source into target.
        if source.end <= self.range.end {
            // Range is completely within the memory.
            self.allocator.copy(source.start, target.start, len);
        } else if source.start >= self.range.end {
            // Range is completely outside the memory.
            self.allocator.fill_zeros(target.start, len);
        } else {
            // Range is partially within the memory.
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "bounds have already been checked"
            )]
            {
                let offset = self.range.end - source.start;
                self.allocator.copy(source.start, target.start, offset);
                self.allocator
                    .fill_zeros(target.start + offset, len - offset);
            }
        }
    }

    pub fn try_copy(&mut self, source: usize, target: usize, len: usize) -> Result<(), ()> {
        if len == 0 {
            return Ok(());
        }

        let source = {
            let offset = self.range.start + source;
            offset..offset + len
        };
        let target = {
            let offset = self.range.start + target;
            offset..offset + len
        };

        // Check memory bounds.
        if source.end > self.range.end || target.end > self.range.end {
            return Err(());
        }

        // Copy the data from source into target.
        if source.end <= self.range.end {
            // Range is completely within the memory.
            self.allocator.copy(source.start, target.start, len);
        } else if source.start >= self.range.end {
            // Range is completely outside the memory.
            self.allocator.fill_zeros(target.start, len);
        } else {
            // Range is partially within the memory.
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "bounds have already been checked"
            )]
            {
                let offset = self.range.end - source.start;
                self.allocator.copy(source.start, target.start, offset);
                self.allocator
                    .fill_zeros(target.start + offset, len - offset);
            }
        }

        Ok(())
    }

    pub fn copy_nonoverlapping(&mut self, source: usize, target: usize, len: usize) {
        if len == 0 {
            return;
        }

        let source = {
            let offset = self.range.start + source;
            offset..offset + len
        };
        let target = {
            let offset = self.range.start + target;
            offset..offset + len
        };

        // Grow memory if necessary, zero-filling as needed until `offset`.
        self.maybe_grow(source.end, source.end);
        self.maybe_grow(target.end, target.start);

        // Copy the data from source into target.
        if source.end <= self.range.end {
            // Range is completely within the memory.
            self.allocator
                .copy_nonoverlapping(source.start, target.start, len);
        } else if source.start >= self.range.end {
            // Range is completely outside the memory.
            self.allocator.fill_zeros(target.start, len);
        } else {
            // Range is partially within the memory.
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "bounds have already been checked"
            )]
            {
                let offset = self.range.end - source.start;
                self.allocator
                    .copy_nonoverlapping(source.start, target.start, offset);
                self.allocator
                    .fill_zeros(target.start + offset, len - offset);
            }
        }
    }

    pub fn try_copy_nonoverlapping(
        &mut self,
        source: usize,
        target: usize,
        len: usize,
    ) -> Result<(), ()> {
        if len == 0 {
            return Ok(());
        }

        let source = {
            let offset = self.range.start + source;
            offset..offset + len
        };
        let target = {
            let offset = self.range.start + target;
            offset..offset + len
        };

        // Check memory bounds.
        if source.end > self.range.end || target.end > self.range.end {
            return Err(());
        }

        // Copy the data from source into target.
        if source.end <= self.range.end {
            // Range is completely within the memory.
            self.allocator
                .copy_nonoverlapping(source.start, target.start, len);
        } else if source.start >= self.range.end {
            // Range is completely outside the memory.
            self.allocator.fill_zeros(target.start, len);
        } else {
            // Range is partially within the memory.
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "bounds have already been checked"
            )]
            {
                let offset = self.range.end - source.start;
                self.allocator
                    .copy_nonoverlapping(source.start, target.start, offset);
                self.allocator
                    .fill_zeros(target.start + offset, len - offset);
            }
        }

        Ok(())
    }

    pub fn fill_zeroes(&mut self, offset: usize, len: usize) {
        if len == 0 {
            return;
        }

        // Compute allocator buffer range.
        let range = {
            let offset = self.range.start + offset;
            offset..offset + len
        };

        // Grow memory if necessary, zero-filling as needed until `offset`.
        self.maybe_grow(range.end, range.start);

        // Zero-fill the requested range.
        self.allocator.fill_zeros(range.start, len);
    }

    pub fn try_fill_zeroes(&mut self, offset: usize, len: usize) -> Result<(), ()> {
        if len == 0 {
            return Ok(());
        }

        // Compute allocator buffer range.
        let range = {
            let offset = self.range.start + offset;
            offset..offset + len
        };

        // Check memory bounds.
        if range.end > self.range.end {
            return Err(());
        }

        // Zero-fill the requested range.
        self.allocator.fill_zeros(range.start, len);

        Ok(())
    }

    /// Maybe grow the memory to accomodate `len` elements, zero-filling bytes until `offset`.
    ///
    /// Both `len` and `offset` are in global offsets (allocator-relative, not memory-relative).
    fn maybe_grow(&mut self, len: usize, offset: usize) {
        if len > self.range.end {
            let extended_len = len.next_multiple_of(32);

            // Ensure that growing the memory will not overlap with other memories.
            #[cfg(debug_assertions)]
            self.allocator
                .check_memory_growth(self.range.start, extended_len);

            // Ensure the buffer has the requested capacity.
            self.allocator.maybe_grow(extended_len);

            // Zero-fill from the current `self.range.end` until `offset`.
            if let Some(delta) = offset.checked_sub(self.range.end) {
                self.allocator.fill_zeros(self.range.end, delta);
            }
            // Zero-fill from `offset` until `extended_len`.
            if extended_len.wrapping_sub(len) > 0 {
                self.allocator
                    .fill_zeros(len, extended_len.wrapping_sub(len));
            }

            self.range.end = extended_len;
        }
    }
}

impl Default for Memory {
    fn default() -> Self {
        Rc::new(MemoryAllocator::default()).new_memory(0)
    }
}

impl fmt::Debug for Memory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Memory")
            .field(&self.allocator.as_slice(self.range.start, self.len()))
            .finish()
    }
}

#[cfg(debug_assertions)]
impl Drop for Memory {
    fn drop(&mut self) {
        self.allocator.drop_memory(self.range.start);
    }
}

pub struct CowSlice(CowSliceImpl);

enum CowSliceImpl {}

struct CowSliceRef(Rc<Memory>);

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
#[inline]
fn cost(memory_size: usize) -> Result<u64, VMError> {
    let memory_size_word = memory_size
        .checked_add(WORD_SIZE_IN_BYTES_USIZE.wrapping_sub(1))
        .ok_or(ExceptionalHalt::OutOfGas)?
        / WORD_SIZE_IN_BYTES_USIZE;

    let gas_cost = (memory_size_word
        .checked_mul(memory_size_word)
        .ok_or(ExceptionalHalt::OutOfGas)?
        / MEMORY_EXPANSION_QUOTIENT)
        .checked_add(
            3usize
                .checked_mul(memory_size_word)
                .ok_or(ExceptionalHalt::OutOfGas)?,
        )
        .ok_or(ExceptionalHalt::OutOfGas)?;

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
        .ok_or(ExceptionalHalt::OutOfBounds.into())
}

// #[cfg(test)]
// mod test {
//     use super::*;

//     #[test]
//     fn memory_allocator_default() {
//         let allocator = MemoryAllocator::default();

//         assert!(allocator.buffer.is_null());
//         assert_eq!(allocator.len, 0);
//         assert!(allocator.tracker.is_empty());
//         #[cfg(debug_assertions)]
//         assert!(allocator.memory_stack.is_empty());
//     }

//     #[test]
//     #[cfg(debug_assertions)]
//     fn memory_allocator_check_memory_growth() {
//         // Memory is last in stack.
//         let mut allocator = MemoryAllocator::default();
//         allocator.memory_stack.push(0);

//         allocator.check_memory_growth(0, 0);
//         allocator.check_memory_growth(0, 10);
//         allocator.check_memory_growth(0, 100);

//         // Memory has space to grow.
//         let mut allocator = MemoryAllocator::default();
//         allocator.memory_stack.push(0);
//         allocator.memory_stack.push(100);

//         allocator.check_memory_growth(0, 0);
//         allocator.check_memory_growth(0, 10);
//         allocator.check_memory_growth(0, 100);
//     }

//     #[test]
//     #[should_panic]
//     #[cfg(debug_assertions)]
//     fn memory_allocator_check_memory_growth_memory_not_found() {
//         let allocator = MemoryAllocator::default();
//         allocator.check_memory_growth(0, 0);
//     }

//     #[test]
//     #[should_panic]
//     #[cfg(debug_assertions)]
//     fn memory_allocator_check_memory_growth_not_enough_space() {
//         let mut allocator = MemoryAllocator::default();
//         allocator.memory_stack.push(0);
//         allocator.memory_stack.push(10);

//         allocator.check_memory_growth(0, 100);
//     }
// }

// #[cfg(test)]
// mod test {
//     #![allow(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
//     use ethrex_common::U256;

//     use crate::memory::Memory;

//     #[test]
//     fn test_basic_store_data() {
//         let mut mem = Memory::new();

//         mem.store_data(0, &[1, 2, 3, 4, 0, 0, 0, 0, 0, 0]).unwrap();

//         assert_eq!(&mem.buffer.borrow()[0..10], &[1, 2, 3, 4, 0, 0, 0, 0, 0, 0]);
//         assert_eq!(mem.len(), 32);
//     }

//     #[test]
//     fn test_words() {
//         let mut mem = Memory::new();

//         mem.store_word(0, U256::from(4)).unwrap();

//         assert_eq!(mem.load_word(0).unwrap(), U256::from(4));
//         assert_eq!(mem.len(), 32);
//     }

//     #[test]
//     fn test_copy_word_within() {
//         {
//             let mut mem = Memory::new();

//             mem.store_word(0, U256::from(4)).unwrap();
//             mem.copy_within(0, 32, 32).unwrap();

//             assert_eq!(mem.load_word(32).unwrap(), U256::from(4));
//             assert_eq!(mem.len(), 64);
//         }

//         {
//             let mut mem = Memory::new();

//             mem.store_word(32, U256::from(4)).unwrap();
//             mem.copy_within(32, 0, 32).unwrap();

//             assert_eq!(mem.load_word(0).unwrap(), U256::from(4));
//             assert_eq!(mem.len(), 64);
//         }

//         {
//             let mut mem = Memory::new();

//             mem.store_word(0, U256::from(4)).unwrap();
//             mem.copy_within(0, 0, 32).unwrap();

//             assert_eq!(mem.load_word(0).unwrap(), U256::from(4));
//             assert_eq!(mem.len(), 32);
//         }

//         {
//             let mut mem = Memory::new();

//             mem.store_word(0, U256::from(4)).unwrap();
//             mem.copy_within(32, 0, 32).unwrap();

//             assert_eq!(mem.load_word(0).unwrap(), U256::zero());
//             assert_eq!(mem.len(), 64);
//         }
//     }
// }
