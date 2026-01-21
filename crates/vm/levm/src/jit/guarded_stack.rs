//! Guarded stack allocation for JIT stack overflow detection.
//!
//! Uses mmap with guard pages to detect stack overflow via SIGSEGV,
//! similar to how the JVM handles stack overflow. This is more efficient
//! than checking bounds on every push operation.
//!
//! ## Memory Layout
//!
//! ```text
//! LOW address                                              HIGH address
//! [HIGH guard] [pos 1023 ... pos 0] [LOW guard]
//!              ^                  ^
//!              LOW addr           HIGH addr
//! ```
//!
//! - Position 0 is at the HIGH address end (adjacent to LOW guard)
//! - Position 1023 is at the LOW address end (adjacent to HIGH guard)
//! - Stack grows from position 0 toward position 1023 (high to low addresses)
//!
//! ## Mirroring for Large Pages
//!
//! For page sizes > 8KB, the stack data doesn't span the full page boundaries,
//! so single writes might not fault even when overflowing. Mirrored writes
//! solve this by writing to both ends of the stack region.

use ethrex_common::U256;
use std::cell::Cell;
use std::ptr::NonNull;
use std::sync::OnceLock;

use crate::constants::STACK_LIMIT;

/// Size of a stack slot in bytes (U256 = 32 bytes)
const SLOT_SIZE: usize = std::mem::size_of::<U256>();

/// Total stack size in bytes
const STACK_SIZE: usize = STACK_LIMIT * SLOT_SIZE;

/// Threshold for enabling mirroring (8KB)
const MIRRORING_THRESHOLD: usize = 8 * 1024;

/// Get the system page size.
pub fn get_page_size() -> usize {
    static PAGE_SIZE: OnceLock<usize> = OnceLock::new();
    *PAGE_SIZE.get_or_init(|| {
        // SAFETY: sysconf is safe to call
        #[expect(unsafe_code, reason = "libc::sysconf call to get page size")]
        let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if size <= 0 {
            // Fallback to common page size
            4096
        } else {
            size as usize
        }
    })
}

/// Number of guard pages at each end of the stack
const GUARD_PAGES: usize = 1;

/// Get the guard size (one page).
fn get_guard_size() -> usize {
    get_page_size() * GUARD_PAGES
}

/// A stack with guard pages for overflow AND underflow detection.
///
/// Memory layout:
/// ```text
/// [HIGH Guard] [Stack Data ...] [LOW Guard]
/// ^-- Low addr                  ^-- High addr
///              ^-- stack_values points here
///                 Stack grows DOWN towards LOW guard (overflow)
///                 Stack grows UP towards HIGH guard (underflow on empty)
/// ```
///
/// When code tries to push beyond the stack limit, the mirrored write
/// triggers SIGSEGV in the LOW guard. When code tries to pop from an
/// empty stack, it reads from the HIGH guard, triggering SIGSEGV.
pub struct GuardedStack {
    /// Base address of the entire allocation (including guard pages)
    base: NonNull<u8>,
    /// Total allocation size (high guard + stack + low guard)
    alloc_size: usize,
    /// Pointer to the start of the stack data (after HIGH guard)
    stack_values: NonNull<U256>,
    /// System page size
    page_size: usize,
    /// Whether mirroring is enabled (page_size > 8KB)
    use_mirroring: bool,
}

// Thread-local storage for the currently executing JIT context.
// Used by the signal handler to determine which context faulted.
thread_local! {
    /// Pointer to the currently executing GuardedStack's HIGH guard region start
    static CURRENT_HIGH_GUARD_START: Cell<usize> = const { Cell::new(0) };
    /// Pointer to the currently executing GuardedStack's HIGH guard region end
    static CURRENT_HIGH_GUARD_END: Cell<usize> = const { Cell::new(0) };
    /// Pointer to the currently executing GuardedStack's LOW guard region start
    static CURRENT_LOW_GUARD_START: Cell<usize> = const { Cell::new(0) };
    /// Pointer to the currently executing GuardedStack's LOW guard region end
    static CURRENT_LOW_GUARD_END: Cell<usize> = const { Cell::new(0) };
    /// Whether a stack overflow was detected
    static STACK_OVERFLOW_DETECTED: Cell<bool> = const { Cell::new(false) };
    /// Whether a stack underflow was detected
    static STACK_UNDERFLOW_DETECTED: Cell<bool> = const { Cell::new(false) };
}

