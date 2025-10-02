use std::arch::aarch64::*;

const BLAKE2B_IV: [u64; 12] = [
    0x6A09E667F3BCC908,
    0xBB67AE8584CAA73B,
    0x3C6EF372FE94F82B,
    0xA54FF53A5F1D36F1,
    0x510E527FADE682D1,
    0x9B05688C2B3E6C1F,
    0x1F83D9ABFB41BD6B,
    0x5BE0CD19137E2179,
    // Second half of blake2b_iv with inverted bits (for final block).
    0x510E527FADE682D1,
    0x9B05688C2B3E6C1F,
    0xE07C265404BE4294,
    0x5BE0CD19137E2179,
];

pub fn blake2b_f(r: usize, h: &mut [u64; 8], m: &[u64; 16], t: &[u64; 2], f: bool) {
    unsafe {
        // Initialize local work vector.
        let uint64x2x4_t(h0, h1, h2, h3) = vld1q_u64_x4(h.as_ptr().cast::<u64>().add(0));
        let mut a = uint64x2x2_t(h0, h1);
        let mut b = uint64x2x2_t(h2, h3);
        let mut c = vld1q_u64_x2(BLAKE2B_IV.as_ptr());
        let mut d = vld1q_u64_x2(BLAKE2B_IV.as_ptr().add(4 + ((f as usize) << 2)));

        // Apply block number to local work vector.
        d.0 = veorq_u64(d.0, vld1q_u64(t.as_ptr()));

        if let Some(mut r) = r.checked_sub(1) {
            let uint64x2x4_t(m0, m1, m2, m3) = vld1q_u64_x4(m.as_ptr().add(0));
            let uint64x2x4_t(m4, m5, m6, m7) = vld1q_u64_x4(m.as_ptr().add(8));

            'process: {
                // Round #0:
                //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
                //   Into: [0 2 4 6 1 3 5 7 E 8 A C F 9 B D]
                let r0a = uint64x2x2_t(vtrn1q_u64(m0, m1), vtrn1q_u64(m2, m3));
                let r0b = uint64x2x2_t(vtrn2q_u64(m0, m1), vtrn2q_u64(m2, m3));
                let r0c = uint64x2x2_t(vtrn1q_u64(m7, m4), vtrn1q_u64(m5, m6));
                let r0d = uint64x2x2_t(vtrn2q_u64(m7, m4), vtrn2q_u64(m5, m6));
                inner(&mut a, &mut b, &mut c, &mut d, r0a, r0b, r0c, r0d);
                r = match r.checked_sub(1) {
                    Some(x) => x,
                    None => break 'process,
                };

                // Round #1:
                //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
                //   Into: [E 4 9 D A 8 F 6 5 1 0 B 3 C 2 7]
                let r1a = uint64x2x2_t(vtrn1q_u64(m7, m2), vtrn2q_u64(m4, m6));
                let r1b = uint64x2x2_t(vtrn1q_u64(m5, m4), vextq_u64::<1>(m7, m3));
                let r1c = uint64x2x2_t(vtrn2q_u64(m2, m0), vcopyq_laneq_u64::<1, 1>(m0, m5));
                let r1d = uint64x2x2_t(vextq_u64::<1>(m1, m6), vcopyq_laneq_u64::<1, 1>(m1, m3));
                inner(&mut a, &mut b, &mut c, &mut d, r1a, r1b, r1c, r1d);
                r = match r.checked_sub(1) {
                    Some(x) => x,
                    None => break 'process,
                };

                // Round #2:
                //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
                //   Into: [B C 5 F 8 0 2 D 9 A 3 7 4 E 6 1]
                let r2a = uint64x2x2_t(vextq_u64::<1>(m5, m6), vtrn2q_u64(m2, m7));
                let r2b = uint64x2x2_t(vtrn1q_u64(m4, m0), vcopyq_laneq_u64::<1, 1>(m1, m6));
                let r2c = uint64x2x2_t(vextq_u64::<1>(m4, m5), vtrn2q_u64(m1, m3));
                let r2d = uint64x2x2_t(vtrn1q_u64(m2, m7), vcopyq_laneq_u64::<1, 1>(m3, m0));
                inner(&mut a, &mut b, &mut c, &mut d, r2a, r2b, r2c, r2d);
                r = match r.checked_sub(1) {
                    Some(x) => x,
                    None => break 'process,
                };

                // Round #3:
                //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
                //   Into: [7 3 D B 9 1 C E F 2 5 4 8 6 A 0]
                let r3a = uint64x2x2_t(vtrn2q_u64(m3, m1), vtrn2q_u64(m6, m5));
                let r3b = uint64x2x2_t(vtrn2q_u64(m4, m0), vtrn1q_u64(m6, m7));
                let r3c = uint64x2x2_t(vextq_u64::<1>(m7, m1), vextq_u64::<1>(m2, m2));
                let r3d = uint64x2x2_t(vtrn1q_u64(m4, m3), vtrn1q_u64(m5, m0));
                inner(&mut a, &mut b, &mut c, &mut d, r3a, r3b, r3c, r3d);
                r = match r.checked_sub(1) {
                    Some(x) => x,
                    None => break 'process,
                };

                // Round #4:
                //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
                //   Into: [9 5 2 A 0 7 4 F 3 E B 6 D 1 C 8]
                let r4a = uint64x2x2_t(vtrn2q_u64(m4, m2), vtrn1q_u64(m1, m5));
                let r4b = uint64x2x2_t(
                    vcopyq_laneq_u64::<1, 1>(m0, m3),
                    vcopyq_laneq_u64::<1, 1>(m2, m7),
                );
                let r4c = uint64x2x2_t(vextq_u64::<1>(m1, m7), vextq_u64::<1>(m5, m3));
                let r4d = uint64x2x2_t(vtrn2q_u64(m6, m0), vtrn1q_u64(m6, m4));
                inner(&mut a, &mut b, &mut c, &mut d, r4a, r4b, r4c, r4d);
                r = match r.checked_sub(1) {
                    Some(x) => x,
                    None => break 'process,
                };

                // Round #5:
                //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
                //   Into: [2 6 0 8 C A B 3 1 4 7 F 9 D 5 E]
                let r5a = uint64x2x2_t(vtrn1q_u64(m1, m3), vtrn1q_u64(m0, m4));
                let r5b = uint64x2x2_t(vtrn1q_u64(m6, m5), vtrn2q_u64(m5, m1));
                let r5c = uint64x2x2_t(vextq_u64::<1>(m0, m2), vtrn2q_u64(m3, m7));
                let r5d = uint64x2x2_t(vtrn2q_u64(m4, m6), vextq_u64::<1>(m2, m7));
                inner(&mut a, &mut b, &mut c, &mut d, r5a, r5b, r5c, r5d);
                r = match r.checked_sub(1) {
                    Some(x) => x,
                    None => break 'process,
                };

                // Round #6:
                //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
                //   Into: [C 1 E 4 5 F D A 8 0 6 9 B 7 3 2]
                let r6a = uint64x2x2_t(vcopyq_laneq_u64::<1, 1>(m6, m0), vtrn1q_u64(m7, m2));
                let r6b = uint64x2x2_t(vtrn2q_u64(m2, m7), vextq_u64::<1>(m6, m5));
                let r6c = uint64x2x2_t(vtrn1q_u64(m4, m0), vcopyq_laneq_u64::<1, 1>(m3, m4));
                let r6d = uint64x2x2_t(vtrn2q_u64(m5, m3), vextq_u64::<1>(m1, m1));
                inner(&mut a, &mut b, &mut c, &mut d, r6a, r6b, r6c, r6d);
                r = match r.checked_sub(1) {
                    Some(x) => x,
                    None => break 'process,
                };

                // Round #7:
                //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
                //   Into: [D 7 C 3 B E 1 9 2 5 F 8 A 0 4 6]
                let r7a = uint64x2x2_t(vtrn2q_u64(m6, m3), vcopyq_laneq_u64::<1, 1>(m6, m1));
                let r7b = uint64x2x2_t(vextq_u64::<1>(m5, m7), vtrn2q_u64(m0, m4));
                let r7c = uint64x2x2_t(vcopyq_laneq_u64::<1, 1>(m1, m2), vextq_u64::<1>(m7, m4));
                let r7d = uint64x2x2_t(vtrn1q_u64(m5, m0), vtrn1q_u64(m2, m3));
                inner(&mut a, &mut b, &mut c, &mut d, r7a, r7b, r7c, r7d);
                r = match r.checked_sub(1) {
                    Some(x) => x,
                    None => break 'process,
                };

                // Round #8:
                //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
                //   Into: [6 E B 0 F 9 3 8 A C D 1 5 2 7 4]
                let r8a = uint64x2x2_t(vtrn1q_u64(m3, m7), vextq_u64::<1>(m5, m0));
                let r8b = uint64x2x2_t(vtrn2q_u64(m7, m4), vextq_u64::<1>(m1, m4));
                let r8c = uint64x2x2_t(vtrn1q_u64(m5, m6), vtrn2q_u64(m6, m0));
                let r8d = uint64x2x2_t(vextq_u64::<1>(m2, m1), vextq_u64::<1>(m3, m2));
                inner(&mut a, &mut b, &mut c, &mut d, r8a, r8b, r8c, r8d);
                r = match r.checked_sub(1) {
                    Some(x) => x,
                    None => break 'process,
                };

                // Round #9:
                //   From: [0 1 2 3 4 5 6 7 8 9 A B C D E F]
                //   Into: [A 8 7 1 2 4 6 5 D F 9 3 0 B E C]
                let r9a = uint64x2x2_t(vtrn1q_u64(m5, m4), vtrn2q_u64(m3, m0));
                let r9b = uint64x2x2_t(vtrn1q_u64(m1, m2), vcopyq_laneq_u64::<1, 1>(m3, m2));
                let r9c = uint64x2x2_t(vtrn2q_u64(m6, m7), vtrn2q_u64(m4, m1));
                let r9d = uint64x2x2_t(vcopyq_laneq_u64::<1, 1>(m0, m5), vtrn1q_u64(m7, m6));
                inner(&mut a, &mut b, &mut c, &mut d, r9a, r9b, r9c, r9d);
                r = match r.checked_sub(1) {
                    Some(x) => x,
                    None => break 'process,
                };

                loop {
                    inner(&mut a, &mut b, &mut c, &mut d, r0a, r0b, r0c, r0d);
                    r = match r.checked_sub(1) {
                        Some(x) => x,
                        None => break 'process,
                    };

                    inner(&mut a, &mut b, &mut c, &mut d, r1a, r1b, r1c, r1d);
                    r = match r.checked_sub(1) {
                        Some(x) => x,
                        None => break 'process,
                    };

                    inner(&mut a, &mut b, &mut c, &mut d, r2a, r2b, r2c, r2d);
                    r = match r.checked_sub(1) {
                        Some(x) => x,
                        None => break 'process,
                    };

                    inner(&mut a, &mut b, &mut c, &mut d, r3a, r3b, r3c, r3d);
                    r = match r.checked_sub(1) {
                        Some(x) => x,
                        None => break 'process,
                    };

                    inner(&mut a, &mut b, &mut c, &mut d, r4a, r4b, r4c, r4d);
                    r = match r.checked_sub(1) {
                        Some(x) => x,
                        None => break 'process,
                    };

                    inner(&mut a, &mut b, &mut c, &mut d, r5a, r5b, r5c, r5d);
                    r = match r.checked_sub(1) {
                        Some(x) => x,
                        None => break 'process,
                    };

                    inner(&mut a, &mut b, &mut c, &mut d, r6a, r6b, r6c, r6d);
                    r = match r.checked_sub(1) {
                        Some(x) => x,
                        None => break 'process,
                    };

                    inner(&mut a, &mut b, &mut c, &mut d, r7a, r7b, r7c, r7d);
                    r = match r.checked_sub(1) {
                        Some(x) => x,
                        None => break 'process,
                    };

                    inner(&mut a, &mut b, &mut c, &mut d, r8a, r8b, r8c, r8d);
                    r = match r.checked_sub(1) {
                        Some(x) => x,
                        None => break 'process,
                    };

                    inner(&mut a, &mut b, &mut c, &mut d, r9a, r9b, r9c, r9d);
                    r = match r.checked_sub(1) {
                        Some(x) => x,
                        None => break 'process,
                    };
                }
            }
        }

        // Merge local work vector.
        vst1q_u64_x2(
            h.as_mut_ptr().add(0),
            uint64x2x2_t(veor3q_u64(h0, a.0, c.0), veor3q_u64(h1, a.1, c.1)),
        );
        vst1q_u64_x2(
            h.as_mut_ptr().add(4),
            uint64x2x2_t(veor3q_u64(h2, b.0, d.0), veor3q_u64(h3, b.1, d.1)),
        );
    }
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn inner(
    a: &mut uint64x2x2_t,
    b: &mut uint64x2x2_t,
    c: &mut uint64x2x2_t,
    d: &mut uint64x2x2_t,
    d0: uint64x2x2_t,
    d1: uint64x2x2_t,
    d2: uint64x2x2_t,
    d3: uint64x2x2_t,
) {
    unsafe {
        // G(d0)
        *a = uint64x2x2_t(vaddq_u64(a.0, b.0), vaddq_u64(a.1, b.1));
        *a = uint64x2x2_t(vaddq_u64(a.0, d0.0), vaddq_u64(a.1, d0.1));
        *d = uint64x2x2_t(vxarq_u64::<32>(d.0, a.0), vxarq_u64::<32>(d.1, a.1));
        *c = uint64x2x2_t(vaddq_u64(c.0, d.0), vaddq_u64(c.1, d.1));
        *b = uint64x2x2_t(vxarq_u64::<24>(b.0, c.0), vxarq_u64::<24>(b.1, c.1));

        // G(d1)
        *a = uint64x2x2_t(vaddq_u64(a.0, b.0), vaddq_u64(a.1, b.1));
        *a = uint64x2x2_t(vaddq_u64(a.0, d1.0), vaddq_u64(a.1, d1.1));
        *d = uint64x2x2_t(vxarq_u64::<16>(d.0, a.0), vxarq_u64::<16>(d.1, a.1));
        *c = uint64x2x2_t(vaddq_u64(c.0, d.0), vaddq_u64(c.1, d.1));
        *b = uint64x2x2_t(vxarq_u64::<63>(b.0, c.0), vxarq_u64::<63>(b.1, c.1));

        // Apply diagonalization.
        *a = uint64x2x2_t(vextq_u64::<1>(a.1, a.0), vextq_u64::<1>(a.0, a.1));
        *c = uint64x2x2_t(vextq_u64::<1>(c.0, c.1), vextq_u64::<1>(c.1, c.0));
        *d = uint64x2x2_t(d.1, d.0);

        // G(d2)
        *a = uint64x2x2_t(vaddq_u64(a.0, b.0), vaddq_u64(a.1, b.1));
        *a = uint64x2x2_t(vaddq_u64(a.0, d2.0), vaddq_u64(a.1, d2.1));
        *d = uint64x2x2_t(vxarq_u64::<32>(d.0, a.0), vxarq_u64::<32>(d.1, a.1));
        *c = uint64x2x2_t(vaddq_u64(c.0, d.0), vaddq_u64(c.1, d.1));
        *b = uint64x2x2_t(vxarq_u64::<24>(b.0, c.0), vxarq_u64::<24>(b.1, c.1));

        // G(d3)
        *a = uint64x2x2_t(vaddq_u64(a.0, b.0), vaddq_u64(a.1, b.1));
        *a = uint64x2x2_t(vaddq_u64(a.0, d3.0), vaddq_u64(a.1, d3.1));
        *d = uint64x2x2_t(vxarq_u64::<16>(d.0, a.0), vxarq_u64::<16>(d.1, a.1));
        *c = uint64x2x2_t(vaddq_u64(c.0, d.0), vaddq_u64(c.1, d.1));
        *b = uint64x2x2_t(vxarq_u64::<63>(b.0, c.0), vxarq_u64::<63>(b.1, c.1));

        // Revert diagonalization.
        *a = uint64x2x2_t(vextq_u64::<1>(a.0, a.1), vextq_u64::<1>(a.1, a.0));
        *c = uint64x2x2_t(vextq_u64::<1>(c.1, c.0), vextq_u64::<1>(c.0, c.1));
        *d = uint64x2x2_t(d.1, d.0);
    }
}
