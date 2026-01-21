//! JIT execution context
//!
//! The `JitContext` struct mirrors `CallFrame` but uses raw pointers
//! and a C-compatible layout for use by JIT-compiled stencils.
//!
//! ## Mirrored Stack Access
//!
//! For systems with large page sizes (>8KB), we use mirrored writes to ensure
//! guard page faults work correctly. The `stack_mirror` pointer moves in the
//! opposite direction from `stack_top`:
//!
//! - Push: `stack_top -= 1`, `stack_mirror += 1`
//! - Pop: `stack_top += 1`, `stack_mirror -= 1`
//!
//! On overflow, the mirror write faults in the LOW guard.
//! On underflow, the primary read faults in the HIGH guard.

use ethrex_common::U256;

use crate::call_frame::CallFrame;
use crate::constants::STACK_LIMIT;

/// Jump buffer for non-local exit from JIT code.
/// Uses platform-specific size (macOS ARM64 requires larger buffer).
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub type JmpBuf = [u64; 25]; // jmp_buf on macOS ARM64

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
pub type JmpBuf = [u64; 32]; // jmp_buf on Linux ARM64

#[cfg(target_arch = "x86_64")]
pub type JmpBuf = [u64; 8]; // jmp_buf on x86_64

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub type JmpBuf = [u64; 64]; // Conservative fallback

/// JIT execution context - mirrors CallFrame with C-compatible layout.
///
/// This struct is passed to JIT-compiled stencils and provides access
/// to the execution state (stack, memory, gas, etc.).
///
/// # Safety
///
/// This struct uses raw pointers and must be carefully managed:
/// - `stack_values` must point to a valid `[U256; 1024]` array
/// - `memory_ptr` must point to valid memory or be null
/// - `bytecode` must point to valid bytecode
/// - `jump_table` must be valid for the bytecode length
/// - `vm_ptr` is opaque and used for callbacks to the VM
#[repr(C)]
pub struct JitContext {
    // Stack (same layout as Stack struct)
    /// Pointer to the stack values array (Box<[U256; 1024]>)
    pub stack_values: *mut U256,
    /// Stack offset - grows downward from STACK_LIMIT (1024)
    pub stack_offset: usize,

    // Pointer-based stack access (for guard page overflow detection)
    /// Current stack top pointer (where next push goes, grows DOWN toward LOW guard)
    /// When this pointer goes into guard page region, SIGSEGV is raised
    pub stack_top: *mut U256,
    /// Stack limit pointer (one past the last valid slot - for underflow check)
    pub stack_limit: *const U256,

    // Mirror stack pointer (for large page mirroring)
    /// Mirror stack pointer - moves OPPOSITE direction from stack_top
    /// Points to position (STACK_LIMIT - 1 - current_offset) relative to stack_values
    /// When stack overflows, mirror write faults in LOW guard
    /// Only used when use_mirroring is true
    pub stack_mirror: *mut U256,
    /// Whether mirroring is enabled (page_size > 8KB)
    pub use_mirroring: bool,

    // Gas
    /// Remaining gas (same i64 as CallFrame for performance)
    pub gas_remaining: i64,

    // Memory
    /// Pointer to memory buffer
    pub memory_ptr: *mut u8,
    /// Current memory size in bytes
    pub memory_size: usize,
    /// Memory capacity (allocated size)
    pub memory_capacity: usize,

    // PC and bytecode
    /// Current program counter (for PC opcode, CODECOPY, etc.)
    pub pc: usize,
    /// Pointer to bytecode
    pub bytecode: *const u8,
    /// Bytecode length
    pub bytecode_len: usize,

    // Jump dispatch table
    /// Maps bytecode PC -> native address for JUMP/JUMPI
    /// Only valid entries are for JUMPDEST locations
    pub jump_table: *const *const u8,

    // VM pointer for callbacks (SLOAD, CALL, etc.)
    /// Opaque pointer to the VM for operations that need interpreter support
    pub vm_ptr: *mut (),

    // Exit information
    /// Exit reason set by stencils (0 = continue, 1 = stop, 2 = return, etc.)
    pub exit_reason: u32,
    /// Return data offset (for RETURN/REVERT)
    pub return_offset: usize,
    /// Return data size (for RETURN/REVERT)
    pub return_size: usize,

