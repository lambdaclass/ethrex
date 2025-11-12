#[cfg(all(target_arch = "aarch64", target_os = "linux"))]
std::arch::global_asm!(include_str!("keccak1600-armv8-elf.s"), options(raw));
#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
std::arch::global_asm!(include_str!("keccak1600-armv8-macho.s"), options(raw));
#[cfg(target_arch = "x86_64")]
std::arch::global_asm!(include_str!("keccak1600-x86_64.s"), options(att_syntax));

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

    pub fn keccak_hash(data: impl AsRef<[u8]>) -> [u8; 32] {
        let mut state = Keccak256::new();
        state.update(data);
        state.finalize()
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

#[cfg(test)]
mod test {
    use super::*;
    use std::array;

    const BLOCK_SIZE: usize = 136;

    #[test]
    fn keccak_empty() {
        assert_eq!(
            keccak_hash(b"")
                .into_iter()
                .map(|x| format!("{x:02x}"))
                .collect::<String>(),
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
        );
    }

    #[test]
    fn keccak_half_block() {
        let buf: [u8; BLOCK_SIZE >> 1] =
            array::from_fn(|i| (i << 5 & 0xF0 | ((i << 1) + 1) & 0x0F) as u8);

        assert_eq!(
            keccak_hash(buf)
                .into_iter()
                .map(|x| format!("{x:02x}"))
                .collect::<String>(),
            "337bf14237b641240bd3204e9991c8b96a5349613735ade90a5c2b8806355c11",
        );
    }

    #[test]
    fn keccak_full_block() {
        let buf: [u8; BLOCK_SIZE] =
            array::from_fn(|i| (i << 5 & 0xF0 | ((i << 1) + 1) & 0x0F) as u8);

        assert_eq!(
            keccak_hash(buf)
                .into_iter()
                .map(|x| format!("{x:02x}"))
                .collect::<String>(),
            "3f7424fa94a2f8c5a733b86dac312d85685f9af3dea919694cc6a8abfc075460",
        );
    }

    #[test]
    fn keccak_almost_full_block() {
        let buf: [u8; BLOCK_SIZE - 1] =
            array::from_fn(|i| (i << 5 & 0xF0 | ((i << 1) + 1) & 0x0F) as u8);

        assert_eq!(
            keccak_hash(buf)
                .into_iter()
                .map(|x| format!("{x:02x}"))
                .collect::<String>(),
            "3e4916729e2522af4937548f5848a5b49067eec910a0a6a890b0c71dde08854e",
        );
    }

    #[test]
    fn keccak_asm_empty() {
        let keccak = Keccak256::new();
        assert_eq!(
            keccak
                .finalize()
                .into_iter()
                .map(|x| format!("{x:02x}"))
                .collect::<String>(),
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
        );
    }

    #[test]
    fn keccak_asm_half_block() {
        let mut keccak = Keccak256::new();
        let buf: [u8; BLOCK_SIZE >> 1] =
            array::from_fn(|i| (i << 5 & 0xF0 | ((i << 1) + 1) & 0x0F) as u8);
        keccak.update(buf);

        assert_eq!(
            keccak
                .finalize()
                .into_iter()
                .map(|x| format!("{x:02x}"))
                .collect::<String>(),
            "337bf14237b641240bd3204e9991c8b96a5349613735ade90a5c2b8806355c11",
        );
    }

    #[test]
    fn keccak_asm_full_block() {
        let mut keccak = Keccak256::new();
        let buf: [u8; BLOCK_SIZE] =
            array::from_fn(|i| (i << 5 & 0xF0 | ((i << 1) + 1) & 0x0F) as u8);
        keccak.update(buf);

        assert_eq!(
            keccak
                .finalize()
                .into_iter()
                .map(|x| format!("{x:02x}"))
                .collect::<String>(),
            "3f7424fa94a2f8c5a733b86dac312d85685f9af3dea919694cc6a8abfc075460",
        );
    }

    #[test]
    fn keccak_asm_almost_full_block() {
        let mut keccak = Keccak256::new();
        let buf: [u8; BLOCK_SIZE - 1] =
            array::from_fn(|i| (i << 5 & 0xF0 | ((i << 1) + 1) & 0x0F) as u8);
        keccak.update(buf);

        assert_eq!(
            keccak
                .finalize()
                .into_iter()
                .map(|x| format!("{x:02x}"))
                .collect::<String>(),
            "3e4916729e2522af4937548f5848a5b49067eec910a0a6a890b0c71dde08854e",
        );
    }

    #[test]
    fn keccak_asm_two_half_updates() {
        let mut keccak = Keccak256::new();

        let full: [u8; BLOCK_SIZE] =
            array::from_fn(|i| (i << 5 & 0xF0 | ((i << 1) + 1) & 0x0F) as u8);

        let half = BLOCK_SIZE / 2;

        keccak.update(&full[..half]);
        keccak.update(&full[half..]);

        let buf = keccak
            .finalize()
            .into_iter()
            .map(|x| format!("{x:02x}"))
            .collect::<String>();

        assert_eq!(
            buf,
            "3f7424fa94a2f8c5a733b86dac312d85685f9af3dea919694cc6a8abfc075460"
        );
    }

    #[test]
    fn keccak_compare_one_shot_vs_two_updates() {
        let full: Vec<u8> = (0..BLOCK_SIZE)
            .map(|i| (i << 5 & 0xF0 | ((i << 1) + 1) & 0x0F) as u8)
            .collect();

        let mut k1 = Keccak256::new();
        let mut k2 = Keccak256::new();

        k1.update(&full);

        k2.update(&full[..BLOCK_SIZE / 2]);
        k2.update(&full[BLOCK_SIZE / 2..]);

        let h1 = k1.finalize();

        let h2 = k2.finalize();

        assert_eq!(h1, h2);
    }

    #[test]
    fn keccac_compare_small_than_block() {
        let mut one = Keccak256::new();
        let mut two = Keccak256::new();

        let a = vec![1u8; 30];
        let b = vec![1u8; 40];

        one.update(&a);
        one.update(&b);

        two.update([1u8; 70]);

        assert_eq!(one.finalize(), two.finalize());
    }
}
