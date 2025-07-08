use std::arch::x86_64::*;

#[repr(align(16))]
struct AlignTo16<T>(T);

/// SIGMA needs to be aligned to at least 16 bytes
const SIGMA: [AlignTo16<[u32; 16]>; 10] = [
    AlignTo16([0, 2, 4, 6, 1, 3, 5, 7, 14, 8, 10, 12, 15, 9, 11, 13]),
    AlignTo16([14, 4, 9, 13, 10, 8, 15, 6, 5, 1, 0, 11, 3, 12, 2, 7]),
    AlignTo16([11, 12, 5, 15, 8, 0, 2, 13, 9, 10, 3, 7, 4, 14, 6, 1]),
    AlignTo16([7, 3, 13, 11, 9, 1, 12, 14, 15, 2, 5, 4, 8, 6, 10, 0]),
    AlignTo16([9, 5, 2, 10, 0, 7, 4, 15, 3, 14, 11, 6, 13, 1, 12, 8]),
    AlignTo16([2, 6, 0, 8, 12, 10, 11, 3, 1, 4, 7, 15, 9, 13, 5, 14]),
    AlignTo16([12, 1, 14, 4, 5, 15, 13, 10, 8, 0, 6, 9, 11, 7, 3, 2]),
    AlignTo16([13, 7, 12, 3, 11, 14, 1, 9, 2, 5, 15, 8, 10, 0, 4, 6]),
    AlignTo16([6, 14, 11, 0, 15, 9, 3, 8, 10, 12, 13, 1, 5, 2, 7, 4]),
    AlignTo16([10, 8, 7, 1, 2, 4, 6, 5, 13, 15, 9, 3, 0, 11, 14, 12]),
];

#[repr(align(32))]
struct AlignTo32<T>(T);

/// IV needs to be aligned to 32 bytes
const IV: AlignTo32<[u64; 8]> = AlignTo32([
    0x6a09e667f3bcc908,
    0xbb67ae8584caa73b,
    0x3c6ef372fe94f82b,
    0xa54ff53a5f1d36f1,
    0x510e527fade682d1,
    0x9b05688c2b3e6c1f,
    0x1f83d9abfb41bd6b,
    0x5be0cd19137e2179,
]);

