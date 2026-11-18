//! Variable-time modular inversion modulo an odd 256-bit modulus, using Bernstein-Yang divsteps in
//! batches of 62 ("safegcd", with the variable-time batching from "Fast constant-time gcd
//! computation and modular inversion", Bernstein and Yang, 2019). The inputs are public precompile
//! arguments, so variable time is fine, and it is several times faster than an exponentiation.

const M62: u64 = u64::MAX >> 2;

/// An odd modulus below 2^256, with the constants the inversion needs.
pub(crate) struct Modulus {
    /// The modulus as little-endian 64-bit limbs.
    pub(crate) limbs: [u64; 4],
    /// The modulus as five signed 62-bit limbs.
    signed62: [i64; 5],
    /// `modulus^-1 mod 2^62`.
    inv62: u64,
}

impl Modulus {
    pub(crate) const fn new(limbs: [u64; 4]) -> Self {
        Self {
            limbs,
            signed62: to_signed62(limbs),
            inv62: inverse_mod_2_64(limbs[0]) & M62,
        }
    }

    /// `value^-1 mod m` for `0 < value < m` with `gcd(value, m) = 1`, as little-endian limbs.
    pub(crate) fn inverse(&self, value: [u64; 4]) -> [u64; 4] {
        let mut d = [0i64; 5];
        let mut e = [1i64, 0, 0, 0, 0];
        let mut f = self.signed62;
        let mut g = to_signed62(value);
        let mut len = 5usize;
        // eta = -delta, with delta starting at 1.
        let mut eta: i64 = -1;

        loop {
            let (t, next_eta) = divsteps_62(eta, f[0], g[0]);
            eta = next_eta;
            self.update_de(&mut d, &mut e, &t);
            update_fg(len, &mut f, &mut g, &t);

            if g[0] == 0 && g[1..len].iter().all(|&limb| limb == 0) {
                break;
            }

            // Shrink the working length once the top limbs of f and g are
            // both just sign extension.
            let fn_ = f[len - 1];
            let gn = g[len - 1];
            let not_shrinkable = len < 2 || (fn_ ^ (fn_ >> 63)) != 0 || (gn ^ (gn >> 63)) != 0;
            if !not_shrinkable {
                f[len - 2] |= fn_.wrapping_shl(62);
                g[len - 2] |= gn.wrapping_shl(62);
                len -= 1;
            }
        }

        // g is zero and f is +-gcd = +-1; d holds +-value^-1.
        from_signed62(self.normalize(d, f[len - 1]))
    }

    /// `[d, e] = t * [d, e] / 2^62 (mod m)`, keeping limbs in range.
    fn update_de(&self, d: &mut [i64; 5], e: &mut [i64; 5], t: &Transition) {
        let (u, v, q, r) = (t.u, t.v, t.q, t.r);
        let modulus = &self.signed62;
        // Add multiples of the modulus so the low 62 bits become zero; start
        // with corrections for negative d and e.
        let sd = d[4] >> 63;
        let se = e[4] >> 63;
        let mut md = (u & sd).wrapping_add(v & se);
        let mut me = (q & sd).wrapping_add(r & se);

        let mut cd = i128::from(u) * i128::from(d[0]) + i128::from(v) * i128::from(e[0]);
        let mut ce = i128::from(q) * i128::from(d[0]) + i128::from(r) * i128::from(e[0]);

        md = md.wrapping_sub(
            (self.inv62.wrapping_mul(cd as u64).wrapping_add(md as u64) & M62) as i64,
        );
        me = me.wrapping_sub(
            (self.inv62.wrapping_mul(ce as u64).wrapping_add(me as u64) & M62) as i64,
        );

        cd += i128::from(modulus[0]) * i128::from(md);
        ce += i128::from(modulus[0]) * i128::from(me);
        cd >>= 62;
        ce >>= 62;

        for i in 1..5 {
            cd += i128::from(u) * i128::from(d[i]) + i128::from(v) * i128::from(e[i]);
            ce += i128::from(q) * i128::from(d[i]) + i128::from(r) * i128::from(e[i]);
            cd += i128::from(modulus[i]) * i128::from(md);
            ce += i128::from(modulus[i]) * i128::from(me);
            d[i - 1] = (cd as u64 & M62) as i64;
            e[i - 1] = (ce as u64 & M62) as i64;
            cd >>= 62;
            ce >>= 62;
        }
        d[4] = cd as i64;
        e[4] = ce as i64;
    }

    /// Maps `r` in `(-2m, m)` to `[0, m)`, negating first when `sign < 0`.
    fn normalize(&self, mut r: [i64; 5], sign: i64) -> [i64; 5] {
        let m62 = M62.cast_signed();
        let cond_add = r[4] >> 63;
        for (limb, modulus) in r.iter_mut().zip(self.signed62) {
            *limb += modulus & cond_add;
        }
        let cond_negate = sign >> 63;
        for limb in r.iter_mut() {
            *limb = (*limb ^ cond_negate) - cond_negate;
        }
        propagate(&mut r, m62);
        let cond_add = r[4] >> 63;
        for (limb, modulus) in r.iter_mut().zip(self.signed62) {
            *limb += modulus & cond_add;
        }
        propagate(&mut r, m62);
        r
    }
}

