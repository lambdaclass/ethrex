#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
core::arch::global_asm!(include_str!("keccak1600-armv8-elf.s"), options(raw));
#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
core::arch::global_asm!(include_str!("keccak1600-armv8-macho.s"), options(raw));
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("keccak1600-x86_64.s"), options(att_syntax));

pub use imp::*;

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
mod imp {
    const BLOCK_SIZE: usize = 136;

    #[derive(Default, Clone, Copy)]
    #[repr(transparent)]
    struct State([u64; 25]);

    unsafe extern "C" {
        #[link_name = "SHA3_absorb"]
        unsafe fn SHA3_absorb(state: *mut State, buf: *const u8, len: usize, r: usize) -> usize;
        unsafe fn SHA3_squeeze(state: *mut State, buf: *mut u8, len: usize, r: usize);
    }

    /// Hashes straight from the input instead of going through `Keccak256`: the streaming state
    /// is moved and copied around (tail buffer, `finalize(self)`), and for the short messages
    /// the EVM hashes most (one permutation) that bookkeeping costs up to a tenth of the hash.
    pub fn keccak_hash(data: impl AsRef<[u8]>) -> [u8; 32] {
        let data = data.as_ref();
        let full_blocks_len = data.len() - data.len() % BLOCK_SIZE;
        let tail = &data[full_blocks_len..];

        let mut last_block = [0u8; BLOCK_SIZE];
        last_block[..tail.len()].copy_from_slice(tail);
        last_block[tail.len()] = 0x01;
        last_block[BLOCK_SIZE - 1] |= 0x80;

        let mut state = State::default();
        let mut hash = [0u8; 32];
        unsafe {
            if full_blocks_len != 0 {
                SHA3_absorb(&mut state, data.as_ptr(), full_blocks_len, BLOCK_SIZE);
            }
            SHA3_absorb(&mut state, last_block.as_ptr(), BLOCK_SIZE, BLOCK_SIZE);
            SHA3_squeeze(&mut state, hash.as_mut_ptr(), hash.len(), BLOCK_SIZE);
        }
        hash
    }

    #[derive(Clone)]
    pub struct Keccak256 {
        state: State,
        tail_buf: [u8; BLOCK_SIZE],
        tail_len: usize,
    }

    impl Default for Keccak256 {
        fn default() -> Self {
            Self {
                state: State::default(),
                tail_buf: [0; BLOCK_SIZE],
                tail_len: 0,
            }
        }
    }

    impl Keccak256 {
        #[inline]
        pub fn new() -> Self {
            Self::default()
        }

        #[inline]
        pub fn update(&mut self, data: impl AsRef<[u8]>) -> Self {
            let mut data = data.as_ref();
            unsafe {
                // partial block
                if self.tail_len > 0 {
                    let need = BLOCK_SIZE - self.tail_len;
                    if data.len() < need {
                        // still partial block
                        self.tail_buf[self.tail_len..self.tail_len + data.len()]
                            .copy_from_slice(data);
                        self.tail_len += data.len();
                        return self.clone();
                    }

                    // complete block
                    self.tail_buf[self.tail_len..BLOCK_SIZE].copy_from_slice(&data[..need]);

                    SHA3_absorb(
                        &mut self.state,
                        self.tail_buf.as_ptr(),
                        self.tail_buf.len(),
                        BLOCK_SIZE,
                    );

                    self.tail_len = 0;
                    self.tail_buf.fill(0);
                    data = &data[need..];
                }
            }

            match data {
                [] => {}
                data if data.len() < BLOCK_SIZE => unsafe {
                    self.tail_len = data.len();
                    self.tail_buf
                        .get_unchecked_mut(..self.tail_len)
                        .copy_from_slice(data);
                },
                data => unsafe {
                    let rem = SHA3_absorb(&mut self.state, data.as_ptr(), data.len(), BLOCK_SIZE);
                    self.tail_len = rem;
                    if rem != 0 {
                        let tail_data = data.get_unchecked(data.len() - rem..);
                        self.tail_buf
                            .get_unchecked_mut(..rem)
                            .copy_from_slice(tail_data);
                    }
                },
            }
            self.clone()
        }

        #[inline]
        pub fn finalize(mut self) -> [u8; 32] {
            let mut hash_buf = [0u8; 32];

            unsafe {
                *self.tail_buf.get_unchecked_mut(self.tail_len) = 0x01;
                *self.tail_buf.get_unchecked_mut(BLOCK_SIZE - 1) |= 0x80;

                SHA3_absorb(
                    &mut self.state,
                    self.tail_buf.as_ptr(),
                    self.tail_buf.len(),
                    BLOCK_SIZE,
                );

                SHA3_squeeze(
                    &mut self.state,
                    hash_buf.as_mut_ptr(),
                    hash_buf.len(),
                    BLOCK_SIZE,
                );
            }

            hash_buf
        }
    }

