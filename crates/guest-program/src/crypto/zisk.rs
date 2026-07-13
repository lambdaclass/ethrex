//! ZisK crypto provider.
//!
//! Binds the standardized zkVM cryptographic accelerator C interface exposed by
//! `ziskos` (ZisK v1.0.0-alpha and later). The symbols and struct layouts mirror
//! upstream `zkvm-interface/zkvm_accelerators.h`:
//!
//! - Every accelerator returns `zkvm_status` (`ZKVM_EOK = 0`, `ZKVM_EFAIL = -1`).
//! - Verification-style calls report their boolean result through a separate
//!   `bool*` out-parameter, not the return code.
//! - `zkvm_secp256k1_ecrecover` returns the 64-byte uncompressed public key
//!   (x || y); the caller keccak-hashes it to derive the address.
//! - All fixed-size arguments are 8-byte-aligned byte arrays.
//!
//! The declarations are kept as hand-written `extern "C"` bindings (rather than
//! calling `ziskos` Rust APIs) so this module type-checks on the host, where the
//! prover backend is compiled, without pulling the guest-only `ziskos`
//! implementation into the link. The symbols are resolved only when `cargo-zisk`
//! links the guest ELF.

use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError};

/// `zkvm_status` success value. Any other value is a failure.
const ZKVM_EOK: i32 = 0;

// 8-byte-aligned byte-array structs matching the `zkvm_bytes_*` types in
// `zkvm_accelerators.h`.
#[repr(C, align(8))]
struct B16([u8; 16]);
#[repr(C, align(8))]
struct B32([u8; 32]);
#[repr(C, align(8))]
struct B48([u8; 48]);
#[repr(C, align(8))]
struct B64([u8; 64]);
#[repr(C, align(8))]
struct B96([u8; 96]);
#[repr(C, align(8))]
struct B128([u8; 128]);
#[repr(C, align(8))]
struct B192([u8; 192]);

/// `zkvm_bn254_pairing_pair`: G1 (64) followed by G2 (128).
#[repr(C)]
struct Bn254PairingPair {
    g1: B64,
    g2: B128,
}

/// `zkvm_bls12_381_g1_msm_pair`: G1 point (96) followed by scalar (32).
#[repr(C)]
struct Bls12G1MsmPair {
    point: B96,
    scalar: B32,
}

/// `zkvm_bls12_381_g2_msm_pair`: G2 point (192) followed by scalar (32).
#[repr(C)]
struct Bls12G2MsmPair {
    point: B192,
    scalar: B32,
}

/// `zkvm_bls12_381_pairing_pair`: G1 (96) followed by G2 (192).
#[repr(C)]
struct Bls12PairingPair {
    g1: B96,
    g2: B192,
}

// FFI bindings to the standardized ZisK accelerator interface (`libziskos`).
unsafe extern "C" {
    fn zkvm_keccak256(data: *const u8, len: usize, output: *mut B32) -> i32;

    fn zkvm_sha256(data: *const u8, len: usize, output: *mut B32) -> i32;

    fn zkvm_secp256k1_ecrecover(
        msg: *const B32,
        sig: *const B64,
        recid: u8,
        output: *mut B64,
    ) -> i32;

    fn zkvm_secp256r1_verify(
        msg: *const B32,
        sig: *const B64,
        pubkey: *const B64,
        verified: *mut bool,
    ) -> i32;

    fn zkvm_modexp(
        base: *const u8,
        base_len: usize,
        exp: *const u8,
        exp_len: usize,
        modulus: *const u8,
        mod_len: usize,
        output: *mut u8,
    ) -> i32;

    fn zkvm_bn254_g1_add(p1: *const B64, p2: *const B64, result: *mut B64) -> i32;

    fn zkvm_bn254_g1_mul(point: *const B64, scalar: *const B32, result: *mut B64) -> i32;

    fn zkvm_bn254_pairing(
        pairs: *const Bn254PairingPair,
        num_pairs: usize,
        verified: *mut bool,
    ) -> i32;

    fn zkvm_blake2f(rounds: u32, h: *mut B64, m: *const B128, t: *const B16, f: u8) -> i32;

    fn zkvm_kzg_point_eval(
        commitment: *const B48,
        z: *const B32,
        y: *const B32,
        proof: *const B48,
        verified: *mut bool,
    ) -> i32;

    fn zkvm_bls12_g1_add(p1: *const B96, p2: *const B96, result: *mut B96) -> i32;

    fn zkvm_bls12_g1_msm(pairs: *const Bls12G1MsmPair, num_pairs: usize, result: *mut B96) -> i32;

    fn zkvm_bls12_g2_add(p1: *const B192, p2: *const B192, result: *mut B192) -> i32;

    fn zkvm_bls12_g2_msm(pairs: *const Bls12G2MsmPair, num_pairs: usize, result: *mut B192) -> i32;

    fn zkvm_bls12_pairing(
        pairs: *const Bls12PairingPair,
        num_pairs: usize,
        verified: *mut bool,
    ) -> i32;

    fn zkvm_bls12_map_fp_to_g1(field_element: *const B48, result: *mut B96) -> i32;

    fn zkvm_bls12_map_fp2_to_g2(field_element: *const B96, result: *mut B192) -> i32;
}

