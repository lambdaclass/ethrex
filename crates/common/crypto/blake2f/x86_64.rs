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
    let m = m.as_ptr().cast::<__m128i>();
    let t = t.as_ptr().cast::<__m128i>();

    _mm256_zeroall();

    // Initialize local work vector.
    let mut a = unsafe { _mm256_loadu_si256(h.add(0)) };
    let mut b = unsafe { _mm256_loadu_si256(h.add(1)) };
    let mut c = BLAKE2B_IV[0];
    let mut d = BLAKE2B_IV[1 + f as usize];

    // Apply block counter to local work vector.
    d = _mm256_xor_si256(d, _mm256_zextsi128_si256(unsafe { _mm_loadu_si128(t) }));

    if r > 0 {
        let ror24 = _mm256_broadcastsi128_si256(ROR24_INDICES);
        let ror16 = _mm256_broadcastsi128_si256(ROR16_INDICES);

        // Preprocess message.
        let m0 = _mm256_broadcastsi128_si256(unsafe { _mm_loadu_si128(m.add(0)) });
        let m1 = _mm256_broadcastsi128_si256(unsafe { _mm_loadu_si128(m.add(1)) });
        let m2 = _mm256_broadcastsi128_si256(unsafe { _mm_loadu_si128(m.add(2)) });
        let m3 = _mm256_broadcastsi128_si256(unsafe { _mm_loadu_si128(m.add(3)) });
        let m4 = _mm256_broadcastsi128_si256(unsafe { _mm_loadu_si128(m.add(4)) });
        let m5 = _mm256_broadcastsi128_si256(unsafe { _mm_loadu_si128(m.add(5)) });
        let m6 = _mm256_broadcastsi128_si256(unsafe { _mm_loadu_si128(m.add(6)) });
        let m7 = _mm256_broadcastsi128_si256(unsafe { _mm_loadu_si128(m.add(7)) });

        // Round #0:
        //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
        //   Into: [0 2 4 6 1 3 5 7 E 8 A C F 9 B D]
        let r0a = _mm256_blend_epi32::<0xF0>(
            _mm256_unpacklo_epi64(m0, m1),
            _mm256_unpacklo_epi64(m2, m3),
        );
        let r0b = _mm256_blend_epi32::<0xF0>(
            _mm256_unpackhi_epi64(m0, m1),
            _mm256_unpackhi_epi64(m2, m3),
        );
        let r0c = _mm256_blend_epi32::<0xF0>(
            _mm256_unpacklo_epi64(m7, m4),
            _mm256_unpacklo_epi64(m5, m6),
        );
        let r0d = _mm256_blend_epi32::<0xF0>(
            _mm256_unpackhi_epi64(m7, m4),
            _mm256_unpackhi_epi64(m5, m6),
        );

        // Round #1:
        //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
        //   Into: [E 4 9 D A 8 F 6 5 1 0 B 3 C 2 7]
        let r1a = _mm256_blend_epi32::<0xF0>(
            _mm256_unpacklo_epi64(m7, m2),
            _mm256_unpackhi_epi64(m4, m6),
        );
        let r1b = _mm256_blend_epi32::<0xF0>(
            _mm256_unpacklo_epi64(m5, m4),
            _mm256_alignr_epi8::<8>(m3, m7),
        );
        let r1c = _mm256_blend_epi32::<0xF0>(
            _mm256_unpackhi_epi64(m2, m0),
            _mm256_blend_epi32::<0xCC>(m0, m5),
        );
        let r1d = _mm256_blend_epi32::<0xF0>(
            _mm256_alignr_epi8::<8>(m6, m1),
            _mm256_blend_epi32::<0xCC>(m1, m3),
        );

        // Round #2:
        //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
        //   Into: [B C 5 F 8 0 2 D 9 A 3 7 4 E 6 1]
        let r2a = _mm256_blend_epi32::<0xF0>(
            _mm256_alignr_epi8::<8>(m6, m5),
            _mm256_unpackhi_epi64(m2, m7),
        );
        let r2b = _mm256_blend_epi32::<0xF0>(
            _mm256_unpacklo_epi32(m4, m0),
            _mm256_blend_epi32::<0xCC>(m1, m6),
        );
        let r2c = _mm256_blend_epi32::<0xF0>(
            _mm256_alignr_epi8::<8>(m5, m4),
            _mm256_unpackhi_epi64(m1, m3),
        );
        let r2d = _mm256_blend_epi32::<0xF0>(
            _mm256_unpacklo_epi64(m2, m7),
            _mm256_blend_epi32::<0xCC>(m3, m0),
        );

        // Round #3:
        //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
        //   Into: [7 3 D B 9 1 C E F 2 5 4 8 6 A 0]
        let r3a = _mm256_blend_epi32::<0xF0>(
            _mm256_unpackhi_epi64(m3, m1),
            _mm256_unpackhi_epi64(m6, m5),
        );
        let r3b = _mm256_blend_epi32::<0xF0>(
            _mm256_unpackhi_epi64(m4, m0),
            _mm256_unpacklo_epi64(m6, m7),
        );
        let r3c = _mm256_blend_epi32::<0xF0>(
            _mm256_alignr_epi8::<8>(m1, m7),
            _mm256_shuffle_epi32::<0x4E>(m2),
        );
        let r3d = _mm256_blend_epi32::<0xF0>(
            _mm256_unpacklo_epi64(m4, m3),
            _mm256_unpacklo_epi64(m5, m0),
        );

        // Round #4:
        //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
        //   Into: [9 5 2 A 0 7 4 F 3 E B 6 D 1 C 8]
        let r4a = _mm256_blend_epi32::<0xF0>(
            _mm256_unpackhi_epi64(m4, m2),
            _mm256_unpacklo_epi64(m1, m5),
        );
        let r4b = _mm256_blend_epi32::<0xF0>(
            _mm256_blend_epi32::<0xCC>(m0, m3),
            _mm256_blend_epi32::<0xCC>(m2, m7),
        );
        let r4c = _mm256_blend_epi32::<0xF0>(
            _mm256_alignr_epi8::<8>(m7, m1),
            _mm256_alignr_epi8::<8>(m3, m5),
        );
        let r4d = _mm256_blend_epi32::<0xF0>(
            _mm256_unpackhi_epi64(m6, m0),
            _mm256_unpacklo_epi64(m6, m4),
        );

        // Round #5:
        //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
        //   Into: [2 6 0 8 C A B 3 1 4 7 F 9 D 5 E]
        let r5a = _mm256_blend_epi32::<0xF0>(
            _mm256_unpacklo_epi64(m1, m3),
            _mm256_unpacklo_epi64(m0, m4),
        );
        let r5b = _mm256_blend_epi32::<0xF0>(
            _mm256_unpacklo_epi64(m6, m5),
            _mm256_unpackhi_epi64(m5, m1),
        );
        let r5c = _mm256_blend_epi32::<0xF0>(
            _mm256_alignr_epi8::<8>(m2, m0),
            _mm256_unpackhi_epi64(m3, m7),
        );
        let r5d = _mm256_blend_epi32::<0xF0>(
            _mm256_unpackhi_epi64(m4, m6),
            _mm256_alignr_epi8::<8>(m7, m2),
        );

        // Round #6:
        //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
        //   Into: [C 1 E 4 5 F D A 8 0 6 9 B 7 3 2]
        let r6a = _mm256_blend_epi32::<0xF0>(
            _mm256_blend_epi32::<0xCC>(m6, m0),
            _mm256_unpacklo_epi64(m7, m2),
        );
        let r6b = _mm256_blend_epi32::<0xF0>(
            _mm256_unpackhi_epi64(m2, m7),
            _mm256_alignr_epi8::<8>(m5, m6),
        );
        let r6c = _mm256_blend_epi32::<0xF0>(
            _mm256_unpacklo_epi64(m4, m0),
            _mm256_blend_epi32::<0xCC>(m3, m4),
        );
        let r6d = _mm256_blend_epi32::<0xF0>(
            _mm256_unpackhi_epi64(m5, m3),
            _mm256_shuffle_epi32::<0x4E>(m1),
        );

        // Round #7:
        //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
        //   Into: [D 7 C 3 B E 1 9 2 5 F 8 A 0 4 6]
        let r7a = _mm256_blend_epi32::<0xF0>(
            _mm256_unpackhi_epi64(m6, m3),
            _mm256_blend_epi32::<0xCC>(m6, m1),
        );
        let r7b = _mm256_blend_epi32::<0xF0>(
            _mm256_alignr_epi8::<8>(m7, m5),
            _mm256_unpackhi_epi64(m0, m4),
        );
        let r7c = _mm256_blend_epi32::<0xF0>(
            _mm256_blend_epi32::<0xCC>(m1, m2),
            _mm256_alignr_epi8::<8>(m4, m7),
        );
        let r7d = _mm256_blend_epi32::<0xF0>(
            _mm256_unpacklo_epi64(m5, m0),
            _mm256_unpacklo_epi64(m2, m3),
        );

        // Round #8:
        //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
        //   Into: [6 E B 0 F 9 3 8 A C D 1 5 2 7 4]
        let r8a = _mm256_blend_epi32::<0xF0>(
            _mm256_unpacklo_epi64(m3, m7),
            _mm256_alignr_epi8::<8>(m0, m5),
        );
        let r8b = _mm256_blend_epi32::<0xF0>(
            _mm256_unpackhi_epi64(m7, m4),
            _mm256_alignr_epi8::<8>(m4, m1),
        );
        let r8c = _mm256_blend_epi32::<0xF0>(
            _mm256_unpacklo_epi64(m5, m6),
            _mm256_unpackhi_epi64(m6, m0),
        );
        let r8d = _mm256_blend_epi32::<0xF0>(
            _mm256_alignr_epi8::<8>(m1, m2),
            _mm256_alignr_epi8::<8>(m2, m3),
        );

        // Round #9:
        //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
        //   Into: [A 8 7 1 2 4 6 5 D F 9 3 0 B E C]
        let r9a = _mm256_blend_epi32::<0xF0>(
            _mm256_unpacklo_epi64(m5, m4),
            _mm256_unpackhi_epi64(m3, m0),
        );
        let r9b = _mm256_blend_epi32::<0xF0>(
            _mm256_unpacklo_epi64(m1, m2),
            _mm256_blend_epi32::<0xC0>(m3, m2),
        );
        let r9c = _mm256_blend_epi32::<0xF0>(
            _mm256_unpackhi_epi64(m6, m7),
            _mm256_unpackhi_epi64(m4, m1),
        );
        let r9d = _mm256_blend_epi32::<0xF0>(
            _mm256_blend_epi32::<0xCC>(m0, m5),
            _mm256_unpacklo_epi64(m7, m6),
        );

        // Process rounds.
        loop {
            macro_rules! impl_round {
                ( $d0:expr, $d1:expr, $d2:expr, $d3:expr $(,)? ) => {
                    // G(d0)
                    a = _mm256_add_epi64(a, b);
                    a = _mm256_add_epi64(a, $d0);
                    d = _mm256_xor_si256(d, a);
                    d = _mm256_shuffle_epi32::<0xB1>(d);
                    c = _mm256_add_epi64(c, d);
                    b = _mm256_xor_si256(b, c);
                    b = _mm256_shuffle_epi8(b, ror24);

                    // G(d1)
                    a = _mm256_add_epi64(a, b);
                    a = _mm256_add_epi64(a, $d1);
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
                    a = _mm256_add_epi64(a, $d2);
                    d = _mm256_xor_si256(d, a);
                    d = _mm256_shuffle_epi32::<0xB1>(d);
                    c = _mm256_add_epi64(c, d);
                    b = _mm256_xor_si256(b, c);
                    b = _mm256_shuffle_epi8(b, ror24);

                    // G(d3)
                    a = _mm256_add_epi64(a, b);
                    a = _mm256_add_epi64(a, $d3);
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

            impl_round!(r0a, r0b, r0c, r0d);
            impl_round!(r1a, r1b, r1c, r1d);
            impl_round!(r1a, r2b, r2c, r2d);
            impl_round!(r3a, r3b, r3c, r3d);
            impl_round!(r4a, r4b, r4c, r4d);
            impl_round!(r5a, r5b, r5c, r5d);
            impl_round!(r6a, r6b, r6c, r6d);
            impl_round!(r7a, r7b, r7c, r7d);
            impl_round!(r8a, r8b, r8c, r8d);
            impl_round!(r9a, r9b, r9c, r9d);
        }
    }

    // Merge local work vector.
    unsafe {
        let h0 = _mm256_loadu_si256(h.add(0));
        let h1 = _mm256_loadu_si256(h.add(1));

        let t0 = _mm256_xor_si256(a, c);
        let t1 = _mm256_xor_si256(b, d);

        let h0 = _mm256_xor_si256(t0, h0);
        let h1 = _mm256_xor_si256(t1, h1);

        _mm256_storeu_si256(h.add(0), h0);
        _mm256_storeu_si256(h.add(1), h1);
    };
}
