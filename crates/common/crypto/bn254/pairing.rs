//! Optimal ate multi-pairing check for ECPAIRING.
//!
//! It computes the same values as the `ark-ec` BN pairing: the Miller loop uses the
//! same line formulas in homogeneous projective coordinates and the same loop over
//! `6 x0 + 2`, so its output matches `Bn254::multi_miller_loop` exactly, and the
//! final exponentiation follows the same chain. The differences only change the
//! cost:
//! - each line is evaluated as soon as it is computed, instead of first storing
//!   every line of every pair in a vector (`G2Prepared`);
//! - all pairs share one accumulator. Without the `parallel` feature, arkworks runs
//!   one Miller loop per group of four pairs and multiplies the results, which
//!   repeats the 64 squarings of the accumulator for every group;
//! - lines are multiplied into the accumulator two at a time;
//! - the exponentiations by `x0` in the final exponentiation use an addition chain.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use ark_bn254::{Fq, Fq2, Fq6, Fq12, G1Affine, G2Affine};
use ark_ec::bn::BnConfig;
use ark_ec::short_weierstrass::SWCurveConfig;
use ark_ff::{
    AdditiveGroup, BigInteger, CyclotomicMultSubgroup, Field, Fp6Config, Fp12Config, One,
    PrimeField,
};

/// `6 x0 + 2` in NAF, least significant digit first.
const ATE_LOOP_COUNT: &[i8] = <ark_bn254::Config as BnConfig>::ATE_LOOP_COUNT;

/// `x / 2`. Montgomery form is linear, so halving the representative halves the
/// value: add the (odd) modulus first when the representative is odd. Both are
/// below 2^255, so the sum fits in four limbs.
fn halve(x: Fq2) -> Fq2 {
    let halve_fq = |value: Fq| {
        let mut repr = value.0;
        if repr.is_odd() {
            repr.add_with_carry(&Fq::MODULUS);
        }
        repr.div2();
        Fq::new_unchecked(repr)
    };
    Fq2::new(halve_fq(x.c0), halve_fq(x.c1))
}

/// A line `c0 + (c1 + c2 v) w` before its evaluation at a G1 point.
type Line = (Fq2, Fq2, Fq2);

/// A G2 point in homogeneous projective coordinates `(X/Z, Y/Z)`.
struct G2Projective {
    x: Fq2,
    y: Fq2,
    z: Fq2,
}

impl G2Projective {
    /// Doubles the point and returns the tangent line (`ark-ec`'s
    /// `G2HomProjective::double_in_place`).
    fn double_step(&mut self) -> Line {
        let a = halve(self.x * self.y);
        let b = self.y.square();
        let c = self.z.square();
        let e = <ark_bn254::g2::Config as SWCurveConfig>::COEFF_B * (c.double() + c);
        let f = e.double() + e;
        let g = halve(b + f);
        let h = (self.y + self.z).square() - (b + c);
        let i = e - b;
        let j = self.x.square();
        let e_square = e.square();

        self.x = a * (b - f);
        self.y = g.square() - (e_square.double() + e_square);
        self.z = b * h;
        (-h, j.double() + j, i)
    }

    /// Adds the affine point `(qx, qy)` and returns the line through both
    /// (`ark-ec`'s `G2HomProjective::add_in_place`).
    fn add_step(&mut self, qx: Fq2, qy: Fq2) -> Line {
        let theta = self.y - qy * self.z;
        let lambda = self.x - qx * self.z;
        let c = theta.square();
        let d = lambda.square();
        let e = lambda * d;
        let f = self.z * c;
        let g = self.x * d;
        let h = e + f - g.double();
        self.x = lambda * h;
        self.y = theta * (g - h) - e * self.y;
        self.z *= e;
        let j = theta * qx - lambda * qy;
        (lambda, -theta, j)
    }
}

/// The untwist-Frobenius-twist endomorphism on an affine G2 point.
fn mul_by_char(mut x: Fq2, mut y: Fq2) -> (Fq2, Fq2) {
    x.frobenius_map_in_place(1);
    x *= <ark_bn254::Config as BnConfig>::TWIST_MUL_BY_Q_X;
    y.frobenius_map_in_place(1);
    y *= <ark_bn254::Config as BnConfig>::TWIST_MUL_BY_Q_Y;
    (x, y)
}

/// One pair's inputs and its running multiple of Q.
struct PairState {
    p: G1Affine,
    q: G2Affine,
    r: G2Projective,
}

