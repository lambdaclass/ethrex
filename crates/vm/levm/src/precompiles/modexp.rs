#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::indexing_slicing,
    clippy::unwrap_used
)]

use malachite::{
    Natural,
    base::num::{
        arithmetic::traits::{ModInverse, Parity},
        basic::traits::{One, Zero},
        logic::traits::{BitAccess, BitBlockAccess, CountOnes, SignificantBits},
    },
};
use std::cmp::Ordering;

#[inline(always)]
pub fn modexp(base: Natural, exponent: Natural, modulo: Natural) -> Natural {
    // Special cases.
    if modulo == Natural::ZERO || modulo == Natural::ONE {
        return Natural::ZERO;
    } else if exponent == Natural::ZERO {
        return Natural::ONE;
    } else if exponent == Natural::ONE || base == Natural::ZERO || base == Natural::ONE {
        return if base >= modulo { base % modulo } else { base };
    }

    // Backend selection.
    if modulo.count_ones() == 1 {
        modexp_impl::<PowerOfTwoBackend>(base.clone(), exponent.clone(), modulo.clone())
    } else if modulo.odd() {
        modexp_impl::<MontgomeryBackend>(base.clone(), exponent.clone(), modulo.clone())
    } else {
        modexp_impl::<FallbackBackend>(base.clone(), exponent.clone(), modulo.clone())
    }
}

trait BackendApi {
    fn new(modulo: Natural, k: u64) -> Self;

    fn reduce(&self, value: Natural) -> Natural;

    fn convert_into_internal_representation(&self, value: Natural) -> Natural;
    fn convert_from_internal_representation(&self, value: Natural) -> Natural;
}

fn modexp_impl<B>(base: Natural, exponent: Natural, modulo: Natural) -> Natural
where
    B: BackendApi,
{
    // Compute parameters `k` and `l`.
    let k = find_optimal_k(&exponent);
    let l = exponent.significant_bits();

    let backend = B::new(modulo, k);
    let base = backend.convert_into_internal_representation(base);

    // Compute precalculated powers.
    //   x³, x⁵..., x^i where i<=2^k-1
    let precomputed_powers = {
        let data_len = 1usize << (k - 1);
        let mut data = Vec::with_capacity(data_len);

        let base_squared = backend.reduce(&base * &base);

        let mut acc = base.clone();
        data.push(base); // Push x¹.
        for _ in 1..data_len {
            acc *= &base_squared;
            acc = backend.reduce(acc);
            data.push(acc.clone());
        }

        data.into_boxed_slice()
    };

    // Run the sliding window algorithm.
    let mut y = backend.convert_into_internal_representation(Natural::ONE);
    let mut i = l - 1;
    loop {
        if !exponent.get_bit(i) {
            y = &y * &y;
            y = backend.reduce(y);

            i = match i.checked_sub(1) {
                Some(x) => x,
                None => break,
            };
        } else {
            let mut s = (i + 1).saturating_sub(k);
            s += (&exponent >> s).trailing_zeros().unwrap();
            for _ in 0..=i - s {
                y = &y * &y;
                y = backend.reduce(y);
            }
            // y *= &precomputed_powers[((&exponent >> (s + 1)).iter_u64_digits().next().unwrap()
            //     & ((1u64 << (i - s)) - 1)) as usize];
            y *= &precomputed_powers[exponent
                .get_bits(s + 1, i + 1)
                .limbs()
                .next()
                .unwrap_or_default() as usize];
            y = backend.reduce(y);
            i = match s.checked_sub(1) {
                Some(x) => x,
                None => break,
            };
        }
    }

    backend.convert_from_internal_representation(y)
}

fn find_optimal_k(exponent: &Natural) -> u64 {
    let exponent_bits = exponent.significant_bits();

    // Will overflow:
    //   - An `f64`'s significand for `k > 22`.
    //   - An `u64` for `k > 27`.
    //   - An `u128` for `k > 58`.
    //
    // Limited to 22 for practical and performance reasons:
    //   - A `k = 22` will generate a precompile list of 48MiB, excluding the internal allocations
    //     of the values.
    //   - Limiting it to 22 will allow us to operate fast and efficiently within `f64` boundaries.
    for k in 1u64..=22 {
        let num = k * (k + 1) * (2 << k);
        let den = (1 << (k + 1)) - k - 2;

        if (num as f64 / den as f64 + 1.0).round() as u64 >= exponent_bits {
            return k;
        }
    }

    16
}

pub struct PowerOfTwoBackend {
    mask: Natural,
}

impl BackendApi for PowerOfTwoBackend {
    fn new(modulo: Natural, _: u64) -> Self {
        Self {
            mask: modulo - Natural::ONE,
        }
    }

    fn reduce(&self, value: Natural) -> Natural {
        value & &self.mask
    }

    fn convert_into_internal_representation(&self, value: Natural) -> Natural {
        value & &self.mask
    }

    fn convert_from_internal_representation(&self, value: Natural) -> Natural {
        value
    }
}

pub struct MontgomeryBackend {
    n: Natural,
    r: Natural,

    np: Natural,
    rp: Natural,

    r_bits: u64,
    r_mask: Natural,
}

impl BackendApi for MontgomeryBackend {
    fn new(modulo: Natural, k: u64) -> Self {
        let n = modulo;
        let r_bits = n.significant_bits() + k;
        let r = Natural::ONE << r_bits;

        let rp = (&r % &n).mod_inverse(&n).unwrap();
        let np = (&r * &rp - Natural::ONE) / &n;

        let r_mask = &r - Natural::ONE;
        Self {
            n,
            r,
            np,
            rp,
            r_bits,
            r_mask,
        }
    }

    fn reduce(&self, value: Natural) -> Natural {
        let m = ((&value & &self.r_mask) * &self.np) & &self.r_mask;
        let t = (value + m * &self.n) >> self.r_bits;
        if t >= self.n { t - &self.n } else { t }
    }

    fn convert_into_internal_representation(&self, value: Natural) -> Natural {
        value * &self.r % &self.n
    }

    fn convert_from_internal_representation(&self, value: Natural) -> Natural {
        value * &self.rp % &self.n
    }
}

pub struct FallbackBackend {
    modulo: Natural,

    num_bits: u64,
    constant: Natural,
}

impl BackendApi for FallbackBackend {
    fn new(modulo: Natural, k: u64) -> Self {
        let num_bits = 2 * (k + modulo.significant_bits());
        let constant = (Natural::ONE << num_bits) / &modulo;

        Self {
            modulo,
            num_bits,
            constant,
        }
    }

    fn reduce(&self, value: Natural) -> Natural {
        let quotient = (&value * &self.constant) >> self.num_bits;
        let delta = quotient * &self.modulo;
        value - delta
    }

    fn convert_into_internal_representation(&self, value: Natural) -> Natural {
        match value.cmp(&self.modulo) {
            Ordering::Less => value,
            Ordering::Equal => Natural::ZERO,
            Ordering::Greater => value % &self.modulo,
        }
    }

    fn convert_from_internal_representation(&self, value: Natural) -> Natural {
        value
    }
}