/// Copy the first `N` bytes of `bytes` into a fixed array, erroring if too short.
#[inline]
fn take<const N: usize>(bytes: &[u8]) -> Result<[u8; N], CryptoError> {
    bytes
        .get(..N)
        .and_then(|s| <[u8; N]>::try_from(s).ok())
        .ok_or(CryptoError::InvalidInput("zisk: input slice too short"))
}

/// Concatenate two 48-byte limbs into a 96-byte array (BLS12-381 G1 / Fp2).
#[inline]
fn cat96(x: [u8; 48], y: [u8; 48]) -> [u8; 96] {
    let mut out = [0u8; 96];
    out[..48].copy_from_slice(&x);
    out[48..].copy_from_slice(&y);
    out
}

/// Concatenate four 48-byte limbs into a 192-byte array (BLS12-381 G2).
#[inline]
fn cat192(a: [u8; 48], b: [u8; 48], c: [u8; 48], d: [u8; 48]) -> [u8; 192] {
    let mut out = [0u8; 192];
    out[..48].copy_from_slice(&a);
    out[48..96].copy_from_slice(&b);
    out[96..144].copy_from_slice(&c);
    out[144..].copy_from_slice(&d);
    out
}

/// ZisK crypto provider.
///
/// Uses ZisK's native accelerators (keccak256, SHA-256, ECDSA secp256k1/r1,
/// BN254, BLS12-381, modexp, blake2f, KZG) via the standardized `zkvm_*` C
/// interface exposed by `ziskos`.
#[derive(Debug)]
pub struct ZiskCrypto;

impl Crypto for ZiskCrypto {
    fn secp256k1_ecrecover(
        &self,
        sig: &[u8; 64],
        recid: u8,
        msg: &[u8; 32],
    ) -> Result<[u8; 32], CryptoError> {
        let msg = B32(*msg);
        let sig = B64(*sig);
        let mut pubkey = B64([0u8; 64]);
        let status = unsafe { zkvm_secp256k1_ecrecover(&msg, &sig, recid, &mut pubkey) };
        if status != ZKVM_EOK {
            return Err(CryptoError::RecoveryFailed);
        }
        // `zkvm_secp256k1_ecrecover` returns the uncompressed public key (x || y).
        // The trait contract is the keccak hash of that key (address = last 20 bytes).
        Ok(self.keccak256(&pubkey.0))
    }

    fn recover_signer(&self, sig: &[u8; 65], msg: &[u8; 32]) -> Result<Address, CryptoError> {
        // EIP-2: reject high-s signatures (s > secp256k1n/2)
        const SECP256K1_N_HALF: [u8; 32] = [
            0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0x5d, 0x57, 0x6e, 0x73, 0x57, 0xa4, 0x50, 0x1d, 0xdf, 0xe9, 0x2f, 0x46,
            0x68, 0x1b, 0x20, 0xa0,
        ];
        #[allow(clippy::indexing_slicing)]
        if sig[32..64] > SECP256K1_N_HALF[..] {
            return Err(CryptoError::InvalidSignature);
        }

        let sig64 = take::<64>(sig)?;
        let hash = self.secp256k1_ecrecover(&sig64, sig[64], msg)?;
        Ok(Address::from_slice(&hash[12..]))
    }