impl GuardedStack {
    /// Create a new guarded stack.
    ///
    /// Allocates memory with guard pages at both ends:
    /// - HIGH guard at low address end (catches underflow)
    /// - LOW guard at high address end (catches overflow)
    pub fn new() -> Result<Self, std::io::Error> {
        let page_size = get_page_size();
        let guard_size = get_guard_size();
        let use_mirroring = page_size > MIRRORING_THRESHOLD;

        // Total size: HIGH guard + stack + LOW guard
        let alloc_size = guard_size + STACK_SIZE + guard_size;

        // Allocate memory using mmap
        #[expect(unsafe_code, reason = "libc::mmap call for memory allocation")]
        let base = unsafe {
            let ptr = libc::mmap(
                std::ptr::null_mut(),
                alloc_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            );

            if ptr == libc::MAP_FAILED {
                return Err(std::io::Error::last_os_error());
            }

            NonNull::new(ptr as *mut u8).expect("mmap returned null")
        };

        // Make the HIGH guard pages inaccessible (at low address end)
        #[expect(unsafe_code, reason = "libc::mprotect call to create guard page")]
        unsafe {
            let result = libc::mprotect(
                base.as_ptr() as *mut libc::c_void,
                guard_size,
                libc::PROT_NONE,
            );

            if result != 0 {
                libc::munmap(base.as_ptr() as *mut libc::c_void, alloc_size);
                return Err(std::io::Error::last_os_error());
            }
        }

        // Make the LOW guard pages inaccessible (at high address end)
        #[expect(unsafe_code, reason = "libc::mprotect call to create guard page")]
        unsafe {
            let low_guard_start = base.as_ptr().add(guard_size + STACK_SIZE);
            let result = libc::mprotect(
                low_guard_start as *mut libc::c_void,
                guard_size,
                libc::PROT_NONE,
            );

            if result != 0 {
                libc::munmap(base.as_ptr() as *mut libc::c_void, alloc_size);
                return Err(std::io::Error::last_os_error());
            }
        }

        // Stack values start after the HIGH guard pages
        #[expect(unsafe_code, reason = "Pointer arithmetic to get stack start")]
        let stack_values = unsafe {
            NonNull::new_unchecked(base.as_ptr().add(guard_size) as *mut U256)
        };

        Ok(Self {
            base,
            alloc_size,
            stack_values,
            page_size,
            use_mirroring,
        })
    }

    /// Get a pointer to the stack values array.
    ///
    /// The returned pointer points to `STACK_LIMIT` U256 values.
    /// Index 0 is at the LOW guard boundary (high address end).
    /// Index STACK_LIMIT-1 is at the HIGH guard boundary (low address end).
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut U256 {
        self.stack_values.as_ptr()
    }

    /// Get whether mirroring is enabled for this stack.
    #[inline]
    pub fn use_mirroring(&self) -> bool {
        self.use_mirroring
    }

    /// Get the page size.
    #[inline]
    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// Get the HIGH guard region address range (catches underflow).
    ///
    /// Returns (start, end) addresses of the HIGH guard region.
    /// This is at the LOW address end of the allocation.
    #[inline]
    pub fn high_guard_region(&self) -> (usize, usize) {
        let start = self.base.as_ptr() as usize;
        let end = start + get_guard_size();
        (start, end)
    }

    /// Get the LOW guard region address range (catches overflow).
    ///
    /// Returns (start, end) addresses of the LOW guard region.
    /// This is at the HIGH address end of the allocation.
    #[inline]
    pub fn low_guard_region(&self) -> (usize, usize) {
        let start = self.base.as_ptr() as usize + get_guard_size() + STACK_SIZE;
        let end = start + get_guard_size();
        (start, end)
    }

    /// Register this stack as the active stack for the current thread.
    ///
    /// This must be called before executing JIT code so the signal
    /// handler knows which guard regions to check.
    pub fn register_active(&self) {
        let (high_start, high_end) = self.high_guard_region();
        let (low_start, low_end) = self.low_guard_region();

        CURRENT_HIGH_GUARD_START.with(|s| s.set(high_start));
        CURRENT_HIGH_GUARD_END.with(|e| e.set(high_end));
        CURRENT_LOW_GUARD_START.with(|s| s.set(low_start));
        CURRENT_LOW_GUARD_END.with(|e| e.set(low_end));
        STACK_OVERFLOW_DETECTED.with(|d| d.set(false));
        STACK_UNDERFLOW_DETECTED.with(|d| d.set(false));
    }

    /// Unregister the active stack for the current thread.
    pub fn unregister_active(&self) {
        CURRENT_HIGH_GUARD_START.with(|s| s.set(0));
        CURRENT_HIGH_GUARD_END.with(|e| e.set(0));
        CURRENT_LOW_GUARD_START.with(|s| s.set(0));
        CURRENT_LOW_GUARD_END.with(|e| e.set(0));
    }

    /// Check if a stack overflow was detected.
    pub fn overflow_detected() -> bool {
        STACK_OVERFLOW_DETECTED.with(|d| d.get())
    }

    /// Check if a stack underflow was detected.
    pub fn underflow_detected() -> bool {
        STACK_UNDERFLOW_DETECTED.with(|d| d.get())
    }

    /// Clear the overflow flag.
    pub fn clear_overflow() {
        STACK_OVERFLOW_DETECTED.with(|d| d.set(false));
    }

    /// Clear the underflow flag.
    pub fn clear_underflow() {
        STACK_UNDERFLOW_DETECTED.with(|d| d.set(false));
    }
}

impl Drop for GuardedStack {
    fn drop(&mut self) {
        #[expect(unsafe_code, reason = "libc::munmap call to free memory")]
        unsafe {
            libc::munmap(self.base.as_ptr() as *mut libc::c_void, self.alloc_size);
        }
    }
}