    // PUSH value (set by dispatch loop before calling PUSH wrapper)
    /// Current push value for PUSH wrappers to use
    pub push_value: U256,

    // Environment data
    /// Current contract address (ADDRESS opcode)
    pub address: [u8; 20],
    /// Caller address (CALLER opcode)
    pub caller: [u8; 20],
    /// Call value in wei (CALLVALUE opcode)
    pub callvalue: U256,
    /// Pointer to calldata (CALLDATALOAD, CALLDATACOPY)
    pub calldata_ptr: *const u8,
    /// Calldata length (CALLDATASIZE)
    pub calldata_len: usize,

    // JIT non-local exit support
    /// Jump buffer for longjmp-based exit from JIT code
    pub jmp_buf: JmpBuf,
    /// Exit callback function pointer - called by stencils to exit JIT
    /// The callback should perform longjmp to return to execute_jit
    pub exit_callback: Option<extern "C" fn(*mut JitContext) -> !>,
}

/// Exit reasons from JIT execution
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitExitReason {
    /// Continue execution (should not be seen after JIT returns)
    Continue = 0,
    /// STOP opcode - successful execution with no return data
    Stop = 1,
    /// RETURN opcode - successful execution with return data
    Return = 2,
    /// REVERT opcode - reverted execution with return data
    Revert = 3,
    /// Out of gas error
    OutOfGas = 4,
    /// Stack underflow error
    StackUnderflow = 5,
    /// Stack overflow error
    StackOverflow = 6,
    /// Invalid jump destination
    InvalidJump = 7,
    /// Exit to interpreter (for CALL, CREATE, etc.)
    ExitToInterpreter = 8,
    /// Invalid opcode
    InvalidOpcode = 9,
    /// Jump taken - ctx.pc has the destination
    Jump = 10,
}

impl JitContext {
    /// Create a new JitContext from a CallFrame.
    ///
    /// # Safety
    ///
    /// The returned context contains raw pointers that are only valid
    /// for the lifetime of the CallFrame. The caller must ensure the
    /// CallFrame outlives the JitContext.
    #[allow(clippy::as_conversions)]
    #[expect(unsafe_code, reason = "Creates JitContext with raw pointers from CallFrame")]
    pub unsafe fn from_call_frame(
        frame: &mut CallFrame,
        jump_table: *const *const u8,
        vm_ptr: *mut (),
    ) -> Self {
        let memory_ref = frame.memory.buffer.borrow();
        let (memory_ptr, memory_capacity) = if memory_ref.is_empty() {
            (std::ptr::null_mut(), 0)
        } else {
            (memory_ref.as_ptr() as *mut u8, memory_ref.capacity())
        };
        drop(memory_ref);

        let stack_values = frame.stack.values.as_mut_ptr();

        Self {
            // Stack (index-based)
            stack_values,
            stack_offset: frame.stack.offset,

            // Stack (pointer-based for guard page detection)
            // stack_top = base + offset (points to current top)
            // stack_limit = base + STACK_LIMIT (one past end, for underflow check)
            stack_top: unsafe { stack_values.add(frame.stack.offset) },
            stack_limit: unsafe { stack_values.add(STACK_LIMIT) as *const U256 },

            // Mirror pointer for large page systems (set by execute_jit_with_guard)
            // Initial position: STACK_LIMIT - 1 - offset (opposite end)
            // For empty stack (offset = STACK_LIMIT): mirror = -1 (in HIGH guard)
            stack_mirror: if frame.stack.offset >= STACK_LIMIT {
                // Empty stack: mirror points before stack_values (into HIGH guard)
                unsafe { stack_values.sub(1) }
            } else {
                // Non-empty: mirror at opposite position
                unsafe { stack_values.add(STACK_LIMIT - 1 - frame.stack.offset) }
            },
            use_mirroring: false, // Set by execute_jit_with_guard

            // Gas
            gas_remaining: frame.gas_remaining,

            // Memory
            memory_ptr,
            memory_size: frame.memory.len,
            memory_capacity,

            // Bytecode
            pc: frame.pc,
            bytecode: frame.bytecode.bytecode.as_ptr(),
            bytecode_len: frame.bytecode.bytecode.len(),

            // Jump table and VM
            jump_table,
            vm_ptr,

            // Exit info
            exit_reason: JitExitReason::Continue as u32,
            return_offset: 0,
            return_size: 0,

            // PUSH value (set by dispatch loop)
            push_value: U256::zero(),

            // Environment data
            address: frame.to.0,
            caller: frame.msg_sender.0,
            callvalue: frame.msg_value,
            calldata_ptr: frame.calldata.as_ptr(),
            calldata_len: frame.calldata.len(),

            // JIT non-local exit support (initialized by execute_jit)
            jmp_buf: Default::default(),
            exit_callback: None,
        }
    }

