//! Keccak-f[1600] with each of the 25 state lanes in its own 128-bit register, for CPUs with
//! AVX-512F and AVX-512VL. The 32 vector registers hold the whole state through all 24 rounds,
//! the three-input `vpternlogq` does Theta's XORs and Chi in one instruction each, and `vprolq`
//! does the rotations. The registers are 128 bits wide, so CPUs that execute 512-bit operations
//! as two halves still run this at full rate.

use super::imp::{BLOCK_SIZE, State};
use core::arch::x86_64::*;

const ROUND_CONSTANTS: [u64; 24] = [
    0x0000_0000_0000_0001,
    0x0000_0000_0000_8082,
    0x8000_0000_0000_808a,
    0x8000_0000_8000_8000,
    0x0000_0000_0000_808b,
    0x0000_0000_8000_0001,
    0x8000_0000_8000_8081,
    0x8000_0000_0000_8009,
    0x0000_0000_0000_008a,
    0x0000_0000_0000_0088,
    0x0000_0000_8000_8009,
    0x0000_0000_8000_000a,
    0x0000_0000_8000_808b,
    0x8000_0000_0000_008b,
    0x8000_0000_0000_8089,
    0x8000_0000_0000_8003,
    0x8000_0000_0000_8002,
    0x8000_0000_0000_0080,
    0x0000_0000_0000_800a,
    0x8000_0000_8000_000a,
    0x8000_0000_8000_8081,
    0x8000_0000_0000_8080,
    0x0000_0000_8000_0001,
    0x8000_0000_8000_8008,
];

#[inline]
#[target_feature(enable = "avx512f,avx512vl")]
fn xor(a: __m128i, b: __m128i) -> __m128i {
    _mm_xor_si128(a, b)
}

#[inline]
#[target_feature(enable = "avx512f,avx512vl")]
fn xor3(a: __m128i, b: __m128i, c: __m128i) -> __m128i {
    _mm_ternarylogic_epi64::<0x96>(a, b, c)
}

/// `a ^ (!b & c)`
#[inline]
#[target_feature(enable = "avx512f,avx512vl")]
fn chi(a: __m128i, b: __m128i, c: __m128i) -> __m128i {
    _mm_ternarylogic_epi64::<0xd2>(a, b, c)
}

#[inline]
#[target_feature(enable = "avx512f,avx512vl")]
fn rol<const N: i32>(a: __m128i) -> __m128i {
    _mm_rol_epi64::<N>(a)
}

