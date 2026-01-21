//! Signal handler for JIT stack overflow/underflow detection.
//!
//! Installs a SIGSEGV handler that catches accesses to guard pages
//! and converts them to stack overflow/underflow errors via longjmp.
//!
//! ## Guard Page Layout
//!
//! ```text
//! [HIGH guard] [Stack data] [LOW guard]
//!      ^                         ^
//!  underflow                  overflow
//! ```
//!
//! - HIGH guard fault = underflow (popped from empty stack)
//! - LOW guard fault = overflow (pushed beyond limit)

use std::cell::Cell;
use std::sync::Once;

use super::context::{JitContext, JitExitReason, JmpBuf};
use super::guarded_stack::{
    is_high_guard_fault, is_low_guard_fault, is_stack_guard_fault, mark_stack_overflow,
    mark_stack_underflow,
};

// setjmp/longjmp are not in the libc crate, so we declare them manually
#[expect(unsafe_code, reason = "FFI declarations for setjmp/longjmp")]
unsafe extern "C" {
    fn setjmp(env: *mut libc::c_void) -> libc::c_int;
    fn longjmp(env: *mut libc::c_void, val: libc::c_int) -> !;
}

// Thread-local storage for the current JIT context's jmp_buf
thread_local! {
    /// Pointer to the current JitContext (for signal handler to access jmp_buf)
    static CURRENT_JIT_CONTEXT: Cell<*mut JitContext> = const { Cell::new(std::ptr::null_mut()) };
}

/// Register the current JitContext for signal handling.
///
/// Must be called before executing JIT code so the signal handler
/// can access the jmp_buf for stack overflow recovery.
pub fn register_jit_context(ctx: *mut JitContext) {
    CURRENT_JIT_CONTEXT.with(|c| c.set(ctx));
}

/// Unregister the current JitContext.
pub fn unregister_jit_context() {
    CURRENT_JIT_CONTEXT.with(|c| c.set(std::ptr::null_mut()));
}

/// Get the current JitContext pointer.
fn get_current_jit_context() -> *mut JitContext {
    CURRENT_JIT_CONTEXT.with(|c| c.get())
}

// Initialization flag for signal handler
static SIGNAL_HANDLER_INIT: Once = Once::new();

/// Install the SIGSEGV signal handler.
///
/// This should be called once at startup. The handler will catch
/// SIGSEGV signals and check if they're stack guard faults.
pub fn install_signal_handler() {
    SIGNAL_HANDLER_INIT.call_once(|| {
        #[expect(unsafe_code, reason = "libc::sigaction call to install signal handler")]
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();

            // Use sigaction with SA_SIGINFO to get fault address
            action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
            action.sa_sigaction = sigsegv_handler as usize;

            // Block all signals during handler
            libc::sigemptyset(&mut action.sa_mask);

            let result = libc::sigaction(libc::SIGSEGV, &action, std::ptr::null_mut());
            if result != 0 {
                panic!(
                    "Failed to install SIGSEGV handler: {}",
                    std::io::Error::last_os_error()
                );
            }

            // Also handle SIGBUS on macOS (some guard page faults come as SIGBUS)
            #[cfg(target_os = "macos")]
            {
                let result = libc::sigaction(libc::SIGBUS, &action, std::ptr::null_mut());
                if result != 0 {
                    panic!(
                        "Failed to install SIGBUS handler: {}",
                        std::io::Error::last_os_error()
                    );
                }
            }
        }
    });
}

/// SIGSEGV signal handler.
///
/// Checks if the fault is a stack guard access. If so, sets the
/// appropriate error flag (overflow or underflow) and performs
/// longjmp to exit JIT execution. Otherwise, re-raises the signal
/// for normal handling.
extern "C" fn sigsegv_handler(
    sig: libc::c_int,
    info: *mut libc::siginfo_t,
    _ucontext: *mut libc::c_void,
) {
    #[expect(unsafe_code, reason = "Signal handler accesses raw pointers and calls libc functions")]
    unsafe {
        // Get fault address from siginfo
        let fault_addr = if info.is_null() {
            0usize
        } else {
            (*info).si_addr() as usize
        };

        // Check if this is a stack guard fault
        if is_stack_guard_fault(fault_addr) {
            // Get current JIT context
            let ctx = get_current_jit_context();
            if !ctx.is_null() {
                // Determine if it's overflow or underflow
                if is_low_guard_fault(fault_addr) {
                    // LOW guard hit = overflow (pushed too much)
                    mark_stack_overflow();
                    (*ctx).exit_reason = JitExitReason::StackOverflow as u32;
                } else if is_high_guard_fault(fault_addr) {
                    // HIGH guard hit = underflow (popped empty)
                    mark_stack_underflow();
                    (*ctx).exit_reason = JitExitReason::StackUnderflow as u32;
                }

                // Longjmp back to execute_jit
                longjmp((*ctx).jmp_buf.as_ptr() as *mut _, 1);
                // longjmp doesn't return
            }
        }

        // Not a stack guard fault - re-raise the signal
        // Reset to default handler and re-raise
        let mut default_action: libc::sigaction = std::mem::zeroed();
        default_action.sa_flags = 0;
        default_action.sa_sigaction = libc::SIG_DFL;
        libc::sigemptyset(&mut default_action.sa_mask);
        libc::sigaction(sig, &default_action, std::ptr::null_mut());
        libc::raise(sig);
    }
}

/// Wrapper around setjmp for safe JIT execution.
///
/// Returns 0 on initial call, non-zero when longjmp returns here.
///
/// # Safety
///
/// The jmp_buf must remain valid until either:
/// - JIT execution completes normally
/// - longjmp is called with this jmp_buf
#[inline(always)]
#[expect(unsafe_code, reason = "Wrapper for libc setjmp")]
pub unsafe fn jit_setjmp(jmp_buf: &mut JmpBuf) -> i32 {
    // SAFETY: jmp_buf is properly aligned and sized for the platform
    #[expect(unsafe_code, reason = "FFI call to setjmp")]
    unsafe {
        setjmp(jmp_buf.as_mut_ptr() as *mut _)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal_handler_install() {
        // Should not panic
        install_signal_handler();
        // Installing twice should also be fine (idempotent)
        install_signal_handler();
    }
}