impl PairState {
    fn new(p: G1Affine, q: G2Affine) -> Self {
        Self {
            p,
            q,
            r: G2Projective {
                x: q.x,
                y: q.y,
                z: Fq2::one(),
            },
        }
    }

    /// Evaluates a line at P (D-type twist): the sparse element
    /// `c0 * yP + (c1 * xP + c2 v) w`.
    fn evaluate(&self, (mut c0, mut c1, c2): Line) -> Line {
        c0.mul_assign_by_fp(&self.p.y);
        c1.mul_assign_by_fp(&self.p.x);
        (c0, c1, c2)
    }
}

/// Multiplies evaluated lines into `f` two at a time: the product of two sparse
/// lines and one multiplication by it cost 23 `Fq2` multiplications, against 26
/// for two separate sparse multiplications.
struct LineProduct<'a> {
    f: &'a mut Fq12,
    pending: Option<Line>,
}

impl<'a> LineProduct<'a> {
    fn new(f: &'a mut Fq12) -> Self {
        Self { f, pending: None }
    }

    fn push(&mut self, line: Line) {
        match self.pending.take() {
            None => self.pending = Some(line),
            Some(previous) => mul_by_01234(self.f, &mul_034_by_034(&previous, &line)),
        }
    }

    /// Multiplies in a line left without a partner.
    fn finish(self) {
        if let Some((c0, c3, c4)) = self.pending {
            self.f.mul_by_034(&c0, &c3, &c4);
        }
    }
}

/// The product of the lines `c0 + (c3 + c4 v) w` and `d0 + (d3 + d4 v) w`, as the
/// coefficients of `(z0 + z1 v + z2 v^2) + (z3 + z4 v) w`.
fn mul_034_by_034(&(c0, c3, c4): &Line, &(d0, d3, d4): &Line) -> [Fq2; 5] {
    let x0 = c0 * d0;
    let x3 = c3 * d3;
    let x4 = c4 * d4;
    let x04 = (c0 + c4) * (d0 + d4) - x0 - x4;
    let x03 = (c0 + c3) * (d0 + d3) - x0 - x3;
    let x34 = (c3 + c4) * (d3 + d4) - x3 - x4;
    let mut z0 = x4;
    <ark_bn254::Fq6Config as Fp6Config>::mul_fp2_by_nonresidue_in_place(&mut z0);
    [z0 + x0, x3, x34, x03, x04]
}

/// `f * ((z0 + z1 v + z2 v^2) + (z3 + z4 v) w)`, Karatsuba over `Fq6`.
fn mul_by_01234(f: &mut Fq12, z: &[Fq2; 5]) {
    let c0 = Fq6::new(z[0], z[1], z[2]);
    let c1 = Fq6::new(z[3], z[4], Fq2::ZERO);
    let a = (f.c0 + f.c1) * (c0 + c1);
    let b = f.c0 * c0;
    let mut c = f.c1;
    c.mul_by_01(&z[3], &z[4]);
    f.c1 = a - b - c;
    <ark_bn254::Fq12Config as Fp12Config>::mul_fp6_by_nonresidue_in_place(&mut c);
    f.c0 = c + b;
}

/// The product of the Miller loops of all pairs, which must hold non-identity
/// points.
fn miller_loop(pairs: &[(G1Affine, G2Affine)]) -> Fq12 {
    let mut states: Vec<PairState> = pairs.iter().map(|&(p, q)| PairState::new(p, q)).collect();
    let mut f = Fq12::one();
    for i in (1..ATE_LOOP_COUNT.len()).rev() {
        if i != ATE_LOOP_COUNT.len() - 1 {
            f.square_in_place();
        }
        // Every line of this step multiplies the same squared accumulator.
        let mut lines = LineProduct::new(&mut f);
        for state in states.iter_mut() {
            let line = state.r.double_step();
            lines.push(state.evaluate(line));
        }
        let bit = ATE_LOOP_COUNT[i - 1];
        if bit != 0 {
            for state in states.iter_mut() {
                let qy = if bit > 0 { state.q.y } else { -state.q.y };
                let line = state.r.add_step(state.q.x, qy);
                lines.push(state.evaluate(line));
            }
        }
        lines.finish();
    }
    // x0 is positive, so no inversion; then add pi(Q) and -pi^2(Q).
    let mut lines = LineProduct::new(&mut f);
    for state in states.iter_mut() {
        let (q1x, q1y) = mul_by_char(state.q.x, state.q.y);
        let line = state.r.add_step(q1x, q1y);
        lines.push(state.evaluate(line));
        let (q2x, q2y) = mul_by_char(q1x, q1y);
        let line = state.r.add_step(q2x, -q2y);
        lines.push(state.evaluate(line));
    }
    lines.finish();
    f
}