// https://github.com/sneves/blake2-avx2/blob/master/blake2b.c#L64
#[inline(always)]
unsafe fn word_permutation(
    rounds_to_permute: usize,
    mut a: __m256i,
    mut b: __m256i,
    mut c: __m256i,
    mut d: __m256i,
    m: &[u64; 16],
) -> (__m256i, __m256i) {
    unsafe {
        let rotate24 = _mm256_setr_epi8(
            3, 4, 5, 6, 7, 0, 1, 2, 11, 12, 13, 14, 15, 8, 9, 10, 3, 4, 5, 6, 7, 0, 1, 2, 11, 12,
            13, 14, 15, 8, 9, 10,
        );
        let rotate16 = _mm256_setr_epi8(
            2, 3, 4, 5, 6, 7, 0, 1, 10, 11, 12, 13, 14, 15, 8, 9, 2, 3, 4, 5, 6, 7, 0, 1, 10, 11,
            12, 13, 14, 15, 8, 9,
        );
        for round in 0..rounds_to_permute {
            let subsigma = SIGMA[round % 10].0.as_ptr();

            let b0 = _mm256_i32gather_epi64::<8>(
                m.as_ptr() as *const i64,
                _mm_load_si128(subsigma as *const __m128i),
            );
            // G1
            a = _mm256_add_epi64(a, b0);

            a = _mm256_add_epi64(a, b);
            d = _mm256_xor_si256(d, a);
            d = _mm256_shuffle_epi32(d, 177); // 177 = _MM_SHUFFLE(2, 3, 0, 1)

            c = _mm256_add_epi64(c, d);
            b = _mm256_xor_si256(b, c);
            b = _mm256_shuffle_epi8(b, rotate24);
            //
            let b0 = _mm256_i32gather_epi64::<8>(
                m.as_ptr() as *const i64,
                _mm_load_si128(subsigma.add(4) as *const __m128i),
            );
            // G2
            a = _mm256_add_epi64(a, b0);

            a = _mm256_add_epi64(a, b);
            d = _mm256_xor_si256(d, a);
            d = _mm256_shuffle_epi8(d, rotate16);

            c = _mm256_add_epi64(c, d);
            b = _mm256_xor_si256(b, c);
            b = _mm256_or_si256(_mm256_srli_epi64::<63>(b), _mm256_add_epi64(b, b)); // ROT63
            // DIAG
            a = _mm256_permute4x64_epi64::<147>(a); // 147 = _MM_SHUFFLE(2,1,0,3)
            d = _mm256_permute4x64_epi64::<78>(d); // 78 = _MM_SHUFFLE(1,0,3,2)
            c = _mm256_permute4x64_epi64::<57>(c); // 57 = _MM_SHUFFLE(0,3,2,1)
            //
            let b0 = _mm256_i32gather_epi64::<8>(
                m.as_ptr() as *const i64,
                _mm_load_si128(subsigma.add(8) as *const __m128i),
            );
            // G1
            a = _mm256_add_epi64(a, b0);

            a = _mm256_add_epi64(a, b);
            d = _mm256_xor_si256(d, a);
            d = _mm256_shuffle_epi32(d, 177); // 177 = _MM_SHUFFLE(2, 3, 0, 1)

            c = _mm256_add_epi64(c, d);
            b = _mm256_xor_si256(b, c);
            b = _mm256_shuffle_epi8(b, rotate24);
            //
            let b0 = _mm256_i32gather_epi64::<8>(
                m.as_ptr() as *const i64,
                _mm_load_si128(subsigma.add(12) as *const __m128i),
            );
            // G2
            a = _mm256_add_epi64(a, b0);

            a = _mm256_add_epi64(a, b);
            d = _mm256_xor_si256(d, a);
            d = _mm256_shuffle_epi8(d, rotate16);

            c = _mm256_add_epi64(c, d);
            b = _mm256_xor_si256(b, c);
            b = _mm256_or_si256(_mm256_srli_epi64::<63>(b), _mm256_add_epi64(b, b)); // ROT63
            // UNDIAG
            a = _mm256_permute4x64_epi64::<57>(a); // 57 = _MM_SHUFFLE(0,3,2,1)
            d = _mm256_permute4x64_epi64::<78>(d); // 78 = _MM_SHUFFLE(1,0,3,2)
            c = _mm256_permute4x64_epi64::<147>(c); // 147 = _MM_SHUFFLE(2,1,0,3)
        }
        (_mm256_xor_si256(a, c), _mm256_xor_si256(b, d))
    }
}

/// Based on https://datatracker.ietf.org/doc/html/rfc7693#section-3.2
#[target_feature(enable = "avx2")]
pub fn blake2f_compress_f_inner(
    rounds: usize, // Specifies the rounds to permute
    h: &[u64; 8],  // State vector, defines the work vector (v) and affects the XOR process
    m: &[u64; 16], // The message block to compress
    t: &[u64; 2],  // Affects the work vector (v) before permutations
    f: bool,       // If set as true, inverts all bits
) -> [u64; 8] {
    unsafe {
        let a = _mm256_setr_epi64x(h[0] as i64, h[1] as i64, h[2] as i64, h[3] as i64);
        let b = _mm256_setr_epi64x(h[4] as i64, h[5] as i64, h[6] as i64, h[7] as i64);

        let c = _mm256_load_si256(IV.0.as_ptr() as *const __m256i);
        let d = _mm256_xor_si256(
            _mm256_load_si256(IV.0.as_ptr().add(4) as *const __m256i),
            _mm256_setr_epi64x(t[0] as i64, t[1] as i64, -(f as i64), 0),
        );

        let (out0, out1) = word_permutation(rounds, a, b, c, d, &m);

        let mut output = [0; 8];

        _mm256_store_si256(
            output.as_mut_ptr() as *mut __m256i,
            _mm256_xor_si256(out0, a),
        );
        _mm256_store_si256(
            output.as_mut_ptr().add(4) as *mut __m256i,
            _mm256_xor_si256(out1, b),
        );

        output
    }
}
