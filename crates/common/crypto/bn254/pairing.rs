//! Optimal ate multi-pairing check for ECPAIRING.
//!
//! It computes the same values as the `ark-ec` BN pairing: the Miller loop uses the
//! same line formulas in homogeneous projective coordinates and the same loop over
//! `6 x0 + 2`, so its output matches `Bn254::multi_miller_loop` exactly, and the
//! final exponentiation is arkworks' own. The differences only change the cost:
//! - each line is evaluated as soon as it is computed, instead of first storing
//!   every line of every pair in a vector (`G2Prepared`);
//! - all pairs share one accumulator. Without the `parallel` feature, arkworks runs
//!   one Miller loop per group of four pairs and multiplies the results, which
//!   repeats the 64 squarings of the accumulator for every group;
//! - lines are multiplied into the accumulator two at a time.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;
use ark_bn254::{Bn254, Fq, Fq2, Fq6, Fq12, G1Affine, G2Affine};
use ark_ec::bn::BnConfig;
use ark_ec::pairing::{MillerLoopOutput, Pairing};
use ark_ec::short_weierstrass::SWCurveConfig;
use ark_ff::{AdditiveGroup, Field, Fp6Config, Fp12Config, One};

/// `6 x0 + 2` in NAF, least significant digit first.
const ATE_LOOP_COUNT: &[i8] = <ark_bn254::Config as BnConfig>::ATE_LOOP_COUNT;

/// `1 / 2` in `Fq`.
const TWO_INV: Fq = ark_ff::MontFp!(
    "10944121435919637611123202872628637544348155578648911831344518947322613104292"
);

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
        let mut a = self.x * self.y;
        a.mul_assign_by_fp(&TWO_INV);
        let b = self.y.square();
        let c = self.z.square();
        let e = <ark_bn254::g2::Config as SWCurveConfig>::COEFF_B * (c.double() + c);
        let f = e.double() + e;
        let mut g = b + f;
        g.mul_assign_by_fp(&TWO_INV);
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

/// `true` when the product of the pairings is one. Every pair must hold two
/// non-identity points already checked to be on their curves and in their
/// subgroups.
pub(super) fn pairing_is_one(pairs: &[(G1Affine, G2Affine)]) -> bool {
    // The Miller loop of points in G1 and G2 is never zero, so the final
    // exponentiation always succeeds.
    Bn254::final_exponentiation(MillerLoopOutput(miller_loop(pairs)))
        .is_some_and(|result| result.0.is_one())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_bn254::Fr;
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
