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

/// The input of a call frame: a transaction's data, or a range of the calling frame's memory.
///
/// A call's arguments are read where they are instead of being copied for each call, so a
/// CALL-family opcode costs the same whatever the size of its argument region. This is safe
/// because the caller's memory cannot change while the callee runs: the caller grows its
/// memory over the argument and return regions before the call, and only the running frame
/// writes memory, past its own base. The callee's growth can still move the whole buffer to a
/// new allocation, so a range is located through the buffer on every read rather than held
/// as a pointer.
#[derive(Debug, Clone)]
pub enum CallData {
    Owned(Bytes),
    CallerMemory(CallerRange),
}

/// A range of a calling frame's memory, built only by [`Memory::calldata_view`] so that it
/// always lies in a region that stays unchanged while the callee runs.
#[derive(Debug, Clone)]
pub struct CallerRange {
    buffer: Rc<RefCell<Vec<u8>>>,
    /// Offset of the range in the shared buffer, not relative to the caller's base.
    start: usize,
    len: usize,
}

impl Default for CallData {
    fn default() -> Self {
        Self::Owned(Bytes::new())
    }
}

impl From<Bytes> for CallData {
    fn from(bytes: Bytes) -> Self {
        Self::Owned(bytes)
    }
}

impl CallData {
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            Self::Owned(bytes) => bytes.len(),
            Self::CallerMemory(range) => range.len,
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Passes the input to `f` as a slice. For a range of caller memory the buffer stays
    /// borrowed while `f` runs, so `f` must not write memory.
    #[inline]
    pub fn with_slice<R>(&self, f: impl FnOnce(&[u8]) -> R) -> Result<R, VMError> {
        match self {
            Self::Owned(bytes) => Ok(f(bytes)),
            Self::CallerMemory(range) => {
                let buffer = range.buffer.borrow();
                let end = range.start.checked_add(range.len).ok_or(OutOfBounds)?;
                // The buffer never shrinks below a live frame's memory, so this only fails
                // if that invariant is broken.
                let data = buffer.get(range.start..end).ok_or_else(|| {
                    InternalError::Custom("call data outside the memory buffer".to_owned())
                })?;
                Ok(f(data))
            }
        }
    }

    /// The input as `Bytes`: shared when owned, copied when it is a range of caller memory.
    /// For uses that outlive the call, such as traces.
    pub fn to_bytes(&self) -> Result<Bytes, VMError> {
        match self {
            Self::Owned(bytes) => Ok(bytes.clone()),
            Self::CallerMemory(_) => self.with_slice(Bytes::copy_from_slice),
        }
    }

