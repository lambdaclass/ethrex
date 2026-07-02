//! 4-way batched keccak256.
//!
//! Hashes independent inputs four at a time through a `u64x4` keccak-f1600
//! permutation. On x86_64 with AVX2 (forced by the `x86-64-v3` target) the
//! permutation uses explicit `__m256i` intrinsics — `vpxor` / `vpsllq`+`vpsrlq`
//! +`vpor` / `vpandnq` — so one permutation advances four hashes at once. On
//! every other target it falls back to a portable scalar `[u64; 4]` loop (four
//! serial lanes; correct but not faster than scalar — used only where AVX2 is
//! unavailable, e.g. zkVM guests).
//!
//! Inputs of unequal length are permitted: each lane tracks its own block count
//! and its 256-bit output is snapshotted the round it consumes its final
//! (padded) block, so later permutations of already-finished lanes are harmless.
//!
//! See issue #6947. A vendored `KeccakP1600times4` asm kernel could replace
//! `keccakf4` later without touching the driver.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use super::keccak_hash;

/// keccak256 rate: 1088 bits = 136 bytes = 17 lanes.
const RATE: usize = 136;
const RATE_LANES: usize = RATE / 8;

/// Four independent keccak-f1600 states, one 64-bit lane value per hash.
type Lane = [u64; 4];

