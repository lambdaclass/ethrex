use std::sync::LazyLock;

mod portable;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
std::arch::global_asm!(include_str!("x86_64.s"));

type Blake2Func = fn(usize, &mut [u64; 8], &[u64; 16], &[u64; 2], bool);

static BLAKE2_FUNC: LazyLock<Blake2Func> = LazyLock::new(|| {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if is_x86_feature_detected!("avx2") {
        unsafe extern "C" {
            unsafe fn blake2b_f(h: &mut [u64; 8], m: &[u64; 16], t: &[u64; 2], r: usize, f: bool);
        }

        #[inline(always)]
        fn inner(r: usize, h: &mut [u64; 8], m: &[u64; 16], t: &[u64; 2], f: bool) {
            unsafe {
                blake2b_f(h, m, t, r, f);
            }
        }

        return inner;
    }

    portable::blake2f_compress_f
});

pub fn blake2f_compress_f(rounds: usize, h: &mut [u64; 8], m: &[u64; 16], t: &[u64; 2], f: bool) {
    BLAKE2_FUNC(rounds, h, m, t, f)
}
