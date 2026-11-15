//! Division-based arithmetic on 256-bit words, with the EVM's convention that dividing by
//! zero yields zero: `DIV`, `MOD`, `ADDMOD` and `MULMOD`.
//!
//! Long division (Knuth's algorithm D) over fixed-size word arrays. Each quotient digit
//! comes from one 128-by-64-bit hardware division on x86-64 (a single `div`); `u128`
//! operators would instead call a software routine that branches on the operand sizes,
//! once for the quotient and once more for the remainder. The worst inputs are divisors
//! just above a word boundary, which leave the most digits per division.

use ethereum_types::U256;

/// Numerator words of the widest division: the 512-bit `MULMOD` product.
const MAX_NUMERATOR_WORDS: usize = 8;

#[inline(always)]
fn umul(a: u64, b: u64) -> u128 {
    u128::from(a) * u128::from(b)
}

#[inline(always)]
fn join(high: u64, low: u64) -> u128 {
    (u128::from(high) << 64) | u128::from(low)
}

/// `(high:low) / d` and the remainder. Requires `high < d`, so the quotient fits a word.
#[inline(always)]
fn div_2by1(high: u64, low: u64, d: u64) -> (u64, u64) {
    debug_assert!(high < d);
    #[cfg(target_arch = "x86_64")]
    {
        let (quotient, remainder): (u64, u64);
        // SAFETY: `div` only touches the named registers. It faults when the quotient
        // overflows 64 bits, which `high < d` rules out.
        unsafe {
            core::arch::asm!(
                "div {d}",
                d = in(reg) d,
                inout("rax") low => quotient,
                inout("rdx") high => remainder,
                options(pure, nomem, nostack),
            );
        }
        (quotient, remainder)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let quotient = (join(high, low) / u128::from(d)) as u64;
        (quotient, low.wrapping_sub(quotient.wrapping_mul(d)))
    }
}

/// Divides `(u2:u1:u0)` by the normalized two-word `(d1:d0)`, where `(u2:u1) < (d1:d0)`:
/// the exact quotient digit and the remainder.
///
/// The digit estimated from the top words alone is at most two too large; subtracting it
/// times `d0` from the partial remainder and adding the divisor back while that is
/// negative fixes both.
#[inline(always)]
fn div_3by2(u2: u64, u1: u64, u0: u64, d1: u64, d0: u64) -> (u64, u128) {
    let d = join(d1, d0);
    if u2 < d1 {
        let (mut q, r1) = div_2by1(u2, u1, d1);
        let (mut r, negative) = join(r1, u0).overflowing_sub(umul(q, d0));
        if negative {
            q -= 1;
            let (r2, nonnegative) = r.overflowing_add(d);
            r = r2;
            if !nonnegative {
                q -= 1;
                r = r.wrapping_add(d);
            }
        }
        (q, r)
    } else {
        // u2 == d1, so u1 < d0 and the digit is 2^64 - 1 or 2^64 - 2. With q = 2^64 - 1
        // the remainder is (u1:u0) + d - (d0:0), which is negative exactly when the
        // subtraction borrows without the addition carrying.
        let (x, carry) = join(u1, u0).overflowing_add(d);
        let (r, borrow) = x.overflowing_sub(join(d0, 0));
        if borrow && !carry {
            (u64::MAX - 1, r.wrapping_add(d))
        } else {
            (u64::MAX, r)
        }
    }
}

/// `x -= y * multiplier` over the words of `y`; returns the borrow out of the top word.
#[inline(always)]
fn submul(x: &mut [u64], y: &[u64], multiplier: u64) -> u64 {
    let mut borrow = 0u64;
    for (x, &y) in x.iter_mut().zip(y) {
        let (s, b1) = x.overflowing_sub(borrow);
        let p = umul(y, multiplier);
        let (r, b2) = s.overflowing_sub(p as u64);
        *x = r;
        borrow = (p >> 64) as u64 + u64::from(b1) + u64::from(b2);
    }
    borrow
}

/// `x += y` over the words of `y`, dropping the carry out of the top word.
#[inline(always)]
fn add_assign(x: &mut [u64], y: &[u64]) {
    let mut carry = false;
    for (x, &y) in x.iter_mut().zip(y) {
        let (s, c1) = x.overflowing_add(y);
        let (s, c2) = s.overflowing_add(u64::from(carry));
        *x = s;
        carry = c1 | c2;
    }
}

