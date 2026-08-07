//! Tests for the x-only ECSM linear-combination reconstruction
//! (`lincomb2_with_oracle`) against the software `ProjectivePoint::lincomb`,
//! plus the degenerate-configuration fallback guards.

use super::super::*;

/// secp256k1 curve constant `b = 7`.
fn curve_b() -> FieldElement {
    let mut bytes = [0u8; 32];
    bytes[31] = 7;
    FieldElement::from_bytes(&bytes.into()).unwrap()
}

/// Software stand-in for the ECSM precompile: lift `x` to a curve point and
/// return `x(k·P)` (parity-invariant, like the real ecall).
fn soft_oracle(x: &FieldElement, k: &Scalar) -> Option<FieldElement> {
    let xn = x.normalize();
    let y2 = (xn.square() * xn + curve_b()).normalize();
    let y = Option::<FieldElement>::from(y2.sqrt())?;
    let p = point_from_xy(&xn, &y.normalize())?;
    let prod = (p * k).to_affine();
    Some(affine_xy(&prod)?.0)
}

fn g_times(n: u64) -> ProjectivePoint {
    ProjectivePoint::GENERATOR * Scalar::from(n)
}

#[test]
fn matches_software_lincomb_on_fixed_inputs() {
    let cases = [
        (g_times(3), 123_456_789u64, g_times(7), 987_654_321u64),
        (g_times(11), 2u64.pow(20) + 5, g_times(2), 42u64),
        (ProjectivePoint::GENERATOR, 7u64, g_times(5), 9u64),
    ];
    for (p1, k1, p2, k2) in cases {
        let (k1, k2) = (Scalar::from(k1), Scalar::from(k2));
        let expected = ProjectivePoint::lincomb(&p1, &k1, &p2, &k2);
        let got = lincomb2_with_oracle(&p1.to_affine(), &k1, &p2.to_affine(), &k2, soft_oracle)
            .expect("non-degenerate inputs must reconstruct");
        assert_eq!(got, expected.to_affine());
    }
}

#[test]
fn matches_software_lincomb_on_recovery_shape() {
    // u1·G + u2·R, generator first, like ECDSA recovery.
    let g = ProjectivePoint::GENERATOR;
    let r = g_times(0x1234);
    let u1 = Scalar::from(0xdead_beefu64);
    let u2 = Scalar::from(0x0bad_f00du64);
    let expected = ProjectivePoint::lincomb(&g, &u1, &r, &u2);
    let got = lincomb2_with_oracle(&g.to_affine(), &u1, &r.to_affine(), &u2, soft_oracle)
        .expect("non-degenerate inputs must reconstruct");
    assert_eq!(got, expected.to_affine());
}

#[test]
fn edge_scalars_fall_back() {
    let p1 = g_times(3);
    let p2 = g_times(5);
    let ok = Scalar::from(12345u64);
    for bad in [Scalar::ZERO, Scalar::ONE, -Scalar::ONE] {
        assert!(
            lincomb2_with_oracle(&p1.to_affine(), &bad, &p2.to_affine(), &ok, soft_oracle)
                .is_none()
        );
        assert!(
            lincomb2_with_oracle(&p1.to_affine(), &ok, &p2.to_affine(), &bad, soft_oracle)
                .is_none()
        );
    }
}

#[test]
fn identity_points_fall_back() {
    let p = g_times(3);
    let k = Scalar::from(7u64);
    let id = ProjectivePoint::IDENTITY;
    assert!(lincomb2_with_oracle(&id.to_affine(), &k, &p.to_affine(), &k, soft_oracle).is_none());
    assert!(lincomb2_with_oracle(&p.to_affine(), &k, &id.to_affine(), &k, soft_oracle).is_none());
}

#[test]
fn cancelling_and_doubling_terms_fall_back() {
    let p = g_times(3);
    let k = Scalar::from(7u64);
    // A = B (doubling chord) and A = −B (Q = O): both share x(A) = x(B).
    assert!(lincomb2_with_oracle(&p.to_affine(), &k, &p.to_affine(), &k, soft_oracle).is_none());
    assert!(lincomb2_with_oracle(&p.to_affine(), &k, &(-p).to_affine(), &k, soft_oracle).is_none());
}

