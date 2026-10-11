#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
core::arch::global_asm!(include_str!("keccak1600-armv8-elf.s"), options(raw));
#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
core::arch::global_asm!(include_str!("keccak1600-armv8-macho.s"), options(raw));
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("keccak1600-x86_64.s"), options(att_syntax));
// The AVX-512F code is only ever reached through runtime CPU detection (see `imp::backend`),
// never through compile-time target features: release builds target x86-64-v3 and must keep
// running on CPUs without AVX-512. Detection needs `std`, so `no_std` builds keep the scalar
// code only. `raw` because the AVX-512 mask syntax (`{%k1}`) would otherwise be read as format
// placeholders.
#[cfg(all(target_arch = "x86_64", feature = "std"))]
core::arch::global_asm!(
    include_str!("keccak1600-avx512.s"),
    options(att_syntax, raw)
);

pub use imp::*;

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
mod imp {
    const BLOCK_SIZE: usize = 136;

    #[derive(Default, Clone, Copy)]
    #[repr(transparent)]
    struct State([u64; 25]);

    type AbsorbFn =
        unsafe extern "C" fn(state: *mut State, buf: *const u8, len: usize, r: usize) -> usize;
    type SqueezeFn = unsafe extern "C" fn(state: *mut State, buf: *mut u8, len: usize, r: usize);

    /// One assembly implementation of the sponge. Each `Keccak256` keeps the backend it started
    /// with, so a state is absorbed and squeezed by the same code from start to finish.
    struct Backend {
        absorb: AbsorbFn,
        squeeze: SqueezeFn,
    }

    unsafe extern "C" {
        #[link_name = "SHA3_absorb"]
        unsafe fn SHA3_absorb(state: *mut State, buf: *const u8, len: usize, r: usize) -> usize;
        unsafe fn SHA3_squeeze(state: *mut State, buf: *mut u8, len: usize, r: usize);
        #[cfg(all(target_arch = "x86_64", feature = "std"))]
        unsafe fn SHA3_absorb_avx512(
            state: *mut State,
            buf: *const u8,
            len: usize,
            r: usize,
        ) -> usize;
        #[cfg(all(target_arch = "x86_64", feature = "std"))]
        unsafe fn SHA3_squeeze_avx512(state: *mut State, buf: *mut u8, len: usize, r: usize);
    }

    static SCALAR: Backend = Backend {
        absorb: SHA3_absorb,
        squeeze: SHA3_squeeze,
    };

    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    static AVX512: Backend = Backend {
        absorb: SHA3_absorb_avx512,
        squeeze: SHA3_squeeze_avx512,
    };

    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    fn is_intel() -> bool {
        // SAFETY: every x86_64 CPU implements `cpuid`, and leaf 0 always exists.
        let vendor = unsafe { core::arch::x86_64::__cpuid(0) };
        // "GenuineIntel", spread over EBX, EDX and ECX.
        (vendor.ebx, vendor.edx, vendor.ecx) == (0x756e_6547, 0x4965_6e69, 0x6c65_746e)
    }

