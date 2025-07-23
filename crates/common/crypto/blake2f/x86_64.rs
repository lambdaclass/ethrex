use std::arch::x86_64::*;
use std::{arch::asm, mem::transmute};

const BLAKE2B_IV: [__m256i; 3] = const {
    unsafe {
        transmute::<[u64; 12], [__m256i; 3]>([
            0x6A09E667F3BCC908,
            0xBB67AE8584CAA73B,
            0x3C6EF372FE94F82B,
            0xA54FF53A5F1D36F1,
            0x510E527FADE682D1,
            0x9B05688C2B3E6C1F,
            0x1F83D9ABFB41BD6B,
            0x5BE0CD19137E2179,
            //
            // Second half of blake2b_iv with inverted bits (for final block).
            0x510E527FADE682D1,
            0x9B05688C2B3E6C1F,
            0xE07C265404BE4294,
            0x5BE0CD19137E2179,
        ])
    }
};

const ROR24_INDICES: __m128i = const {
    unsafe {
        transmute::<[u8; 16], __m128i>([
            0x03, 0x04, 0x05, 0x06, 0x07, 0x00, 0x01, 0x02, //
            0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x08, 0x09, 0x0A, //
        ])
    }
};
const ROR16_INDICES: __m128i = const {
    unsafe {
        transmute::<[u8; 16], __m128i>([
            0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x00, 0x01, //
            0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x08, 0x09, //
        ])
    }
};

