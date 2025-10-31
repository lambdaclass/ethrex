use std::sync::LazyLock;

#[cfg(target_arch = "aarch64")]
mod aarch64;
mod portable;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86_64;

type Blake2Func = fn(usize, &mut [u64; 8], &[u64; 16], &[u64; 2], bool);

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

pub fn blake2b_f(rounds: usize, h: &mut [u64; 8], m: &[u64; 16], t: &[u64; 2], f: bool) {
    BLAKE2_FUNC(rounds, h, m, t, f)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blake2b_smoke() {
        let mut h = [1, 2, 3, 4, 5, 6, 7, 8];
        blake2b_f(
            12,
            &mut h,
            &[
                101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116,
            ],
            &[1000, 1001],
            true,
        );
        assert_eq!(
            h,
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
