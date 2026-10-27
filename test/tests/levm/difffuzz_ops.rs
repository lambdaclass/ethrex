//! Differential fuzzing of LEVM's arithmetic, comparison and bitwise opcodes.
//!
//! Covers the 25 opcodes from ADD (0x01) to SAR (0x1d). Each case pushes its operands, runs one
//! opcode and stores the result in memory; a batch of cases runs as a single transaction whose
//! RETURN data is compared, word by word, with an independent model of the Yellow Paper
//! semantics written on `num-bigint`. Operands come from values at the edges the
//! implementations branch on (powers of two and their neighbours, -1, limb patterns), uniform
//! bit lengths, edge-valued limbs and full random words; division-family cases are built as
//! `n = q * d + r` with extreme quotient digits and signs, shifts and BYTE/SIGNEXTEND favor small
//! first operands, and EXP favors structured bases.
//!
//! `difffuzz_ops_quick` runs every opcode on the cross product of the edge values plus a fixed
//! seed of random cases, as part of the regular suite. In the dev profile (`debug_assertions`
//! on), where the CI runs it, it checks half of the cross product (the tuples whose index sum is
//! even, so every edge value still appears in every position) and 4 random batches instead of
//! 40; the release quick run and the ignored `difffuzz_ops_long` cover the rest. The long test
//! generates random cases until a deadline, configured through `DIFFFUZZ_SECS` (default 60),
//! `DIFFFUZZ_SEED` (default 1) and `DIFFFUZZ_THREADS` (default 1):
//!
//! ```text
//! DIFFFUZZ_SECS=3600 DIFFFUZZ_THREADS=16 \
//!     cargo test --release -p ethrex-test --test ethrex_tests difffuzz_ -- --ignored
//! ```

use std::sync::Arc;

use bytes::Bytes;
use ethrex_common::{
    Address, U256,
    types::{Account, Code, Fork, LegacyTransaction, Transaction, TxKind},
};
use ethrex_crypto::NativeCrypto;
use ethrex_levm::{
    EVMConfig, Environment,
    db::gen_db::GeneralizedDatabase,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};
use num_bigint::BigUint;
use rustc_hash::FxHashMap;

use super::test_db::TestDatabase;
use crate::crypto::difffuzz_bls_crypto::{env_or, guarded};

// ------------------------------------------------------------------ reference model

fn modulus() -> BigUint {
    BigUint::from(1u8) << 256
}
fn mask() -> BigUint {
    modulus() - 1u8
}
fn big(v: &[u8; 32]) -> BigUint {
    BigUint::from_bytes_be(v)
}
fn word(v: &BigUint) -> [u8; 32] {
    let bytes = (v % modulus()).to_bytes_be();
    let mut out = [0u8; 32];
    out[32 - bytes.len()..].copy_from_slice(&bytes);
    out
}
fn is_negative(v: &BigUint) -> bool {
    v.bit(255)
}
/// Two's complement magnitude of a 256-bit word.
fn abs(v: &BigUint) -> BigUint {
    if is_negative(v) {
        modulus() - v
    } else {
        v.clone()
    }
}
fn negate(v: &BigUint) -> BigUint {
    if v == &BigUint::ZERO {
        BigUint::ZERO
    } else {
        modulus() - v
    }
}
fn bool_word(b: bool) -> BigUint {
    BigUint::from(b as u8)
}
/// Signed order of two words.
fn signed_lt(a: &BigUint, b: &BigUint) -> bool {
    match (is_negative(a), is_negative(b)) {
        (true, false) => true,
        (false, true) => false,
        _ => a < b,
    }
}

