//! Differential fuzzing of the BLS12-381 pairing check at the `Crypto` API level.
//!
//! `NativeCrypto::bls12_381_pairing_check` (blst, multi-pair Miller loop) runs on the 48-byte
//! coordinates against an independent model on `ark-bls12-381` written from EIP-2537: field
//! elements must be below p, points must be on the curve and in the order-r subgroup, the
//! all-zero encoding is the point at infinity, and the answer is whether the product of the
//! pairings is one. Inputs mix true pairings (`e(aP, bQ) * e(-(ab)P, Q) = 1`, with infinity pairs
//! inserted and the occasional corrupted point), points off the subgroup, off the curve or with
//! coordinates at or above p, G2 points with c0/c1 swapped, and random point lists.
//!
//! The 64-byte padding and the input length are LEVM's to parse, so inputs with non-zero padding
//! or a length that is not a multiple of 384 are skipped here; `levm::difffuzz_bls` drives the
//! same inputs through the EIP-2537 precompile, padding included, and reuses this module's input
//! generators and model.
//!
//! `difffuzz_bls_crypto_quick` runs a fixed seed as part of the regular suite (fewer cases in
//! unoptimized builds), plus a few of each kind of input the generators produce too rarely for
//! that count. The ignored `difffuzz_bls_crypto_long` generates cases until a deadline,
//! configured through `DIFFFUZZ_SECS` (default 60), `DIFFFUZZ_SEED` (default 1) and
//! `DIFFFUZZ_THREADS` (default 1):
//!
//! ```text
//! DIFFFUZZ_SECS=3600 DIFFFUZZ_THREADS=16 \
//!     cargo test --release -p ethrex-test --test ethrex_tests difffuzz_ -- --ignored
//! ```

use ark_bls12_381::{Bls12_381, Fq, Fq2, Fr, G1Affine, G2Affine};
use ark_ec::{AffineRepr, CurveGroup, pairing::Pairing};
use ark_ff::{BigInt, BigInteger, One, PrimeField, Zero};
use ethrex_crypto::{Crypto, NativeCrypto};

/// Bytes of one encoded (G1, G2) pair: four 64-byte padded field elements for G2, two for G1.
const PAIR: usize = 384;

// ------------------------------------------------------------------ reference (EIP-2537)

fn fp(b: &[u8]) -> Option<Fq> {
    if b[..16].iter().any(|&x| x != 0) {
        return None; // padding must be zero
    }
    let mut limbs = [0u64; 6];
    for (i, limb) in limbs.iter_mut().enumerate() {
        let end = 64 - 8 * i;
        *limb = u64::from_be_bytes(b[end - 8..end].try_into().unwrap());
    }
    Fq::from_bigint(BigInt(limbs)) // None when >= p
}

fn g1(b: &[u8]) -> Option<G1Affine> {
    let (x, y) = (fp(&b[..64])?, fp(&b[64..128])?);
    if x.is_zero() && y.is_zero() {
        return Some(G1Affine::identity());
    }
    let p = G1Affine::new_unchecked(x, y);
    (p.is_on_curve() && p.is_in_correct_subgroup_assuming_on_curve()).then_some(p)
}

fn g2(b: &[u8]) -> Option<G2Affine> {
    let x = Fq2::new(fp(&b[..64])?, fp(&b[64..128])?);
    let y = Fq2::new(fp(&b[128..192])?, fp(&b[192..256])?);
    if x.is_zero() && y.is_zero() {
        return Some(G2Affine::identity());
    }
    let q = G2Affine::new_unchecked(x, y);
    (q.is_on_curve() && q.is_in_correct_subgroup_assuming_on_curve()).then_some(q)
}

/// The EIP-2537 pairing check on an input of padded pairs: `None` when the input is rejected,
/// otherwise the 32-byte boolean word. An empty input is the empty product (true); rejecting it
/// is the precompile's call.
pub(crate) fn reference(input: &[u8]) -> Option<Vec<u8>> {
    if !input.len().is_multiple_of(PAIR) {
        return None;
    }
    let mut ps = Vec::new();
    let mut qs = Vec::new();
    for chunk in input.chunks(PAIR) {
        ps.push(g1(&chunk[..128])?);
        qs.push(g2(&chunk[128..])?);
    }
    let one = Bls12_381::multi_pairing(ps, qs).0.is_one();
    let mut out = vec![0u8; 32];
    out[31] = one as u8;
    Some(out)
}

type Pair = (
    ([u8; 48], [u8; 48]),
    ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
);

