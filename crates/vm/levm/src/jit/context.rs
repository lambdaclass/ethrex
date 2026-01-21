//! JIT execution context
//!
//! The `JitContext` struct mirrors `CallFrame` but uses raw pointers
//! and a C-compatible layout for use by JIT-compiled stencils.

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

        Self {
            // Stack
            stack_values: frame.stack.values.as_mut_ptr(),
            stack_offset: frame.stack.offset,

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
    pub fn write_back_to_call_frame(&self, frame: &mut CallFrame) {
        frame.stack.offset = self.stack_offset;
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
    pub unsafe fn push_unchecked(&mut self, value: U256) {
        self.stack_offset = self.stack_offset.wrapping_sub(1);
        // SAFETY: Caller guarantees stack has room
        unsafe { *self.stack_values.add(self.stack_offset) = value };
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
}