    /// The 32-byte word at `offset`, zero-padded past the end of the input (CALLDATALOAD).
    #[inline]
    pub fn load_word(&self, offset: usize) -> Result<U256, VMError> {
        self.with_slice(|data| {
            let mut word = [0u8; WORD_SIZE_IN_BYTES_USIZE];
            if let Some(tail) = data.get(offset..) {
                let available = tail.len().min(WORD_SIZE_IN_BYTES_USIZE);
                #[expect(clippy::indexing_slicing, reason = "available is within both slices")]
                word[..available].copy_from_slice(&tail[..available]);
            }
            u256_from_big_endian_const(word)
        })
    }
}

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

    /// Resets this memory so its buffer can be reused from a pool by the next transaction:
    /// drops all contents (length → 0, capacity retained) and rebases to 0.
    ///
    /// Truncating the buffer to length 0 is REQUIRED for correctness, not just hygiene:
    /// [`Memory::resize`] only zero-fills bytes grown *past* `buffer.len()`, so handing a
    /// non-empty buffer to the next tx would expose stale data from the previous one (a
    /// consensus bug). Capacity is kept so the grown allocation is reused.
    #[inline]
    pub fn reset_for_reuse(&mut self) {
        self.buf_mut().clear();
        self.len = 0;
        self.current_base = 0;
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

    /// Truncates the memory back to base. This is crucial for constrained
    /// memory in zkVMs. The memory is not freed, but rather shrunk in `len`,
    /// so that the already allocated `capacity` is reused.
    #[cfg(target_arch = "riscv64")]
    #[inline]
    pub fn truncate_to_base(&self) {
        self.buf_mut().truncate(self.current_base);
    }

    /// Unchecked shared access to the backing buffer, skipping `RefCell`'s runtime
    /// borrow-flag read/branch/inc/dec. Sound because the guest is single-threaded
    /// and only the current call frame ever mutates its own memory region, so
    /// these borrows never overlap or alias — the same invariant `clean_from_base`
    /// and the `get_unchecked` reads already rely on. Used only on the hot
    /// MLOAD/MSTORE paths where the borrow is taken, used locally, and dropped.
    #[expect(unsafe_code, reason = "single active frame; no overlapping borrows")]
    #[inline]
    fn buf(&self) -> &Vec<u8> {
        unsafe { &*self.buffer.as_ptr() }
    }

    /// Unchecked mutable counterpart of [`Memory::buf`]; same invariant.
    #[expect(unsafe_code, reason = "single active frame; no overlapping borrows")]
    #[expect(clippy::mut_from_ref, reason = "single active frame; see buf()")]
    #[inline]
    fn buf_mut(&self) -> &mut Vec<u8> {
        unsafe { &mut *self.buffer.as_ptr() }
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

    /// Returns a copy of the live byte slice for this frame (from `current_base` to
    /// `current_base + len`).  Used by the struct-log tracer for memory capture.
    pub fn live_bytes(&self) -> Vec<u8> {
        if self.len == 0 {
            return Vec::new();
        }
        let buf = self.buf();
        let end = self.current_base.saturating_add(self.len);
        buf.get(self.current_base..end)
            .map(<[u8]>::to_vec)
            .unwrap_or_default()
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

        let buffer = self.buf_mut();

        #[allow(clippy::arithmetic_side_effects)]
        let real_new_memory_size = new_memory_size + self.current_base;

        if real_new_memory_size > buffer.len() {
            // when resizing, avoid really small resizes.
            let new_size = real_new_memory_size.next_multiple_of(64);
            buffer.resize(new_size, 0);
        }

        Ok(())
    }

    /// Load `size` bytes from the given offset. Returning a Bytes.
    #[inline]
    pub fn load_range(&mut self, offset: usize, size: usize) -> Result<Bytes, VMError> {
        if size == 0 {
            return Ok(Bytes::new());
        }

        let new_size = offset.checked_add(size).ok_or(OutOfBounds)?;
        self.resize(new_size)?;

        let true_offset = offset.wrapping_add(self.current_base);

        let buf = self.buf();

        // SAFETY: resize already makes sure bounds are correct.
        #[allow(unsafe_code)]
        unsafe {
            Ok(Bytes::copy_from_slice(buf.get_unchecked(
                true_offset..(true_offset.wrapping_add(size)),
            )))
        }
    }

    /// The `size` bytes at `offset` as a call's input, read in place rather than copied; see
    /// [`CallData`]. Grows memory over the range first, like `load_range`, so reads past the
    /// current length see zeros.
    #[inline]
    pub fn calldata_view(&mut self, offset: usize, size: usize) -> Result<CallData, VMError> {
        if size == 0 {
            return Ok(CallData::default());
        }

        let new_size = offset.checked_add(size).ok_or(OutOfBounds)?;
        self.resize(new_size)?;

        Ok(CallData::CallerMemory(CallerRange {
            buffer: Rc::clone(&self.buffer),
            start: self.current_base.checked_add(offset).ok_or(OutOfBounds)?,
            len: size,
        }))
    }

    /// Copies `size` bytes of `calldata` from `src_offset` to `dst_offset`, zero-filling past
    /// the end of the input (CALLDATACOPY).
    pub fn store_calldata(
        &mut self,
        dst_offset: usize,
        calldata: &CallData,
        src_offset: usize,
        size: usize,
    ) -> Result<(), VMError> {
        if size == 0 {
            return Ok(());
        }

        // Grow first: growing can move the buffer, and with it a caller range being read.
        let new_size = dst_offset.checked_add(size).ok_or(OutOfBounds)?;
        self.resize(new_size)?;

        let copied = calldata.len().saturating_sub(src_offset).min(size);
        if copied > 0 {
            match calldata {
                CallData::CallerMemory(range) if Rc::ptr_eq(&range.buffer, &self.buffer) => {
                    let from = range.start.checked_add(src_offset).ok_or(OutOfBounds)?;
                    let to = self
                        .current_base
                        .checked_add(dst_offset)
                        .ok_or(OutOfBounds)?;
                    let from_end = from.checked_add(copied).ok_or(OutOfBounds)?;
                    let buffer = self.buf_mut();
                    if from_end > buffer.len() {
                        return Err(InternalError::Custom(
                            "call data outside the memory buffer".to_owned(),
                        )
                        .into());
                    }
                    buffer.copy_within(from..from_end, to);
                }
                _ => calldata.with_slice(|data| {
                    let data = data.get(src_offset..).unwrap_or_default();
                    self.store(data, dst_offset, copied)
                })??,
            }
        }

        if copied < size {
            #[expect(clippy::arithmetic_side_effects, reason = "copied < size")]
            self.store_zeros(dst_offset.wrapping_add(copied), size - copied)?;
        }

        Ok(())
    }

    /// Borrow `size` bytes from the given offset and pass them to `f`, without
    /// allocating a `Bytes` copy of the range.
    ///
    /// `load_range` reads through `self.buf()`, whose `Ref` guard cannot
    /// outlive this call — so a `-> &[u8]` accessor can't be written. Callers that
    /// only need to read the range (e.g. hashing) take the borrow via this closure
    /// instead. Semantics match `load_range` exactly, including zero-padding reads
    /// past the current length (handled by `resize`).
    #[inline]
    pub fn with_range<R>(
        &mut self,
        offset: usize,
        size: usize,
        f: impl FnOnce(&[u8]) -> R,
    ) -> Result<R, VMError> {
        if size == 0 {
            return Ok(f(&[]));
        }

        let new_size = offset.checked_add(size).ok_or(OutOfBounds)?;
        self.resize(new_size)?;

        let true_offset = offset.wrapping_add(self.current_base);

        let buf = self.buf();

        // SAFETY: resize already makes sure bounds are correct.
        #[allow(unsafe_code)]
        let range = unsafe { buf.get_unchecked(true_offset..(true_offset.wrapping_add(size))) };
        Ok(f(range))
    }

    /// Load N bytes from the given offset.
    #[inline(always)]
    pub fn load_range_const<const N: usize>(&mut self, offset: usize) -> Result<[u8; N], VMError> {
        let new_size = offset.checked_add(N).ok_or(OutOfBounds)?;
        self.resize(new_size)?;

        let true_offset = offset.checked_add(self.current_base).ok_or(OutOfBounds)?;

        let buf = self.buf();
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

        let buffer = self.buf_mut();

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
        self.resize(new_size)?;

        let copy_size = data.len().min(total_size);
        if copy_size > 0 {
            self.store(data, offset, copy_size)?;
        }

        #[allow(clippy::arithmetic_side_effects)]
        if copy_size < total_size {
            // SAFETY: copy_size < total_size and offset + total_size didn't overflow (checked above),
            // so offset + copy_size cannot overflow.
            let zero_offset = offset.wrapping_add(copy_size);
            let zero_size = total_size - copy_size;
            let real_offset = self.current_base.wrapping_add(zero_offset);
            let buffer = self.buf_mut();

            // resize ensures bounds are correct
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

        self.resize(new_size)?;
        self.store(&u256_to_big_endian(word), offset, WORD_SIZE_IN_BYTES_USIZE)?;
        Ok(())
    }

    /// Like [`resize`], but `new_memory_size` MUST already be rounded up to a
    /// multiple of `WORD_SIZE_IN_BYTES_USIZE` (e.g. the value returned by
    /// `calculate_memory_size`). Skips the internal re-rounding the caller has
    /// already performed for the gas charge.
    #[inline]
    pub fn resize_prerounded(&mut self, new_memory_size: usize) -> Result<(), VMError> {
        debug_assert_eq!(new_memory_size % WORD_SIZE_IN_BYTES_USIZE, 0);
        if new_memory_size == 0 {
            return Ok(());
        }
        let current_len = self.len();
        if new_memory_size <= current_len {
            return Ok(());
        }
        self.len = new_memory_size;
        let buffer = self.buf_mut();
        #[allow(clippy::arithmetic_side_effects)]
        let real_new_memory_size = new_memory_size + self.current_base;
        if real_new_memory_size > buffer.len() {
            let new_size = real_new_memory_size.next_multiple_of(64);
            buffer.resize(new_size, 0);
        }
        Ok(())
    }

    /// [`store_word`] variant taking the pre-rounded memory size (see
    /// [`resize_prerounded`]) so the word-rounding is computed once by the
    /// caller instead of twice per MSTORE.
    pub fn store_word_prerounded(
        &mut self,
        offset: usize,
        word: U256,
        new_memory_size: usize,
    ) -> Result<(), VMError> {
        self.resize_prerounded(new_memory_size)?;
        self.store(&u256_to_big_endian(word), offset, WORD_SIZE_IN_BYTES_USIZE)?;
        Ok(())
    }

    /// [`load_word`] variant taking the pre-rounded memory size (see
    /// [`resize_prerounded`]) so MLOAD computes the word-rounding once (for its
    /// gas charge) instead of twice.
    pub fn load_word_prerounded(
        &mut self,
        offset: usize,
        new_memory_size: usize,
    ) -> Result<U256, VMError> {
        self.resize_prerounded(new_memory_size)?;
        let true_offset = offset.checked_add(self.current_base).ok_or(OutOfBounds)?;
        let buf = self.buf();
        // SAFETY: resize_prerounded above guarantees `true_offset + 32` is in bounds.
        #[allow(unsafe_code)]
        let value: [u8; WORD_SIZE_IN_BYTES_USIZE] = unsafe {
            *buf.get_unchecked(true_offset..(true_offset.wrapping_add(WORD_SIZE_IN_BYTES_USIZE)))
                .as_ptr()
                .cast::<[u8; WORD_SIZE_IN_BYTES_USIZE]>()
        };
        Ok(u256_from_big_endian_const(value))
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
        let buffer = self.buf_mut();

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
        self.resize(new_size)?;

        let real_offset = self.current_base.wrapping_add(offset);
        let buffer = self.buf_mut();

        // resize ensures bounds are correct
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