/// `x^x0` for `x` in the cyclotomic subgroup, with an addition chain of 62
/// squarings and 17 multiplications (non-adjacent form needs 23 multiplications):
///
/// ```text
/// _10 = 2*1, _100 = 2*_10, _1000 = 2*_100, _10000 = 2*_1000
/// _10001 = 1 + _10000, _10011 = _10 + _10001, _10100 = 1 + _10011
/// _11001 = _1000 + _10001, _100010 = 2*_10001, _100111 = _10011 + _10100
/// _101001 = _10 + _100111
/// i27 = (_100010 << 6 + _100 + _11001) << 7 + _11001
/// i44 = (i27 << 8 + _101001 + _10) << 6 + _10001
/// i70 = ((i44 << 8 + _101001) << 6 + _101001) << 10
/// x0  = (_100111 + i70) << 6 + _101001 + _1000
/// ```
fn cyclotomic_exp_by_seed(x: &Fq12) -> Fq12 {
    let square = |value: Fq12, times: usize| {
        let mut result = value;
        for _ in 0..times {
            result.cyclotomic_square_in_place();
        }
        result
    };
    let x_10 = square(*x, 1);
    let x_100 = square(x_10, 1);
    let x_1000 = square(x_100, 1);
    let x_10000 = square(x_1000, 1);
    let x_10001 = x_10000 * x;
    let x_10011 = x_10 * x_10001;
    let x_10100 = x_10011 * x;
    let x_11001 = x_1000 * x_10001;
    let x_100010 = square(x_10001, 1);
    let x_100111 = x_10011 * x_10100;
    let x_101001 = x_10 * x_100111;

    let mut t = square(x_100010, 6) * x_100 * x_11001;
    t = square(t, 7) * x_11001;
    t = square(t, 8) * x_101001 * x_10;
    t = square(t, 6) * x_10001;
    t = square(t, 8) * x_101001;
    t = square(t, 6) * x_101001;
    t = square(t, 10) * x_100111;
    square(t, 6) * x_101001 * x_1000
}

/// `f^((p^12 - 1) / r)`: the chain of `ark-ec`'s `final_exponentiation`
/// (Fuentes-Castaneda et al., "Faster hashing to G2") with the exponentiations by
/// `x0` done by `cyclotomic_exp_by_seed`. `None` when `f` is zero.
fn final_exponentiation(f: &Fq12) -> Option<Fq12> {
    // Easy part: f^((p^6 - 1)(p^2 + 1)).
    let mut f1 = *f;
    f1.cyclotomic_inverse_in_place();
    let f2 = f.inverse()?;
    let mut r = f1 * f2;
    let f2 = r;
    r.frobenius_map_in_place(2);
    r *= f2;

    // Hard part. x0 is positive, so "exp by -x" is the exponentiation followed by
    // an inverse, which is a conjugation in the cyclotomic subgroup.
    let exp_by_neg_x = |value: &Fq12| {
        let mut result = cyclotomic_exp_by_seed(value);
        result.cyclotomic_inverse_in_place();
        result
    };
    let y0 = exp_by_neg_x(&r);
    let y1 = y0.cyclotomic_square();
    let y2 = y1.cyclotomic_square();
    let mut y3 = y2 * y1;
    let y4 = exp_by_neg_x(&y3);
    let y5 = y4.cyclotomic_square();
    let mut y6 = exp_by_neg_x(&y5);
    y3.cyclotomic_inverse_in_place();
    y6.cyclotomic_inverse_in_place();
    let y7 = y6 * y4;
    let mut y8 = y7 * y3;
    let y9 = y8 * y1;
    let y10 = y8 * y4;
    let y11 = y10 * r;
    let mut y12 = y9;
    y12.frobenius_map_in_place(1);
    let y13 = y12 * y11;
    y8.frobenius_map_in_place(2);
    let y14 = y8 * y13;
    r.cyclotomic_inverse_in_place();
    let mut y15 = r * y9;
    y15.frobenius_map_in_place(3);
    Some(y15 * y14)
}

