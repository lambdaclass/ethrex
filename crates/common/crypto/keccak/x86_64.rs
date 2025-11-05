#![cfg(target_arch = "x86_64")]

use super::State;
use std::arch::global_asm;

global_asm!(include_str!("keccak1600-x86_64.s"));

unsafe extern "C" {
    // unsafe fn KeccakF1600();
    pub unsafe fn SHA3_absorb(state: *mut State, buf: *const u8, len: usize, r: usize) -> usize;
    pub unsafe fn SHA3_squeeze(state: *mut State, buf: *mut u8, len: usize, r: usize);
}