/// Quotient and remainder of the `N`-word `numerator` by a non-zero `divisor`.
///
/// Operands, lengths and results are handled as scalars at constant indexes so they stay
/// in registers: words stored one at a time and read back as one wide load (or the
/// reverse) stall the CPU, which costs as much as a digit step.
#[inline(always)]
fn udivrem<const N: usize>(numerator: [u64; N], divisor: [u64; 4]) -> ([u64; N], U256) {
    const { assert!(N >= 4 && N <= MAX_NUMERATOR_WORDS) };

    let (n, top) = if divisor[3] != 0 {
        (4, divisor[3])
    } else if divisor[2] != 0 {
        (3, divisor[2])
    } else if divisor[1] != 0 {
        (2, divisor[1])
    } else {
        (1, divisor[0])
    };
    let mut m = 0;
    for (i, &word) in numerator.iter().enumerate() {
        if word != 0 {
            m = i + 1;
        }
    }
    // Below the divisor, the numerator fits in its low four words.
    let small = U256([numerator[0], numerator[1], numerator[2], numerator[3]]);
    let mut quotient = [0u64; N];
    if m < n {
        return (quotient, small);
    }
    // Writes digit `j` without indexing by `j`, so the quotient stays in registers.
    let set_digit = |quotient: &mut [u64; N], j: usize, q: u64| {
        for (k, digit) in quotient.iter_mut().enumerate() {
            if k == j {
                *digit = q;
            }
        }
    };

    if n == 1 {
        // The running remainder stays below a one-word divisor, so each digit is a
        // single division and no normalization is needed.
        let mut r = 0;
        for j in (0..N).rev() {
            if j < m {
                let (q, rest) = div_2by1(r, numerator[j], top);
                quotient[j] = q;
                r = rest;
            }
        }
        return (quotient, U256([r, 0, 0, 0]));
    }

    // Shift both operands left until the divisor's top bit is set, which keeps each
    // digit estimate within two of the true digit. The numerator gains a word;
    // `(x >> 1) >> (63 - s)` is `x >> (64 - s)` that also yields 0 when s == 0.
    let shift = top.leading_zeros();
    let mut dn = [0u64; 4];
    dn[0] = divisor[0] << shift;
    for i in 1..4 {
        dn[i] = (divisor[i] << shift) | ((divisor[i - 1] >> 1) >> (63 - shift));
    }
    let mut un = [0u64; MAX_NUMERATOR_WORDS + 1];
    un[0] = numerator[0] << shift;
    for i in 1..N {
        un[i] = (numerator[i] << shift) | ((numerator[i - 1] >> 1) >> (63 - shift));
    }
    un[N] = (numerator[N - 1] >> 1) >> (63 - shift);

    // Take the extra word when it is non-zero or the top word does not fit below the
    // divisor's, so the first window's top words are below the divisor.
    if un[m] != 0 || un[m - 1] >= dn[n - 1] {
        m += 1;
    }
    if m <= n {
        return (quotient, small);
    }

    let (d1, d0) = (dn[n - 1], dn[n - 2]);
    if n == 2 {
        let mut r = join(un[m - 1], un[m - 2]);
        for j in (0..m - 2).rev() {
            let (q, rest) = div_3by2((r >> 64) as u64, r as u64, un[j], d1, d0);
            set_digit(&mut quotient, j, q);
            r = rest;
        }
        let r = r >> shift;
        return (quotient, U256([r as u64, (r >> 64) as u64, 0, 0]));
    }

    for j in (0..m - n).rev() {
        let (u2, u1) = (un[j + n], un[j + n - 1]);
        if u2 == d1 && u1 == d0 {
            // The 3-by-2 digit would not fit a word. The true digit is 2^64 - 1, and
            // subtracting that many divisors clears the window's top word.
            submul(&mut un[j..j + n], &dn[..n], u64::MAX);
            set_digit(&mut quotient, j, u64::MAX);
            continue;
        }
        // The 3-by-2 step yields the window's top two remainder words; the rest of the
        // divisor can make the digit one too large, which adding it back corrects.
        let (mut q, r) = div_3by2(u2, u1, un[j + n - 2], d1, d0);
        let borrow = submul(&mut un[j..j + n - 2], &dn[..n - 2], q);
        let (r, negative) = r.overflowing_sub(u128::from(borrow));
        un[j + n - 2] = r as u64;
        un[j + n - 1] = (r >> 64) as u64;
        if negative {
            q -= 1;
            add_assign(&mut un[j..j + n], &dn[..n]);
        }
        set_digit(&mut quotient, j, q);
    }
    // The remainder is the low `n` words shifted back; words from `un[n]` up are stale.
    let word = |i: usize| {
        let high = if i + 1 < n {
            (un[i + 1] << 1) << (63 - shift)
        } else {
            0
        };
        if i < n { (un[i] >> shift) | high } else { 0 }
    };
    (quotient, U256([word(0), word(1), word(2), word(3)]))
}