impl Default for GuardedStack {
    fn default() -> Self {
        Self::new().expect("failed to allocate guarded stack")
    }
}

// SAFETY: GuardedStack owns its memory and can be sent between threads
#[expect(unsafe_code, reason = "GuardedStack can be safely moved between threads")]
unsafe impl Send for GuardedStack {}

/// Check if a fault address is within the HIGH guard region (underflow).
///
/// Called by the signal handler to determine if a SIGSEGV is a stack underflow.
pub fn is_high_guard_fault(fault_addr: usize) -> bool {
    let start = CURRENT_HIGH_GUARD_START.with(|s| s.get());
    let end = CURRENT_HIGH_GUARD_END.with(|e| e.get());

    if start == 0 && end == 0 {
        return false;
    }

    fault_addr >= start && fault_addr < end
}

/// Check if a fault address is within the LOW guard region (overflow).
///
/// Called by the signal handler to determine if a SIGSEGV is a stack overflow.
pub fn is_low_guard_fault(fault_addr: usize) -> bool {
    let start = CURRENT_LOW_GUARD_START.with(|s| s.get());
    let end = CURRENT_LOW_GUARD_END.with(|e| e.get());

    if start == 0 && end == 0 {
        return false;
    }

    fault_addr >= start && fault_addr < end
}

/// Check if a fault address is within any registered guard region.
///
/// Called by the signal handler to determine if a SIGSEGV is a stack fault.
pub fn is_stack_guard_fault(fault_addr: usize) -> bool {
    is_high_guard_fault(fault_addr) || is_low_guard_fault(fault_addr)
}

/// Mark that a stack overflow was detected (LOW guard fault).
///
/// Called by the signal handler when it detects a LOW guard page fault.
pub fn mark_stack_overflow() {
    STACK_OVERFLOW_DETECTED.with(|d| d.set(true));
}

/// Mark that a stack underflow was detected (HIGH guard fault).
///
/// Called by the signal handler when it detects a HIGH guard page fault.
pub fn mark_stack_underflow() {
    STACK_UNDERFLOW_DETECTED.with(|d| d.set(true));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guarded_stack_creation() {
        let stack = GuardedStack::new().expect("failed to create stack");
        let (high_start, high_end) = stack.high_guard_region();
        let (low_start, low_end) = stack.low_guard_region();

        // Both guards should have proper size
        assert!(high_end > high_start);
        assert_eq!(high_end - high_start, get_guard_size());
        assert!(low_end > low_start);
        assert_eq!(low_end - low_start, get_guard_size());

        // LOW guard should be after stack data
        assert!(low_start > high_end);
    }

    #[test]
    fn test_stack_normal_access() {
        let mut stack = GuardedStack::new().expect("failed to create stack");
        let ptr = stack.as_mut_ptr();

        // Write to valid stack locations (should not fault)
        unsafe {
            // Write to the top of the stack (away from HIGH guard, near LOW guard)
            *ptr.add(STACK_LIMIT - 1) = U256::from(42u64);
            assert_eq!(*ptr.add(STACK_LIMIT - 1), U256::from(42u64));

            // Write to position 0 (near LOW guard)
            *ptr.add(0) = U256::from(123u64);
            assert_eq!(*ptr.add(0), U256::from(123u64));
        }
    }

    #[test]
    fn test_high_guard_region_check() {
        let stack = GuardedStack::new().expect("failed to create stack");
        let (high_start, high_end) = stack.high_guard_region();

        stack.register_active();

        // Address in HIGH guard region should be detected
        assert!(is_high_guard_fault(high_start));
        assert!(is_high_guard_fault(high_end - 1));
        assert!(is_stack_guard_fault(high_start));

        // Address outside HIGH guard region should not be detected
        assert!(!is_high_guard_fault(high_end));
        assert!(!is_high_guard_fault(high_start.saturating_sub(1)));

        stack.unregister_active();

        // After unregistering, nothing should be detected
        assert!(!is_high_guard_fault(high_start));
    }

    #[test]
    fn test_low_guard_region_check() {
        let stack = GuardedStack::new().expect("failed to create stack");
        let (low_start, low_end) = stack.low_guard_region();

        stack.register_active();

        // Address in LOW guard region should be detected
        assert!(is_low_guard_fault(low_start));
        assert!(is_low_guard_fault(low_end - 1));
        assert!(is_stack_guard_fault(low_start));

        // Address outside LOW guard region should not be detected
        assert!(!is_low_guard_fault(low_end));
        assert!(!is_low_guard_fault(low_start.saturating_sub(1)));

        stack.unregister_active();

        // After unregistering, nothing should be detected
        assert!(!is_low_guard_fault(low_start));
    }

    #[test]
    fn test_mirroring_flag() {
        let stack = GuardedStack::new().expect("failed to create stack");
        let page_size = get_page_size();

        // Mirroring should be enabled for large pages (> 8KB)
        // and disabled for small pages (<= 8KB)
        if page_size > MIRRORING_THRESHOLD {
            assert!(stack.use_mirroring(), "Large page size should enable mirroring");
        } else {
            assert!(!stack.use_mirroring(), "Small page size should not enable mirroring");
        }
    }
}