/// `a` is the top of the stack (popped first), then `b`, then `c`.
fn reference(op: u8, a: &BigUint, b: &BigUint, c: &BigUint) -> BigUint {
    let m = modulus();
    let zero = BigUint::ZERO;
    match op {
        0x01 => (a + b) % &m,
        0x02 => (a * b) % &m,
        0x03 => (a + &m - b) % &m,
        0x04 => {
            if b == &zero {
                zero
            } else {
                a / b
            }
        }
        0x05 => {
            if b == &zero {
                return zero;
            }
            let q = abs(a) / abs(b);
            if is_negative(a) != is_negative(b) {
                negate(&q)
            } else {
                q
            }
        }
        0x06 => {
            if b == &zero {
                zero
            } else {
                a % b
            }
        }
        0x07 => {
            if b == &zero {
                return zero;
            }
            let r = abs(a) % abs(b);
            if is_negative(a) { negate(&r) } else { r }
        }
        0x08 => {
            if c == &zero {
                zero
            } else {
                (a + b) % c
            }
        }
        0x09 => {
            if c == &zero {
                zero
            } else {
                (a * b) % c
            }
        }
        0x0a => a.modpow(b, &m),
        0x0b => {
            // SIGNEXTEND(b = a, x = b)
            if a >= &BigUint::from(31u8) {
                return b.clone();
            }
            let bit = 8 * a.to_u64_digits().first().copied().unwrap_or(0) + 7;
            let low_mask = (BigUint::from(1u8) << (bit + 1)) - 1u8;
            if b.bit(bit) {
                (b | (mask() ^ &low_mask)) % &m
            } else {
                b & &low_mask
            }
        }
        0x10 => bool_word(a < b),
        0x11 => bool_word(a > b),
        0x12 => bool_word(signed_lt(a, b)),
        0x13 => bool_word(signed_lt(b, a)),
        0x14 => bool_word(a == b),
        0x15 => bool_word(a == &zero),
        0x16 => a & b,
        0x17 => a | b,
        0x18 => a ^ b,
        0x19 => mask() ^ a,
        0x1a => {
            // BYTE(i = a, x = b)
            if a >= &BigUint::from(32u8) {
                return zero;
            }
            let i = a.to_u64_digits().first().copied().unwrap_or(0);
            (b >> (8 * (31 - i))) & BigUint::from(0xffu8)
        }
        0x1b => {
            // SHL(shift = a, value = b)
            if a >= &BigUint::from(256u16) {
                return zero;
            }
            let s = a.to_u64_digits().first().copied().unwrap_or(0);
            (b << s) % &m
        }
        0x1c => {
            if a >= &BigUint::from(256u16) {
                return zero;
            }
            let s = a.to_u64_digits().first().copied().unwrap_or(0);
            b >> s
        }
        0x1d => {
            // SAR(shift = a, value = b): ~((~b) >> s) for negative b.
            let negative = is_negative(b);
            if a >= &BigUint::from(256u16) {
                return if negative { mask() } else { zero };
            }
            let s = a.to_u64_digits().first().copied().unwrap_or(0);
            if negative {
                mask() - ((mask() - b) >> s)
            } else {
                b >> s
            }
        }
        _ => unreachable!("opcode {op:#x}"),
    }
}

/// (opcode, arity)
const OPS: &[(u8, usize)] = &[
    (0x01, 2),
    (0x02, 2),
    (0x03, 2),
    (0x04, 2),
    (0x05, 2),
    (0x06, 2),
    (0x07, 2),
    (0x08, 3),
    (0x09, 3),
    (0x0a, 2),
    (0x0b, 2),
    (0x10, 2),
    (0x11, 2),
    (0x12, 2),
    (0x13, 2),
    (0x14, 2),
    (0x15, 1),
    (0x16, 2),
    (0x17, 2),
    (0x18, 2),
    (0x19, 1),
    (0x1a, 2),
    (0x1b, 2),
    (0x1c, 2),
    (0x1d, 2),
];

// ------------------------------------------------------------------ LEVM harness

struct Case {
    op: u8,
    operands: Vec<[u8; 32]>, // operands[0] is the top of the stack
}

fn hex(w: &[u8; 32]) -> String {
    w.iter().map(|b| format!("{b:02x}")).collect()
}

/// Runs a batch through LEVM and returns one 32-byte word per case. When the batch fails (an
/// error, a transaction that does not succeed or a panic), runs its cases one at a time and panics
/// with the first one that fails on its own.
fn run_batch(cases: &[Case]) -> Vec<[u8; 32]> {
    let run = |cases: &[Case]| {
        guarded(|| try_run_batch(cases)).unwrap_or_else(|panic| Err(format!("PANIC({panic})")))
    };
    let failure = match run(cases) {
        Ok(words) => return words,
        Err(e) => e,
    };
    for case in cases {
        if let Err(e) = run(std::slice::from_ref(case)) {
            panic!(
                "FAILED op={:#04x} operands(top first)=[{}]: {e}",
                case.op,
                case.operands.iter().map(hex).collect::<Vec<_>>().join(", ")
            );
        }
    }
    panic!(
        "batch of {} cases failed ({failure}), but each case passes on its own",
        cases.len()
    );
}

