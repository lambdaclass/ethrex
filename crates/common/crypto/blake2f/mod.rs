use std::sync::LazyLock;

mod portable;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86_64;

type Blake2Func = fn(usize, &mut [u64; 8], &[u64; 16], &[u64; 2], bool);

static BLAKE2_FUNC: LazyLock<Blake2Func> = LazyLock::new(|| {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if is_x86_feature_detected!("avx2") {
        return self::x86_64::blake2b_f;
    }

    self::portable::blake2b_f
});

pub fn blake2b_f(rounds: usize, h: &mut [u64; 8], m: &[u64; 16], t: &[u64; 2], f: bool) {
    BLAKE2_FUNC(rounds, h, m, t, f)
}