const RC: [u64; 24] = [
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

const RHO: [u32; 24] = [
    1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44,
];

const PI: [usize; 24] = [
    10, 7, 11, 17, 18, 3, 5, 16, 8, 21, 24, 4, 15, 23, 19, 13, 12, 2, 20, 14, 22, 9, 6, 1,
];

// ── AVX2 permutation (explicit intrinsics, not autovectorized) ──────────────
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
use core::arch::x86_64::{
    __m256i, _mm_cvtsi32_si128, _mm256_andnot_si256, _mm256_loadu_si256, _mm256_or_si256,
    _mm256_set1_epi64x, _mm256_setzero_si256, _mm256_sll_epi64, _mm256_srl_epi64,
    _mm256_storeu_si256, _mm256_xor_si256,
};

/// `rotate_left(n)` on four 64-bit lanes at once, `1 <= n <= 63`.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
#[inline(always)]
fn rotl4(x: __m256i, n: u32) -> __m256i {
    // SAFETY: the crate is compiled with target-feature=+avx2 (.cargo/config.toml),
    // so this cfg only compiles when these AVX2 intrinsics are available.
    unsafe {
        let l = _mm256_sll_epi64(x, _mm_cvtsi32_si128(n as i32));
        let r = _mm256_srl_epi64(x, _mm_cvtsi32_si128((64 - n) as i32));
        _mm256_or_si256(l, r)
    }
}

/// keccak-f1600 over four interleaved states, one AVX2 register per state lane.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
fn keccakf4(a: &mut [Lane; 25]) {
    // SAFETY: +avx2 is enabled for the whole crate (see cfg above).
    unsafe {
        let mut s = [_mm256_setzero_si256(); 25];
        for (dst, src) in s.iter_mut().zip(a.iter()) {
            *dst = _mm256_loadu_si256(src.as_ptr() as *const __m256i);
        }

        for &rc in RC.iter() {
            // θ
            let mut c = [_mm256_setzero_si256(); 5];
            for x in 0..5 {
                c[x] = _mm256_xor_si256(
                    _mm256_xor_si256(
                        _mm256_xor_si256(_mm256_xor_si256(s[x], s[x + 5]), s[x + 10]),
                        s[x + 15],
                    ),
                    s[x + 20],
                );
            }
            for x in 0..5 {
                let d = _mm256_xor_si256(c[(x + 4) % 5], rotl4(c[(x + 1) % 5], 1));
                for y in 0..5 {
                    s[x + 5 * y] = _mm256_xor_si256(s[x + 5 * y], d);
                }
            }

            // ρ and π
            let mut last = s[1];
            for i in 0..24 {
                let j = PI[i];
                let tmp = s[j];
                s[j] = rotl4(last, RHO[i]);
                last = tmp;
            }

            // χ
            for y in 0..5 {
                let t = [
                    s[5 * y],
                    s[5 * y + 1],
                    s[5 * y + 2],
                    s[5 * y + 3],
                    s[5 * y + 4],
                ];
                for x in 0..5 {
                    s[5 * y + x] =
                        _mm256_xor_si256(t[x], _mm256_andnot_si256(t[(x + 1) % 5], t[(x + 2) % 5]));
                }
            }

            // ι
            s[0] = _mm256_xor_si256(s[0], _mm256_set1_epi64x(rc as i64));
        }

        for (src, dst) in s.iter().zip(a.iter_mut()) {
            _mm256_storeu_si256(dst.as_mut_ptr() as *mut __m256i, *src);
        }
    }
}

// ── Portable scalar fallback (non-AVX2 targets, e.g. zkVM guests) ───────────
#[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
#[inline(always)]
fn xor(a: Lane, b: Lane) -> Lane {
    [a[0] ^ b[0], a[1] ^ b[1], a[2] ^ b[2], a[3] ^ b[3]]
}

#[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
#[inline(always)]
fn rotl(a: Lane, n: u32) -> Lane {
    [
        a[0].rotate_left(n),
        a[1].rotate_left(n),
        a[2].rotate_left(n),
        a[3].rotate_left(n),
    ]
}

/// `(!b) & c`, per lane.
#[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
#[inline(always)]
fn andn(b: Lane, c: Lane) -> Lane {
    [!b[0] & c[0], !b[1] & c[1], !b[2] & c[2], !b[3] & c[3]]
}

/// keccak-f1600 over four interleaved states (scalar, four serial lanes).
#[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
fn keccakf4(a: &mut [Lane; 25]) {
    for &rc in RC.iter() {
        // θ
        let mut c = [[0u64; 4]; 5];
        for x in 0..5 {
            c[x] = xor(
                xor(xor(xor(a[x], a[x + 5]), a[x + 10]), a[x + 15]),
                a[x + 20],
            );
        }
        for x in 0..5 {
            let d = xor(c[(x + 4) % 5], rotl(c[(x + 1) % 5], 1));
            for y in 0..5 {
                a[x + 5 * y] = xor(a[x + 5 * y], d);
            }
        }

        // ρ and π
        let mut last = a[1];
        for i in 0..24 {
            let j = PI[i];
            let tmp = a[j];
            a[j] = rotl(last, RHO[i]);
            last = tmp;
        }

        // χ
        for y in 0..5 {
            let t = [
                a[5 * y],
                a[5 * y + 1],
                a[5 * y + 2],
                a[5 * y + 3],
                a[5 * y + 4],
            ];
            for x in 0..5 {
                a[5 * y + x] = xor(t[x], andn(t[(x + 1) % 5], t[(x + 2) % 5]));
            }
        }

        // ι
        a[0] = xor(a[0], [rc, rc, rc, rc]);
    }
}

/// Number of blocks input of `len` bytes occupies, including the always-present
/// final padding block (keccak pad10*1).
#[inline(always)]
fn block_count(len: usize) -> usize {
    len / RATE + 1
}

/// Materialize block `bi` of `input` (a full data block, or the final block
/// carrying the remaining data plus keccak256 padding).
#[inline(always)]
fn build_block(input: &[u8], bi: usize) -> [u8; RATE] {
    let mut block = [0u8; RATE];
    let start = bi * RATE;
    let full_blocks = input.len() / RATE;
    if bi < full_blocks {
        block.copy_from_slice(&input[start..start + RATE]);
    } else {
        let rem = &input[start..];
        block[..rem.len()].copy_from_slice(rem);
        block[rem.len()] |= 0x01;
        block[RATE - 1] |= 0x80;
    }
    block
}

/// Hash exactly four inputs in parallel.
fn hash4(inputs: [&[u8]; 4]) -> [[u8; 32]; 4] {
    let mut state = [[0u64; 4]; 25];
    let nb = [
        block_count(inputs[0].len()),
        block_count(inputs[1].len()),
        block_count(inputs[2].len()),
        block_count(inputs[3].len()),
    ];
    let maxb = nb.iter().copied().max().unwrap_or(1);

    let mut out = [[0u8; 32]; 4];
    for bi in 0..maxb {
        for lane in 0..4 {
            if bi < nb[lane] {
                let block = build_block(inputs[lane], bi);
                for w in 0..RATE_LANES {
                    let word = u64::from_le_bytes(block[w * 8..w * 8 + 8].try_into().unwrap());
                    state[w][lane] ^= word;
                }
            }
        }
        keccakf4(&mut state);
        for lane in 0..4 {
            // Snapshot the 256-bit output the round this lane finishes.
            if bi + 1 == nb[lane] {
                for w in 0..4 {
                    out[lane][w * 8..w * 8 + 8].copy_from_slice(&state[w][lane].to_le_bytes());
                }
            }
        }
    }
    out
}

/// keccak256 of each input, computed four at a time. Equivalent to mapping
/// [`keccak_hash`](super::keccak_hash) over `inputs`, but uses the batched
/// permutation for the bulk and scalar hashing for the trailing `< 4`.
pub fn keccak256_batch(inputs: &[&[u8]]) -> Vec<[u8; 32]> {
    let mut out = Vec::with_capacity(inputs.len());
    let mut chunks = inputs.chunks_exact(4);
    for chunk in &mut chunks {
        out.extend_from_slice(&hash4([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    for &rem in chunks.remainder() {
        out.push(keccak_hash(rem));
    }
    out
}
