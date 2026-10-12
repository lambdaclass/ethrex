//! Fixed-width MODEXP for odd moduli of at most 1024 bits.
//!
//! Most MODEXP calls use moduli of 128 bytes or less (field arithmetic helpers,
//! small RSA-free schemes). For those, Montgomery multiplication on 4, 8 or 16
//! 64-bit limbs with a 4-bit exponent window avoids the heap-allocated big
//! integers of the generic path and is several times faster.

/// Returns `base^exp mod modulus` as `modulus.len()` big-endian bytes, or
/// `None` when the inputs are outside what this path handles (even or zero
/// modulus, modulus or base longer than 128 bytes).
pub(crate) fn modexp(base: &[u8], exp: &[u8], modulus: &[u8]) -> Option<Vec<u8>> {
    // The base only has to be below 2^(64 * N), not below the modulus: the
    // Montgomery conversion reduces it.
    let width = modulus.len().max(base.len());
    let result = if width <= 32 {
        pow::<4>(base, exp, modulus)?
    } else if width <= 64 {
        pow::<8>(base, exp, modulus)?
    } else if width <= 128 {
        pow::<16>(base, exp, modulus)?
    } else {
        return None;
    };
    Some(result)
}

fn pow<const N: usize>(base: &[u8], exp: &[u8], modulus: &[u8]) -> Option<Vec<u8>> {
    let m = from_be_bytes::<N>(modulus);
    if m[0] & 1 == 0 {
        // Even, or zero.
        return None;
    }
    let mut out = vec![0u8; modulus.len()];
    if is_one(&m) {
        return Some(out);
    }
    let result = pow_odd(&from_be_bytes::<N>(base), exp, Montgomery::new(m));
    // `result < modulus < 2^(8 * modulus.len())`, so the dropped bytes are zero.
    for (i, byte) in out.iter_mut().rev().enumerate() {
        *byte = (result[i / 8] >> (8 * (i % 8))) as u8;
    }
    Some(out)
}

/// `base^exp mod m` for odd `m > 1` and `base < 2^(64 * N)`.
fn pow_odd<const N: usize>(base: &[u64; N], exp: &[u8], mont: Montgomery<N>) -> [u64; N] {
    let one = mont.to_montgomery(&limb_one());
    let base = mont.to_montgomery(base);

    // table[i] = base^i in Montgomery form.
    let mut table = [one; 16];
    for i in 1..16 {
        table[i] = mont.mul(&table[i - 1], &base);
    }

    let mut acc = one;
    let mut started = false;
    for byte in exp {
        for nibble in [byte >> 4, byte & 0x0f] {
            if started {
                for _ in 0..4 {
                    acc = mont.mul(&acc, &acc);
                }
            }
            if nibble != 0 {
                acc = mont.mul(&acc, &table[usize::from(nibble)]);
                started = true;
            }
        }
    }
    mont.mul(&acc, &limb_one())
}

struct Montgomery<const N: usize> {
    modulus: [u64; N],
    /// `-modulus^-1 mod 2^64`.
    inv: u64,
    /// `2^(128 * N) mod modulus`, used to enter the Montgomery domain.
    r2: [u64; N],
}

impl<const N: usize> Montgomery<N> {
    fn new(modulus: [u64; N]) -> Self {
        // Newton iteration: each step doubles the number of correct low bits,
        // starting from 3 for any odd value.
        let mut inv = modulus[0];
        for _ in 0..5 {
            inv = inv.wrapping_mul(2u64.wrapping_sub(modulus[0].wrapping_mul(inv)));
        }
        // 2^(128 * N) mod m by doubling 1 that many times.
        let mut r2 = limb_one::<N>();
        for _ in 0..128 * N {
            let carry = r2[N - 1] >> 63;
            for j in (1..N).rev() {
                r2[j] = r2[j] << 1 | r2[j - 1] >> 63;
            }
            r2[0] <<= 1;
            if carry != 0 || !less_than(&r2, &modulus) {
                sub_assign(&mut r2, &modulus);
            }
        }
        Self {
            modulus,
            inv: inv.wrapping_neg(),
            r2,
        }
    }

    /// `a * R mod m` for any `a < 2^(64 * N)`: the product with `R^2 mod m` is
    /// below `m * R`, which keeps the Montgomery reduction exact.
    fn to_montgomery(&self, a: &[u64; N]) -> [u64; N] {
        self.mul(a, &self.r2)
    }

    /// CIOS Montgomery multiplication: `a * b / 2^(64 * N) mod m` for
    /// `a * b < m * 2^(64 * N)`.
    fn mul(&self, a: &[u64; N], b: &[u64; N]) -> [u64; N] {
        let m = &self.modulus;
        let mut t = [0u64; N];
        // Words N and N + 1 of the running sum.
        let mut t_n = 0u64;
        for &bi in b {
            let mut carry = 0u64;
            for j in 0..N {
                (t[j], carry) = mac(t[j], a[j], bi, carry);
            }
            let (sum, overflow) = t_n.overflowing_add(carry);
            let t_n1 = u64::from(overflow);

            let factor = t[0].wrapping_mul(self.inv);
            let (_, mut carry) = mac(t[0], factor, m[0], 0);
            for j in 1..N {
                (t[j - 1], carry) = mac(t[j], factor, m[j], carry);
            }
            let (top, overflow) = sum.overflowing_add(carry);
            t[N - 1] = top;
            t_n = t_n1 + u64::from(overflow);
        }
        if t_n != 0 || !less_than(&t, m) {
            sub_assign(&mut t, m);
        }
        t
    }
}

