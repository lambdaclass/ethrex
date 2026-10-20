//! Differential fuzzing of MODEXP and keccak against independent implementations.
//!
//! - MODEXP: `Crypto::modexp` on `NativeCrypto` and on a provider that keeps every `Crypto`
//!   trait default, both against a `num-bigint` model of EIP-198 (result left-padded to the
//!   modulus length, zero for a zero modulus). Operands are shaped around the edges the
//!   implementations branch on: zero, all-ones, powers of two and their neighbours, leading zero
//!   bytes, odd and even moduli of 0 to 192 bytes with the 64-bit limb boundaries favored, and
//!   bases at or around multiples of the modulus.
//! - keccak: the one-shot `keccak_hash` and the streaming `Keccak256`, fed in random splits that
//!   favor cuts at and around the 136-byte rate, both against `tiny-keccak`.
//!
//! The `*_quick` tests run a fixed seed as part of the regular suite (fewer MODEXP cases in
//! unoptimized builds). The ignored `*_long` tests generate cases until a deadline, configured
//! through `DIFFFUZZ_SECS` (default 60), `DIFFFUZZ_SEED` (default 1) and `DIFFFUZZ_THREADS`
//! (default 1):
//!
//! ```text
//! DIFFFUZZ_SECS=3600 DIFFFUZZ_THREADS=16 \
//!     cargo test --release -p ethrex-test --test ethrex_tests difffuzz_ -- --ignored
//! ```

use ethrex_crypto::{
    Crypto, NativeCrypto,
    keccak::{Keccak256, keccak_hash},
};
use num_bigint::BigUint;
use tiny_keccak::{Hasher, Keccak};

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
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ------------------------------------------------------------------ MODEXP

/// Keeps every `Crypto` default, so `modexp` runs the generic arbitrary-precision path.
#[derive(Debug)]
struct DefaultCrypto;
impl Crypto for DefaultCrypto {}

fn modexp_reference(base: &[u8], exp: &[u8], modulus: &[u8]) -> Vec<u8> {
    let m = BigUint::from_bytes_be(modulus);
    let value = if m == BigUint::ZERO {
        BigUint::ZERO
    } else {
        BigUint::from_bytes_be(base).modpow(&BigUint::from_bytes_be(exp), &m)
    };
    let bytes = value.to_bytes_be();
    let mut out = vec![0u8; modulus.len()];
    if value != BigUint::ZERO {
        out[modulus.len() - bytes.len()..].copy_from_slice(&bytes);
    }
    out
}

/// A big-endian value of `len` bytes with a structured shape.
fn shaped(r: &mut Rng, len: usize) -> Vec<u8> {
    let mut v = r.bytes(len);
    if len == 0 {
        return v;
    }
    match r.below(10) {
        0 => v.fill(0),
        1 => v.fill(0xff),
        2 => {
            // A small value padded with leading zeros to the full length.
            v.fill(0);
            v[len - 1] = r.next() as u8;
            if len >= 2 {
                v[len - 2] = r.next() as u8;
            }
        }
        3 => {
            // 2^k - 1, 2^k or 2^k + 1.
            v.fill(0);
            let k = r.below(8 * len as u64) as usize;
            v[len - 1 - k / 8] = 1 << (k % 8);
            match r.below(3) {
                0 => {
                    let x = BigUint::from_bytes_be(&v) - 1u8;
                    let b = x.to_bytes_be();
                    v.fill(0);
                    v[len - b.len()..].copy_from_slice(&b);
                }
                1 => v[len - 1] |= 1,
                _ => {}
            }
        }
        4 => {
            // Leading zero bytes inside the declared length.
            let zeros = r.below(len as u64 + 1) as usize;
            v[..zeros].fill(0);
        }
        _ => {}
    }
    v
}

fn modexp_case(r: &mut Rng) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    // Moduli of up to 128 bytes (16 limbs), mostly odd, most of the time.
    let mlen = match r.below(10) {
        0 => 0,
        1 => 129 + r.below(64) as usize,
        2 => [1, 8, 16, 24, 31, 32, 33, 63, 64, 65, 95, 96, 97, 127, 128][r.below(15) as usize],
        _ => 1 + r.below(128) as usize,
    };
    let mut modulus = shaped(r, mlen);
    if r.below(8) == 0 {
        // Full-width modulus whose lowest limb is 1 and whose other limbs are non-zero.
        let full = [32usize, 64, 128][r.below(3) as usize];
        modulus = r.bytes(full);
        for limb in modulus.chunks_mut(8) {
            if limb.iter().all(|&b| b == 0) {
                limb[7] = 1;
            }
        }
        let n = modulus.len();
        modulus[n - 8..].copy_from_slice(&1u64.to_be_bytes());
        let (blen, elen) = (r.below(full as u64 + 1) as usize, r.below(64) as usize);
        return (shaped(r, blen), shaped(r, elen), modulus);
    }
    if mlen > 0 && r.below(4) != 0 {
        modulus[mlen - 1] |= 1; // odd
    }
    let m = BigUint::from_bytes_be(&modulus);
    let blen = match r.below(4) {
        0 => mlen,
        1 => 0,
        _ => r.below(200) as usize,
    };
    let mut base = shaped(r, blen);
    if m > BigUint::from(2u8) && r.below(5) == 0 {
        // base in {m-1, m, m+1, k*m}
        let v = match r.below(4) {
            0 => &m - 1u8,
            1 => m,
            2 => &m + 1u8,
            _ => &m * BigUint::from(2 + r.below(5)),
        };
        base = v.to_bytes_be();
    }
    let elen = match r.below(6) {
        0 => 0,
        1 => 1,
        _ => r.below(64) as usize,
    };
    let exp = shaped(r, elen);
    (base, exp, modulus)
}

