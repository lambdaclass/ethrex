use std::arch::global_asm;

global_asm!(include_str!("x86_64.s"));

unsafe extern "C" {
    unsafe fn _blake2b_f(r: usize, h: &mut [u64; 8], m: &[u64; 16], t: &[u64; 2], f: bool);
}

#[inline(always)]
pub fn blake2b_f(r: usize, h: &mut [u64; 8], m: &[u64; 16], t: &[u64; 2], f: bool) {
    unsafe {
        _blake2b_f(r, h, m, t, f);
    }
}
