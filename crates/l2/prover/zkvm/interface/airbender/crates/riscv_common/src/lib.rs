#![no_std]

#[inline(always)]
/// Writes a given word into CRS register.
pub fn csr_write_word(word: usize) {
    unsafe {
        core::arch::asm!(
            "csrrw x0, 0x7c0, {rd}",
            rd = in(reg) word,
            options(nomem, nostack, preserves_flags)
        )
    }
}

#[inline(always)]
/// Reads a word from CRS register.
pub fn csr_read_word() -> u32 {
    let mut output;
    unsafe {
        core::arch::asm!(
            "csrrw {rd}, 0x7c0, x0",
            rd = out(reg) output,
            options(nomem, nostack, preserves_flags)
        );
    }

    output
}

#[no_mangle]
pub fn rust_abort() -> ! {
    zksync_os_finish_error()
}

/// Set data as a output of the current execution. Unsatisfiable in circuits
#[inline(never)]
pub fn zksync_os_finish_error() -> ! {
    unsafe {
        core::arch::asm!(
            "csrrw x0, cycle, x0",
            options(nomem, nostack, preserves_flags)
        );
        core::hint::unreachable_unchecked();
    }
}

/// Set data as a output of the current execution.
/// Allows program to pass up to 8 integers as output values.
///
/// By convention, the data that is stored in registers 10-17 after
/// execution has finished is considered 'output' of the computation.
/// Registers 18-25 will be set to 0 as our convention for recursive chain start
#[inline(never)]
pub fn zksync_os_finish_success(data: &[u32; 8]) -> ! {
    let mut result = [0u32; 16];
    result[..8].copy_from_slice(data);
    zksync_os_finish_success_extended(&result)
}

/// Set data as a output of the current execution.
/// By convention, the data that is stored in registers 10-25 after
/// execution has finished is considered 'output' of the computation.
#[inline(never)]
pub fn zksync_os_finish_success_extended(data: &[u32; 16]) -> ! {
    let data_ptr = core::hint::black_box(data.as_ptr().cast::<u32>());
    unsafe {
        core::arch::asm!(
            "lw x10, 0(x26)",
            "lw x11, 4(x26)",
            "lw x12, 8(x26)",
            "lw x13, 12(x26)",
            "lw x14, 16(x26)",
            "lw x15, 20(x26)",
            "lw x16, 24(x26)",
            "lw x17, 28(x26)",
            "lw x18, 32(x26)",
            "lw x19, 36(x26)",
            "lw x20, 40(x26)",
            "lw x21, 44(x26)",
            "lw x22, 48(x26)",
            "lw x23, 52(x26)",
            "lw x24, 56(x26)",
            "lw x25, 60(x26)",
            in("x26") data_ptr,
            out("x10") _,
            out("x11") _,
            out("x12") _,
            out("x13") _,
            out("x14") _,
            out("x15") _,
            out("x16") _,
            out("x17") _,
            out("x18") _,
            out("x19") _,
            out("x20") _,
            out("x21") _,
            out("x22") _,
            out("x23") _,
            out("x24") _,
            out("x25") _,
            options(nostack, preserves_flags)
        )
    }
    loop {
        continue;
    }
}

#[cfg(not(feature = "custom_panic"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    rust_abort();
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NullAllocator;

unsafe impl core::alloc::GlobalAlloc for NullAllocator {
    unsafe fn alloc(&self, _layout: core::alloc::Layout) -> *mut u8 {
        core::hint::unreachable_unchecked()
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {
        core::hint::unreachable_unchecked()
    }

    unsafe fn realloc(
        &self,
        _ptr: *mut u8,
        _layout: core::alloc::Layout,
        _new_size: usize,
    ) -> *mut u8 {
        core::hint::unreachable_unchecked()
    }
}

#[cfg(not(feature = "custom_allocator"))]
#[global_allocator]
static GLOBAL_ALLOCATOR_PLACEHOLDER: NullAllocator = NullAllocator;

#[cfg(feature = "uart")]
#[derive(Default)]
#[repr(C, align(4))]
pub struct QuasiUART {
    buffer: [u8; 4],
    len: usize,
}

#[cfg(feature = "uart")]
impl QuasiUART {
    const HELLO_MARKER: u32 = u32::MAX;

    #[inline(never)]
    pub const fn new() -> Self {
        Self {
            buffer: [0u8; 4],
            len: 0,
        }
    }

    pub fn write_entry_sequence(&mut self, message_len: usize) {
        csr_write_word(Self::HELLO_MARKER as usize);
        // now write length is words for query
        csr_write_word(message_len.next_multiple_of(4) / 4 + 1);
        csr_write_word(message_len);
    }

    pub fn write_word(&self, word: u32) {
        csr_write_word(word as usize);
    }

    pub fn read_word(&self) -> usize {
        csr_read_word() as usize
    }

    fn write_byte(&mut self, byte: u8) {
        self.buffer[self.len] = byte;
        self.len += 1;
        if self.len == 4 {
            self.len = 0;
            let word = u32::from_le_bytes(self.buffer);
            self.write_word(word);
        }
    }

    fn flush(&mut self) {
        if self.len == 0 {
            // cleanup and return
            for dst in self.buffer.iter_mut() {
                *dst = 0;
            }
            return;
        }
        for i in self.len..4 {
            self.buffer[i] = 0u8;
        }
        self.len = 0;
        csr_write_word(u32::from_le_bytes(self.buffer) as usize);
    }

    #[inline(never)]
    pub fn write_debug<T: core::fmt::Debug>(value: &T) {
        use core::fmt::Write;
        let mut writer = Self::new();
        let mut string = heapless::String::<64>::new(); // 64 byte string buffer
        let Ok(_) = write!(string, "{:?}", value) else {
            let _ = writer.write_str("too long debug");
            return;
        };
        let _ = writer.write_str(&string);
    }
}

#[cfg(feature = "uart")]
impl core::fmt::Write for QuasiUART {
    fn write_str(&mut self, s: &str) -> Result<(), core::fmt::Error> {
        self.write_entry_sequence(s.len());
        for c in s.bytes() {
            self.write_byte(c);
        }
        self.flush();

        Ok(())
    }
}