fn from_be_bytes<const N: usize>(bytes: &[u8]) -> [u64; N] {
    let mut limbs = [0u64; N];
    for (i, &byte) in bytes.iter().rev().enumerate() {
        limbs[i / 8] |= u64::from(byte) << (8 * (i % 8));
    }
    limbs
}

fn limb_one<const N: usize>() -> [u64; N] {
    let mut one = [0u64; N];
    one[0] = 1;
    one
}

fn is_one<const N: usize>(a: &[u64; N]) -> bool {
    a[0] == 1 && a[1..].iter().all(|&limb| limb == 0)
}

/// `a + b * c + carry`, returned as `(low, high)` words.
#[inline(always)]
fn mac(a: u64, b: u64, c: u64, carry: u64) -> (u64, u64) {
    let wide = u128::from(a) + u128::from(b) * u128::from(c) + u128::from(carry);
    (wide as u64, (wide >> 64) as u64)
}

fn less_than<const N: usize>(a: &[u64; N], b: &[u64; N]) -> bool {
    for i in (0..N).rev() {
        if a[i] != b[i] {
            return a[i] < b[i];
        }
    }
    false
}

/// `a -= b`, wrapping.
fn sub_assign<const N: usize>(a: &mut [u64; N], b: &[u64; N]) {
    let mut borrow = false;
    for j in 0..N {
        let (d1, b1) = a[j].overflowing_sub(b[j]);
        let (d2, b2) = d1.overflowing_sub(u64::from(borrow));
        a[j] = d2;
        borrow = b1 || b2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use malachite::Natural;
    use malachite::base::num::arithmetic::traits::ModPow;
    use malachite::base::num::conversion::traits::PowerOf2Digits;
    use sha2::{Digest, Sha256};

    fn reference(base: &[u8], exp: &[u8], modulus: &[u8]) -> Vec<u8> {
        let b = Natural::from_power_of_2_digits_desc(8u64, base.iter().cloned()).unwrap();
        let e = Natural::from_power_of_2_digits_desc(8u64, exp.iter().cloned()).unwrap();
        let m = Natural::from_power_of_2_digits_desc(8u64, modulus.iter().cloned()).unwrap();
        let r = (b % &m).mod_pow(e, &m);
        let bytes: Vec<u8> = r.to_power_of_2_digits_desc(8);
        let mut out = vec![0u8; modulus.len()];
        out[modulus.len() - bytes.len()..].copy_from_slice(&bytes);
        out
    }

    fn bytes(seed: u64, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        let mut counter = 0u64;
        while out.len() < len {
            let digest = Sha256::digest([seed.to_le_bytes(), counter.to_le_bytes()].concat());
            out.extend_from_slice(&digest);
            counter += 1;
        }
        out.truncate(len);
        out
    }

    #[test]
    fn matches_reference_on_random_inputs() {
        for seed in 0..3000u64 {
            let mod_len = 1 + usize::try_from(seed % 128).unwrap();
            let base_len = usize::try_from((seed / 3) % 129).unwrap();
            let exp_len = usize::try_from((seed / 7) % 70).unwrap();
            let mut modulus = bytes(seed, mod_len);
            *modulus.last_mut().unwrap() |= 1;
            if seed % 5 == 0 {
                modulus[0] |= 0x80;
            }
            let base = bytes(seed + 1_000_000, base_len);
            let exp = bytes(seed + 2_000_000, exp_len);
            assert_eq!(
                modexp(&base, &exp, &modulus).unwrap(),
                reference(&base, &exp, &modulus),
                "seed {seed}"
            );
        }
    }

    #[test]
    fn edge_cases() {
        let max = [0xffu8; 32];
        let one = [1u8];
        assert_eq!(
            modexp(&max, &max, &max).unwrap(),
            reference(&max, &max, &max)
        );
        assert_eq!(modexp(&max, &[], &max).unwrap(), reference(&max, &[], &max));
        assert_eq!(modexp(&[], &max, &max).unwrap(), reference(&[], &max, &max));
        assert_eq!(modexp(&max, &max, &one).unwrap(), vec![0u8]);
        assert_eq!(modexp(&max, &[0, 0, 0], &one).unwrap(), vec![0u8]);
        // Leading zero bytes in the modulus keep the output width.
        let padded = [0u8, 0, 0, 7];
        assert_eq!(
            modexp(&[3], &[5], &padded).unwrap(),
            reference(&[3], &[5], &padded)
        );
        // Width boundaries: 32/33, 64/65 and 128 bytes, with a long base too.
        for len in [32usize, 33, 64, 65, 128] {
            let mut modulus = vec![0xffu8; len];
            *modulus.last_mut().unwrap() = 0xfd;
            let base = [0xabu8; 128];
            for base_len in [0usize, 1, len, 128] {
                let base = &base[..base_len];
                assert_eq!(
                    modexp(base, &max, &modulus).unwrap(),
                    reference(base, &max, &modulus),
                    "modulus {len} bytes, base {base_len} bytes"
                );
            }
        }
        // Unsupported shapes fall back.
        assert!(modexp(&[2], &[3], &[4]).is_none());
        assert!(modexp(&[2], &[3], &[0]).is_none());
        assert!(modexp(&[2], &[3], &[1u8; 129]).is_none());
        assert!(modexp(&[2u8; 129], &[3], &[7]).is_none());
    }
}
