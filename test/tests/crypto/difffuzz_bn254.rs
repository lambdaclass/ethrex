//! Differential fuzzing of the BN254 precompiles ECADD (0x06), ECMUL (0x07) and ECPAIRING (0x08).
//!
//! Every input goes through LEVM's precompile functions twice, once with `NativeCrypto` (the
//! dedicated G1 and pairing code in `ethrex_crypto::bn254`) and once with a provider that keeps
//! the `Crypto` trait defaults (the generic `ark-bn254` code), and each output must be
//! byte-identical to `substrate-bn`, an independent implementation that shares no code with
//! arkworks, wrapped here directly from EIP-196/197. When the reference rejects an input, both
//! providers must reject it too. A panic in any of the three counts as a mismatch.
//!
//! Inputs cover points from random scalars and from random x coordinates, infinity, negations,
//! off-curve points, coordinates at, above or just below p, scalars of every bit length, around
//! multiples of r, the GLV half size and eigenvalue, true pairing products with 2 to 9 effective
//! pairs, G2 points on the twist outside the subgroup or outside it only by a small-order
//! component, swapped Fq2 halves, bit flips and truncated or extended inputs.
//!
//! `difffuzz_bn254_quick` runs a fixed seed as part of the regular suite, then checks every class
//! of G2 encoding directly, paired with G1 at infinity. In the dev profile (`debug_assertions`
//! on), where the CI runs it, a true pairing input costs about a quarter of a second, so it runs
//! 170 cases instead of 6000 on smaller pools: every G1, scalar and G2 class appears, but only a
//! handful of true pairings and not every pair count. The release quick run and the ignored
//! `difffuzz_bn254_long` cover the rest. The long test generates cases until a deadline,
//! configured through `DIFFFUZZ_SECS` (default 60), `DIFFFUZZ_SEED` (default 1) and
//! `DIFFFUZZ_THREADS` (default 1):
//!
//! ```text
//! DIFFFUZZ_SECS=3600 DIFFFUZZ_THREADS=16 \
//!     cargo test --release -p ethrex-test --test ethrex_tests difffuzz_ -- --ignored
//! ```

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

use ark_bn254::{Fq, Fq2, Fr, G1Affine, G2Affine};
use ark_ec::{AffineRepr, CurveGroup, scalar_mul::glv::GLVConfig};
use ark_ff::{AdditiveGroup, BigInt, BigInteger, PrimeField, Zero};
use ethrex_common::types::Fork;
use ethrex_crypto::{Crypto, NativeCrypto};
use ethrex_levm::precompiles::{ecadd, ecmul, ecpairing};

/// Keeps every `Crypto` default, so the precompiles run the generic `ark-bn254` code.
#[derive(Debug)]
struct DefaultCrypto;
impl Crypto for DefaultCrypto {}

mod reference {
    use substrate_bn::{AffineG1, AffineG2, Fq, Fq2, Fr, G1, G2, Group, Gt, pairing_batch};

