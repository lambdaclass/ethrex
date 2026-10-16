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
//! - the exponentiations by `x0` in the final exponentiation use an addition chain;
//! - on x86-64 with BMI2 and ADX, the field arithmetic is the branch-free assembly
//!   of [`super::fields`].

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use ark_bn254::{G1Affine, G2Affine};
use ark_ec::bn::BnConfig;
use ark_ec::short_weierstrass::SWCurveConfig;

use super::fields::{Fe, Fe2, Fe6, Fe12};

/// `6 x0 + 2` in NAF, least significant digit first.
const ATE_LOOP_COUNT: &[i8] = <ark_bn254::Config as BnConfig>::ATE_LOOP_COUNT;

/// A line `c0 + (c1 + c2 v) w` before its evaluation at a G1 point.
type Line<const ADX: bool> = (Fe2<ADX>, Fe2<ADX>, Fe2<ADX>);

/// A G2 point in homogeneous projective coordinates `(X/Z, Y/Z)`.
struct G2Projective<const ADX: bool> {
    x: Fe2<ADX>,
    y: Fe2<ADX>,
    z: Fe2<ADX>,
}

impl<const ADX: bool> G2Projective<ADX> {
    /// The twist coefficient `b' = 3 / (9 + u)`.
    const COEFF_B: Fe2<ADX> = Fe2::from_ark(<ark_bn254::g2::Config as SWCurveConfig>::COEFF_B);

    /// Doubles the point and returns the tangent line (`ark-ec`'s
    /// `G2HomProjective::double_in_place`).
    fn double_step(&mut self) -> Line<ADX> {
        let a = (self.x * self.y).halve();
        let b = self.y.square();
        let c = self.z.square();
        let e = Self::COEFF_B * (c.double() + c);
        let f = e.double() + e;
        let g = (b + f).halve();
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
    fn add_step(&mut self, qx: Fe2<ADX>, qy: Fe2<ADX>) -> Line<ADX> {
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
        self.z = self.z * e;
        let j = theta * qx - lambda * qy;
        (lambda, -theta, j)
    }
}

/// The untwist-Frobenius-twist endomorphism on an affine G2 point.
fn mul_by_char<const ADX: bool>(x: Fe2<ADX>, y: Fe2<ADX>) -> (Fe2<ADX>, Fe2<ADX>) {
    let twist_x = Fe2::from_ark(<ark_bn254::Config as BnConfig>::TWIST_MUL_BY_Q_X);
    let twist_y = Fe2::from_ark(<ark_bn254::Config as BnConfig>::TWIST_MUL_BY_Q_Y);
    (x.frobenius_map(1) * twist_x, y.frobenius_map(1) * twist_y)
}

/// One pair's inputs and its running multiple of Q.
struct PairState<const ADX: bool> {
    px: Fe<ADX>,
    py: Fe<ADX>,
    qx: Fe2<ADX>,
    qy: Fe2<ADX>,
    r: G2Projective<ADX>,
}

impl<const ADX: bool> PairState<ADX> {
    fn new(p: &G1Affine, q: &G2Affine) -> Self {
        let (qx, qy) = (Fe2::from_ark(q.x), Fe2::from_ark(q.y));
        Self {
            px: Fe::from_ark(p.x),
            py: Fe::from_ark(p.y),
            qx,
            qy,
            r: G2Projective {
                x: qx,
                y: qy,
                z: Fe2::ONE,
            },
        }
    }

    /// Evaluates a line at P (D-type twist): the sparse element
    /// `c0 * yP + (c1 * xP + c2 v) w`.
    fn evaluate(&self, (c0, c1, c2): Line<ADX>) -> Line<ADX> {
        (c0.mul_by_fe(self.py), c1.mul_by_fe(self.px), c2)
    }
}

/// Multiplies evaluated lines into `f` two at a time: the product of two sparse
/// lines and one multiplication by it cost 23 `Fq2` multiplications, against 26
/// for two separate sparse multiplications.
struct LineProduct<'a, const ADX: bool> {
    f: &'a mut Fe12<ADX>,
    pending: Option<Line<ADX>>,
}

impl<'a, const ADX: bool> LineProduct<'a, ADX> {
    fn new(f: &'a mut Fe12<ADX>) -> Self {
        Self { f, pending: None }
    }