    fn keccak256(&self, input: &[u8]) -> [u8; 32] {
        let mut output = B32([0u8; 32]);
        unsafe {
            zkvm_keccak256(input.as_ptr(), input.len(), &mut output);
        }
        output.0
    }

    fn sha256(&self, input: &[u8]) -> [u8; 32] {
        let mut output = B32([0u8; 32]);
        unsafe {
            zkvm_sha256(input.as_ptr(), input.len(), &mut output);
        }
        output.0
    }

    fn blake2_compress(&self, rounds: u32, h: &mut [u64; 8], m: [u64; 16], t: [u64; 2], f: bool) {
        // The state/message/offset are little-endian u64 arrays, ABI-identical to
        // the 8-byte-aligned byte-array structs the interface expects. `h` is
        // updated in place.
        unsafe {
            zkvm_blake2f(
                rounds,
                h.as_mut_ptr().cast::<B64>(),
                m.as_ptr().cast::<B128>(),
                t.as_ptr().cast::<B16>(),
                f as u8,
            );
        }
    }

    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
        let p1 = B64(take::<64>(p1)?);
        let p2 = B64(take::<64>(p2)?);
        let mut result = B64([0u8; 64]);
        let status = unsafe { zkvm_bn254_g1_add(&p1, &p2, &mut result) };
        if status != ZKVM_EOK {
            return Err(CryptoError::InvalidPoint("bn254_g1_add failed"));
        }
        Ok(result.0)
    }

    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
        let point = B64(take::<64>(point)?);
        let scalar = B32(take::<32>(scalar)?);
        let mut result = B64([0u8; 64]);
        let status = unsafe { zkvm_bn254_g1_mul(&point, &scalar, &mut result) };
        if status != ZKVM_EOK {
            return Err(CryptoError::InvalidPoint("bn254_g1_mul failed"));
        }
        Ok(result.0)
    }

    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
        // Each pair is G1 (64 bytes) + G2 (128 bytes).
        let mut buf = Vec::with_capacity(pairs.len());
        for (g1, g2) in pairs {
            buf.push(Bn254PairingPair {
                g1: B64(take::<64>(g1)?),
                g2: B128(take::<128>(g2)?),
            });
        }

        let mut verified = false;
        let status = unsafe { zkvm_bn254_pairing(buf.as_ptr(), buf.len(), &mut verified) };
        if status != ZKVM_EOK {
            return Err(CryptoError::Other("bn254_pairing failed".to_string()));
        }
        Ok(verified)
    }

    fn modexp(&self, base: &[u8], exp: &[u8], modulus: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut result = vec![0u8; modulus.len()];
        let status = unsafe {
            zkvm_modexp(
                base.as_ptr(),
                base.len(),
                exp.as_ptr(),
                exp.len(),
                modulus.as_ptr(),
                modulus.len(),
                result.as_mut_ptr(),
            )
        };
        if status != ZKVM_EOK {
            return Err(CryptoError::Other("modexp failed".to_string()));
        }
        Ok(result)
    }

    fn secp256r1_verify(&self, msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
        let msg = B32(*msg);
        let sig = B64(*sig);
        let pk = B64(*pk);
        let mut verified = false;
        let status = unsafe { zkvm_secp256r1_verify(&msg, &sig, &pk, &mut verified) };
        status == ZKVM_EOK && verified
    }

    fn verify_kzg_proof(
        &self,
        z: &[u8; 32],
        y: &[u8; 32],
        commitment: &[u8; 48],
        proof: &[u8; 48],
    ) -> Result<(), CryptoError> {
        let z = B32(*z);
        let y = B32(*y);
        let commitment = B48(*commitment);
        let proof = B48(*proof);
        let mut verified = false;
        let status = unsafe { zkvm_kzg_point_eval(&commitment, &z, &y, &proof, &mut verified) };
        if status != ZKVM_EOK {
            return Err(CryptoError::Other(
                "KZG point evaluation failed".to_string(),
            ));
        }
        if !verified {
            return Err(CryptoError::VerificationFailed);
        }
        Ok(())
    }

    fn bls12_381_g1_add(
        &self,
        a: ([u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48]),
    ) -> Result<[u8; 96], CryptoError> {
        let p1 = B96(cat96(a.0, a.1));
        let p2 = B96(cat96(b.0, b.1));
        let mut result = B96([0u8; 96]);
        let status = unsafe { zkvm_bls12_g1_add(&p1, &p2, &mut result) };
        if status != ZKVM_EOK {
            return Err(CryptoError::InvalidPoint("bls12_381_g1_add failed"));
        }
        Ok(result.0)
    }

    fn bls12_381_g1_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 96], CryptoError> {
        let mut buf = Vec::with_capacity(pairs.len());
        for ((x, y), scalar) in pairs {
            buf.push(Bls12G1MsmPair {
                point: B96(cat96(*x, *y)),
                scalar: B32(*scalar),
            });
        }

        let mut result = B96([0u8; 96]);
        let status = unsafe { zkvm_bls12_g1_msm(buf.as_ptr(), buf.len(), &mut result) };
        if status != ZKVM_EOK {
            return Err(CryptoError::InvalidPoint("bls12_381_g1_msm failed"));
        }
        Ok(result.0)
    }

    fn bls12_381_g2_add(
        &self,
        a: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    ) -> Result<[u8; 192], CryptoError> {
        let p1 = B192(cat192(a.0, a.1, a.2, a.3));
        let p2 = B192(cat192(b.0, b.1, b.2, b.3));
        let mut result = B192([0u8; 192]);
        let status = unsafe { zkvm_bls12_g2_add(&p1, &p2, &mut result) };
        if status != ZKVM_EOK {
            return Err(CryptoError::InvalidPoint("bls12_381_g2_add failed"));
        }
        Ok(result.0)
    }

    fn bls12_381_g2_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 192], CryptoError> {
        let mut buf = Vec::with_capacity(pairs.len());
        for ((a, b, c, d), scalar) in pairs {
            buf.push(Bls12G2MsmPair {
                point: B192(cat192(*a, *b, *c, *d)),
                scalar: B32(*scalar),
            });
        }

        let mut result = B192([0u8; 192]);
        let status = unsafe { zkvm_bls12_g2_msm(buf.as_ptr(), buf.len(), &mut result) };
        if status != ZKVM_EOK {
            return Err(CryptoError::InvalidPoint("bls12_381_g2_msm failed"));
        }
        Ok(result.0)
    }

    fn bls12_381_pairing_check(
        &self,
        pairs: &[(
            ([u8; 48], [u8; 48]),
            ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        )],
    ) -> Result<bool, CryptoError> {
        let mut buf = Vec::with_capacity(pairs.len());
        for (g1, g2) in pairs {
            buf.push(Bls12PairingPair {
                g1: B96(cat96(g1.0, g1.1)),
                g2: B192(cat192(g2.0, g2.1, g2.2, g2.3)),
            });
        }

        let mut verified = false;
        let status = unsafe { zkvm_bls12_pairing(buf.as_ptr(), buf.len(), &mut verified) };
        if status != ZKVM_EOK {
            return Err(CryptoError::Other(
                "bls12_381_pairing_check failed".to_string(),
            ));
        }
        Ok(verified)
    }

    fn bls12_381_fp_to_g1(&self, fp: &[u8; 48]) -> Result<[u8; 96], CryptoError> {
        let fp = B48(*fp);
        let mut result = B96([0u8; 96]);
        let status = unsafe { zkvm_bls12_map_fp_to_g1(&fp, &mut result) };
        if status != ZKVM_EOK {
            return Err(CryptoError::InvalidPoint("bls12_381_fp_to_g1 failed"));
        }
        Ok(result.0)
    }

    fn bls12_381_fp2_to_g2(&self, fp2: ([u8; 48], [u8; 48])) -> Result<[u8; 192], CryptoError> {
        let fp2 = B96(cat96(fp2.0, fp2.1));
        let mut result = B192([0u8; 192]);
        let status = unsafe { zkvm_bls12_map_fp2_to_g2(&fp2, &mut result) };
        if status != ZKVM_EOK {
            return Err(CryptoError::InvalidPoint("bls12_381_fp2_to_g2 failed"));
        }
        Ok(result.0)
    }
}
