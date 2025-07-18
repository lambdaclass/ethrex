use std::sync::LazyLock;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod avx;

mod portable;

type Blake2Func = unsafe fn(usize, &[u64; 8], &[u64; 16], &[u64; 2], bool) -> [u64; 8];

static BLAKE2_FUNC: LazyLock<Blake2Func> = LazyLock::new(|| {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if is_x86_feature_detected!("avx2") {
        // SAFETY: avx2 verified to be available
        return avx::blake2f_compress_f_inner;
    }
    // SAFETY: safe function
    portable::blake2f_compress_f
});

pub fn blake2f_compress_f(
    rounds: usize,
    h: &[u64; 8],
    m: &[u64; 16],
    t: &[u64; 2],
    f: bool,
) -> [u64; 8] {
    // SAFETY: function guaranteed to be available in the current architecture by the BLAKE2_FUNC initializer
    unsafe { BLAKE2_FUNC(rounds, h, m, t, f) }
}