    fn push(&mut self, line: Line<ADX>) {
        match self.pending.take() {
            None => self.pending = Some(line),
            Some(previous) => mul_by_01234(self.f, &mul_034_by_034(&previous, &line)),
        }
    }

    /// Multiplies in a line left without a partner.
    fn finish(self) {
        if let Some((c0, c3, c4)) = self.pending {
            *self.f = self.f.mul_by_034(c0, c3, c4);
        }
    }
}

/// The product of the lines `c0 + (c3 + c4 v) w` and `d0 + (d3 + d4 v) w`, as the
/// coefficients of `(z0 + z1 v + z2 v^2) + (z3 + z4 v) w`.
fn mul_034_by_034<const ADX: bool>(
    &(c0, c3, c4): &Line<ADX>,
    &(d0, d3, d4): &Line<ADX>,
) -> [Fe2<ADX>; 5] {
    let x0 = c0 * d0;
    let x3 = c3 * d3;
    let x4 = c4 * d4;
    let x04 = (c0 + c4) * (d0 + d4) - x0 - x4;
    let x03 = (c0 + c3) * (d0 + d3) - x0 - x3;
    let x34 = (c3 + c4) * (d3 + d4) - x3 - x4;
    [x4.mul_by_nonresidue() + x0, x3, x34, x03, x04]
}

/// `f * ((z0 + z1 v + z2 v^2) + (z3 + z4 v) w)`, Karatsuba over `Fq6`.
fn mul_by_01234<const ADX: bool>(f: &mut Fe12<ADX>, z: &[Fe2<ADX>; 5]) {
    let c0 = Fe6::new(z[0], z[1], z[2]);
    let c1 = Fe6::new(z[3], z[4], Fe2::ZERO);
    let a = (f.c0 + f.c1) * (c0 + c1);
    let b = f.c0 * c0;
    let c = f.c1.mul_by_01(z[3], z[4]);
    f.c1 = a - b - c;
    f.c0 = c.mul_by_nonresidue() + b;
}

