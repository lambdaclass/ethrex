// Message word schedule permutations for each round are defined by SIGMA constant.
// Extracted from https://datatracker.ietf.org/doc/html/rfc7693#section-2.7
pub const SIGMA: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

// Initialization vector, used to initialize the work vector
// Extracted from https://datatracker.ietf.org/doc/html/rfc7693#appendix-C.2
pub const IV: [u64; 8] = [
    0x6a09e667f3bcc908,
    0xbb67ae8584caa73b,
    0x3c6ef372fe94f82b,
    0xa54ff53a5f1d36f1,
    0x510e527fade682d1,
    0x9b05688c2b3e6c1f,
    0x1f83d9abfb41bd6b,
    0x5be0cd19137e2179,
];

// Rotation constants, used in g
// Extracted from https://datatracker.ietf.org/doc/html/rfc7693#section-2.1
const R1: u32 = 32;
const R2: u32 = 24;
const R3: u32 = 16;
const R4: u32 = 63;

/// The G primitive function mixes two input words, "x" and "y", into
/// four words indexed by "a", "b", "c", and "d" in the working vector
/// v[0..15].  The full modified vector is returned.
/// Based on https://datatracker.ietf.org/doc/html/rfc7693#section-3.1
#[allow(clippy::indexing_slicing)]
fn g(v: [u64; 16], a: usize, b: usize, c: usize, d: usize, x: u64, y: u64) -> [u64; 16] {
    let mut ret = v;
    ret[a] = v[a].wrapping_add(v[b]).wrapping_add(x);
    ret[d] = (ret[d] ^ ret[a]).rotate_right(R1);
    ret[c] = ret[c].wrapping_add(ret[d]);
    ret[b] = (ret[b] ^ ret[c]).rotate_right(R2);
    ret[a] = ret[a].wrapping_add(ret[b]).wrapping_add(y);
    ret[d] = (ret[d] ^ ret[a]).rotate_right(R3);
    ret[c] = ret[c].wrapping_add(ret[d]);
    ret[b] = (ret[b] ^ ret[c]).rotate_right(R4);

    ret
}

/// Perform the permutations on the work vector given the rounds to permute and the message block
#[allow(clippy::indexing_slicing)]
fn word_permutation(rounds_to_permute: usize, v: [u64; 16], m: &[u64; 16]) -> [u64; 16] {
    let mut ret = v;

    for i in 0..rounds_to_permute {
        // Message word selection permutation for each round.

        let s: &[usize; 16] = &SIGMA[i % 10];

        ret = g(ret, 0, 4, 8, 12, m[s[0]], m[s[1]]);
        ret = g(ret, 1, 5, 9, 13, m[s[2]], m[s[3]]);
        ret = g(ret, 2, 6, 10, 14, m[s[4]], m[s[5]]);
        ret = g(ret, 3, 7, 11, 15, m[s[6]], m[s[7]]);

        ret = g(ret, 0, 5, 10, 15, m[s[8]], m[s[9]]);
        ret = g(ret, 1, 6, 11, 12, m[s[10]], m[s[11]]);
        ret = g(ret, 2, 7, 8, 13, m[s[12]], m[s[13]]);
        ret = g(ret, 3, 4, 9, 14, m[s[14]], m[s[15]]);
    }

    ret
}

/// Based on https://datatracker.ietf.org/doc/html/rfc7693#section-3.2
pub fn blake2f_compress_f(
    rounds: usize, // Specifies the rounds to permute
    h: &[u64; 8],  // State vector, defines the work vector (v) and affects the XOR process
    m: &[u64; 16], // The message block to compress
    t: &[u64; 2],  // Affects the work vector (v) before permutations
    f: bool,       // If set as true, inverts all bits
) -> [u64; 8] {
    // Initialize local work vector v[0..15], takes first half from state and second half from IV.
    let mut v: [u64; 16] = [0; 16];
    v[0..8].copy_from_slice(h);
    v[8..16].copy_from_slice(&IV);

    v[12] ^= t[0]; // Low word of the offset
    v[13] ^= t[1]; // High word of the offset

    // If final block flag is true, invert all bits
    if f {
        v[14] = !v[14];
    }

    v = word_permutation(rounds, v, m);

    let mut output = [0; 8];

    // XOR the two halves, put the results in the output slice
    for (i, pos) in output.iter_mut().enumerate() {
        *pos = h.get(i).unwrap() ^ v.get(i).unwrap() ^ v.get(i.overflowing_add(8).0).unwrap();
    }

    output
}

#[cfg(test)]
mod test {
    use crate::blake2f::portable::blake2f_compress_f;
    #[test]
    fn test_12r() {
        let out = blake2f_compress_f(
            12,
            &[1, 2, 3, 4, 5, 6, 7, 8],
            &[
                101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116,
            ],
            &[1000, 1001],
            true,
        );
        assert_eq!(
            out,
            [
                16719151077261791083,
                2946084527549390899,
                18258373236029374890,
                15305391278487550604,
                16233503039257535911,
                17654926667207417465,
                12194914407095793501,
                13409096818966589674
            ]
        );
    }
}
