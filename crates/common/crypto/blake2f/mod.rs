#[cfg(all(feature = "std", target_arch = "aarch64"))]
mod aarch64;
mod portable;
#[cfg(all(feature = "std", any(target_arch = "x86", target_arch = "x86_64")))]
mod x86_64;

#[cfg(feature = "std")]
use std::sync::LazyLock;

#[cfg(feature = "std")]
type Blake2Func = fn(usize, &mut [u64; 8], &[u64; 16], &[u64; 2], bool);

#[cfg(feature = "std")]
static BLAKE2_FUNC: LazyLock<Blake2Func> = LazyLock::new(|| {
    #[cfg(target_arch = "aarch64")]
    if std::arch::is_aarch64_feature_detected!("neon")
        && std::arch::is_aarch64_feature_detected!("sha3")
    {
        return self::aarch64::blake2b_f;
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if std::arch::is_x86_feature_detected!("avx2") {
        return self::x86_64::blake2b_f;
    }

    self::portable::blake2b_f
});

#[cfg(feature = "std")]
pub fn blake2b_f(rounds: usize, h: &mut [u64; 8], m: &[u64; 16], t: &[u64; 2], f: bool) {
    BLAKE2_FUNC(rounds, h, m, t, f)
}

#[cfg(not(feature = "std"))]
pub fn blake2b_f(rounds: usize, h: &mut [u64; 8], m: &[u64; 16], t: &[u64; 2], f: bool) {
    self::portable::blake2b_f(rounds, h, m, t, f)
}
