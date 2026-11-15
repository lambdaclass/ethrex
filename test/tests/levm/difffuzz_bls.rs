//! Differential fuzzing of the EIP-2537 pairing check precompile (0x0f).
//!
//! LEVM's `bls12_pairing_check` with `NativeCrypto` (blst) runs on whole precompile inputs, 64-byte
//! padding and length checks included, against the independent `ark-bls12-381` model of
//! EIP-2537. The input generators and the model live in `crypto::difffuzz_bls_crypto`, which
//! checks the same pairing code one layer down, through the `Crypto` API.
//!
//! `difffuzz_bls_quick` runs a fixed seed as part of the regular suite, plus a few of each kind of
//! input the generators produce too rarely for that count (empty, truncated, overlong and
//! non-zero padding inputs, and pairs with one side at infinity). In the dev profile
//! (`debug_assertions` on), where the CI runs it, it generates 48 inputs instead of 600: every
//! class still appears, but only a few times each. The release quick run and the ignored
//! `difffuzz_bls_long` cover the rest. The long test generates cases until a deadline,
//! configured through `DIFFFUZZ_SECS` (default 60), `DIFFFUZZ_SEED` (default 1) and
//! `DIFFFUZZ_THREADS` (default 1):
//!
//! ```text
//! DIFFFUZZ_SECS=3600 DIFFFUZZ_THREADS=16 \
//!     cargo test --release -p ethrex-test --test ethrex_tests difffuzz_ -- --ignored
//! ```

use ethrex_common::types::Fork;
use ethrex_crypto::NativeCrypto;
use ethrex_levm::precompiles::bls12_pairing_check;

use crate::crypto::difffuzz_bls_crypto::{
    Pools, Rng, env_or, gen_input, guarded, hex, rare_inputs,
};

/// The precompile rejects an empty input, unlike the empty product the shared model accepts.
fn reference(input: &[u8]) -> Option<Vec<u8>> {
    if input.is_empty() {
        return None;
    }
    crate::crypto::difffuzz_bls_crypto::reference(input)
}

fn levm(input: &[u8]) -> Option<Vec<u8>> {
    let mut gas = 1u64 << 40;
    bls12_pairing_check(input, &mut gas, Fork::Osaka, &NativeCrypto)
        .ok()
        .map(|b| b.to_vec())
}

/// Returns (cases, true results, mismatches).
fn run(inputs: impl IntoIterator<Item = Vec<u8>>) -> (usize, usize, usize) {
    let (mut cases, mut trues, mut bad) = (0, 0, 0);
    for input in inputs {
        cases += 1;
        let expected = reference(&input);
        if expected.as_ref().is_some_and(|v| v[31] == 1) {
            trues += 1;
        }
        let got = guarded(|| levm(&input));
        if got.as_ref() != Ok(&expected) {
            bad += 1;
            if bad <= 10 {
                eprintln!(
                    "BLS MISMATCH input={} levm={} reference={:?}",
                    hex(&input),
                    match got {
                        Ok(v) => format!("{:?}", v.map(|v| hex(&v))),
                        Err(panic) => format!("PANIC({panic})"),
                    },
                    expected.map(|v| hex(&v))
                );
            }
        }
    }
    (cases, trues, bad)
}

/// Unoptimized builds run the arkworks pairings far slower, so they run fewer cases.
const QUICK_INPUTS: usize = if cfg!(debug_assertions) { 48 } else { 600 };

#[test]
fn difffuzz_bls_quick() {
    let mut r = Rng(0xB15_0B15);
    let pools = Pools::new(&mut r);
    let (n, trues, bad) = run((0..QUICK_INPUTS).map(|_| gen_input(&mut r, &pools)));
    let (n_rare, _, bad_rare) = run(rare_inputs(&mut r, &pools));
    eprintln!(
        "difffuzz_bls_quick: cases={} true={trues} mismatches={}",
        n + n_rare,
        bad + bad_rare
    );
    assert!(trues > QUICK_INPUTS / 12, "too few true pairings: {trues}");
    assert_eq!(bad + bad_rare, 0);
}

#[test]
#[ignore = "long-running differential fuzzer, see the module docs"]
fn difffuzz_bls_long() {
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
        "difffuzz_bls_long: seed={seed} threads={threads} secs={secs} cases={n} true={trues} mismatches={bad}"
    );
    assert_eq!(bad, 0);
}