    /// Write the JitContext state back to a CallFrame.
    ///
    /// This should be called after JIT execution completes to sync
    /// the modified state back to the interpreter's CallFrame.
    #[allow(clippy::as_conversions)]
    pub fn write_back_to_call_frame(&self, frame: &mut CallFrame) {
        // Compute stack_offset from stack_top if using pointer-based access
        // offset = (stack_top - stack_values) / sizeof(U256)
        if !self.stack_top.is_null() && !self.stack_values.is_null() {
            #[expect(unsafe_code, reason = "Pointer arithmetic for stack offset calculation")]
            let offset = unsafe { self.stack_top.offset_from(self.stack_values) as usize };
            frame.stack.offset = offset;
        } else {
            frame.stack.offset = self.stack_offset;
        }
        frame.gas_remaining = self.gas_remaining;
        frame.pc = self.pc;
        // Memory size is updated but the buffer pointer should not change
        // during JIT execution (memory expansion exits to interpreter)
        frame.memory.len = self.memory_size;
    }

    /// Get the exit reason as an enum
    pub fn exit_reason(&self) -> JitExitReason {
        match self.exit_reason {
            0 => JitExitReason::Continue,
            1 => JitExitReason::Stop,
            2 => JitExitReason::Return,
            3 => JitExitReason::Revert,
            4 => JitExitReason::OutOfGas,
            5 => JitExitReason::StackUnderflow,
            6 => JitExitReason::StackOverflow,
            7 => JitExitReason::InvalidJump,
            8 => JitExitReason::ExitToInterpreter,
            10 => JitExitReason::Jump,
            _ => JitExitReason::InvalidOpcode,
        }
    }
}

// Helper methods for stencils to use
impl JitContext {
    /// Pop a value from the stack (unsafe, no bounds check)
    ///
    /// # Safety
    /// Caller must ensure stack has at least one item
    #[inline(always)]
    #[expect(unsafe_code, reason = "Raw pointer dereference for stack access")]
    pub unsafe fn pop_unchecked(&mut self) -> U256 {
        // SAFETY: Caller guarantees stack has at least one item
        let value = unsafe { *self.stack_values.add(self.stack_offset) };
        self.stack_offset = self.stack_offset.wrapping_add(1);
        value
    }

    /// Push a value to the stack (unsafe, no bounds check)
    ///
    /// # Safety
    /// Caller must ensure stack has room
    #[inline(always)]
    #[expect(unsafe_code, reason = "Raw pointer dereference for stack access")]
    pub unsafe fn push_unchecked(&mut self, value: U256) {
        self.stack_offset = self.stack_offset.wrapping_sub(1);
        // SAFETY: Caller guarantees stack has room
        unsafe { *self.stack_values.add(self.stack_offset) = value };
    }

    /// Pop a value using pointer-based access (for guard page mode)
    ///
    /// # Safety
    /// - Caller must ensure stack has at least one item
    /// - stack_top must be valid
    #[inline(always)]
    #[expect(unsafe_code, reason = "Raw pointer dereference for stack access")]
    pub unsafe fn pop_ptr(&mut self) -> U256 {
        unsafe {
            let value = *self.stack_top;
            self.stack_top = self.stack_top.add(1);
            value
        }
    }