    /// Picks the backend once per process. The AVX-512F code is only used on Intel: on AMD it is
    /// no faster than the scalar code on Zen 5 for EVM-sized inputs, and Zen 4 splits every
    /// 512-bit operation into two 256-bit halves, where the upstream figures put it about 9%
    /// behind the scalar code.
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    fn backend() -> &'static Backend {
        static SELECTED: std::sync::OnceLock<&'static Backend> = std::sync::OnceLock::new();
        SELECTED.get_or_init(|| {
            if std::is_x86_feature_detected!("avx512f") && is_intel() {
                &AVX512
            } else {
                &SCALAR
            }
        })
    }

    #[cfg(not(all(target_arch = "x86_64", feature = "std")))]
    fn backend() -> &'static Backend {
        &SCALAR
    }

    pub fn keccak_hash(data: impl AsRef<[u8]>) -> [u8; 32] {
        hash_with(backend(), data.as_ref())
    }

    /// Hashes straight from the input instead of going through `Keccak256`: the streaming state
    /// is moved and copied around (tail buffer, `finalize(self)`), and for the short messages
    /// the EVM hashes most (one permutation) that bookkeeping costs up to a tenth of the hash.
    #[inline]
    fn hash_with(backend: &Backend, data: &[u8]) -> [u8; 32] {
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
                (backend.absorb)(&mut state, data.as_ptr(), full_blocks_len, BLOCK_SIZE);
            }
            (backend.absorb)(&mut state, last_block.as_ptr(), BLOCK_SIZE, BLOCK_SIZE);
            (backend.squeeze)(&mut state, hash.as_mut_ptr(), hash.len(), BLOCK_SIZE);
        }
        hash
    }

    #[derive(Clone)]
    pub struct Keccak256 {
        backend: &'static Backend,
        state: State,
        tail_buf: [u8; BLOCK_SIZE],
        tail_len: usize,
    }

    impl Default for Keccak256 {
        fn default() -> Self {
            Self::with_backend(backend())
        }
    }

    impl Keccak256 {
        #[inline]
        pub fn new() -> Self {
            Self::default()
        }

        #[inline]
        fn with_backend(backend: &'static Backend) -> Self {
            Self {
                backend,
                state: State::default(),
                tail_buf: [0; BLOCK_SIZE],
                tail_len: 0,
            }
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

                    (self.backend.absorb)(
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
                    let rem = (self.backend.absorb)(
                        &mut self.state,
                        data.as_ptr(),
                        data.len(),
                        BLOCK_SIZE,
                    );
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

                (self.backend.absorb)(
                    &mut self.state,
                    self.tail_buf.as_ptr(),
                    self.tail_buf.len(),
                    BLOCK_SIZE,
                );

                (self.backend.squeeze)(
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
        use alloc::{vec, vec::Vec};
        use tiny_keccak::{Hasher, Keccak};

        /// Every backend the running CPU can execute, whether dispatch would pick it or not, so
        /// both the scalar fallback and the AVX-512F code are checked on an AVX-512 machine.
        fn runnable_backends() -> Vec<&'static Backend> {
            #[allow(unused_mut)]
            let mut backends = vec![&SCALAR];
            #[cfg(all(target_arch = "x86_64", feature = "std"))]
            if std::is_x86_feature_detected!("avx512f") {
                backends.push(&AVX512);
            }
            backends
        }

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

        fn streaming_one_update(backend: &'static Backend, data: &[u8]) -> [u8; 32] {
            let mut hasher = Keccak256::with_backend(backend);
            hasher.update(data);
            hasher.finalize()
        }

        #[cfg(all(target_arch = "x86_64", feature = "std"))]
        #[test]
        fn dispatch_selects_avx512_only_on_intel() {
            let expected: &'static Backend =
                if std::is_x86_feature_detected!("avx512f") && is_intel() {
                    &AVX512
                } else {
                    &SCALAR
                };
            assert!(core::ptr::eq(backend(), expected));
            assert!(core::ptr::eq(Keccak256::new().backend, expected));
        }

        #[test]
        fn matches_reference_for_every_length_around_the_rate() {
            let mut rng = Rng(1);
            let data = rng.bytes(3 * BLOCK_SIZE + 1);
            for backend in runnable_backends() {
                for len in 0..=data.len() {
                    let expected = reference(&data[..len]);
                    assert_eq!(hash_with(backend, &data[..len]), expected, "len {len}");
                    assert_eq!(
                        streaming_one_update(backend, &data[..len]),
                        expected,
                        "len {len}"
                    );
                }
            }
        }

        #[test]
        fn matches_reference_for_random_lengths() {
            let mut rng = Rng(2);
            for backend in runnable_backends() {
                for _ in 0..500 {
                    let data = rng.message(10_000);
                    let expected = reference(&data);
                    assert_eq!(hash_with(backend, &data), expected, "len {}", data.len());
                    assert_eq!(
                        streaming_one_update(backend, &data),
                        expected,
                        "len {}",
                        data.len()
                    );
                }
            }
        }

        #[test]
        fn keccak_hash_matches_reference() {
            let mut rng = Rng(3);
            for _ in 0..500 {
                let data = rng.message(10_000);
                assert_eq!(keccak_hash(&data), reference(&data), "len {}", data.len());
            }
        }

        #[test]
        fn streaming_matches_reference_across_chunk_boundaries() {
            // Chunk sizes that land exactly on, just before and just after the rate, mixed
            // with arbitrary ones, so a partial tail is completed, flushed and restarted in
            // every possible position.
            const EDGES: [usize; 8] = [0, 1, 7, 8, 135, 136, 137, 272];
            let mut rng = Rng(4);
            for backend in runnable_backends() {
                for _ in 0..500 {
                    let data = rng.message(10_000);
                    let mut hasher = Keccak256::with_backend(backend);
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
        }

        #[test]
        fn finalizing_a_clone_leaves_the_original_usable() {
            // The RLPx MAC reads its running digest by finalizing a clone mid-stream.
            let mut rng = Rng(5);
            for backend in runnable_backends() {
                for _ in 0..100 {
                    let first = rng.message(1_000);
                    let second = rng.message(1_000);
                    let mut hasher = Keccak256::with_backend(backend);
                    hasher.update(&first);
                    assert_eq!(hasher.clone().finalize(), reference(&first));
                    hasher.update(&second);
                    assert_eq!(hasher.finalize(), reference(&[first, second].concat()));
                }
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
