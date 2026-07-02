//! 4-way batched keccak256.
//!
//! On x86_64 with AVX2 (forced by the `x86-64-v3` target) [`keccak256_batch`]
//! hashes independent inputs four at a time through a `u64x4` keccak-f1600
//! permutation built from explicit `__m256i` intrinsics — `vpxor` /
//! `vpsllq`+`vpsrlq`+`vpor` / `vpandnq` — so one permutation advances four
//! hashes at once. The state stays in memory (25 contiguous `__m256i`) and each
//! round streams through it, so only the θ parities or one χ plane are live at a
//! time (~6 `ymm`), avoiding the spilling the full 25+5-lane working set forces
//! through the 16 available registers.
//!
//! On every other target there is no SIMD kernel: the scalar backend
//! (CRYPTOGAMS asm on x86_64/aarch64, `tiny_keccak` elsewhere) beats a portable
//! 4-lane loop, so [`keccak256_batch`] just maps [`keccak_hash`](super::keccak_hash)
//! over the inputs. This keeps aarch64 hosts (Apple Silicon, Graviton) on their
//! fast scalar asm rather than a slower portable kernel.
//!
//! Inputs of unequal length are permitted: each lane tracks its own block count
//! and its 256-bit output is snapshotted the round it consumes its final
//! (padded) block, so later permutations of already-finished lanes are harmless.
//!
//! See issue #6947. A vendored `KeccakP1600times4` asm kernel could replace
//! `keccakf4` later without touching the driver.

use super::keccak_hash;

// ── Scalar fallback: no SIMD win over the asm scalar backend ─────────────────
#[cfg(all(
    not(feature = "std"),
    not(all(target_arch = "x86_64", target_feature = "avx2"))
))]
use alloc::vec::Vec;

#[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
pub fn keccak256_batch(inputs: &[&[u8]]) -> Vec<[u8; 32]> {
    inputs.iter().map(|i| keccak_hash(i)).collect()
}

// ── AVX2 4-way kernel ────────────────────────────────────────────────────────
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
pub use avx2::keccak256_batch;

#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
mod avx2 {
    #[cfg(not(feature = "std"))]
    use alloc::vec::Vec;

    use super::keccak_hash;
    use core::arch::x86_64::{
        __m256i, _mm_cvtsi32_si128, _mm256_andnot_si256, _mm256_loadu_si256, _mm256_or_si256,
        _mm256_set1_epi64x, _mm256_setzero_si256, _mm256_sll_epi64, _mm256_srl_epi64,
        _mm256_storeu_si256, _mm256_xor_si256,
    };

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

    /// `rotate_left(n)` on four 64-bit lanes at once, `1 <= n <= 63`.
    #[inline(always)]
    fn rotl4(x: __m256i, n: u32) -> __m256i {
        // SAFETY: the module only compiles with target-feature=+avx2, so these
        // AVX2 intrinsics are available.
        unsafe {
            let l = _mm256_sll_epi64(x, _mm_cvtsi32_si128(n as i32));
            let r = _mm256_srl_epi64(x, _mm_cvtsi32_si128((64 - n) as i32));
            _mm256_or_si256(l, r)
        }
    }

    #[inline(always)]
    fn xor4(a: __m256i, b: __m256i) -> __m256i {
        // SAFETY: +avx2 is guaranteed for this module.
        unsafe { _mm256_xor_si256(a, b) }
    }

    /// `(!b) & c`, four lanes at once.
    #[inline(always)]
    fn andn4(b: __m256i, c: __m256i) -> __m256i {
        // SAFETY: +avx2 is guaranteed for this module.
        unsafe { _mm256_andnot_si256(b, c) }
    }

    /// keccak-f1600 over four interleaved states.
    fn keccakf4(a: &mut [Lane; 25]) {
        let p = a.as_mut_ptr() as *mut __m256i;
        // SAFETY: `a` is 25 contiguous `[u64; 4]` == 25 `__m256i`; all accesses
        // use unaligned load/store and stay within `0..25`.
        unsafe {
            let ld = |i: usize| _mm256_loadu_si256(p.add(i));
            let st = |i: usize, v: __m256i| _mm256_storeu_si256(p.add(i), v);

            for &rc in RC.iter() {
                // θ: column parities, then fold D into every lane in place.
                let mut bc = [_mm256_setzero_si256(); 5];
                for (x, slot) in bc.iter_mut().enumerate() {
                    *slot = xor4(
                        xor4(xor4(xor4(ld(x), ld(x + 5)), ld(x + 10)), ld(x + 15)),
                        ld(x + 20),
                    );
                }
                for x in 0..5 {
                    let d = xor4(bc[(x + 4) % 5], rotl4(bc[(x + 1) % 5], 1));
                    let mut i = x;
                    while i < 25 {
                        st(i, xor4(ld(i), d));
                        i += 5;
                    }
                }

                // ρ + π: rotate-and-permute the lanes along the fixed cycle.
                let mut t = ld(1);
                for i in 0..24 {
                    let j = PI[i];
                    let tmp = ld(j);
                    st(j, rotl4(t, RHO[i]));
                    t = tmp;
                }

                // χ: one plane (5 lanes) at a time.
                let mut y = 0;
                while y < 25 {
                    let a0 = ld(y);
                    let a1 = ld(y + 1);
                    let a2 = ld(y + 2);
                    let a3 = ld(y + 3);
                    let a4 = ld(y + 4);
                    st(y, xor4(a0, andn4(a1, a2)));
                    st(y + 1, xor4(a1, andn4(a2, a3)));
                    st(y + 2, xor4(a2, andn4(a3, a4)));
                    st(y + 3, xor4(a3, andn4(a4, a0)));
                    st(y + 4, xor4(a4, andn4(a0, a1)));
                    y += 5;
                }

                // ι
                st(0, xor4(ld(0), _mm256_set1_epi64x(rc as i64)));
            }
        }
    }

    /// Number of blocks input of `len` bytes occupies, including the
    /// always-present final padding block (keccak pad10*1).
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

    /// keccak256 of each input, computed four at a time (scalar for the trailing
    /// `< 4`).
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
}