#[test]
fn k_half_n_minus_1_reconstructs_correctly() {
    // k = (n-1)/2 satisfies k·P = -(k+1)·P for any P, so the oracle returns
    // the same x-coordinate for both the k and k+1 calls (xa = xc). The
    // solve_y algebra still holds: lambda² = 2·xa + xp = t, so the check
    // passes and the correct ya is recovered.
    let two_inv = Scalar::from(2u64)
        .invert_vartime()
        .expect("2 is invertible mod n");
    let k_half = -Scalar::ONE * two_inv; // (n-1)/2

    let p1 = g_times(5);
    let p2 = g_times(11);
    let k2 = Scalar::from(99999u64);

    let expected = ProjectivePoint::lincomb(&p1, &k_half, &p2, &k2);
    let got = lincomb2_with_oracle(&p1.to_affine(), &k_half, &p2.to_affine(), &k2, soft_oracle)
        .expect("k=(n-1)/2 is not near-edge and must reconstruct correctly");
    assert_eq!(got, expected.to_affine());
}

#[test]
fn cross_point_cancellation_falls_back() {
    // Construct k1, k2, P1 ≠ ±P2 such that k1·P1 = -(k2·P2), so
    // k1·P1 + k2·P2 = O. The shared x-coordinate makes dxq = 0 → None.
    // P1 = 3G, P2 = 7G: k1·3G = -k2·7G → k1 = -k2·7·3^{-1} mod n.
    let p1 = g_times(3);
    let p2 = g_times(7);
    let k2 = Scalar::from(12345u64);
    let three_inv = Scalar::from(3u64)
        .invert_vartime()
        .expect("3 is invertible mod n");
    let k1 = -(k2 * Scalar::from(7u64) * three_inv);
    assert!(
        lincomb2_with_oracle(&p1.to_affine(), &k1, &p2.to_affine(), &k2, soft_oracle).is_none(),
        "cross-point cancellation (P1 ≠ ±P2, result = O) must fall back"
    );
}

#[test]
fn solve_y_rejects_inconsistent_oracle_xc() {
    // Directly test that solve_y's lambda² == t check fires when xc is wrong.
    // This is the oracle-misbehavior guard: it cannot easily be reached via
    // lincomb2_with_oracle because the oracle is Fn (no mutable state to
    // return xa correct and xc wrong in separate calls).
    let (xp, yp) = affine_xy(&g_times(3).to_affine()).unwrap();
    let k = Scalar::from(12345u64);

    let xa = soft_oracle(&xp, &k).unwrap();
    let xc_correct = soft_oracle(&xp, &(k + Scalar::ONE)).unwrap();
    // xc from k+100 is inconsistent with xa from k — lambda²=t must reject it.
    let xc_wrong = soft_oracle(&xp, &(k + Scalar::from(100u64))).unwrap();

    let dx = (xa - xp).normalize();
    let inv_den = Option::<FieldElement>::from((yp.double() * dx).invert())
        .expect("dx is nonzero for k=12345");

    assert!(
        solve_y(&xp, &yp, &xa, &xc_correct, &dx, &inv_den).is_some(),
        "correct xc must pass the lambda² check"
    );
    assert!(
        solve_y(&xp, &yp, &xa, &xc_wrong, &dx, &inv_den).is_none(),
        "inconsistent xc (oracle misbehavior) must be rejected by the lambda² check"
    );
}

#[test]
fn odd_y_base_point_reconstructs_correctly() {
    // Validates the solve_y sign-selection argument: when P1 has odd y the
    // reconstruction must still match ProjectivePoint::lincomb.
    let (p1, _k_gen) = (2u64..200)
        .find_map(|n| {
            let p = g_times(n);
            let (_, y) = affine_xy(&p.to_affine())?;
            if y.normalize().to_bytes()[31] & 1 == 1 {
                Some((p, n))
            } else {
                None
            }
        })
        .expect("at least one of the first 200 multiples of G has odd y");

    let p2 = g_times(13);
    let k1 = Scalar::from(54321u64);
    let k2 = Scalar::from(11111u64);
    let expected = ProjectivePoint::lincomb(&p1, &k1, &p2, &k2);
    let got = lincomb2_with_oracle(&p1.to_affine(), &k1, &p2.to_affine(), &k2, soft_oracle)
        .expect("odd-y base point is non-degenerate and must reconstruct correctly");
    assert_eq!(got, expected.to_affine());
}