/// Returns the number of mismatches (the first ones printed).
fn modexp_check(r: &mut Rng, n: usize) -> usize {
    let providers: [(&str, &dyn Crypto); 2] = [
        ("NativeCrypto", &NativeCrypto),
        ("trait defaults", &DefaultCrypto),
    ];
    let mut bad = 0;
    for _ in 0..n {
        let (base, exp, modulus) = modexp_case(r);
        let expected = modexp_reference(&base, &exp, &modulus);
        for (name, crypto) in providers {
            let got = crypto.modexp(&base, &exp, &modulus);
            if !matches!(&got, Ok(v) if *v == expected) {
                bad += 1;
                if bad <= 20 {
                    eprintln!(
                        "MODEXP ({name}) MISMATCH base={} exp={} mod={} got={:?} expected={}",
                        hex(&base),
                        hex(&exp),
                        hex(&modulus),
                        got.map(|v| hex(&v)),
                        hex(&expected)
                    );
                }
            }
        }
    }
    bad
}

/// Unoptimized builds run the bigint arithmetic far slower, so they run fewer cases.
const MODEXP_QUICK_CASES: usize = if cfg!(debug_assertions) { 1200 } else { 6000 };

#[test]
fn difffuzz_modexp_quick() {
    let mut r = Rng(0x0DD_E7E5);
    assert_eq!(modexp_check(&mut r, MODEXP_QUICK_CASES), 0);
}

// ------------------------------------------------------------------ keccak

fn keccak_reference(data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut h = Keccak::v256();
    h.update(data);
    h.finalize(&mut out);
    out
}

/// Splits `len` into parts, favoring cuts at and around the 136-byte rate.
fn split(r: &mut Rng, len: usize) -> Vec<usize> {
    let mut parts = Vec::new();
    let mut left = len;
    while left > 0 {
        let part = match r.below(4) {
            0 => 0,
            1 => (136 * (1 + r.below(3) as usize))
                .saturating_add(r.below(3) as usize)
                .saturating_sub(1),
            _ => r.below(300) as usize,
        }
        .min(left);
        parts.push(part);
        left -= part;
    }
    parts
}

fn keccak_check_one(r: &mut Rng, data: &[u8]) -> bool {
    let expected = keccak_reference(data);
    if keccak_hash(data) != expected {
        eprintln!(
            "KECCAK one-shot MISMATCH len={} data={}",
            data.len(),
            hex(data)
        );
        return false;
    }
    let mut hasher = Keccak256::new();
    let mut offset = 0;
    let parts = split(r, data.len());
    for part in &parts {
        let _ = hasher.update(&data[offset..offset + part]);
        offset += part;
    }
    let streamed: [u8; 32] = hasher.finalize();
    if streamed != expected {
        eprintln!(
            "KECCAK streaming MISMATCH len={} parts={parts:?}",
            data.len()
        );
        return false;
    }
    true
}

fn keccak_check(r: &mut Rng, rounds: usize, max_len: usize) -> usize {
    let mut bad = 0;
    for _ in 0..rounds {
        let len = match r.below(4) {
            0 => r.below(max_len as u64 + 1) as usize,
            1 => 136 * r.below(8) as usize + r.below(3) as usize,
            _ => r.below(600) as usize,
        };
        let mut data = r.bytes(len);
        if r.below(8) == 0 {
            data.fill([0u8, 0xff, 0x80, 0x01][r.below(4) as usize]);
        }
        if !keccak_check_one(r, &data) {
            bad += 1;
        }
    }
    bad
}

#[test]
fn difffuzz_keccak_quick() {
    let mut r = Rng(0x0C0F_FEE5);
    let mut bad = 0;
    // Every length across the first eight blocks.
    for len in 0..=(136 * 8 + 2) {
        let data = r.bytes(len);
        if !keccak_check_one(&mut r, &data) {
            bad += 1;
        }
    }
    bad += keccak_check(&mut r, 3000, 20_000);
    assert_eq!(bad, 0);
}

// ------------------------------------------------------------------ long runs

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Runs `f` on `DIFFFUZZ_THREADS` threads until `DIFFFUZZ_SECS` elapse; `f` returns
/// (cases, mismatches) for one round.
fn long_run(name: &str, f: fn(&mut Rng) -> (usize, usize)) {
    let secs: u64 = env_or("DIFFFUZZ_SECS", 60);
    let seed: u64 = env_or("DIFFFUZZ_SEED", 1);
    let threads: usize = env_or("DIFFFUZZ_THREADS", 1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let handles: Vec<_> = (0..threads)
        .map(|t| {
            std::thread::spawn(move || {
                let mut r = Rng(seed ^ (t as u64).wrapping_mul(0xA24B_AED4_963E_E407));
                let (mut cases, mut bad) = (0usize, 0usize);
                while std::time::Instant::now() < deadline {
                    let (c, b) = f(&mut r);
                    cases += c;
                    bad += b;
                }
                (cases, bad)
            })
        })
        .collect();
    let (mut cases, mut bad) = (0, 0);
    for h in handles {
        let (c, b) = h.join().unwrap();
        cases += c;
        bad += b;
    }
    eprintln!("{name}: seed={seed} threads={threads} secs={secs} cases={cases} mismatches={bad}");
    assert_eq!(bad, 0);
}

#[test]
#[ignore = "long-running differential fuzzer, see the module docs"]
fn difffuzz_modexp_long() {
    long_run("difffuzz_modexp_long", |r| (1000, modexp_check(r, 1000)));
}

#[test]
#[ignore = "long-running differential fuzzer, see the module docs"]
fn difffuzz_keccak_long() {
    long_run("difffuzz_keccak_long", |r| {
        (1000, keccak_check(r, 1000, 100_000))
    });
}
