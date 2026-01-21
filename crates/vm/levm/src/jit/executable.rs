//! Executable memory buffer for JIT-compiled code.
//!
//! This module provides a safe wrapper around platform-specific APIs
//! for allocating, writing, and executing JIT-compiled code.

use std::ptr::NonNull;

use crate::jit::stencils::{RelocKind, Relocation, Stencil};

/// Error type for executable buffer operations
#[derive(Debug, Clone)]
pub enum ExecutableError {
    /// Failed to allocate memory
    AllocationFailed,
    /// Failed to change memory protection
    ProtectionFailed,
    /// Buffer is too small
    BufferTooSmall,
    /// Invalid relocation
    InvalidRelocation,
}

impl std::fmt::Display for ExecutableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AllocationFailed => write!(f, "Failed to allocate executable memory"),
            Self::ProtectionFailed => write!(f, "Failed to change memory protection"),
            Self::BufferTooSmall => write!(f, "Executable buffer too small"),
            Self::InvalidRelocation => write!(f, "Invalid relocation"),
        }
    }
}

impl std::error::Error for ExecutableError {}

/// A buffer of executable memory for JIT-compiled code.
///
/// This struct manages a region of memory that can be written to
/// during compilation and then made executable for running.
///
/// # Safety
///
/// This struct uses raw memory allocation and mprotect/VirtualProtect
/// for managing executable memory. The memory is:
/// - Initially allocated as read-write
/// - Made read-execute after compilation is complete
///
/// # Platform Support
///
/// - Unix (Linux, macOS): Uses mmap/mprotect
/// - Windows: Uses VirtualAlloc/VirtualProtect (TODO)
pub struct ExecutableBuffer {
    /// Pointer to the allocated memory
    ptr: NonNull<u8>,
    /// Total capacity of the buffer
    capacity: usize,
    /// Current write position
    len: usize,
    /// Whether the buffer has been made executable
    is_executable: bool,
    /// Pending relocations to patch
    pending_relocations: Vec<PendingRelocation>,
}

/// A relocation that needs to be patched after all stencils are copied
#[derive(Debug, Clone)]
pub struct PendingRelocation {
    /// Offset in the buffer where the relocation is
    pub buffer_offset: usize,
    /// The relocation info
    pub reloc: Relocation,
    /// Next PC (bytecode offset of the next instruction)
    pub next_pc: usize,
}

/// Error handler addresses for JIT code
#[derive(Debug, Clone, Copy)]
pub struct ErrorHandlers {
    /// Address of the exit handler (used for STOP, errors, etc.)
    pub exit_jit: usize,
}

impl ExecutableBuffer {
    /// Create a new executable buffer with the given capacity.
    ///
    /// The buffer is initially writable but not executable.
    pub fn new(capacity: usize) -> Result<Self, ExecutableError> {
        let ptr = Self::allocate(capacity)?;
        Ok(Self {
            ptr,
            capacity,
            len: 0,
            is_executable: false,
            pending_relocations: Vec::new(),
        })
    }