/// The product of the Miller loops of all pairs, which must hold non-identity
/// points.
fn miller_loop<const ADX: bool>(pairs: &[(G1Affine, G2Affine)]) -> Fe12<ADX> {
    let mut states: Vec<PairState<ADX>> = pairs.iter().map(|(p, q)| PairState::new(p, q)).collect();
    let mut f = Fe12::ONE;
    for i in (1..ATE_LOOP_COUNT.len()).rev() {
        if i != ATE_LOOP_COUNT.len() - 1 {
            f = f.square();
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
                let qy = if bit > 0 { state.qy } else { -state.qy };
                let line = state.r.add_step(state.qx, qy);
                lines.push(state.evaluate(line));
            }
        }
        lines.finish();
    }
    // x0 is positive, so no inversion; then add pi(Q) and -pi^2(Q).
    let mut lines = LineProduct::new(&mut f);
    for state in states.iter_mut() {
        let (q1x, q1y) = mul_by_char(state.qx, state.qy);
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
fn cyclotomic_exp_by_seed<const ADX: bool>(x: Fe12<ADX>) -> Fe12<ADX> {
    let square = |value: Fe12<ADX>, times: usize| {
        let mut result = value;
        for _ in 0..times {
            result = result.cyclotomic_square();
        }
        result
    };
    let x_10 = square(x, 1);
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
fn final_exponentiation<const ADX: bool>(f: Fe12<ADX>) -> Option<Fe12<ADX>> {
    // Easy part: f^((p^6 - 1)(p^2 + 1)).
    let r = f.conjugate() * f.inverse()?;
    let r = r.frobenius_map(2) * r;

    // Hard part. x0 is positive, so "exp by -x" is the exponentiation followed by
    // an inverse, which is a conjugation in the cyclotomic subgroup.
    let exp_by_neg_x = |value: Fe12<ADX>| cyclotomic_exp_by_seed(value).conjugate();
    let y0 = exp_by_neg_x(r);
    let y1 = y0.cyclotomic_square();
    let y2 = y1.cyclotomic_square();
    let y3 = y2 * y1;
    let y4 = exp_by_neg_x(y3);
    let y5 = y4.cyclotomic_square();
    let y6 = exp_by_neg_x(y5).conjugate();
    let y3 = y3.conjugate();
    let y7 = y6 * y4;
    let y8 = y7 * y3;
    let y9 = y8 * y1;
    let y10 = y8 * y4;
    let y11 = y10 * r;
    let y13 = y9.frobenius_map(1) * y11;
    let y14 = y8.frobenius_map(2) * y13;
    let y15 = (r.conjugate() * y9).frobenius_map(3);
    Some(y15 * y14)
}

fn check<const ADX: bool>(pairs: &[(G1Affine, G2Affine)]) -> bool {
    // The Miller loop of points in G1 and G2 is never zero, so the final
    // exponentiation always succeeds.
    final_exponentiation(miller_loop::<ADX>(pairs)).is_some_and(|result| result.is_one())
}

/// `true` when the product of the pairings is one. Every pair must hold two
/// non-identity points already checked to be on their curves and in their
/// subgroups.
pub(super) fn pairing_is_one(pairs: &[(G1Affine, G2Affine)]) -> bool {
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if std::is_x86_feature_detected!("bmi2") && std::is_x86_feature_detected!("adx") {
        return check::<true>(pairs);
    }
    check::<false>(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_bn254::{Bn254, Fr};
    use ark_ec::pairing::{MillerLoopOutput, Pairing};
    use ark_ec::{AffineRepr, CurveGroup};
    use ark_ff::{CyclotomicMultSubgroup, Field, One, PrimeField};
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

    fn adx_available() -> bool {
        #[cfg(all(target_arch = "x86_64", feature = "std"))]
        {
            std::is_x86_feature_detected!("bmi2") && std::is_x86_feature_detected!("adx")
        }
        #[cfg(not(all(target_arch = "x86_64", feature = "std")))]
        {
            false
        }
    }

    fn miller_loop_matches<const ADX: bool>() {
        // Up to nine pairs, across arkworks' groups of four.
        for n in 1..=9 {
            for seed in 0..3 {
                let input = random_pairs(n, seed);
                let (g1, g2): (Vec<_>, Vec<_>) = input.iter().copied().unzip();
                assert_eq!(
                    miller_loop::<ADX>(&input).to_ark(),
                    Bn254::multi_miller_loop(g1.clone(), g2.clone()).0,
                    "{n} pairs, seed {seed}"
                );
                assert!(!check::<ADX>(&input));
                assert!(!Bn254::multi_pairing(g1, g2).0.is_one());
            }
        }
    }

    #[test]
    fn miller_loop_matches_arkworks() {
        miller_loop_matches::<false>();
        if adx_available() {
            miller_loop_matches::<true>();
        }
    }

    fn final_exponentiation_matches<const ADX: bool>() {
        for n in 1..=3 {
            for seed in 10..20 {
                let input = random_pairs(n, seed);
                let f = miller_loop::<ADX>(&input);
                let ark_f = f.to_ark();
                assert_eq!(
                    final_exponentiation(f).map(Fe12::to_ark),
                    Bn254::final_exponentiation(MillerLoopOutput(ark_f)).map(|r| r.0),
                    "{n} pairs, seed {seed}"
                );
                // An element of the cyclotomic subgroup for the exponentiation alone.
                let mut c = ark_f;
                c.cyclotomic_inverse_in_place();
                c *= ark_f.inverse().unwrap();
                let c_p6 = c;
                c.frobenius_map_in_place(2);
                c *= c_p6;
                assert_eq!(
                    cyclotomic_exp_by_seed(Fe12::<ADX>::from_ark(c)).to_ark(),
                    c.cyclotomic_exp([crate::bn254::SEED])
                );
            }
        }
    }

    #[test]
    fn final_exponentiation_matches_arkworks() {
        final_exponentiation_matches::<false>();
        if adx_available() {
            final_exponentiation_matches::<true>();
        }
    }

    fn balanced_products<const ADX: bool>() {
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
            assert!(check::<ADX>(&input), "{} pairs", input.len());
            // Changing one point breaks the equation.
            input[0].0 = (input[0].0 + G1Affine::generator()).into_affine();
            assert!(!check::<ADX>(&input));
        }
    }

    #[test]
    fn balanced_products_are_one() {
        balanced_products::<false>();
        if adx_available() {
            balanced_products::<true>();
        }
    }
}