    fn pad(input: &[u8], len: usize) -> Vec<u8> {
        let mut v = input[..input.len().min(len)].to_vec();
        v.resize(len, 0);
        v
    }
    /// Rejects values >= p.
    fn fq(b: &[u8]) -> Option<Fq> {
        Fq::from_slice(b).ok()
    }
    fn g1(b: &[u8]) -> Option<G1> {
        let (x, y) = (fq(&b[..32])?, fq(&b[32..64])?);
        if b[..64].iter().all(|&v| v == 0) {
            return Some(G1::zero());
        }
        AffineG1::new(x, y).ok().map(Into::into)
    }
    fn out_g1(p: G1) -> Vec<u8> {
        let mut out = vec![0u8; 64];
        if let Some(a) = AffineG1::from_jacobian(p) {
            a.x().to_big_endian(&mut out[..32]).unwrap();
            a.y().to_big_endian(&mut out[32..]).unwrap();
        }
        out
    }
    pub fn add(input: &[u8]) -> Option<Vec<u8>> {
        let i = pad(input, 128);
        Some(out_g1(g1(&i[..64])? + g1(&i[64..128])?))
    }
    pub fn mul(input: &[u8]) -> Option<Vec<u8>> {
        let i = pad(input, 96);
        let p = g1(&i[..64])?;
        // Reduces any 256-bit scalar mod r, as EIP-196 requires.
        let k = Fr::from_slice(&i[64..96]).ok()?;
        Some(out_g1(p * k))
    }
    pub fn pairing(input: &[u8]) -> Option<Vec<u8>> {
        if !input.len().is_multiple_of(192) {
            return None;
        }
        let mut pairs = Vec::new();
        for c in input.chunks(192) {
            let p = g1(&c[..64])?;
            // EIP-197 encodes each Fq2 coordinate imaginary part first.
            let (x_im, x_re) = (fq(&c[64..96])?, fq(&c[96..128])?);
            let (y_im, y_re) = (fq(&c[128..160])?, fq(&c[160..192])?);
            let q: G2 = if c[64..192].iter().all(|&v| v == 0) {
                G2::zero()
            } else {
                // Checks the curve equation and r·Q = 0.
                AffineG2::new(Fq2::new(x_re, x_im), Fq2::new(y_re, y_im))
                    .ok()?
                    .into()
            };
            if !p.is_zero() && !q.is_zero() {
                pairs.push((p, q));
            }
        }
        let one = pairs.is_empty() || pairing_batch(&pairs) == Gt::one();
        let mut out = vec![0u8; 32];
        out[31] = one as u8;
        Some(out)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Op {
    Add,
    Mul,
    Pairing,
}

/// `Ok(None)` is a rejected input, `Err` a panic message.
type Outcome = Result<Option<Vec<u8>>, String>;

fn guarded(f: impl FnOnce() -> Option<Vec<u8>>) -> Outcome {
    catch_unwind(AssertUnwindSafe(f)).map_err(|e| {
        e.downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| e.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".into())
    })
}

fn levm(op: Op, input: &[u8], crypto: &dyn Crypto) -> Outcome {
    guarded(|| {
        let mut gas = 1u64 << 40;
        let result = match op {
            Op::Add => ecadd(input, &mut gas, Fork::Prague, crypto),
            Op::Mul => ecmul(input, &mut gas, Fork::Prague, crypto),
            Op::Pairing => ecpairing(input, &mut gas, Fork::Prague, crypto),
        };
        result.ok().map(|b| b.to_vec())
    })
}

fn reference(op: Op, input: &[u8]) -> Outcome {
    guarded(|| match op {
        Op::Add => reference::add(input),
        Op::Mul => reference::mul(input),
        Op::Pairing => reference::pairing(input),
    })
}

// ---------------------------------------------------------------- input generation

/// SplitMix64: deterministic, so a seed reproduces a failing case.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pct(&mut self, p: u64) -> bool {
        self.next() % 100 < p
    }
    fn b32(&mut self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for c in out.chunks_mut(8) {
            c.copy_from_slice(&self.next().to_be_bytes());
        }
        out
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

fn be32(limbs: &BigInt<4>) -> [u8; 32] {
    limbs.to_bytes_be().try_into().unwrap()
}
fn fe32<F: PrimeField<BigInt = BigInt<4>>>(f: &F) -> [u8; 32] {
    be32(&f.into_bigint())
}
fn from_be(b: &[u8; 32]) -> BigInt<4> {
    let mut l = [0u64; 4];
    for (i, chunk) in b.chunks_exact(8).enumerate() {
        l[3 - i] = u64::from_be_bytes(chunk.try_into().unwrap());
    }
    BigInt(l)
}
fn add_be(a: &[u8; 32], b: &[u8; 32]) -> Option<[u8; 32]> {
    let mut x = from_be(a);
    (!x.add_with_carry(&from_be(b))).then(|| be32(&x))
}
fn small(v: u64) -> BigInt<4> {
    BigInt([v, 0, 0, 0])
}
fn rand_fr(r: &mut Rng) -> Fr {
    Fr::from_be_bytes_mod_order(&r.b32())
}
fn rand_fq(r: &mut Rng) -> Fq {
    Fq::from_be_bytes_mod_order(&r.b32())
}
fn flip_bits(r: &mut Rng, data: &mut [u8]) {
    if data.is_empty() {
        return;
    }
    for _ in 0..1 + r.below(3) {
        let i = r.below(data.len());
        data[i] ^= 1 << r.below(8);
    }
}

fn enc_g1(p: &G1Affine) -> [u8; 64] {
    let mut out = [0u8; 64];
    if !p.is_zero() {
        out[..32].copy_from_slice(&fe32(&p.x));
        out[32..].copy_from_slice(&fe32(&p.y));
    }
    out
}
fn enc_g2(q: &G2Affine) -> [u8; 128] {
    let mut out = [0u8; 128];
    if !q.is_zero() {
        out[..32].copy_from_slice(&fe32(&q.x.c1));
        out[32..64].copy_from_slice(&fe32(&q.x.c0));
        out[64..96].copy_from_slice(&fe32(&q.y.c1));
        out[96..].copy_from_slice(&fe32(&q.y.c0));
    }
    out
}

struct Pools {
    /// `g1_random` random points followed by a few fixed ones.
    g1: Vec<G1Affine>,
    g1_random: usize,
    g2: Vec<G2Affine>,
    /// Points on the twist but outside the order-r subgroup.
    twist: Vec<G2Affine>,
    /// Points of small order (10069 or 5864401, the small prime factors of the G2
    /// cofactor), alone or added to a subgroup point: outside the subgroup, but
    /// only by a small component.
    torsion: Vec<G2Affine>,
}

fn rand_g1(r: &mut Rng) -> G1Affine {
    (G1Affine::generator() * rand_fr(r)).into_affine()
}
fn rand_g2(r: &mut Rng) -> G2Affine {
    (G2Affine::generator() * rand_fr(r)).into_affine()
}
/// A point found from a random x, not as a multiple of the generator (G1 has cofactor 1, so it
/// is still in the group).
fn g1_from_x(r: &mut Rng) -> G1Affine {
    loop {
        if let Some(p) = G1Affine::get_point_from_x_unchecked(rand_fq(r), r.pct(50)) {
            return p;
        }
    }
}
fn rand_twist(r: &mut Rng) -> G2Affine {
    loop {
        let x = Fq2::new(rand_fq(r), rand_fq(r));
        if let Some(q) = G2Affine::get_point_from_x_unchecked(x, r.pct(50))
            && !q.is_in_correct_subgroup_assuming_on_curve()
        {
            return q;
        }
    }
}

/// `r * h / q` for the prime factors q = 10069 and q = 5864401 of the G2 cofactor h.
const TORSION_SCALARS: [[u64; 8]; 2] = [
    [
        0x1dbb_fa43_015f_1659,
        0x88a0_ba93_34d5_d06e,
        0xc616_fa58_e5b7_2d72,
        0x8ec1_2d22_f654_1370,
        0x85dd_3e29_9a8f_ab26,
        0xf2ec_8020_adba_71a1,
        0xb0cf_b56a_567e_a5a2,
        0x0000_3b89_d84c_6eaa,
    ],
    [
        0xac3b_8251_8671_0dfd,
        0x9cc4_40ec_f042_4dfa,
        0x2306_6c21_490c_32db,
        0x7ab5_0d66_e032_2103,
        0xfe02_6954_6e82_65a2,
        0xa406_f5bb_d101_9d45,
        0x3511_a2a9_daae_b99c,
        0x0000_001a_2b77_49d0,
    ],
];

fn rand_torsion(r: &mut Rng) -> G2Affine {
    loop {
        let t = rand_twist(r).mul_bigint(TORSION_SCALARS[r.below(2)]);
        if t.is_zero() {
            continue;
        }
        return match r.below(3) {
            0 => t.into_affine(),
            1 => (t + rand_g2(r)).into_affine(),
            _ => (t.double() - rand_g2(r)).into_affine(),
        };
    }
}

impl Pools {
    /// `g1_random` random G1 points and `g2_size` points in each G2 pool.
    fn new(r: &mut Rng, g1_random: usize, g2_size: usize) -> Self {
        let mut g1: Vec<G1Affine> = (0..g1_random).map(|_| rand_g1(r)).collect();
        for k in 1..=6u64 {
            g1.push((G1Affine::generator() * Fr::from(k)).into_affine());
        }
        // x = 0 is on the curve when 3 is a square.
        if let Some(p) = G1Affine::get_point_from_x_unchecked(Fq::zero(), true) {
            g1.push(p);
            g1.push(-p);
        }
        Self {
            g1,
            g1_random,
            g2: (0..g2_size).map(|_| rand_g2(r)).collect(),
            twist: (0..g2_size).map(|_| rand_twist(r)).collect(),
            torsion: (0..g2_size).map(|_| rand_torsion(r)).collect(),
        }
    }
    fn refresh(&mut self, r: &mut Rng, iteration: u64) {
        if iteration.is_multiple_of(256) {
            let i = r.below(self.g1_random);
            self.g1[i] = rand_g1(r);
        }
        if iteration.is_multiple_of(4096) {
            let i = r.below(self.g2.len());
            self.g2[i] = rand_g2(r);
            let j = r.below(self.twist.len());
            self.twist[j] = rand_twist(r);
            let k = r.below(self.torsion.len());
            self.torsion[k] = rand_torsion(r);
        }
    }
}

struct Consts {
    p: [u8; 32],
    p_minus_1: [u8; 32],
    r: BigInt<4>,
    lambda: Fr,
}

impl Consts {
    fn new() -> Self {
        let mut p_minus_1 = Fq::MODULUS;
        p_minus_1.sub_with_borrow(&small(1));
        Self {
            p: be32(&Fq::MODULUS),
            p_minus_1: be32(&p_minus_1),
            r: Fr::MODULUS,
            lambda: <ark_bn254::g1::Config as GLVConfig>::LAMBDA,
        }
    }
}

fn gen_g1(r: &mut Rng, pl: &Pools, c: &Consts) -> [u8; 64] {
    let p = pl.g1[r.below(pl.g1.len())];
    let half = r.below(2) * 32;
    let mut e = enc_g1(&p);
    match r.below(100) {
        0..=34 => {}
        35..=42 => e = [0; 64],
        43..=50 => e = enc_g1(&-p),
        51..=55 => e = enc_g1(&G1Affine::generator()),
        56..=63 => e = enc_g1(&g1_from_x(r)),
        // Off the curve (or, rarely, a different valid point).
        64..=69 => {
            let v: [u8; 32] = e[half..half + 32].try_into().unwrap();
            if let Some(v) = add_be(&v, &be32(&small(1 + r.below(3) as u64))) {
                e[half..half + 32].copy_from_slice(&v);
            }
        }
        // Same residue, but the coordinate is >= p: must be rejected.
        70..=75 => {
            let v: [u8; 32] = e[half..half + 32].try_into().unwrap();
            if let Some(v) = add_be(&v, &c.p) {
                e[half..half + 32].copy_from_slice(&v);
            }
        }
        76..=79 => {
            let v = if r.pct(50) { c.p } else { c.p_minus_1 };
            e[half..half + 32].copy_from_slice(&v);
        }
        80..=84 => e.copy_from_slice(&r.bytes(64)),
        85..=88 => e[half..half + 32].fill(0),
        89..=93 => flip_bits(r, &mut e),
        94..=95 => e = [0xff; 64],
        _ => e = enc_g1(&(G1Affine::generator() * Fr::from(1 + r.below(64) as u64)).into_affine()),
    }
    e
}

fn gen_scalar(r: &mut Rng, c: &Consts) -> [u8; 32] {
    match r.below(100) {
        0..=4 => be32(&small(r.below(4) as u64)),
        // Uniform bit length, so short scalars (double-and-add) are as common as long ones.
        5..=39 => {
            let bits = 1 + r.below(256);
            let mut b = r.b32();
            let clear = 256 - bits;
            for byte in b.iter_mut().take(clear / 8) {
                *byte = 0;
            }
            b[clear / 8] &= 0xff >> (clear % 8);
            b[clear / 8] |= 0x80 >> (clear % 8);
            b
        }
        40..=49 => {
            let picks = [
                0x7fff, 0x8000, 0xffff, 0x1_0000, 0x1_0001, 0x1_ffff, 0x2_0000,
            ];
            let v = if r.pct(70) {
                picks[r.below(picks.len())]
            } else {
                r.next() & ((1 << (15 + r.below(3))) - 1)
            };
            be32(&small(v))
        }
        // Multiples of r plus or minus a little: must reduce mod r.
        50..=64 => {
            let mut v = c.r;
            for _ in 0..r.below(5) {
                if v.add_with_carry(&c.r) {
                    return r.b32();
                }
            }
            let d = small(if r.pct(50) {
                r.below(4) as u64
            } else {
                r.next() & 0xffff
            });
            let overflow = if r.pct(50) {
                v.add_with_carry(&d)
            } else {
                v.sub_with_borrow(&d)
            };
            if overflow { r.b32() } else { be32(&v) }
        }
        // Around the 2^127 / 2^128 GLV half-size and the top bits.
        65..=74 => {
            let bit = [126u32, 127, 128, 129, 253, 254, 255][r.below(7)];
            let mut v = BigInt::<4>::zero();
            v.0[(bit / 64) as usize] = 1 << (bit % 64);
            let d = small(r.below(3) as u64);
            if r.pct(50) {
                v.add_with_carry(&d);
            } else {
                v.sub_with_borrow(&d);
            }
            be32(&v)
        }
        // Near the GLV eigenvalue: k = a + b·lambda with small or half-size a, b.
        75..=84 => {
            let part = |r: &mut Rng| -> Fr {
                let v = if r.pct(50) {
                    r.below(8) as u64
                } else {
                    r.next()
                };
                let f = Fr::from(v);
                if r.pct(30) {
                    f * Fr::from(1u128 << 64)
                } else {
                    f
                }
            };
            let (a, b) = (part(r), part(r));
            let k = if r.pct(50) {
                a + b * c.lambda
            } else {
                a - b * c.lambda
            };
            let mut v = k.into_bigint();
            if r.pct(25) {
                let _ = v.add_with_carry(&c.r);
            }
            be32(&v)
        }
        85..=94 => r.b32(),
        _ => [0xff; 32],
    }
}

fn gen_g2(r: &mut Rng, pl: &Pools, c: &Consts) -> [u8; 128] {
    gen_g2_class(r, pl, c, None)
}

/// The first value of each branch of `gen_g2_class`.
const G2_CLASSES: [usize; 9] = [0, 60, 68, 73, 78, 83, 87, 91, 96];

/// A G2 encoding from the branch `class` (in 0..100) selects, or from a random branch.
fn gen_g2_class(r: &mut Rng, pl: &Pools, c: &Consts, class: Option<usize>) -> [u8; 128] {
    let q = pl.g2[r.below(pl.g2.len())];
    let mut e = enc_g2(&q);
    let quarter = r.below(4) * 32;
    match class.unwrap_or_else(|| r.below(100)) {
        0..=59 => {}
        60..=67 => e = [0; 128],
        // On the twist, outside the subgroup: must be rejected.
        68..=72 => e = enc_g2(&pl.twist[r.below(pl.twist.len())]),
        // Outside the subgroup only by a small-order component: must be rejected.
        73..=77 => e = enc_g2(&pl.torsion[r.below(pl.torsion.len())]),
        78..=82 => {
            let v: [u8; 32] = e[quarter..quarter + 32].try_into().unwrap();
            if let Some(v) = add_be(&v, &be32(&small(1))) {
                e[quarter..quarter + 32].copy_from_slice(&v);
            }
        }
        83..=86 => {
            let v: [u8; 32] = e[quarter..quarter + 32].try_into().unwrap();
            if let Some(v) = add_be(&v, &c.p) {
                e[quarter..quarter + 32].copy_from_slice(&v);
            }
        }
        // Real and imaginary parts swapped.
        87..=90 => {
            let swapped = [&e[32..64], &e[..32], &e[96..128], &e[64..96]].concat();
            e.copy_from_slice(&swapped);
        }
        91..=95 => e.copy_from_slice(&r.bytes(128)),
        _ => flip_bits(r, &mut e),
    }
    e
}

fn gen_pairing(r: &mut Rng, pl: &Pools, c: &Consts) -> Vec<u8> {
    let mut pairs: Vec<([u8; 64], [u8; 128])> = Vec::new();
    if r.pct(45) {
        // prod_i e(a_i P_i, b_i Q) · e(-(sum_i a_i b_i P_i), Q) = 1 with 2..=9 effective
        // pairs (odd counts too), so the expected answer is true unless corrupted below.
        let q = pl.g2[r.below(pl.g2.len())];
        let k = 2 + r.below(8);
        let mut sum = ark_bn254::G1Projective::zero();
        for _ in 0..k - 1 {
            let p = pl.g1[r.below(pl.g1.len())];
            let (a, b) = (rand_fr(r), rand_fr(r));
            pairs.push((
                enc_g1(&(p * a).into_affine()),
                enc_g2(&(q * b).into_affine()),
            ));
            sum += p * (a * b);
        }
        pairs.push((enc_g1(&(-sum).into_affine()), enc_g2(&q)));
        for _ in 0..r.below(3) {
            let (g1, g2) = if r.pct(50) {
                ([0u8; 64], enc_g2(&pl.g2[r.below(pl.g2.len())]))
            } else {
                (enc_g1(&pl.g1[r.below(pl.g1.len())]), [0u8; 128])
            };
            let at = r.below(pairs.len() + 1);
            pairs.insert(at, (g1, g2));
        }
        if r.pct(20) {
            let i = r.below(pairs.len());
            if r.pct(50) {
                pairs[i].0 = gen_g1(r, pl, c);
            } else {
                pairs[i].1 = gen_g2(r, pl, c);
            }
        }
    } else {
        let n = if r.pct(5) {
            0
        } else if r.pct(85) {
            1 + r.below(4)
        } else {
            1 + r.below(8)
        };
        for _ in 0..n {
            pairs.push((gen_g1(r, pl, c), gen_g2(r, pl, c)));
        }
    }
    let mut out: Vec<u8> = pairs
        .iter()
        .flat_map(|(a, b)| a.iter().chain(b.iter()).copied())
        .collect();
    if r.pct(3) {
        if r.pct(50) && !out.is_empty() {
            out.truncate(r.below(out.len()));
        } else {
            let n = 1 + r.below(191);
            out.extend(r.bytes(n));
        }
    }
    out
}

fn vary_length(r: &mut Rng, mut input: Vec<u8>) -> Vec<u8> {
    match r.below(100) {
        0..=69 => {}
        70..=84 => input.truncate(r.below(input.len())),
        _ => {
            let n = 1 + r.below(64);
            input.extend(r.bytes(n));
        }
    }
    input
}

fn gen_input(r: &mut Rng, pl: &Pools, c: &Consts) -> (Op, Vec<u8>) {
    let (op, mut input) = match r.below(100) {
        0..=44 => {
            let a = gen_g1(r, pl, c);
            let b = match r.below(100) {
                0..=14 => a,
                15..=24 => {
                    let p = pl.g1[r.below(pl.g1.len())];
                    let input = [enc_g1(&p), enc_g1(&-p)].concat();
                    return (Op::Add, vary_length(r, input));
                }
                _ => gen_g1(r, pl, c),
            };
            (Op::Add, vary_length(r, [a, b].concat()))
        }
        45..=89 => {
            let input = [&gen_g1(r, pl, c)[..], &gen_scalar(r, c)[..]].concat();
            (Op::Mul, vary_length(r, input))
        }
        _ => (Op::Pairing, gen_pairing(r, pl, c)),
    };
    if r.pct(3) {
        flip_bits(r, &mut input);
    }
    (op, input)
}

// ---------------------------------------------------------------- driver

#[derive(Default)]
struct Stats {
    total: [u64; 3],
    accepted: [u64; 3],
    rejected: [u64; 3],
    pairing_true: u64,
    native_vs_ref: u64,
    default_vs_ref: u64,
    panics: u64,
}

impl Stats {
    fn merge(&mut self, other: &Stats) {
        for i in 0..3 {
            self.total[i] += other.total[i];
            self.accepted[i] += other.accepted[i];
            self.rejected[i] += other.rejected[i];
        }
        self.pairing_true += other.pairing_true;
        self.native_vs_ref += other.native_vs_ref;
        self.default_vs_ref += other.default_vs_ref;
        self.panics += other.panics;
    }
    fn failures(&self) -> u64 {
        self.native_vs_ref + self.default_vs_ref + self.panics
    }
}

impl fmt::Display for Stats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "add={} (ok {} / rejected {}) mul={} (ok {} / rejected {}) \
             pairing={} (ok {} [true {}] / rejected {}) | \
             native_vs_ref={} default_vs_ref={} panics={}",
            self.total[0],
            self.accepted[0],
            self.rejected[0],
            self.total[1],
            self.accepted[1],
            self.rejected[1],
            self.total[2],
            self.accepted[2],
            self.pairing_true,
            self.rejected[2],
            self.native_vs_ref,
            self.default_vs_ref,
            self.panics,
        )
    }
}

fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}
fn show(o: &Outcome) -> String {
    match o {
        Ok(Some(v)) => hex(v),
        Ok(None) => "ERR".into(),
        Err(e) => format!("PANIC({e})"),
    }
}

/// Checks one input against the reference and records it in `stats`.
fn check(op: Op, input: &[u8], stats: &mut Stats) {
    let expected = reference(op, input);
    let native = levm(op, input, &NativeCrypto);
    let default = levm(op, input, &DefaultCrypto);
    let i = op as usize;
    stats.total[i] += 1;
    match &expected {
        Ok(Some(v)) => {
            stats.accepted[i] += 1;
            if op == Op::Pairing && v[31] == 1 {
                stats.pairing_true += 1;
            }
        }
        Ok(None) => stats.rejected[i] += 1,
        Err(_) => {}
    }
    let failures_before = stats.failures();
    if [&expected, &native, &default].iter().any(|o| o.is_err()) {
        stats.panics += 1;
    }
    if native != expected {
        stats.native_vs_ref += 1;
    }
    if default != expected {
        stats.default_vs_ref += 1;
    }
    if stats.failures() > failures_before && failures_before < 20 {
        eprintln!(
            "BN254 MISMATCH {op:?} input={} native={} default={} reference={}",
            hex(input),
            show(&native),
            show(&default),
            show(&expected)
        );
    }
}