/// Runs a batch through LEVM as one transaction and returns one 32-byte word per case.
fn try_run_batch(cases: &[Case]) -> Result<Vec<[u8; 32]>, String> {
    let mut code = Vec::with_capacity(cases.len() * 110);
    for (i, case) in cases.iter().enumerate() {
        for operand in case.operands.iter().rev() {
            code.push(0x7f); // PUSH32
            code.extend_from_slice(operand);
        }
        code.push(case.op);
        let offset = (i * 32) as u32;
        code.extend_from_slice(&[
            0x62,
            (offset >> 16) as u8,
            (offset >> 8) as u8,
            offset as u8,
        ]);
        code.push(0x52); // MSTORE
    }
    let size = (cases.len() * 32) as u32;
    code.extend_from_slice(&[0x62, (size >> 16) as u8, (size >> 8) as u8, size as u8]); // size
    code.extend_from_slice(&[0x60, 0x00, 0xf3]); // PUSH1 0, RETURN

    let contract = Address::from_low_u64_be(0xC0DE);
    let sender = Address::from_low_u64_be(0x5E4D);
    let mut db = TestDatabase::new();
    db.accounts.insert(
        contract,
        Account::new(
            U256::zero(),
            Code::from_bytecode(Bytes::from(code), &NativeCrypto),
            1,
            FxHashMap::default(),
        ),
    );
    db.accounts.insert(
        sender,
        Account::new(U256::MAX >> 8, Code::default(), 0, FxHashMap::default()),
    );
    let mut db = GeneralizedDatabase::new(Arc::new(db));
    let gas_limit = 16_000_000;
    let fork = Fork::Osaka;
    let env = Environment {
        origin: sender,
        gas_limit,
        block_gas_limit: 30_000_000,
        config: EVMConfig::new(fork, EVMConfig::canonical_values(fork)),
        chain_id: U256::one(),
        disable_balance_check: true,
        disable_nonce_check: true,
        ..Default::default()
    };
    let tx = Transaction::LegacyTransaction(LegacyTransaction {
        gas: gas_limit,
        to: TxKind::Call(contract),
        ..Default::default()
    });
    let mut vm = VM::new(
        env,
        &mut db,
        &tx,
        LevmCallTracer::disabled(),
        VMType::L1,
        &NativeCrypto,
        None,
    )
    .map_err(|e| format!("vm: {e:?}"))?;
    let report = vm.execute().map_err(|e| format!("execute: {e:?}"))?;
    if !report.is_success() {
        return Err(format!("did not succeed: {:?}", report.result));
    }
    if report.output.len() != cases.len() * 32 {
        return Err(format!("output size {}", report.output.len()));
    }
    Ok(report
        .output
        .chunks(32)
        .map(|c| c.try_into().unwrap())
        .collect())
}

/// Returns the number of mismatches (the first ones printed).
fn check(cases: &[Case]) -> usize {
    let got = run_batch(cases);
    let mut bad = 0;
    for (case, got) in cases.iter().zip(got) {
        let z = [0u8; 32];
        let a = big(case.operands.first().unwrap_or(&z));
        let b = big(case.operands.get(1).unwrap_or(&z));
        let c = big(case.operands.get(2).unwrap_or(&z));
        let expected = word(&reference(case.op, &a, &b, &c));
        if got != expected {
            bad += 1;
            if bad <= 20 {
                eprintln!(
                    "MISMATCH op={:#04x} operands(top first)=[{}] levm={} reference={}",
                    case.op,
                    case.operands.iter().map(hex).collect::<Vec<_>>().join(", "),
                    hex(&got),
                    hex(&expected)
                );
            }
        }
    }
    bad
}

// ------------------------------------------------------------------ generators

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
}

/// A word from four little-endian-ordered 64-bit limbs.
fn from_limbs(l: [u64; 4]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, limb) in l.iter().enumerate() {
        out[24 - 8 * i..32 - 8 * i].copy_from_slice(&limb.to_be_bytes());
    }
    out
}
fn from_big(v: &BigUint) -> [u8; 32] {
    word(v)
}
fn pow2(k: u32) -> BigUint {
    BigUint::from(1u8) << k
}