    /// Get the current length (bytes written)
    pub fn len(&self) -> usize {
        self.len
    }

    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Get the base pointer
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr.as_ptr()
    }

    /// Get the base pointer as mutable (only valid before make_executable)
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        debug_assert!(!self.is_executable, "Cannot write to executable buffer");
        self.ptr.as_ptr()
    }

    /// Copy a stencil's bytes to the buffer.
    ///
    /// Records relocations for later patching.
    /// `next_pc` is the bytecode PC of the next instruction (for NextStencil relocations).
    pub fn copy_stencil(&mut self, stencil: &Stencil, next_pc: usize) -> Result<usize, ExecutableError> {
        let start_offset = self.len;
        let needed = self.len.saturating_add(stencil.bytes.len());

        if needed > self.capacity {
            return Err(ExecutableError::BufferTooSmall);
        }

        // Copy the bytes
        #[expect(unsafe_code, reason = "Raw pointer write to executable buffer")]
        unsafe {
            std::ptr::copy_nonoverlapping(
                stencil.bytes.as_ptr(),
                self.ptr.as_ptr().add(self.len),
                stencil.bytes.len(),
            );
        }

        // Record relocations
        for reloc in stencil.relocations {
            self.pending_relocations.push(PendingRelocation {
                buffer_offset: start_offset.saturating_add(reloc.offset),
                reloc: *reloc,
                next_pc,
            });
        }

        self.len = needed;
        Ok(start_offset)
    }

    /// Write raw bytes to the buffer.
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<usize, ExecutableError> {
        let start_offset = self.len;
        let needed = self.len.saturating_add(bytes.len());

        if needed > self.capacity {
            return Err(ExecutableError::BufferTooSmall);
        }

        #[expect(unsafe_code, reason = "Raw pointer write to executable buffer")]
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.ptr.as_ptr().add(self.len),
                bytes.len(),
            );
        }

        self.len = needed;
        Ok(start_offset)
    }

    /// Patch all pending relocations.
    ///
    /// # Arguments
    /// * `pc_to_native` - Maps bytecode PC to native offset in the buffer
    /// * `handlers` - Error handler addresses
    #[allow(clippy::indexing_slicing)]
    pub fn patch_relocations(
        &mut self,
        pc_to_native: &[Option<usize>],
        handlers: &ErrorHandlers,
    ) -> Result<(), ExecutableError> {
        let base = self.ptr.as_ptr() as usize;

        for pending in &self.pending_relocations {
            let patch_addr = base.saturating_add(pending.buffer_offset);

            let target_addr = match pending.reloc.kind {
                RelocKind::NextStencil => {
                    // Find the native address of the next instruction
                    // If next_pc is beyond bytecode or no instruction starts there, use exit handler
                    if pending.next_pc < pc_to_native.len() {
                        pc_to_native[pending.next_pc]
                            .map(|offset| base.saturating_add(offset))
                            .unwrap_or(handlers.exit_jit)
                    } else {
                        // End of bytecode - point to exit handler
                        handlers.exit_jit
                    }
                }
                RelocKind::ExitJit => handlers.exit_jit,
                RelocKind::ImmediateValue | RelocKind::JumpTableEntry => {
                    // These are handled separately during stencil copying
                    continue;
                }
            };

            // Patch based on architecture and relocation size
            #[cfg(target_arch = "x86_64")]
            #[expect(unsafe_code, reason = "Raw pointer write for relocation patching")]
            unsafe {
                match pending.reloc.size {
                    4 => {
                        // 32-bit relative offset
                        let rel_offset = (target_addr as i64)
                            .saturating_sub(patch_addr as i64)
                            .saturating_sub(4); // Account for the 4-byte offset itself
                        #[allow(clippy::as_conversions)]
                        std::ptr::write_unaligned(patch_addr as *mut i32, rel_offset as i32);
                    }
                    8 => {
                        // 64-bit absolute address
                        #[allow(clippy::as_conversions)]
                        std::ptr::write_unaligned(patch_addr as *mut u64, target_addr as u64);
                    }
                    _ => return Err(ExecutableError::InvalidRelocation),
                }
            }

            #[cfg(target_arch = "aarch64")]
            #[expect(unsafe_code, reason = "Raw pointer write for relocation patching")]
            unsafe {
                // ARM64 branch instructions use 26-bit signed offset, shifted left by 2
                // BL encoding: 1001 01xx xxxx xxxx xxxx xxxx xxxx xxxx (0x9400_0000)
                // B encoding:  0001 01xx xxxx xxxx xxxx xxxx xxxx xxxx (0x1400_0000)
                #[allow(clippy::as_conversions)]
                let rel_offset = (target_addr as i64).saturating_sub(patch_addr as i64);
                let imm26 = (rel_offset >> 2) as i32;

                // Read current instruction
                #[allow(clippy::as_conversions)]
                let insn = std::ptr::read_unaligned(patch_addr as *const u32);
                let opcode = insn & 0xfc00_0000;

                if opcode == 0x9400_0000 {
                    // It's a BL instruction
                    // For NextStencil: convert BL to B (tail jump, don't save return address)
                    // For ExitJit: keep as BL (we want to call exit handler which restores frame)
                    let new_opcode = if pending.reloc.kind == RelocKind::NextStencil {
                        // Convert BL to B for tail chaining between stencils
                        0x1400_0000u32
                    } else {
                        // Keep BL for exit handler calls
                        0x9400_0000u32
                    };
                    #[allow(clippy::as_conversions)]
                    let new_insn = new_opcode | ((imm26 as u32) & 0x03ff_ffff);
                    #[allow(clippy::as_conversions)]
                    std::ptr::write_unaligned(patch_addr as *mut u32, new_insn);
                }
                // If not a BL instruction (e.g., ADRP for immediates), skip for now
            }

            #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
            {
                return Err(ExecutableError::InvalidRelocation);
            }
        }

        Ok(())
    }

    /// Patch an immediate value at a specific offset.
    ///
    /// Used for PUSH instructions where we need to embed the pushed value.
    #[allow(clippy::as_conversions)]
    pub fn patch_immediate(&mut self, offset: usize, value: &[u8; 32]) -> Result<(), ExecutableError> {
        if offset.saturating_add(32) > self.len {
            return Err(ExecutableError::InvalidRelocation);
        }

        #[expect(unsafe_code, reason = "Raw pointer write for immediate patching")]
        unsafe {
            std::ptr::copy_nonoverlapping(
                value.as_ptr(),
                self.ptr.as_ptr().add(offset),
                32,
            );
        }

        Ok(())
    }

    /// Make the buffer executable (and no longer writable).
    pub fn make_executable(&mut self) -> Result<(), ExecutableError> {
        if self.is_executable {
            return Ok(());
        }

        Self::protect_exec(self.ptr.as_ptr(), self.capacity)?;
        self.is_executable = true;
        Ok(())
    }

    /// Get a function pointer to execute the code at the given offset.
    ///
    /// # Safety
    ///
    /// The buffer must have been made executable and the offset must
    /// point to valid code.
    #[allow(clippy::as_conversions)]
    #[expect(unsafe_code, reason = "Returns function pointer from executable buffer")]
    pub unsafe fn get_function<F>(&self, offset: usize) -> Option<F>
    where
        F: Copy,
    {
        if !self.is_executable || offset >= self.len {
            return None;
        }

        // SAFETY: Caller guarantees offset points to valid code
        #[expect(unsafe_code, reason = "Transmute pointer to function type")]
        unsafe {
            let ptr = self.ptr.as_ptr().add(offset);
            Some(std::mem::transmute_copy(&ptr))
        }
    }

    // Platform-specific implementations

    #[cfg(unix)]
    fn allocate(size: usize) -> Result<NonNull<u8>, ExecutableError> {
        use libc::{MAP_ANON, MAP_PRIVATE, PROT_READ, PROT_WRITE};

        #[expect(unsafe_code, reason = "libc::mmap call for memory allocation")]
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANON,
                -1,
                0,
            )
        };

        if ptr == libc::MAP_FAILED {
            return Err(ExecutableError::AllocationFailed);
        }

        #[allow(clippy::as_conversions)]
        NonNull::new(ptr as *mut u8).ok_or(ExecutableError::AllocationFailed)
    }

    #[cfg(unix)]
    fn protect_exec(ptr: *mut u8, size: usize) -> Result<(), ExecutableError> {
        use libc::{PROT_EXEC, PROT_READ};

        #[allow(clippy::as_conversions)]
        #[expect(unsafe_code, reason = "libc::mprotect call to make memory executable")]
        let result = unsafe { libc::mprotect(ptr as *mut libc::c_void, size, PROT_READ | PROT_EXEC) };

        if result != 0 {
            return Err(ExecutableError::ProtectionFailed);
        }

        Ok(())
    }

    #[cfg(unix)]
    fn deallocate(ptr: *mut u8, size: usize) {
        #[allow(clippy::as_conversions)]
        #[expect(unsafe_code, reason = "libc::munmap call to free memory")]
        unsafe {
            libc::munmap(ptr as *mut libc::c_void, size);
        }
    }

    #[cfg(not(unix))]
    fn allocate(_size: usize) -> Result<NonNull<u8>, ExecutableError> {
        // TODO: Windows support with VirtualAlloc
        Err(ExecutableError::AllocationFailed)
    }

    #[cfg(not(unix))]
    fn protect_exec(_ptr: *mut u8, _size: usize) -> Result<(), ExecutableError> {
        Err(ExecutableError::ProtectionFailed)
    }

    #[cfg(not(unix))]
    fn deallocate(_ptr: *mut u8, _size: usize) {}
}

impl Drop for ExecutableBuffer {
    fn drop(&mut self) {
        Self::deallocate(self.ptr.as_ptr(), self.capacity);
    }
}

// ExecutableBuffer is Send but not Sync (can be moved between threads but not shared)
#[expect(unsafe_code, reason = "ExecutableBuffer can be safely moved between threads")]
unsafe impl Send for ExecutableBuffer {}