/// `numerator / divisor`, or zero when `divisor` is zero.
#[inline(always)]
pub fn div(numerator: U256, divisor: U256) -> U256 {
    if divisor.is_zero() {
        return U256::zero();
    }
    let q = udivrem(numerator.0, divisor.0).0;
    U256([q[0], q[1], q[2], q[3]])
}

/// `numerator % divisor`, or zero when `divisor` is zero.
#[inline(always)]
pub fn rem(numerator: U256, divisor: U256) -> U256 {
    if divisor.is_zero() {
        return U256::zero();
    }
    udivrem(numerator.0, divisor.0).1
}

/// `(a + b) % m` without wrapping the sum, or zero when `m` is zero.
#[inline(always)]
pub fn add_mod(a: U256, b: U256, m: U256) -> U256 {
    if m.is_zero() {
        return U256::zero();
    }
    let (sum, carry) = a.overflowing_add(b);
    // Reduced operands (the usual case in field arithmetic) need at most one subtraction.
    if a < m && b < m {
        return if carry || sum >= m {
            sum.overflowing_sub(m).0
        } else {
            sum
        };
    }
    let s = sum.0;
    let numerator = [s[0], s[1], s[2], s[3], u64::from(carry)];
    udivrem(numerator, m.0).1
}

/// `(a * b) % m` over the full 512-bit product, or zero when `m` is zero.
#[inline(always)]
pub fn mul_mod(a: U256, b: U256, m: U256) -> U256 {
    if m.is_zero() {
        return U256::zero();
    }
    let (a, b) = (a.0, b.0);
    let mut product = [0u64; 8];
    for i in 0..4 {
        let mut carry = 0u64;
        for j in 0..4 {
            let t = umul(a[i], b[j]) + u128::from(product[i + j]) + u128::from(carry);
            product[i + j] = t as u64;
            carry = (t >> 64) as u64;
        }
        product[i + 4] = carry;
    }
    udivrem(product, m.0).1
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::U512;

    /// SplitMix64: deterministic operands without a dev-dependency.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }

        /// A word with a random bit length, biased towards values next to word
        /// boundaries and all-ones runs, where digit estimates need corrections.
        fn word(&mut self) -> U256 {
            let bits = (self.next() % 257) as usize;
            let mut x = U256([self.next(), self.next(), self.next(), self.next()]);
            match self.next() % 4 {
                0 => x = !U256::zero() ^ (U256::from(self.next() % 4) << (self.next() % 256)),
                1 => x = U256::one() << (self.next() % 256),
                _ => {}
            }
            if bits < 256 {
                x &= (U256::one() << bits) - 1;
            }
            if self.next().is_multiple_of(8) {
                x = x.overflowing_add(U256::from(self.next() % 3)).0;
            }
            x
        }
    }

    fn truncate(x: U512) -> U256 {
        U256([x.0[0], x.0[1], x.0[2], x.0[3]])
    }

    /// Checks `div` and `rem` against the generic long division of the `uint` crate.
    fn check_div_rem(a: U256, b: U256) {
        let (q, r) = if b.is_zero() {
            (U256::zero(), U256::zero())
        } else {
            a.div_mod(b)
        };
        assert_eq!(div(a, b), q, "{a:#x} / {b:#x}");
        assert_eq!(rem(a, b), r, "{a:#x} % {b:#x}");
    }

    #[test]
    fn div_2by1_matches_wide_division() {
        let mut rng = Rng(1);
        for _ in 0..100_000 {
            let d = rng.next().max(1);
            let (high, low) = (rng.next() % d, rng.next());
            let n = join(high, low);
            let expected = ((n / u128::from(d)) as u64, (n % u128::from(d)) as u64);
            assert_eq!(div_2by1(high, low, d), expected, "{n:#x} / {d:#x}");
        }
        assert_eq!(
            div_2by1(u64::MAX - 1, u64::MAX, u64::MAX),
            (u64::MAX, u64::MAX - 1)
        );
    }

    #[test]
    fn div_rem_matches_reference() {
        let mut rng = Rng(2);
        for _ in 0..200_000 {
            check_div_rem(rng.word(), rng.word());
        }
    }

    #[test]
    fn div_rem_edge_cases() {
        let max = U256::MAX;
        let operands = [
            U256::zero(),
            U256::one(),
            U256::from(6),
            U256::from(7),
            U256::from(u64::MAX),
            U256::one() << 64,
            (U256::one() << 64) + 0x33,
            (U256::one() << 128) - 1,
            (U256::one() << 128) + 0x33,
            (U256::one() << 192) - 1,
            U256::one() << 255,
            (U256::one() << 255) + 1,
            max - 1,
            max,
        ];
        for a in operands {
            for b in operands {
                check_div_rem(a, b);
            }
        }
    }

    /// Published long-division vectors aimed at the rare correction steps: a digit
    /// estimate one too large, and a 3-by-2 digit that overflows a word.
    #[test]
    fn wide_division_vectors() {
        let vectors = [
            ("2", "1"),
            ("10000000000000000", "2"),
            ("7000000000000000", "8000000000000000"),
            ("8000000000000000", "8000000000000000"),
            ("8000000000000001", "8000000000000000"),
            (
                "80000000000000010000000000000000",
                "80000000000000000000000000000000",
            ),
            (
                "80000000000000000000000000000000",
                "80000000000000000000000000000001",
            ),
            ("478392145435897052", "111"),
            (
                "400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
                "800000000000000000000000000000000000000000000000",
            ),
            (
                "80000000000000000000000000000000000000000000000000000000000000000000000000000000",
                "800000000000000000000000000000000000000000000000",
            ),
            (
                "1e00000000000000000000090000000000000000000000000000000000000000000000000000000000000000000000000000000009000000000000000000",
                "a",
            ),
            (
                "767676767676767676000000767676767676",
                "2900760076761e00020076760000000076767676000000",
            ),
            ("12121212121212121212121212121212", "232323232323232323"),
            (
                "abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0abc0",
                "1c01c01c01c01c01c01c01c01c",
            ),
            (
                "fffff716b61616160b0b0b2b0b0b0becf4bef50a0df4f48b090b2b0bc60a0a00",
                "fffff716b61616160b0b0b2b0b230b000008010d0a2b00",
            ),
            (
                "50beb1c60141a0000dc2b0b0b0b0b0b410a0a0df4f40b090b2b0bc60a0a00",
                "2000110000000d0a300e750a000000090a0a",
            ),
            (
                "4b00000b41000b0b0b2b0b0b0b0b0b410a0aeff4f40b090b2b0bc60a0a1000",
                "4b00000b41000b0b0b2b0b0b0b0b0b410a0aeff4f40b0a0a",
            ),
            (
                "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "7",
            ),
            (
                "f6376770abd3a36b20394c5664afef1194c801c3f05e42566f085ed24d002bb0",
                "b368d219438b7f3f",
            ),
            ("0", "10900000000000000000000000000000000000000000000000000"),
            (
                "767676767676000000000076000000000000005600000000000000000000",
                "767676767676000000000076000000760000",
            ),
            (
                "8200000000000000000000000000000000000000000000000000000000000000",
                "8200000000000000fe000004000000ffff000000fffff700",
            ),
            ("dac7fff9ffd9e1322626262626262600", "d021262626262626"),
            (
                "8000000000000001800000000000000080000000000000008000000000000000",
                "800000000000000080000000000000008000000000000000",
            ),
            (
                "40000000fd000000db00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001",
                "40000000fd000000db0000000000000000000040000000fd000000db000001",
            ),
            (
                "40000000fd000000db00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001",
                "40000000fd000000db0000000000000000000040000000fd000000db0000d3",
            ),
            (
                "1f000000000000000000000000000000200000000100000000000000000000",
                "100000000ffffffffffffffff0000000000002e000000",
            ),
            (
                "7effffff80000000000000000000000000020000440000000000000000000001",
                "7effffff800000007effffff800000008000ff0000010000",
            ),
            (
                "5fd8fffffffffffffffffffffffffffffc090000ce700004d0c9ffffff000001",
                "2ffffffffffffffffffffffffffffffffff000000030000",
            ),
            (
                "62d8fffffffffffffffffffffffffffffc18000000000000000000ca00000001",
                "2ffffffffffffffffffffffffffffffffff200000000000",
            ),
            (
                "7effffff8000000000000000000000000000000000000000d900000000000001",
                "7effffff8000000000000000000000000000000000008001",
            ),
            (
                "6400aff20ff00200004e7fd1eff08ffca0afd1eff08ffca0a",
                "210000000000000022",
            ),
            (
                "7effffff8000000000000000000000000000000000150000d900000000000001",
                "7effffff8000000000000000000000000000000000f9e101",
            ),
            (
                "ffffffff0100000000000000000000000000ffff0000ffffffff0100000000",
                "ffffffff010000000000000000000000ffff0000ffffff",
            ),
            (
                "ff00ffffffffffffffcaffffffff0100",
                "100000000000000ff800000000000ff",
            ),
            (
                "9000ffffffffffffffcaffffffff0100",
                "800000000000007fc000000000007f80",
            ),
            (
                "6e2d23924d38f0ab643864e9b2a328a54914f48533114fae3475168bfd74a61ae91e676b4a4f33a5b3b6cc189536ccb4afc46d02b061d6daaf0298c993376ab4",
                "80000000001000008000020000200001",
            ),
        ];
        for (numerator, divisor) in vectors {
            let numerator = U512::from_str_radix(numerator, 16).unwrap();
            let divisor = U256::from_str_radix(divisor, 16).unwrap();
            let (q, r) = numerator.div_mod(U512::from(divisor));
            let (quotient, remainder) = udivrem(numerator.0, divisor.0);
            assert_eq!(U512(quotient), q, "{numerator:#x} / {divisor:#x}");
            assert_eq!(remainder, truncate(r), "{numerator:#x} % {divisor:#x}");
        }
    }

    #[test]
    fn add_mod_matches_reference() {
        let mut rng = Rng(3);
        for _ in 0..200_000 {
            let (a, b, m) = (rng.word(), rng.word(), rng.word());
            let expected = if m.is_zero() {
                U256::zero()
            } else {
                truncate((U512::from(a) + U512::from(b)) % U512::from(m))
            };
            assert_eq!(
                add_mod(a, b, m),
                expected,
                "a = {a:#x}, b = {b:#x}, m = {m:#x}"
            );
        }
        let max = U256::MAX;
        assert_eq!(add_mod(max, max, max), U256::zero());
        assert_eq!(add_mod(max - 1, max - 1, max), max - 2);
        assert_eq!(add_mod(max, max, U256::one()), U256::zero());
        assert_eq!(add_mod(max, U256::one(), U256::zero()), U256::zero());
    }

    #[test]
    fn mul_mod_matches_reference() {
        let mut rng = Rng(4);
        for _ in 0..200_000 {
            let (a, b, m) = (rng.word(), rng.word(), rng.word());
            let expected = if m.is_zero() {
                U256::zero()
            } else {
                truncate(a.full_mul(b) % U512::from(m))
            };
            assert_eq!(
                mul_mod(a, b, m),
                expected,
                "a = {a:#x}, b = {b:#x}, m = {m:#x}"
            );
        }
        let max = U256::MAX;
        assert_eq!(mul_mod(max, max, max), U256::zero());
        assert_eq!(mul_mod(max, max, max - 1), U256::one());
        assert_eq!(mul_mod(max, max, U256::one()), U256::zero());
        assert_eq!(mul_mod(max, U256::zero(), U256::from(7)), U256::zero());
        assert_eq!(mul_mod(max, max, U256::zero()), U256::zero());
    }
}