/// Unoptimized builds run the curve arithmetic far slower (one true pairing input takes about a
/// quarter of a second there), so they use smaller pools and fewer cases.
const UNOPTIMIZED: bool = cfg!(debug_assertions);
const QUICK_CASES: u64 = if UNOPTIMIZED { 170 } else { 6000 };
const QUICK_G1_RANDOM: usize = if UNOPTIMIZED { 24 } else { 120 };
const QUICK_G2_POOL: usize = if UNOPTIMIZED { 8 } else { 32 };

#[test]
fn difffuzz_bn254_quick() {
    let mut r = Rng(0xB254_0B25);
    let c = Consts::new();
    let mut pools = Pools::new(&mut r, QUICK_G1_RANDOM, QUICK_G2_POOL);
    let mut stats = Stats::default();
    for iteration in 1..=QUICK_CASES {
        pools.refresh(&mut r, iteration);
        let (op, input) = gen_input(&mut r, &pools, &c);
        check(op, &input, &mut stats);
    }
    let true_pairings = stats.pairing_true;
    // Pairing inputs, the only ones with G2 points, are a tenth of the mix: too few in
    // unoptimized builds to reach every G2 encoding. Each one is also checked directly, paired
    // with G1 at infinity so that no implementation computes a pairing.
    for _ in 0..3 {
        for class in G2_CLASSES {
            let g2 = gen_g2_class(&mut r, &pools, &c, Some(class));
            let input = [[0u8; 64].as_slice(), &g2].concat();
            check(Op::Pairing, &input, &mut stats);
        }
    }
    eprintln!("difffuzz_bn254_quick: {stats}");
    assert!(true_pairings > 0, "no true pairing generated");
    assert_eq!(stats.failures(), 0, "{stats}");
}

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[test]
#[ignore = "long-running differential fuzzer, see the module docs"]
fn difffuzz_bn254_long() {
    let secs: u64 = env_or("DIFFFUZZ_SECS", 60);
    let seed: u64 = env_or("DIFFFUZZ_SEED", 1);
    let threads: usize = env_or("DIFFFUZZ_THREADS", 1);
    let deadline = Instant::now() + Duration::from_secs(secs);
    let handles: Vec<_> = (0..threads)
        .map(|t| {
            std::thread::spawn(move || {
                let mut r = Rng(seed ^ (t as u64).wrapping_mul(0xA24B_AED4_963E_E407));
                let c = Consts::new();
                let mut pools = Pools::new(&mut r, 120, 32);
                let mut stats = Stats::default();
                let mut iteration = 0u64;
                while Instant::now() < deadline {
                    for _ in 0..64 {
                        iteration += 1;
                        pools.refresh(&mut r, iteration);
                        let (op, input) = gen_input(&mut r, &pools, &c);
                        check(op, &input, &mut stats);
                    }
                }
                stats
            })
        })
        .collect();
    let mut stats = Stats::default();
    for h in handles {
        stats.merge(&h.join().unwrap());
    }
    eprintln!("difffuzz_bn254_long: seed={seed} threads={threads} secs={secs} {stats}");
    assert_eq!(stats.failures(), 0, "{stats}");
}