    #[cfg(test)]
    mod tests {
        // `alloc` explicitly: the crate is `no_std` without the `std` feature.
        extern crate alloc;

        use super::*;
        use alloc::vec::Vec;
        use tiny_keccak::{Hasher, Keccak};

        fn reference(data: &[u8]) -> [u8; 32] {
            let mut out = [0u8; 32];
            let mut hasher = Keccak::v256();
            hasher.update(data);
            hasher.finalize(&mut out);
            out
        }

        /// splitmix64: deterministic, so a failure reproduces exactly.
        struct Rng(u64);

        impl Rng {
            fn next(&mut self) -> u64 {
                self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut z = self.0;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                z ^ (z >> 31)
            }

            fn below(&mut self, bound: usize) -> usize {
                (self.next() % bound as u64) as usize
            }

            fn bytes(&mut self, len: usize) -> Vec<u8> {
                (0..len).map(|_| self.next() as u8).collect()
            }

            fn message(&mut self, max_len: usize) -> Vec<u8> {
                let len = self.below(max_len + 1);
                self.bytes(len)
            }
        }

        fn streaming_one_update(data: &[u8]) -> [u8; 32] {
            let mut hasher = Keccak256::new();
            hasher.update(data);
            hasher.finalize()
        }

        #[test]
        fn matches_reference_for_every_length_around_the_rate() {
            let mut rng = Rng(1);
            let data = rng.bytes(3 * BLOCK_SIZE + 1);
            for len in 0..=data.len() {
                let expected = reference(&data[..len]);
                assert_eq!(keccak_hash(&data[..len]), expected, "len {len}");
                assert_eq!(streaming_one_update(&data[..len]), expected, "len {len}");
            }
        }

        #[test]
        fn matches_reference_for_random_lengths() {
            let mut rng = Rng(2);
            for _ in 0..500 {
                let data = rng.message(10_000);
                let expected = reference(&data);
                assert_eq!(keccak_hash(&data), expected, "len {}", data.len());
                assert_eq!(streaming_one_update(&data), expected, "len {}", data.len());
            }
        }

        #[test]
        fn streaming_matches_reference_across_chunk_boundaries() {
            // Chunk sizes that land exactly on, just before and just after the rate, mixed
            // with arbitrary ones, so a partial tail is completed, flushed and restarted in
            // every possible position.
            const EDGES: [usize; 8] = [0, 1, 7, 8, 135, 136, 137, 272];
            let mut rng = Rng(4);
            for _ in 0..500 {
                let data = rng.message(10_000);
                let mut hasher = Keccak256::new();
                let mut fed = 0;
                while fed < data.len() {
                    let chunk = if rng.below(2) == 0 {
                        EDGES[rng.below(EDGES.len())]
                    } else {
                        rng.below(600)
                    };
                    let end = (fed + chunk).min(data.len());
                    hasher.update(&data[fed..end]);
                    fed = end;
                }
                assert_eq!(hasher.finalize(), reference(&data), "len {}", data.len());
            }
        }

        #[test]
        fn finalizing_a_clone_leaves_the_original_usable() {
            // The RLPx MAC reads its running digest by finalizing a clone mid-stream.
            let mut rng = Rng(5);
            for _ in 0..100 {
                let first = rng.message(1_000);
                let second = rng.message(1_000);
                let mut hasher = Keccak256::new();
                hasher.update(&first);
                assert_eq!(hasher.clone().finalize(), reference(&first));
                hasher.update(&second);
                assert_eq!(hasher.finalize(), reference(&[first, second].concat()));
            }
        }
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
mod imp {
    use tiny_keccak::{Hasher, Keccak};

    pub fn keccak_hash(data: impl AsRef<[u8]>) -> [u8; 32] {
        let mut out = [0u8; 32];
        let mut h = Keccak::v256();
        h.update(data.as_ref());
        h.finalize(&mut out);
        out
    }

    #[derive(Clone)]
    pub struct Keccak256 {
        h: Keccak,
    }

    impl Default for Keccak256 {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Keccak256 {
        #[inline]
        pub fn new() -> Self {
            Self { h: Keccak::v256() }
        }

        #[inline]
        pub fn update(&mut self, data: impl AsRef<[u8]>) -> Self {
            let d = data.as_ref();
            if !d.is_empty() {
                self.h.update(d);
            }
            self.clone()
        }

        #[inline]
        pub fn finalize(self) -> [u8; 32] {
            let mut out = [0u8; 32];
            self.h.finalize(&mut out);
            out
        }
    }
}