/// Values at the edges the implementations branch on.
fn specials() -> Vec<[u8; 32]> {
    let m = modulus();
    let mut v: Vec<BigUint> = Vec::new();
    for x in [
        0u64, 1, 2, 3, 5, 7, 8, 15, 16, 30, 31, 32, 33, 63, 64, 65, 127, 128, 129, 191, 192, 255,
        256, 257, 0xff, 0xffff, 0x1_0000,
    ] {
        v.push(BigUint::from(x));
    }
    for k in [
        8u32, 16, 31, 32, 63, 64, 65, 96, 127, 128, 129, 160, 191, 192, 193, 224, 254, 255,
    ] {
        v.push(pow2(k));
        v.push(pow2(k) - 1u8);
        v.push(pow2(k) + 1u8);
    }
    v.push(&m - 1u8); // -1
    v.push(&m - 2u8); // -2
    v.push(pow2(255) + pow2(254)); // a large negative
    // Limb patterns.
    for limbs in [
        [0, 0, 0, 1],
        [0, 0, 1, 0],
        [0, 1, 0, 0],
        [u64::MAX, 0, 0, 0],
        [0, u64::MAX, 0, 0],
        [0, 0, u64::MAX, 0],
        [0, 0, 0, u64::MAX],
        [u64::MAX, u64::MAX, 0, 0],
        [0, 0, u64::MAX, u64::MAX],
        [1 << 63, 0, 0, 1 << 63],
        [1, 1, 1, 1],
        [u64::MAX, 1, u64::MAX, 1],
    ] {
        v.push(big(&from_limbs(limbs)));
    }
    v.iter().map(from_big).collect()
}

fn random_operand(r: &mut Rng, specials: &[[u8; 32]]) -> [u8; 32] {
    match r.below(100) {
        0..=24 => specials[r.below(specials.len() as u64) as usize],
        25..=49 => {
            // Uniform bit length.
            let bits = 1 + r.below(256) as u32;
            let mut l = [r.next(), r.next(), r.next(), r.next()];
            for (i, limb) in l.iter_mut().enumerate() {
                let lo = 64 * i as u32;
                if bits <= lo {
                    *limb = 0;
                } else if bits < lo + 64 {
                    *limb &= (1u64 << (bits - lo)) - 1;
                    if bits - lo > 0 {
                        *limb |= 1u64 << (bits - lo - 1);
                    }
                }
            }
            from_limbs(l)
        }
        50..=69 => {
            // Each limb from a small edge set.
            let pick = |r: &mut Rng| {
                [0u64, 1, u64::MAX, 1 << 63, (1 << 63) - 1, r.next()][r.below(6) as usize]
            };
            from_limbs([pick(r), pick(r), pick(r), pick(r)])
        }
        70..=79 => {
            let mut x = big(&specials[r.below(specials.len() as u64) as usize]);
            x += BigUint::from(r.below(3));
            from_big(&x)
        }
        _ => from_limbs([r.next(), r.next(), r.next(), r.next()]),
    }
}

/// Division operands where the quotient digits are extreme: `n = q * d + r`.
fn division_case(r: &mut Rng, specials: &[[u8; 32]]) -> ([u8; 32], [u8; 32]) {
    let d_bits = 1 + r.below(256) as u32;
    let mut d = big(&random_operand(r, specials)) % pow2(d_bits);
    if d == BigUint::ZERO {
        d = BigUint::from(1u8);
    }
    // Normalized top digit patterns that make digit estimates overshoot.
    if r.below(3) == 0 {
        let top = d.bits().max(1) as u32;
        d |= pow2(top - 1);
        if top >= 2 && r.below(2) == 0 {
            d |= pow2(top - 2) - 1u8;
        }
    }
    let room = 256 - d.bits() as u32;
    let q = match r.below(4) {
        0 => pow2(room) - 1u8,
        1 => pow2(room.saturating_sub(1)),
        _ => big(&random_operand(r, specials)) % pow2(room.max(1)),
    };
    let rem = match r.below(3) {
        0 => BigUint::ZERO,
        1 => &d - 1u8,
        _ => big(&random_operand(r, specials)) % &d,
    };
    let n = (&q * &d + rem) % modulus();
    (from_big(&n), from_big(&d))
}