const fn inverse_mod_2_64(value: u64) -> u64 {
    // Newton iteration doubles the number of correct low bits each step,
    // starting from 3 correct bits for any odd value.
    let mut inv = value;
    let mut i = 0;
    while i < 5 {
        inv = inv.wrapping_mul(2u64.wrapping_sub(value.wrapping_mul(inv)));
        i += 1;
    }
    inv
}

const fn to_signed62(a: [u64; 4]) -> [i64; 5] {
    [
        (a[0] & M62) as i64,
        ((a[0] >> 62 | a[1] << 2) & M62) as i64,
        ((a[1] >> 60 | a[2] << 4) & M62) as i64,
        ((a[2] >> 58 | a[3] << 6) & M62) as i64,
        (a[3] >> 56) as i64,
    ]
}

fn from_signed62(v: [i64; 5]) -> [u64; 4] {
    let v = v.map(|limb| limb as u64);
    [
        v[0] | v[1] << 62,
        v[1] >> 2 | v[2] << 60,
        v[2] >> 4 | v[3] << 58,
        v[3] >> 6 | v[4] << 56,
    ]
}

/// The 2x2 transition matrix of 62 divsteps, scaled by 2^62.
struct Transition {
    u: i64,
    v: i64,
    q: i64,
    r: i64,
}

/// Runs 62 divsteps on the low 62 bits of `f` and `g` (both odd/any) and
/// returns the transition matrix and the new eta.
fn divsteps_62(mut eta: i64, f0: i64, g0: i64) -> (Transition, i64) {
    let (mut u, mut v, mut q, mut r): (u64, u64, u64, u64) = (1, 0, 0, 1);
    let mut f = f0 as u64;
    let mut g = g0 as u64;
    let mut i: u32 = 62;

    loop {
        // Count zeros only up to i with a sentinel bit.
        let zeros = (g | (u64::MAX << i)).trailing_zeros();
        // Each of those divsteps just halves g.
        g >>= zeros;
        u <<= zeros;
        v <<= zeros;
        eta -= i64::from(zeros);
        i -= zeros;
        if i == 0 {
            break;
        }
        let limit: u32;
        let mask: u64;
        let w: u64;
        if eta < 0 {
            // Negate eta and replace (f, g) with (g, -f).
            eta = -eta;
            let tmp = f;
            f = g;
            g = tmp.wrapping_neg();
            let tmp = u;
            u = q;
            q = tmp.wrapping_neg();
            let tmp = v;
            v = r;
            r = tmp.wrapping_neg();
            // Cancel up to 6 bits of g, but no more than i, and no more
            // than eta + 1 before eta's sign flips again.
            limit = ((eta + 1) as u32).min(i);
            mask = (u64::MAX >> (64 - limit)) & 63;
            w = f
                .wrapping_mul(g)
                .wrapping_mul(f.wrapping_mul(f).wrapping_sub(2))
                & mask;
        } else {
            // Cancel up to 4 bits of g.
            limit = ((eta + 1) as u32).min(i);
            mask = (u64::MAX >> (64 - limit)) & 15;
            let w0 = f.wrapping_add(((f.wrapping_add(1)) & 4) << 1);
            w = w0.wrapping_neg().wrapping_mul(g) & mask;
        }
        g = g.wrapping_add(f.wrapping_mul(w));
        q = q.wrapping_add(u.wrapping_mul(w));
        r = r.wrapping_add(v.wrapping_mul(w));
    }

    (
        Transition {
            u: u as i64,
            v: v as i64,
            q: q as i64,
            r: r as i64,
        },
        eta,
    )
}

/// `[f, g] = t * [f, g] / 2^62` over the first `len` limbs.
fn update_fg(len: usize, f: &mut [i64; 5], g: &mut [i64; 5], t: &Transition) {
    let (u, v, q, r) = (t.u, t.v, t.q, t.r);
    let mut cf = i128::from(u) * i128::from(f[0]) + i128::from(v) * i128::from(g[0]);
    let mut cg = i128::from(q) * i128::from(f[0]) + i128::from(r) * i128::from(g[0]);
    cf >>= 62;
    cg >>= 62;
    for i in 1..len {
        cf += i128::from(u) * i128::from(f[i]) + i128::from(v) * i128::from(g[i]);
        cg += i128::from(q) * i128::from(f[i]) + i128::from(r) * i128::from(g[i]);
        f[i - 1] = (cf as u64 & M62) as i64;
        g[i - 1] = (cg as u64 & M62) as i64;
        cf >>= 62;
        cg >>= 62;
    }
    f[len - 1] = cf as i64;
    g[len - 1] = cg as i64;
}

fn propagate(r: &mut [i64; 5], m62: i64) {
    for i in 0..4 {
        r[i + 1] += r[i] >> 62;
        r[i] &= m62;
    }
}