/// Splits 384-byte padded pairs into the 48-byte tuples the crypto API takes. Inputs whose
/// padding is non-zero or whose length is off are skipped: that parsing belongs to LEVM.
fn to_pairs(input: &[u8]) -> Option<Vec<Pair>> {
    if !input.len().is_multiple_of(PAIR) {
        return None;
    }
    let fe = |b: &[u8]| -> Option<[u8; 48]> {
        if b[..16].iter().any(|&x| x != 0) {
            return None;
        }
        b[16..64].try_into().ok()
    };
    input
        .chunks(PAIR)
        .map(|c| {
            Some((
                (fe(&c[0..64])?, fe(&c[64..128])?),
                (
                    fe(&c[128..192])?,
                    fe(&c[192..256])?,
                    fe(&c[256..320])?,
                    fe(&c[320..384])?,
                ),
            ))
        })
        .collect()
}

fn native(input: &[u8]) -> Option<Vec<u8>> {
    let pairs = to_pairs(input)?;
    NativeCrypto.bls12_381_pairing_check(&pairs).ok().map(|ok| {
        let mut out = vec![0u8; 32];
        out[31] = ok as u8;
        out
    })
}

// ------------------------------------------------------------------ generators

/// SplitMix64: deterministic, so a seed reproduces a failing case.
pub(crate) struct Rng(pub(crate) u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn b(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

fn rand_fr(r: &mut Rng) -> Fr {
    Fr::from_be_bytes_mod_order(&r.b(32))
}
fn rand_fq(r: &mut Rng) -> Fq {
    Fq::from_be_bytes_mod_order(&r.b(48))
}

fn enc_fq(f: &Fq) -> [u8; 64] {
    let mut out = [0u8; 64];
    out[16..].copy_from_slice(&f.into_bigint().to_bytes_be());
    out
}
fn enc_g1(p: &G1Affine) -> Vec<u8> {
    if p.is_zero() {
        return vec![0u8; 128];
    }
    [enc_fq(&p.x), enc_fq(&p.y)].concat()
}
fn enc_g2(q: &G2Affine) -> Vec<u8> {
    if q.is_zero() {
        return vec![0u8; 256];
    }
    [
        enc_fq(&q.x.c0),
        enc_fq(&q.x.c1),
        enc_fq(&q.y.c0),
        enc_fq(&q.y.c1),
    ]
    .concat()
}

/// Points the generators draw from: subgroup points and on-curve points outside the subgroup.
pub(crate) struct Pools {
    g1: Vec<G1Affine>,
    g2: Vec<G2Affine>,
    g1_off_subgroup: Vec<G1Affine>,
    g2_off_subgroup: Vec<G2Affine>,
}

impl Pools {
    pub(crate) fn new(r: &mut Rng) -> Self {
        let g1 = (0..24)
            .map(|_| (G1Affine::generator() * rand_fr(r)).into_affine())
            .collect();
        let g2 = (0..12)
            .map(|_| (G2Affine::generator() * rand_fr(r)).into_affine())
            .collect();
        let mut g1_off_subgroup = Vec::new();
        while g1_off_subgroup.len() < 12 {
            if let Some(p) = G1Affine::get_point_from_x_unchecked(rand_fq(r), r.below(2) == 0)
                && !p.is_in_correct_subgroup_assuming_on_curve()
            {
                g1_off_subgroup.push(p);
            }
        }
        let mut g2_off_subgroup = Vec::new();
        while g2_off_subgroup.len() < 12 {
            let x = Fq2::new(rand_fq(r), rand_fq(r));
            if let Some(q) = G2Affine::get_point_from_x_unchecked(x, r.below(2) == 0)
                && !q.is_in_correct_subgroup_assuming_on_curve()
            {
                g2_off_subgroup.push(q);
            }
        }
        Self {
            g1,
            g2,
            g1_off_subgroup,
            g2_off_subgroup,
        }
    }
}

/// Adds p to the 48-byte value inside a 64-byte padded field element (same residue, out of range).
fn add_p(field: &mut [u8]) {
    let mut v = BigInt::<6>::zero();
    for i in 0..6 {
        let end = 64 - 8 * i;
        v.0[i] = u64::from_be_bytes(field[end - 8..end].try_into().unwrap());
    }
    if !v.add_with_carry(&Fq::MODULUS) {
        field[16..].copy_from_slice(&v.to_bytes_be());
    }
}

fn corrupt(r: &mut Rng, point: &mut [u8]) {
    let fields = point.len() / 64;
    let f = 64 * r.below(fields as u64) as usize;
    match r.below(5) {
        0 => point[f + 63] ^= 1,                  // off curve (almost surely)
        1 => add_p(&mut point[f..f + 64]),        // >= p
        2 => point[f + r.below(16) as usize] = 1, // non-zero padding
        3 => point[f + 16..f + 64].copy_from_slice(&r.b(48)),
        _ => {
            let i = r.below(point.len() as u64) as usize;
            point[i] ^= 1 << r.below(8);
        }
    }
}

fn gen_g1(r: &mut Rng, pl: &Pools) -> Vec<u8> {
    match r.below(100) {
        0..=59 => enc_g1(&pl.g1[r.below(pl.g1.len() as u64) as usize]),
        60..=69 => vec![0u8; 128],
        70..=79 => enc_g1(&pl.g1_off_subgroup[r.below(12) as usize]),
        80..=84 => enc_g1(&G1Affine::generator()),
        _ => {
            let mut e = enc_g1(&pl.g1[r.below(pl.g1.len() as u64) as usize]);
            corrupt(r, &mut e);
            e
        }
    }
}

fn gen_g2(r: &mut Rng, pl: &Pools) -> Vec<u8> {
    match r.below(100) {
        0..=59 => enc_g2(&pl.g2[r.below(pl.g2.len() as u64) as usize]),
        60..=69 => vec![0u8; 256],
        70..=79 => enc_g2(&pl.g2_off_subgroup[r.below(12) as usize]),
        80..=84 => {
            // c0/c1 swapped
            let e = enc_g2(&pl.g2[r.below(pl.g2.len() as u64) as usize]);
            [&e[64..128], &e[..64], &e[192..256], &e[128..192]].concat()
        }
        _ => {
            let mut e = enc_g2(&pl.g2[r.below(pl.g2.len() as u64) as usize]);
            corrupt(r, &mut e);
            e
        }
    }
}

/// One precompile input: a true pairing product 45% of the time, random pairs otherwise, with
/// the occasional empty, truncated or overlong input.
pub(crate) fn gen_input(r: &mut Rng, pl: &Pools) -> Vec<u8> {
    let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    if r.below(100) < 45 {
        // e(aP, bQ) * e(-(ab)P, Q) = 1
        let p = pl.g1[r.below(pl.g1.len() as u64) as usize];
        let q = pl.g2[r.below(pl.g2.len() as u64) as usize];
        let (a, b) = (rand_fr(r), rand_fr(r));
        pairs.push((
            enc_g1(&(p * a).into_affine()),
            enc_g2(&(q * b).into_affine()),
        ));
        pairs.push((enc_g1(&(-(p * (a * b))).into_affine()), enc_g2(&q)));
        for _ in 0..r.below(3) {
            let pair = if r.below(2) == 0 {
                (
                    vec![0u8; 128],
                    enc_g2(&pl.g2[r.below(pl.g2.len() as u64) as usize]),
                )
            } else {
                (
                    enc_g1(&pl.g1[r.below(pl.g1.len() as u64) as usize]),
                    vec![0u8; 256],
                )
            };
            let at = r.below(pairs.len() as u64 + 1) as usize;
            pairs.insert(at, pair);
        }
        if r.below(5) == 0 {
            let i = r.below(pairs.len() as u64) as usize;
            if r.below(2) == 0 {
                pairs[i].0 = gen_g1(r, pl);
            } else {
                pairs[i].1 = gen_g2(r, pl);
            }
        }
    } else {
        for _ in 0..1 + r.below(5) {
            pairs.push((gen_g1(r, pl), gen_g2(r, pl)));
        }
    }
    let mut out: Vec<u8> = pairs
        .into_iter()
        .flat_map(|(a, b)| [a, b].concat())
        .collect();
    let roll = r.below(100);
    mutate_length(r, &mut out, roll);
    out
}

/// Empties (`roll` 0), truncates (1) or extends (2) the input; other rolls leave it alone.
fn mutate_length(r: &mut Rng, input: &mut Vec<u8>, roll: u64) {
    match roll {
        0 => input.clear(),
        1 if !input.is_empty() => input.truncate(r.below(input.len() as u64) as usize),
        2 => {
            let n = 1 + r.below(383) as usize;
            input.extend(r.b(n));
        }
        _ => {}
    }
}

/// Inputs `gen_input` produces too rarely for the quick tests' case counts in unoptimized
/// builds: generated inputs emptied, truncated or extended (1% of its inputs each) or with one
/// padding byte of a field element set, and single pairs with one side at infinity, whose other
/// point (generated, or on the curve outside the subgroup) must still be validated.
pub(crate) fn rare_inputs(r: &mut Rng, pl: &Pools) -> Vec<Vec<u8>> {
    let mut inputs: Vec<Vec<u8>> = (0..18)
        .map(|i| {
            let mut input = gen_input(r, pl);
            if i % 6 < 3 {
                mutate_length(r, &mut input, i % 6);
            } else if input.len() >= 64 {
                let field = 64 * r.below(input.len() as u64 / 64) as usize;
                input[field + r.below(16) as usize] = 1;
            }
            input
        })
        .collect();
    for i in 0..8 {
        inputs.push([vec![0u8; 128], gen_g2(r, pl)].concat());
        inputs.push([vec![0u8; 128], enc_g2(&pl.g2_off_subgroup[i])].concat());
        inputs.push([gen_g1(r, pl), vec![0u8; 256]].concat());
        inputs.push([enc_g1(&pl.g1_off_subgroup[i]), vec![0u8; 256]].concat());
    }
    inputs
}

pub(crate) fn hex(v: &[u8]) -> String {
    v.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

// ------------------------------------------------------------------ driver

/// Returns (cases compared, true results, mismatches).
fn run(inputs: impl IntoIterator<Item = Vec<u8>>) -> (usize, usize, usize) {
    let (mut cases, mut trues, mut bad) = (0, 0, 0);
    for input in inputs {
        if to_pairs(&input).is_none() {
            continue;
        }
        cases += 1;
        let expected = reference(&input);
        if expected.as_ref().is_some_and(|v| v[31] == 1) {
            trues += 1;
        }
        let got = native(&input);
        if got != expected {
            bad += 1;
            if bad <= 10 {
                eprintln!(
                    "BLS MISMATCH input={} NativeCrypto={:?} reference={:?}",
                    hex(&input),
                    got.map(|v| hex(&v)),
                    expected.map(|v| hex(&v))
                );
            }
        }
    }
    (cases, trues, bad)
}

/// Unoptimized builds run the arkworks pairings far slower, so they run fewer cases.
const QUICK_INPUTS: usize = if cfg!(debug_assertions) { 48 } else { 700 };

#[test]
fn difffuzz_bls_crypto_quick() {
    let mut r = Rng(0xB15_0B15);
    let pools = Pools::new(&mut r);
    let (n, trues, bad) = run((0..QUICK_INPUTS).map(|_| gen_input(&mut r, &pools)));
    let (n_rare, _, bad_rare) = run(rare_inputs(&mut r, &pools));
    eprintln!(
        "difffuzz_bls_crypto_quick: cases={} true={trues} mismatches={}",
        n + n_rare,
        bad + bad_rare
    );
    assert!(trues > QUICK_INPUTS / 14, "too few true pairings: {trues}");
    assert_eq!(bad + bad_rare, 0);
}

#[test]
#[ignore = "long-running differential fuzzer, see the module docs"]
fn difffuzz_bls_crypto_long() {
    let secs: u64 = env_or("DIFFFUZZ_SECS", 60);
    let seed: u64 = env_or("DIFFFUZZ_SEED", 1);
    let threads: usize = env_or("DIFFFUZZ_THREADS", 1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let handles: Vec<_> = (0..threads)
        .map(|t| {
            std::thread::spawn(move || {
                let mut r = Rng(seed ^ (t as u64).wrapping_mul(0xA24B_AED4_963E_E407));
                let mut pools = Pools::new(&mut r);
                let (mut n, mut trues, mut bad) = (0, 0, 0);
                let mut rounds = 0u64;
                while std::time::Instant::now() < deadline {
                    let (a, b, c) = run((0..200).map(|_| gen_input(&mut r, &pools)));
                    n += a;
                    trues += b;
                    bad += c;
                    rounds += 1;
                    if rounds.is_multiple_of(20) {
                        pools = Pools::new(&mut r);
                    }
                }
                (n, trues, bad)
            })
        })
        .collect();
    let (mut n, mut trues, mut bad) = (0, 0, 0);
    for h in handles {
        let (a, b, c) = h.join().unwrap();
        n += a;
        trues += b;
        bad += c;
    }
    eprintln!(
        "difffuzz_bls_crypto_long: seed={seed} threads={threads} secs={secs} cases={n} true={trues} mismatches={bad}"
    );
    assert_eq!(bad, 0);
}
