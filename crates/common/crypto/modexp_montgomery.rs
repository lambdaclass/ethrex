//! Fixed-width MODEXP for odd moduli of at most 1024 bits.
//!
//! Most MODEXP calls use moduli of 128 bytes or less (field arithmetic helpers,
//! small RSA-free schemes). For those, Montgomery multiplication on 4, 8 or 16
//! 64-bit limbs with a 4-bit exponent window avoids the heap-allocated big
//! integers of the generic path and is several times faster.

use crate::safegcd::Modulus;

/// Returns `base^exp mod modulus` as `modulus.len()` big-endian bytes, or
/// `None` when the inputs are outside what this path handles (even or zero
/// modulus, modulus or base longer than 128 bytes).
pub(crate) fn modexp(base: &[u8], exp: &[u8], modulus: &[u8]) -> Option<Vec<u8>> {
    if let Some(inverse) = known_prime_inverse(base, exp, modulus) {
        return Some(inverse);
    }
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

/// Well-known 256-bit primes. For a prime `m`, Fermat's little theorem makes `base^(m - 2) mod m`
/// the inverse of `base` modulo `m` (and 0 when `m` divides `base`), and contracts commonly invert
/// field elements with exactly that MODEXP call: early in 2026, 98% of mainnet MODEXP calls were
/// inversions modulo the BN254 scalar field.
const KNOWN_PRIMES: [Modulus; 7] = [
    // BN254 scalar field.
    Modulus::new([
        0x43e1_f593_f000_0001,
        0x2833_e848_79b9_7091,
        0xb850_45b6_8181_585d,
        0x3064_4e72_e131_a029,
    ]),
    // BN254 base field.
    Modulus::new([
        0x3c20_8c16_d87c_fd47,
        0x9781_6a91_6871_ca8d,
        0xb850_45b6_8181_585d,
        0x3064_4e72_e131_a029,
    ]),
    // secp256k1 base field.
    Modulus::new([
        0xffff_fffe_ffff_fc2f,
        0xffff_ffff_ffff_ffff,
        0xffff_ffff_ffff_ffff,
        0xffff_ffff_ffff_ffff,
    ]),
    // secp256k1 group order.
    Modulus::new([
        0xbfd2_5e8c_d036_4141,
        0xbaae_dce6_af48_a03b,
        0xffff_ffff_ffff_fffe,
        0xffff_ffff_ffff_ffff,
    ]),
    // P-256 base field.
    Modulus::new([
        0xffff_ffff_ffff_ffff,
        0x0000_0000_ffff_ffff,
        0x0000_0000_0000_0000,
        0xffff_ffff_0000_0001,
    ]),
    // P-256 group order.
    Modulus::new([
        0xf3b9_cac2_fc63_2551,
        0xbce6_faad_a717_9e84,
        0xffff_ffff_ffff_ffff,
        0xffff_ffff_0000_0000,
    ]),
    // BLS12-381 scalar field.
    Modulus::new([
        0xffff_ffff_0000_0001,
        0x53bd_a402_fffe_5bfe,
        0x3339_d808_09a1_d805,
        0x73ed_a753_299d_7d48,
    ]),
];

/// `base^exp mod modulus` computed as an inversion when the modulus is one of [`KNOWN_PRIMES`] and
/// the exponent is the modulus minus 2, as `modulus.len()` big-endian bytes; `None` otherwise, or
/// when the base has more than 32 significant bytes. Leading zero bytes are allowed in every operand.
fn known_prime_inverse(base: &[u8], exp: &[u8], modulus: &[u8]) -> Option<Vec<u8>> {
    let modulus_digits = strip_leading_zeros(modulus);
    // Every known prime has 32 significant bytes.
    if modulus_digits.len() != 32 {
        return None;
    }
    let m = from_be_bytes::<4>(modulus_digits);
    let prime = KNOWN_PRIMES.iter().find(|prime| prime.limbs == m)?;
    let exp_digits = strip_leading_zeros(exp);
    let base_digits = strip_leading_zeros(base);
    if exp_digits.len() > 32 || base_digits.len() > 32 {
        return None;
    }
    let mut m_minus_2 = m;
    sub_assign(&mut m_minus_2, &limb_two());
    if from_be_bytes::<4>(exp_digits) != m_minus_2 {
        return None;
    }
    // Every known prime is above 2^253, so a 32-byte base is below 8m.
    let mut value = from_be_bytes::<4>(base_digits);
    while !less_than(&value, &m) {
        sub_assign(&mut value, &m);
    }
    let result = if value == [0; 4] {
        // 0^(m - 2) = 0.
        value
    } else {
        prime.inverse(value)
    };
    let mut out = vec![0u8; modulus.len()];
    for (i, byte) in out.iter_mut().rev().take(32).enumerate() {
        *byte = (result[i / 8] >> (8 * (i % 8))) as u8;
    }
    Some(out)
}

fn strip_leading_zeros(bytes: &[u8]) -> &[u8] {
    let first = bytes
        .iter()
        .position(|&byte| byte != 0)
        .unwrap_or(bytes.len());
    &bytes[first..]
}

fn limb_two() -> [u64; 4] {
    [2, 0, 0, 0]
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
        let mut mont = Self {
            modulus,
            inv: inv.wrapping_neg(),
            r2: [0; N],
        };
        // R^2 mod m, with R = 2^(64 * N): start from 2 in Montgomery form, 2R mod m, and square
        // it log2(64 * N) times. Each Montgomery squaring maps the form of x to the form of x^2,
        // so the result is the form of 2^(64 * N) = R, which is R^2 mod m.
        let mut x = mont.r_mod_m();
        double_mod(&mut x, &modulus);
        for _ in 0..(64 * N).trailing_zeros() {
            x = mont.mul(&x, &x);
        }
        mont.r2 = x;
        mont
    }

    /// `R mod m`. `m` shifted left by its leading zero bits, `ms`, is at least `R / 2`, so
    /// `R mod ms` is `R - ms` reduced once; each halving of `ms` back to `m` then needs one
    /// conditional subtraction. For a modulus that fills its limbs that is a couple of steps,
    /// where doubling 1 up to `R` would take `64 * N`.
    fn r_mod_m(&self) -> [u64; N] {
        let m = &self.modulus;
        let leading_zeros = leading_zeros(m);
        let mut shifted = *m;
        shift_left(&mut shifted, leading_zeros);
        // R - shifted, as the two's complement of `shifted` in N limbs.
        let mut x = [0u64; N];
        sub_assign(&mut x, &shifted);
        if !less_than(&x, &shifted) {
            sub_assign(&mut x, &shifted);
        }
        for _ in 0..leading_zeros {
            shift_right_one(&mut shifted);
            if !less_than(&x, &shifted) {
                sub_assign(&mut x, &shifted);
            }
        }
        x
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

/// Leading zero bits of `a` as a `64 * N`-bit number.
fn leading_zeros<const N: usize>(a: &[u64; N]) -> u32 {
    let mut zeros = 0;
    for &limb in a.iter().rev() {
        zeros += limb.leading_zeros();
        if limb != 0 {
            break;
        }
    }
    zeros
}

/// `a <<= bits` for `bits < 64 * N`, dropping the bits shifted out.
fn shift_left<const N: usize>(a: &mut [u64; N], bits: u32) {
    let (words, bits) = ((bits / 64) as usize, bits % 64);
    for i in (0..N).rev() {
        let high = if i >= words { a[i - words] } else { 0 };
        let low = if i > words { a[i - words - 1] } else { 0 };
        a[i] = if bits == 0 {
            high
        } else {
            high << bits | low >> (64 - bits)
        };
    }
}

fn shift_right_one<const N: usize>(a: &mut [u64; N]) {
    for i in 0..N - 1 {
        a[i] = a[i] >> 1 | a[i + 1] << 63;
    }
    a[N - 1] >>= 1;
}

/// `a = 2a mod m` for `a < m`.
fn double_mod<const N: usize>(a: &mut [u64; N], m: &[u64; N]) {
    let carry = a[N - 1] >> 63;
    for i in (1..N).rev() {
        a[i] = a[i] << 1 | a[i - 1] >> 63;
    }
    a[0] <<= 1;
    if carry != 0 || !less_than(a, m) {
        sub_assign(a, m);
    }
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

    fn r2_matches_reference<const N: usize>() {
        for bits in 2..=64 * N {
            for seed in 0..3u64 {
                // An odd modulus of exactly `bits` bits.
                let mut modulus = bytes(seed + 10_000 * bits as u64, 8 * N);
                let full_bytes = 8 * N - bits.div_ceil(8);
                modulus[..full_bytes].fill(0);
                modulus[full_bytes] &= 0xff >> ((8 - bits % 8) % 8);
                modulus[full_bytes] |= 0x80 >> ((8 - bits % 8) % 8);
                *modulus.last_mut().unwrap() |= 1;
                let mont = Montgomery::<N>::new(from_be_bytes(&modulus));
                let m =
                    Natural::from_power_of_2_digits_desc(8u64, modulus.iter().cloned()).unwrap();
                let expected = (Natural::from(1u8) << (128 * N as u64)) % &m;
                let got =
                    Natural::from_power_of_2_digits_desc(64u64, mont.r2.iter().rev().cloned())
                        .unwrap();
                assert_eq!(got, expected, "N {N}, {bits}-bit modulus, seed {seed}");
            }
        }
    }

    #[test]
    fn r2_matches_reference_for_every_modulus_length() {
        r2_matches_reference::<4>();
        r2_matches_reference::<8>();
        r2_matches_reference::<16>();
    }

    fn limbs_to_be(limbs: &[u64; 4]) -> Vec<u8> {
        limbs
            .iter()
            .rev()
            .flat_map(|limb| limb.to_be_bytes())
            .collect()
    }

    #[test]
    fn known_primes_are_their_published_constants_and_probable_primes() {
        let published = [
            "30644e72e131a029b85045b68181585d2833e84879b9709143e1f593f0000001",
            "30644e72e131a029b85045b68181585d97816a916871ca8d3c208c16d87cfd47",
            "fffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2f",
            "fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141",
            "ffffffff00000001000000000000000000000000ffffffffffffffffffffffff",
            "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551",
            "73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001",
        ];
        for (prime, hex) in KNOWN_PRIMES.iter().zip(published) {
            let bytes = limbs_to_be(&prime.limbs);
            let hex_bytes: Vec<u8> = (0..32)
                .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
                .collect();
            assert_eq!(bytes, hex_bytes);
            let m = Natural::from_power_of_2_digits_desc(8u64, bytes.iter().cloned()).unwrap();
            let m_minus_1 = &m - Natural::from(1u8);
            for a in [2u8, 3, 5, 7, 11, 13] {
                assert_eq!(
                    Natural::from(a).mod_pow(&m_minus_1, &m),
                    Natural::from(1u8),
                    "{hex}"
                );
            }
        }
    }

    #[test]
    fn known_prime_inverse_matches_reference() {
        for (k, prime) in KNOWN_PRIMES.iter().enumerate() {
            let m = limbs_to_be(&prime.limbs);
            let mut m_minus_2 = prime.limbs;
            sub_assign(&mut m_minus_2, &limb_two());
            let exp = limbs_to_be(&m_minus_2);
            let mut bases: Vec<Vec<u8>> = (0..64)
                .map(|seed| bytes(seed + 70_000 + 100 * k as u64, 32))
                .collect();
            let m_nat = Natural::from_power_of_2_digits_desc(8u64, m.iter().cloned()).unwrap();
            // 0, m, 2m and 7m: multiples of m, whose MODEXP is 0.
            for multiple in [0u8, 1, 2, 7] {
                let v: Vec<u8> = (&m_nat * Natural::from(multiple)).to_power_of_2_digits_desc(8);
                if v.len() <= 32 {
                    bases.push(v);
                }
            }
            bases.push(vec![]);
            bases.push(vec![1]);
            bases.push(vec![0xff; 32]);
            bases.push({
                let mut v = m.clone();
                *v.last_mut().unwrap() -= 1;
                v
            });
            bases.push({
                let mut v = m.clone();
                *v.last_mut().unwrap() += 1;
                v
            });
            for base in &bases {
                for (pad_b, pad_e, pad_m) in
                    [(0usize, 0usize, 0usize), (3, 1, 0), (0, 0, 32), (1, 5, 2)]
                {
                    let b = [vec![0u8; pad_b], base.clone()].concat();
                    let e = [vec![0u8; pad_e], exp.clone()].concat();
                    let md = [vec![0u8; pad_m], m.clone()].concat();
                    assert!(known_prime_inverse(&b, &e, &md).is_some(), "prime {k}");
                    assert_eq!(
                        modexp(&b, &e, &md),
                        Some(reference(&b, &e, &md)),
                        "prime {k}"
                    );
                }
            }
            // Other exponents and over-wide bases take the general path.
            let mut m_minus_1 = prime.limbs;
            sub_assign(&mut m_minus_1, &limb_one());
            let mut m_minus_3 = m_minus_2;
            sub_assign(&mut m_minus_3, &limb_one());
            for other in [
                limbs_to_be(&m_minus_1),
                limbs_to_be(&m_minus_3),
                [vec![1u8], exp.clone()].concat(),
            ] {
                assert!(known_prime_inverse(&bases[0], &other, &m).is_none());
            }
            assert!(
                known_prime_inverse(&[vec![1u8], bases[0].clone()].concat(), &exp, &m).is_none()
            );
            let mut not_prime = m.clone();
            *not_prime.last_mut().unwrap() ^= 2;
            assert!(known_prime_inverse(&bases[0], &exp, &not_prime).is_none());
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