    /// Push a value using pointer-based access (for guard page mode)
    ///
    /// When stack overflows, this will write to the guard page and trigger SIGSEGV.
    ///
    /// # Safety
    /// - stack_top must be valid (or in guard page for overflow detection)
    #[inline(always)]
    #[expect(unsafe_code, reason = "Raw pointer dereference for stack access")]
    pub unsafe fn push_ptr(&mut self, value: U256) {
        unsafe {
            self.stack_top = self.stack_top.sub(1);
            *self.stack_top = value;
            // If stack_top is now in guard page, the write above triggered SIGSEGV
        }
    }

    /// Push a value using mirrored pointer-based access (for large page systems)
    ///
    /// Writes to BOTH primary and mirror positions. On overflow, the mirror
    /// write faults in the LOW guard page.
    ///
    /// # Safety
    /// - stack_top and stack_mirror must be valid (or in guard page for overflow detection)
    #[inline(always)]
    #[expect(unsafe_code, reason = "Raw pointer dereference for mirrored stack access")]
    pub unsafe fn push_ptr_mirrored(&mut self, value: U256) {
        unsafe {
            // Move pointers in opposite directions
            self.stack_top = self.stack_top.sub(1);
            self.stack_mirror = self.stack_mirror.add(1);

            // Write to BOTH locations - mirror write faults on overflow
            *self.stack_top = value;
            *self.stack_mirror = value; // Faults when N > 1023 (mirror into LOW guard)
        }
    }

    /// Pop a value using mirrored pointer-based access
    ///
    /// Single read from primary position. Underflow naturally faults when
    /// stack_top is in HIGH guard (empty stack).
    ///
    /// # Safety
    /// - stack_top must be valid (or in guard page for underflow detection)
    #[inline(always)]
    #[expect(unsafe_code, reason = "Raw pointer dereference for mirrored stack access")]
    pub unsafe fn pop_ptr_mirrored(&mut self) -> U256 {
        unsafe {
            let value = *self.stack_top;
            self.stack_top = self.stack_top.add(1);
            self.stack_mirror = self.stack_mirror.sub(1);
            value
        }
    }

    /// Update mirror pointer to stay in sync with stack_top.
    ///
    /// Call this after operations that modify stack_top by more than 1 position.
    #[inline(always)]
    #[expect(unsafe_code, reason = "Raw pointer arithmetic for mirror synchronization")]
    pub unsafe fn sync_stack_mirror(&mut self) {
        unsafe {
            // Mirror position = STACK_LIMIT - 1 - (stack_top - stack_values)
            // = STACK_LIMIT - 1 - offset
            let offset = self.stack_top.offset_from(self.stack_values) as usize;
            if offset >= STACK_LIMIT {
                // Empty stack: mirror points before stack_values (into HIGH guard)
                self.stack_mirror = self.stack_values.sub(1);
            } else {
                self.stack_mirror = self.stack_values.add(STACK_LIMIT - 1 - offset);
            }
        }
    }

    /// Check if stack would underflow on pop (pointer-based)
    #[inline(always)]
    pub fn would_underflow_ptr(&self) -> bool {
        self.stack_top as *const U256 >= self.stack_limit
    }

    /// Check if stack has at least `n` items
    #[inline(always)]
    pub fn has_stack_items(&self, n: usize) -> bool {
        // Stack grows downward, so offset + n <= STACK_LIMIT means we have n items
        self.stack_offset.wrapping_add(n) <= STACK_LIMIT
    }

    /// Check if stack has room for `n` more items
    #[inline(always)]
    pub fn has_stack_room(&self, n: usize) -> bool {
        self.stack_offset >= n
    }

    /// Get the current stack offset from the pointer.
    ///
    /// This computes `stack_offset` from `stack_top` for code that needs
    /// the index-based representation.
    #[inline(always)]
    pub fn stack_offset_from_ptr(&self) -> usize {
        if self.stack_top.is_null() || self.stack_values.is_null() {
            self.stack_offset
        } else {
            // SAFETY: Both pointers are valid and point into the same allocation
            #[expect(unsafe_code, reason = "Pointer arithmetic for stack offset calculation")]
            unsafe {
                self.stack_top.offset_from(self.stack_values) as usize
            }
        }
    }

    /// Synchronize stack_offset from stack_top.
    ///
    /// Call this after pointer-based operations to update the index-based offset.
    #[inline(always)]
    pub fn sync_stack_offset(&mut self) {
        self.stack_offset = self.stack_offset_from_ptr();
    }
}
