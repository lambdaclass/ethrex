#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod avx;

mod portable;

#[inline(always)]
pub fn blake2f_compress_f(
    rounds: usize,
    h: &[u64; 8],
    m: &[u64; 16],
    t: &[u64; 2],
    f: bool,
) -> [u64; 8] {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if is_x86_feature_detected!("avx2") {
        // SAFETY: avx2 verified to be available
        return unsafe { avx::blake2f_compress_f_inner(rounds, h, m, t, f) };
    }
    // SAFETY: safe function
    portable::blake2f_compress_f(rounds, h, m, t, f)
}