#[target_feature(enable = "avx2")]
pub fn blake2b_f(mut r: usize, h: &mut [u64; 8], m: &[u64; 16], t: &[u64; 2], f: bool) {
    let h = h.as_mut_ptr().cast::<__m256i>();
    let m = unsafe { transmute::<&[u64; 16], &[__m128i; 8]>(m) };
    let t = unsafe { transmute::<&[u64; 2], &__m128i>(t) };

    // Initialize local work vector.
    let mut a = unsafe { _mm256_loadu_si256(h.add(0)) };
    let mut b = unsafe { _mm256_loadu_si256(h.add(1)) };
    let mut c = BLAKE2B_IV[0];
    let mut d = BLAKE2B_IV[1 + f as usize];

    // Apply block counter to local work vector.
    d = _mm256_xor_si256(d, _mm256_zextsi128_si256(*t));

    if r > 0 {
        let ror24 = _mm256_broadcastsi128_si256(ROR24_INDICES);
        let ror16 = _mm256_broadcastsi128_si256(ROR16_INDICES);

        // Preprocess message.
        let m0 = _mm256_broadcastsi128_si256(m[0]);
        let m1 = _mm256_broadcastsi128_si256(m[1]);
        let m2 = _mm256_broadcastsi128_si256(m[2]);
        let m3 = _mm256_broadcastsi128_si256(m[3]);
        let m4 = _mm256_broadcastsi128_si256(m[4]);
        let m5 = _mm256_broadcastsi128_si256(m[5]);
        let m6 = _mm256_broadcastsi128_si256(m[6]);
        let m7 = _mm256_broadcastsi128_si256(m[7]);

        // Process rounds.
        loop {
            macro_rules! impl_round {
                ( $d0:expr, $d1:expr, $d2:expr, $d3:expr $(,)? ) => {
                    let d0: __m256i = $d0;
                    let d1: __m256i = $d1;
                    let d2: __m256i = $d2;
                    let d3: __m256i = $d3;

                    // G(d0)
                    a = _mm256_add_epi64(a, b);
                    a = _mm256_add_epi64(a, d0);
                    d = _mm256_xor_si256(d, a);
                    d = _mm256_shuffle_epi32::<0xB1>(d);
                    c = _mm256_add_epi64(c, d);
                    b = _mm256_xor_si256(b, c);
                    b = _mm256_shuffle_epi8(b, ror24);

                    // G(d1)
                    a = _mm256_add_epi64(a, b);
                    a = _mm256_add_epi64(a, d1);
                    d = _mm256_xor_si256(d, a);
                    d = _mm256_shuffle_epi8(d, ror16);
                    c = _mm256_add_epi64(c, d);
                    b = _mm256_xor_si256(b, c);
                    b = _mm256_or_si256(_mm256_srli_si256::<63>(b), _mm256_slli_si256::<1>(b));

                    // Apply diagonalization.
                    b = _mm256_permute4x64_epi64::<0x39>(b);
                    d = _mm256_permute4x64_epi64::<0x93>(d);
                    c = _mm256_permute2x128_si256::<0x01>(c, c);

                    // G(d2)
                    a = _mm256_add_epi64(a, b);
                    a = _mm256_add_epi64(a, d2);
                    d = _mm256_xor_si256(d, a);
                    d = _mm256_shuffle_epi32::<0xB1>(d);
                    c = _mm256_add_epi64(c, d);
                    b = _mm256_xor_si256(b, c);
                    b = _mm256_shuffle_epi8(b, ror24);

                    // G(d3)
                    a = _mm256_add_epi64(a, b);
                    a = _mm256_add_epi64(a, d3);
                    d = _mm256_xor_si256(d, a);
                    d = _mm256_shuffle_epi8(d, ror16);
                    c = _mm256_add_epi64(c, d);
                    b = _mm256_xor_si256(b, c);
                    b = _mm256_or_si256(_mm256_srli_si256::<63>(b), _mm256_slli_si256::<1>(b));

                    // Revert diagonalization.
                    b = _mm256_permute4x64_epi64::<0x93>(b);
                    d = _mm256_permute4x64_epi64::<0x39>(d);
                    c = _mm256_permute2x128_si256::<0x01>(c, c);

                    r = match r.checked_sub(1) {
                        Some(x) => x,
                        None => break,
                    };
                };
            }

            // Round #0:
            //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
            //   Into: [0 2 4 6 1 3 5 7 E 8 A C F 9 B D]
            impl_round!(
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpacklo_epi64(m0, m1),
                    _mm256_unpacklo_epi64(m2, m3),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpackhi_epi64(m0, m1),
                    _mm256_unpackhi_epi64(m2, m3),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpacklo_epi64(m7, m4),
                    _mm256_unpacklo_epi64(m5, m6),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpackhi_epi64(m7, m4),
                    _mm256_unpackhi_epi64(m5, m6),
                ),
            );

            // Round #1:
            //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
            //   Into: [E 4 9 D A 8 F 6 5 1 0 B 3 C 2 7]
            impl_round!(
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpacklo_epi64(m7, m2),
                    _mm256_unpackhi_epi64(m4, m6),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpacklo_epi64(m5, m4),
                    _mm256_alignr_epi8::<8>(m3, m7),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpackhi_epi64(m2, m0),
                    _mm256_blend_epi32::<0xCC>(m0, m5),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_alignr_epi8::<8>(m6, m1),
                    _mm256_blend_epi32::<0xCC>(m1, m3),
                ),
            );

            // Round #2:
            //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
            //   Into: [B C 5 F 8 0 2 D 9 A 3 7 4 E 6 1]
            impl_round!(
                _mm256_blend_epi32::<0xF0>(
                    _mm256_alignr_epi8::<8>(m6, m5),
                    _mm256_unpackhi_epi64(m2, m7),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpacklo_epi32(m4, m0),
                    _mm256_blend_epi32::<0xCC>(m1, m6),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_alignr_epi8::<8>(m5, m4),
                    _mm256_unpackhi_epi64(m1, m3),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpacklo_epi64(m2, m7),
                    _mm256_blend_epi32::<0xCC>(m3, m0),
                ),
            );

            // Round #3:
            //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
            //   Into: [7 3 D B 9 1 C E F 2 5 4 8 6 A 0]
            impl_round!(
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpackhi_epi64(m3, m1),
                    _mm256_unpackhi_epi64(m6, m5),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpackhi_epi64(m4, m0),
                    _mm256_unpacklo_epi64(m6, m7),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_alignr_epi8::<8>(m1, m7),
                    _mm256_shuffle_epi32::<0x4E>(m2),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpacklo_epi64(m4, m3),
                    _mm256_unpacklo_epi64(m5, m0),
                ),
            );

            // Round #4:
            //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
            //   Into: [9 5 2 A 0 7 4 F 3 E B 6 D 1 C 8]
            impl_round!(
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpackhi_epi64(m4, m2),
                    _mm256_unpacklo_epi64(m1, m5),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_blend_epi32::<0xCC>(m0, m3),
                    _mm256_blend_epi32::<0xCC>(m2, m7),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_alignr_epi8::<8>(m7, m1),
                    _mm256_alignr_epi8::<8>(m3, m5),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpackhi_epi64(m6, m0),
                    _mm256_unpacklo_epi64(m6, m4),
                ),
            );

            // Round #5:
            //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
            //   Into: [2 6 0 8 C A B 3 1 4 7 F 9 D 5 E]
            impl_round!(
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpacklo_epi64(m1, m3),
                    _mm256_unpacklo_epi64(m0, m4),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpacklo_epi64(m6, m5),
                    _mm256_unpackhi_epi64(m5, m1),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_alignr_epi8::<8>(m2, m0),
                    _mm256_unpackhi_epi64(m3, m7),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpackhi_epi64(m4, m6),
                    _mm256_alignr_epi8::<8>(m7, m2),
                ),
            );

            // Round #6:
            //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
            //   Into: [C 1 E 4 5 F D A 8 0 6 9 B 7 3 2]
            impl_round!(
                _mm256_blend_epi32::<0xF0>(
                    _mm256_blend_epi32::<0xCC>(m6, m0),
                    _mm256_unpacklo_epi64(m7, m2),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpackhi_epi64(m2, m7),
                    _mm256_alignr_epi8::<8>(m5, m6),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpacklo_epi64(m4, m0),
                    _mm256_blend_epi32::<0xCC>(m3, m4),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpackhi_epi64(m5, m3),
                    _mm256_shuffle_epi32::<0x4E>(m1),
                ),
            );

            // Round #7:
            //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
            //   Into: [D 7 C 3 B E 1 9 2 5 F 8 A 0 4 6]
            impl_round!(
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpackhi_epi64(m6, m3),
                    _mm256_blend_epi32::<0xCC>(m6, m1),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_alignr_epi8::<8>(m7, m5),
                    _mm256_unpackhi_epi64(m0, m4),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_blend_epi32::<0xCC>(m1, m2),
                    _mm256_alignr_epi8::<8>(m4, m7),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpacklo_epi64(m5, m0),
                    _mm256_unpacklo_epi64(m2, m3),
                ),
            );

            // Round #8:
            //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
            //   Into: [6 E B 0 F 9 3 8 A C D 1 5 2 7 4]
            impl_round!(
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpacklo_epi64(m3, m7),
                    _mm256_alignr_epi8::<8>(m0, m5),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpackhi_epi64(m7, m4),
                    _mm256_alignr_epi8::<8>(m4, m1),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpacklo_epi64(m5, m6),
                    _mm256_unpackhi_epi64(m6, m0),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_alignr_epi8::<8>(m1, m2),
                    _mm256_alignr_epi8::<8>(m2, m3),
                ),
            );

            // Round #9:
            //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
            //   Into: [A 8 7 1 2 4 6 5 D F 9 3 0 B E C]
            impl_round!(
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpacklo_epi64(m5, m4),
                    _mm256_unpackhi_epi64(m3, m0),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpacklo_epi64(m1, m2),
                    _mm256_blend_epi32::<0xC0>(m3, m2),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_unpackhi_epi64(m6, m7),
                    _mm256_unpackhi_epi64(m4, m1),
                ),
                _mm256_blend_epi32::<0xF0>(
                    _mm256_blend_epi32::<0xCC>(m0, m5),
                    _mm256_unpacklo_epi64(m7, m6),
                ),
            );
        }
    }

    // Merge local work vector.
    unsafe {
        let mut t0 = _mm256_xor_si256(a, c);
        let mut t1 = _mm256_xor_si256(b, d);

        asm!(
            "vpxor {t0}, {t0}, [{h} + 0x00]",
            "vpxor {t1}, {t1}, [{h} + 0x20]",
            t0 = inout(ymm_reg) t0,
            t1 = inout(ymm_reg) t1,
            h = in(reg) h,
        );

        _mm256_storeu_si256(h.add(0), t0);
        _mm256_storeu_si256(h.add(1), t1);
    };
}