/// `true` when the product of the pairings is one. Every pair must hold two
/// non-identity points already checked to be on their curves and in their
/// subgroups.
pub(super) fn pairing_is_one(pairs: &[(G1Affine, G2Affine)]) -> bool {
    // The Miller loop of points in G1 and G2 is never zero, so the final
    // exponentiation always succeeds.
    final_exponentiation(&miller_loop(pairs)).is_some_and(|result| result.is_one())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_bn254::{Bn254, Fr};
    use ark_ec::pairing::{MillerLoopOutput, Pairing};
    use ark_ec::{AffineRepr, CurveGroup};
    use ark_ff::PrimeField;
    use sha2::{Digest, Sha256};

    fn scalar(seed: u64) -> Fr {
        Fr::from_be_bytes_mod_order(&Sha256::digest(seed.to_le_bytes()))
    }

    fn random_pairs(n: u64, seed: u64) -> Vec<(G1Affine, G2Affine)> {
        (0..n)
            .map(|i| {
                (
                    (G1Affine::generator() * scalar(seed * 100 + 2 * i)).into_affine(),
                    (G2Affine::generator() * scalar(seed * 100 + 2 * i + 1)).into_affine(),
                )
            })
            .collect()
    }

    #[test]
    fn halve_matches_multiplying_by_one_half() {
        let two_inv = Fq::from(2u64).inverse().unwrap();
        for seed in 0..2000u64 {
            let x = Fq2::new(
                Fq::from_be_bytes_mod_order(&Sha256::digest(seed.to_le_bytes())),
                Fq::from_be_bytes_mod_order(&Sha256::digest((seed + 1_000_000).to_le_bytes())),
            );
            let mut expected = x;
            expected.mul_assign_by_fp(&two_inv);
            assert_eq!(halve(x), expected);
        }
        for edge in [
            Fq::from(0u64),
            Fq::from(1u64),
            -Fq::from(1u64),
            -Fq::from(2u64),
        ] {
            let x = Fq2::new(edge, -edge);
            let mut expected = x;
            expected.mul_assign_by_fp(&two_inv);
            assert_eq!(halve(x), expected);
        }
    }

    #[test]
    fn miller_loop_matches_arkworks() {
        // Up to nine pairs, across arkworks' groups of four.
        for n in 1..=9 {
            for seed in 0..3 {
                let input = random_pairs(n, seed);
                let (g1, g2): (Vec<_>, Vec<_>) = input.iter().copied().unzip();
                assert_eq!(
                    miller_loop(&input),
                    Bn254::multi_miller_loop(g1.clone(), g2.clone()).0,
                    "{n} pairs, seed {seed}"
                );
                assert!(!pairing_is_one(&input));
                assert!(!Bn254::multi_pairing(g1, g2).0.is_one());
            }
        }
    }

    #[test]
    fn final_exponentiation_matches_arkworks() {
        for n in 1..=3 {
            for seed in 10..20 {
                let input = random_pairs(n, seed);
                let f = miller_loop(&input);
                assert_eq!(
                    final_exponentiation(&f),
                    Bn254::final_exponentiation(MillerLoopOutput(f)).map(|r| r.0),
                    "{n} pairs, seed {seed}"
                );
                // An element of the cyclotomic subgroup for the exponentiation alone.
                let mut c = f;
                c.cyclotomic_inverse_in_place();
                c *= f.inverse().unwrap();
                let c_p6 = c;
                c.frobenius_map_in_place(2);
                c *= c_p6;
                assert_eq!(
                    cyclotomic_exp_by_seed(&c),
                    c.cyclotomic_exp([crate::bn254::SEED])
                );
            }
        }
    }

    #[test]
    fn balanced_products_are_one() {
        // e(aP, bQ) * e(-(ab)P, Q) * prod e(P_i, Q_i) e(-P_i, Q_i) = 1.
        for extra in 0..6u64 {
            let (a, b) = (scalar(1000 + extra), scalar(2000 + extra));
            let mut input = vec![
                (
                    (G1Affine::generator() * a).into_affine(),
                    (G2Affine::generator() * b).into_affine(),
                ),
                (
                    (G1Affine::generator() * -(a * b)).into_affine(),
                    G2Affine::generator(),
                ),
            ];
            for (p, q) in random_pairs(extra, 3000 + extra) {
                input.push((p, q));
                input.push((-p, q));
            }
            assert!(pairing_is_one(&input), "{} pairs", input.len());
            // Changing one point breaks the equation.
            input[0].0 = (input[0].0 + G1Affine::generator()).into_affine();
            assert!(!pairing_is_one(&input));
        }
    }
}