fn random_case(r: &mut Rng, specials: &[[u8; 32]]) -> Case {
    let (op, arity) = OPS[r.below(OPS.len() as u64) as usize];
    let mut operands: Vec<[u8; 32]> = (0..arity).map(|_| random_operand(r, specials)).collect();
    match op {
        // Division-family: half the time use constructed quotient/divisor pairs.
        0x04..=0x07 if r.below(2) == 0 => {
            let (n, d) = division_case(r, specials);
            operands = vec![n, d];
            if op == 0x05 || op == 0x07 {
                if r.below(2) == 0 {
                    operands[0] = from_big(&negate(&big(&operands[0])));
                }
                if r.below(2) == 0 {
                    operands[1] = from_big(&negate(&big(&operands[1])));
                }
            }
        }
        // ADDMOD/MULMOD: modulus from the division generator, operands >= modulus often.
        0x08 | 0x09 if r.below(2) == 0 => {
            let (n, d) = division_case(r, specials);
            operands[2] = d;
            operands[0] = n;
        }
        // Shifts/BYTE/SIGNEXTEND: small first operand most of the time.
        0x0b | 0x1a | 0x1b | 0x1c | 0x1d if r.below(4) != 0 => {
            operands[0] = from_big(&BigUint::from(r.below(300)));
        }
        // EXP: structured bases.
        0x0a if r.below(2) == 0 => {
            let base = match r.below(5) {
                0 => pow2(r.below(256) as u32),
                1 => modulus() - 1u8,
                2 => BigUint::from(r.below(4)),
                3 => pow2(r.below(256) as u32) + 1u8,
                _ => big(&operands[0]),
            };
            operands[0] = from_big(&base);
            if r.below(2) == 0 {
                operands[1] = from_big(&BigUint::from(r.below(600)));
            }
        }
        _ => {}
    }
    Case { op, operands }
}

/// Every opcode on the cross product of the special values (ternary ops on a subset). With
/// `half`, only the tuples whose index sum is even: every value still meets half of the others,
/// in every operand position.
fn edge_cases(half: bool) -> Vec<Case> {
    let s = specials();
    let sub: Vec<_> = s.iter().step_by(5).collect();
    let keep = |index_sum: usize| !half || index_sum.is_multiple_of(2);
    let mut cases = Vec::new();
    for &(op, arity) in OPS {
        match arity {
            1 => cases.extend(s.iter().map(|a| Case {
                op,
                operands: vec![*a],
            })),
            2 => {
                for (i, a) in s.iter().enumerate() {
                    for (j, b) in s.iter().enumerate() {
                        if keep(i + j) {
                            cases.push(Case {
                                op,
                                operands: vec![*a, *b],
                            });
                        }
                    }
                }
            }
            _ => {
                for (i, a) in sub.iter().enumerate() {
                    for (j, b) in sub.iter().enumerate() {
                        for (k, c) in sub.iter().enumerate() {
                            if keep(i + j + k) {
                                cases.push(Case {
                                    op,
                                    operands: vec![**a, **b, **c],
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    cases
}

const BATCH: usize = 2000;

/// Unoptimized builds run LEVM and the bigint model far slower: there the quick test checks half
/// of the edge-value products and fewer random batches.
const UNOPTIMIZED: bool = cfg!(debug_assertions);
const QUICK_RANDOM_BATCHES: usize = if UNOPTIMIZED { 4 } else { 40 };

#[test]
fn difffuzz_ops_quick() {
    let mut bad = 0;
    for chunk in edge_cases(UNOPTIMIZED).chunks(BATCH) {
        bad += check(chunk);
    }
    let s = specials();
    let mut r = Rng(0x0D1F_F00D);
    for _ in 0..QUICK_RANDOM_BATCHES {
        let cases: Vec<Case> = (0..BATCH).map(|_| random_case(&mut r, &s)).collect();
        bad += check(&cases);
    }
    assert_eq!(bad, 0, "{bad} mismatches");
}

#[test]
#[ignore = "long-running differential fuzzer, see the module docs"]
fn difffuzz_ops_long() {
    let secs: u64 = env_or("DIFFFUZZ_SECS", 60);
    let seed: u64 = env_or("DIFFFUZZ_SEED", 1);
    let threads: usize = env_or("DIFFFUZZ_THREADS", 1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let handles: Vec<_> = (0..threads)
        .map(|t| {
            std::thread::spawn(move || {
                let s = specials();
                let mut r = Rng(seed ^ (t as u64).wrapping_mul(0xA24B_AED4_963E_E407));
                let (mut cases_run, mut bad) = (0u64, 0usize);
                while std::time::Instant::now() < deadline {
                    let cases: Vec<Case> = (0..BATCH).map(|_| random_case(&mut r, &s)).collect();
                    bad += check(&cases);
                    cases_run += BATCH as u64;
                }
                (cases_run, bad)
            })
        })
        .collect();
    let (mut total, mut bad) = (0u64, 0usize);
    for h in handles {
        let (c, b) = h.join().unwrap();
        total += c;
        bad += b;
    }
    eprintln!(
        "difffuzz_ops_long: seed={seed} threads={threads} secs={secs} cases={total} mismatches={bad}"
    );
    assert_eq!(bad, 0);
}