/// The permutation, on lanes indexed `x + 5 * y`.
#[inline]
#[target_feature(enable = "avx512f,avx512vl")]
fn permute(lanes: [__m128i; 25]) -> [__m128i; 25] {
    let [
        mut a0,
        mut a1,
        mut a2,
        mut a3,
        mut a4,
        mut a5,
        mut a6,
        mut a7,
        mut a8,
        mut a9,
        mut a10,
        mut a11,
        mut a12,
        mut a13,
        mut a14,
        mut a15,
        mut a16,
        mut a17,
        mut a18,
        mut a19,
        mut a20,
        mut a21,
        mut a22,
        mut a23,
        mut a24,
    ] = lanes;
    for &rc in &ROUND_CONSTANTS {
        // Theta: C[x] is the column parity, and each lane takes C[x - 1] ^ rol(C[x + 1], 1)
        // in one three-input XOR.
        let c0 = xor3(xor3(a0, a5, a10), a15, a20);
        let c1 = xor3(xor3(a1, a6, a11), a16, a21);
        let c2 = xor3(xor3(a2, a7, a12), a17, a22);
        let c3 = xor3(xor3(a3, a8, a13), a18, a23);
        let c4 = xor3(xor3(a4, a9, a14), a19, a24);
        let r0 = rol::<1>(c0);
        let r1 = rol::<1>(c1);
        let r2 = rol::<1>(c2);
        let r3 = rol::<1>(c3);
        let r4 = rol::<1>(c4);
        // Rho and Pi
        let b0 = xor3(a0, c4, r1);
        let b10 = rol::<1>(xor3(a1, c0, r2));
        let b20 = rol::<62>(xor3(a2, c1, r3));
        let b5 = rol::<28>(xor3(a3, c2, r4));
        let b15 = rol::<27>(xor3(a4, c3, r0));
        let b16 = rol::<36>(xor3(a5, c4, r1));
        let b1 = rol::<44>(xor3(a6, c0, r2));
        let b11 = rol::<6>(xor3(a7, c1, r3));
        let b21 = rol::<55>(xor3(a8, c2, r4));
        let b6 = rol::<20>(xor3(a9, c3, r0));
        let b7 = rol::<3>(xor3(a10, c4, r1));
        let b17 = rol::<10>(xor3(a11, c0, r2));
        let b2 = rol::<43>(xor3(a12, c1, r3));
        let b12 = rol::<25>(xor3(a13, c2, r4));
        let b22 = rol::<39>(xor3(a14, c3, r0));
        let b23 = rol::<41>(xor3(a15, c4, r1));
        let b8 = rol::<45>(xor3(a16, c0, r2));
        let b18 = rol::<15>(xor3(a17, c1, r3));
        let b3 = rol::<21>(xor3(a18, c2, r4));
        let b13 = rol::<8>(xor3(a19, c3, r0));
        let b14 = rol::<18>(xor3(a20, c4, r1));
        let b24 = rol::<2>(xor3(a21, c0, r2));
        let b9 = rol::<61>(xor3(a22, c1, r3));
        let b19 = rol::<56>(xor3(a23, c2, r4));
        let b4 = rol::<14>(xor3(a24, c3, r0));
        // Chi
        a0 = chi(b0, b1, b2);
        a1 = chi(b1, b2, b3);
        a2 = chi(b2, b3, b4);
        a3 = chi(b3, b4, b0);
        a4 = chi(b4, b0, b1);
        a5 = chi(b5, b6, b7);
        a6 = chi(b6, b7, b8);
        a7 = chi(b7, b8, b9);
        a8 = chi(b8, b9, b5);
        a9 = chi(b9, b5, b6);
        a10 = chi(b10, b11, b12);
        a11 = chi(b11, b12, b13);
        a12 = chi(b12, b13, b14);
        a13 = chi(b13, b14, b10);
        a14 = chi(b14, b10, b11);
        a15 = chi(b15, b16, b17);
        a16 = chi(b16, b17, b18);
        a17 = chi(b17, b18, b19);
        a18 = chi(b18, b19, b15);
        a19 = chi(b19, b15, b16);
        a20 = chi(b20, b21, b22);
        a21 = chi(b21, b22, b23);
        a22 = chi(b22, b23, b24);
        a23 = chi(b23, b24, b20);
        a24 = chi(b24, b20, b21);
        // Iota
        a0 = xor(a0, _mm_cvtsi64_si128(rc as i64));
    }
    [
        a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15, a16, a17, a18, a19,
        a20, a21, a22, a23, a24,
    ]
}

#[inline]
#[target_feature(enable = "avx512f,avx512vl")]
fn load_state(state: &State) -> [__m128i; 25] {
    let mut lanes = [_mm_setzero_si128(); 25];
    for (lane, &word) in lanes.iter_mut().zip(&state.0) {
        *lane = _mm_cvtsi64_si128(word as i64);
    }
    lanes
}

#[inline]
#[target_feature(enable = "avx512f,avx512vl")]
fn store_state(state: &mut State, lanes: [__m128i; 25]) {
    for (word, lane) in state.0.iter_mut().zip(lanes) {
        *word = _mm_cvtsi128_si64(lane) as u64;
    }
}

/// Same contract as the assembly `SHA3_absorb`, for the 136-byte rate only.
#[target_feature(enable = "avx512f,avx512vl")]
pub(super) unsafe extern "C" fn absorb(
    state: *mut State,
    mut buf: *const u8,
    mut len: usize,
    rate: usize,
) -> usize {
    debug_assert_eq!(rate, BLOCK_SIZE);
    let state = unsafe { &mut *state };
    let mut lanes = load_state(state);
    while len >= BLOCK_SIZE {
        for (i, lane) in lanes.iter_mut().take(BLOCK_SIZE / 8).enumerate() {
            let word = unsafe { buf.add(8 * i).cast::<u64>().read_unaligned() };
            *lane = xor(*lane, _mm_cvtsi64_si128(word as i64));
        }
        lanes = permute(lanes);
        buf = unsafe { buf.add(BLOCK_SIZE) };
        len -= BLOCK_SIZE;
    }
    store_state(state, lanes);
    len
}

/// Same contract as the assembly `SHA3_squeeze`, for the 136-byte rate only.
#[target_feature(enable = "avx512f,avx512vl")]
pub(super) unsafe extern "C" fn squeeze(
    state: *mut State,
    mut out: *mut u8,
    mut len: usize,
    rate: usize,
) {
    debug_assert_eq!(rate, BLOCK_SIZE);
    let state = unsafe { &mut *state };
    loop {
        let n = len.min(BLOCK_SIZE);
        unsafe { core::ptr::copy_nonoverlapping(state.0.as_ptr().cast::<u8>(), out, n) };
        len -= n;
        if len == 0 {
            return;
        }
        out = unsafe { out.add(n) };
        let lanes = permute(load_state(state));
        store_state(state, lanes);
    }
}
