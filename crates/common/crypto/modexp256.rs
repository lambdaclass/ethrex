//! Fixed-width MODEXP for odd moduli of at most 256 bits.
//!
//! Most MODEXP calls use moduli of 32 bytes or less (RSA-free contracts, field
//! arithmetic helpers). For those, Montgomery multiplication on four 64-bit
//! limbs with a 4-bit exponent window avoids the heap-allocated big integers
//! of the generic path and is several times faster.

use ethereum_types::{U256, U512};

/// Returns `base^exp mod modulus` as `modulus.len()` big-endian bytes, or
/// `None` when the inputs are outside what this path handles (even modulus,
/// modulus or base longer than 32 bytes). `modulus` must be non-zero.
pub(crate) fn modexp(base: &[u8], exp: &[u8], modulus: &[u8]) -> Option<Vec<u8>> {
    if modulus.len() > 32 || base.len() > 32 {
        return None;
    }
    let m = U256::from_big_endian(modulus);
    if m.is_zero() || !m.bit(0) {
        return None;
    }
    let result = if m == U256::one() {
        U256::zero()
    } else {
        pow_odd(U256::from_big_endian(base) % m, exp, m)
    };
    let bytes = result.to_big_endian();
    // `result < modulus < 2^(8 * modulus.len())`, so the dropped bytes are zero.
    Some(bytes[32 - modulus.len()..].to_vec())
}

/// `base^exp mod m` for odd `m > 1` and `base < m`.
fn pow_odd(base: U256, exp: &[u8], m: U256) -> U256 {
    let mont = Montgomery::new(m);
    let one = mont.to_montgomery(U256::one());
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
    mont.to_canonical(&acc)
}

struct Montgomery {
    modulus: [u64; 4],
    /// `-modulus^-1 mod 2^64`.
    inv: u64,
    /// `2^512 mod modulus`, used to enter the Montgomery domain.
    r2: [u64; 4],
}

impl Montgomery {
    fn new(m: U256) -> Self {
        let modulus = m.0;
        // Newton iteration: each step doubles the number of correct low bits,
        // starting from 3 for any odd value.
        let mut inv = modulus[0];
        for _ in 0..5 {
            inv = inv.wrapping_mul(2u64.wrapping_sub(modulus[0].wrapping_mul(inv)));
        }
        // 2^512 does not fit in U512, so reduce (2^512 - 1) and add one back.
        let r2 = (U512::MAX % U512::from(m) + U512::one()) % U512::from(m);
        let r2 = U256([r2.0[0], r2.0[1], r2.0[2], r2.0[3]]);
        Self {
            modulus,
            inv: inv.wrapping_neg(),
            r2: r2.0,
        }
    }

    fn to_montgomery(&self, value: U256) -> [u64; 4] {
        self.mul(&value.0, &self.r2)
    }

    fn to_canonical(&self, value: &[u64; 4]) -> U256 {
        U256(self.mul(value, &[1, 0, 0, 0]))
    }

    /// CIOS Montgomery multiplication: `a * b / 2^256 mod m` for `a, b < m`.
    fn mul(&self, a: &[u64; 4], b: &[u64; 4]) -> [u64; 4] {
        let m = &self.modulus;
        let mut t = [0u64; 6];
        for &bi in b {
            let mut carry = 0u64;
            for j in 0..4 {
                (t[j], carry) = mac(t[j], a[j], bi, carry);
            }
            let (sum, overflow) = t[4].overflowing_add(carry);
            t[4] = sum;
            t[5] = u64::from(overflow);

            let factor = t[0].wrapping_mul(self.inv);
            let (_, mut carry) = mac(t[0], factor, m[0], 0);
            for j in 1..4 {
                (t[j - 1], carry) = mac(t[j], factor, m[j], carry);
            }
            let (sum, overflow) = t[4].overflowing_add(carry);
            t[3] = sum;
            t[4] = t[5] + u64::from(overflow);
        }
        let mut result = [t[0], t[1], t[2], t[3]];
        if t[4] != 0 || !less_than(&result, m) {
            let mut borrow = false;
            for j in 0..4 {
                let (d1, b1) = result[j].overflowing_sub(m[j]);
                let (d2, b2) = d1.overflowing_sub(u64::from(borrow));
                result[j] = d2;
                borrow = b1 || b2;
            }
        }
        result
    }
}

/// `a + b * c + carry`, returned as `(low, high)` words.
#[inline(always)]
fn mac(a: u64, b: u64, c: u64, carry: u64) -> (u64, u64) {
    let wide = u128::from(a) + u128::from(b) * u128::from(c) + u128::from(carry);
    (wide as u64, (wide >> 64) as u64)
}

fn less_than(a: &[u64; 4], b: &[u64; 4]) -> bool {
    for i in (0..4).rev() {
        if a[i] != b[i] {
            return a[i] < b[i];
        }
    }
    false
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
            let mod_len = 1 + usize::try_from(seed % 32).unwrap();
            let base_len = usize::try_from((seed / 3) % 33).unwrap();
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
        // Unsupported shapes fall back.
        assert!(modexp(&[2], &[3], &[4]).is_none());
        assert!(modexp(&[2], &[3], &[0]).is_none());
        assert!(modexp(&[2], &[3], &[1u8; 33]).is_none());
        assert!(modexp(&[2u8; 33], &[3], &[7]).is_none());
    }
}
