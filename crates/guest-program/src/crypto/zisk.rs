use ethereum_types::Address;
use ethrex_crypto::{Crypto, CryptoError};

/// FFI bindings to ZisK's crypto accelerator functions exposed via `ziskos`.
unsafe extern "C" {
    pub fn keccak256_c(input: *const u8, input_len: usize, output: *mut u8);

    pub fn sha256_c(input: *const u8, input_len: usize, output: *mut u8);

    pub fn bn254_g1_add_c(p1: *const u8, p2: *const u8, ret: *mut u8) -> u8;

    pub fn bn254_g1_mul_c(point: *const u8, scalar: *const u8, ret: *mut u8) -> u8;

    pub fn bn254_pairing_check_c(pairs: *const u8, num_pairs: usize) -> u8;

    pub fn secp256k1_ecdsa_verify_and_address_recover_c(
        sig: *const u8,
        msg: *const u8,
        pk: *const u8,
        output: *mut u8,
    ) -> u8;

    pub fn secp256k1_ecdsa_address_recover_c(
        sig: *const u8,
        recid: u8,
        msg: *const u8,
        output: *mut u8,
    ) -> u8;

    pub fn modexp_bytes_c(
        base_ptr: *const u8,
        base_len: usize,
        exp_ptr: *const u8,
        exp_len: usize,
        modulus_ptr: *const u8,
        modulus_len: usize,
        ret_ptr: *mut u8,
    ) -> usize;

    pub fn blake2b_compress_c(rounds: u32, h: *mut u64, m: *const u64, t: *const u64, f: u8);

    pub fn secp256r1_ecdsa_verify_c(msg: *const u8, sig: *const u8, pk: *const u8) -> bool;

    pub fn verify_kzg_proof_c(
        z: *const u8,
        y: *const u8,
        commitment: *const u8,
        proof: *const u8,
    ) -> bool;

    pub fn bls12_381_g1_add_c(ret: *mut u8, a: *const u8, b: *const u8) -> u8;

    pub fn bls12_381_g1_msm_c(ret: *mut u8, pairs: *const u8, num_pairs: usize) -> u8;

    pub fn bls12_381_g2_add_c(ret: *mut u8, a: *const u8, b: *const u8) -> u8;

    pub fn bls12_381_g2_msm_c(ret: *mut u8, pairs: *const u8, num_pairs: usize) -> u8;

    pub fn bls12_381_pairing_check_c(pairs: *const u8, num_pairs: usize) -> u8;

    pub fn bls12_381_fp_to_g1_c(ret: *mut u8, fp: *const u8) -> u8;

    pub fn bls12_381_fp2_to_g2_c(ret: *mut u8, fp2: *const u8) -> u8;
}

/// ZisK crypto provider.
///
/// Uses k256 for ECDSA (secp256k1) and substrate-bn for all BN254 operations.
/// Overrides `mulmod256` and `modexp` with ZisK's native circuit instructions via `ziskos`.
///
/// When building actual ZisK guest binaries, ZisK's patched crate versions
/// of k256 and substrate-bn are used transparently via Cargo patches.
#[derive(Debug)]
pub struct ZiskCrypto;

impl Crypto for ZiskCrypto {
    fn secp256k1_ecrecover(
        &self,
        sig: &[u8; 64],
        recid: u8,
        msg: &[u8; 32],
    ) -> Result<[u8; 32], CryptoError> {
        let mut output = [0u8; 32];
        let ret = unsafe {
            secp256k1_ecdsa_address_recover_c(
                sig.as_ptr(),
                recid,
                msg.as_ptr(),
                output.as_mut_ptr(),
            )
        };
        match ret {
            0 => Ok(output),
            _ => Err(CryptoError::RecoveryFailed),
        }
    }

    fn recover_signer(&self, sig: &[u8; 65], msg: &[u8; 32]) -> Result<Address, CryptoError> {
        // Extract signature (first 64 bytes) and recovery id (last byte)
        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(&sig[..64]);
        let recid = sig[64];

        let mut output = [0u8; 32];
        let ret = unsafe {
            secp256k1_ecdsa_address_recover_c(
                sig_bytes.as_ptr(),
                recid,
                msg.as_ptr(),
                output.as_mut_ptr(),
            )
        };
        match ret {
            0 => {
                // The output is already the keccak256 hash of the public key (last 20 bytes = address)
                Ok(Address::from_slice(&output[12..]))
            }
            _ => Err(CryptoError::RecoveryFailed),
        }
    }

    fn keccak256(&self, input: &[u8]) -> [u8; 32] {
        let mut output = [0u8; 32];
        unsafe {
            keccak256_c(input.as_ptr(), input.len(), output.as_mut_ptr());
        }
        output
    }

    fn sha256(&self, input: &[u8]) -> [u8; 32] {
        let mut output = [0u8; 32];
        unsafe {
            sha256_c(input.as_ptr(), input.len(), output.as_mut_ptr());
        }
        output
    }

    fn blake2_compress(&self, rounds: u32, h: &mut [u64; 8], m: [u64; 16], t: [u64; 2], f: bool) {
        unsafe {
            blake2b_compress_c(rounds, h.as_mut_ptr(), m.as_ptr(), t.as_ptr(), f as u8);
        }
    }

    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
        let mut result = [0u8; 64];
        let ret = unsafe { bn254_g1_add_c(p1.as_ptr(), p2.as_ptr(), result.as_mut_ptr()) };
        match ret {
            0 | 1 => Ok(result),
            2 => Err(CryptoError::Other(
                "bn254_g1_add inputs not in field".to_string(),
            )),
            3 => Err(CryptoError::Other(
                "bn254_g1_add point not a member of the field".to_string(),
            )),
            _ => Err(CryptoError::Other("bn254_g1_add failed".to_string())),
        }
    }

    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
        let mut result = [0u8; 64];
        let ret = unsafe { bn254_g1_mul_c(point.as_ptr(), scalar.as_ptr(), result.as_mut_ptr()) };
        match ret {
            0 | 1 => Ok(result), // 0=success, 1=success_infinity
            2 => Err(CryptoError::Other(
                "bn254_g1_mul inputs not in field".to_string(),
            )),
            3 => Err(CryptoError::Other(
                "bn254_g1_mul point not a member of the field".to_string(),
            )),
            _ => Err(CryptoError::Other("bn254_g1_mul failed".to_string())),
        }
    }

    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
        // Each pair is G1 (64 bytes) + G2 (128 bytes) = 192 bytes
        let mut pairs_bytes = Vec::new();
        for (g1, g2) in pairs {
            pairs_bytes.extend_from_slice(g1);
            pairs_bytes.extend_from_slice(g2);
        }

        let ret = unsafe { bn254_pairing_check_c(pairs_bytes.as_ptr(), pairs.len()) };
        match ret {
            0 => Ok(true),
            1 => Ok(false),
            2 => Err(CryptoError::Other(
                "bn254 G1 inputs not in field".to_string(),
            )),
            3 => Err(CryptoError::Other(
                "bn254 G1 point not a member of the field".to_string(),
            )),
            4 => Err(CryptoError::Other(
                "bn254 G2 inputs not in field".to_string(),
            )),
            5 => Err(CryptoError::Other(
                "bn254 G2 point not on curve".to_string(),
            )),
            6 => Err(CryptoError::Other(
                "bn254 pairing check subgroup check failed".to_string(),
            )),
            _ => Err(CryptoError::Other("bn254_pairing_check failed".to_string())),
        }
    }

    fn modexp(&self, base: &[u8], exp: &[u8], modulus: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut result = vec![0u8; modulus.len()];
        unsafe {
            modexp_bytes_c(
                base.as_ptr(),
                base.len(),
                exp.as_ptr(),
                exp.len(),
                modulus.as_ptr(),
                modulus.len(),
                result.as_mut_ptr(),
            );
        }
        Ok(result)
    }

    fn secp256r1_verify(&self, msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
        unsafe { secp256r1_ecdsa_verify_c(msg.as_ptr(), sig.as_ptr(), pk.as_ptr()) }
    }

    fn verify_kzg_proof(
        &self,
        z: &[u8; 32],
        y: &[u8; 32],
        commitment: &[u8; 48],
        proof: &[u8; 48],
    ) -> Result<(), CryptoError> {
        let valid = unsafe {
            verify_kzg_proof_c(z.as_ptr(), y.as_ptr(), commitment.as_ptr(), proof.as_ptr())
        };
        if !valid {
            return Err(CryptoError::Other(
                "KZG proof verification failed".to_string(),
            ));
        }
        Ok(())
    }

    fn bls12_381_g1_add(
        &self,
        a: ([u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48]),
    ) -> Result<[u8; 96], CryptoError> {
        // G1Point is ([u8; 48], [u8; 48])
        let mut a_bytes = [0u8; 96];
        a_bytes[..48].copy_from_slice(&a.0);
        a_bytes[48..].copy_from_slice(&a.1);

        let mut b_bytes = [0u8; 96];
        b_bytes[..48].copy_from_slice(&b.0);
        b_bytes[48..].copy_from_slice(&b.1);

        let mut result = [0u8; 96];
        let ret_code =
            unsafe { bls12_381_g1_add_c(result.as_mut_ptr(), a_bytes.as_ptr(), b_bytes.as_ptr()) };

        match ret_code {
            0 | 1 => Ok(result),
            2 => Err(CryptoError::Other(
                "bls12_381_g1_add inputs not in field".to_string(),
            )),
            3 => Err(CryptoError::Other(
                "bls12_381_g1_add point not on curve".to_string(),
            )),
            _ => Err(CryptoError::Other("bls12_381_g1_add failed".to_string())),
        }
    }

    fn bls12_381_g1_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 96], CryptoError> {
        // Input is (G1Point, [u8; 32]) = (([u8; 48], [u8; 48]), [u8; 32])
        // Each pair is 96 + 32 = 128 bytes
        let mut pairs_bytes = Vec::new();
        let mut num_pairs = 0usize;
        for (point, scalar) in pairs {
            pairs_bytes.extend_from_slice(&point.0);
            pairs_bytes.extend_from_slice(&point.1);
            pairs_bytes.extend_from_slice(scalar);
            num_pairs += 1;
        }

        let mut result = [0u8; 96];
        let ret_code =
            unsafe { bls12_381_g1_msm_c(result.as_mut_ptr(), pairs_bytes.as_ptr(), num_pairs) };

        match ret_code {
            0 | 1 => Ok(result),
            2 => Err(CryptoError::Other(
                "bls12_381_g1_msm inputs not in field".to_string(),
            )),
            3 => Err(CryptoError::Other(
                "bls12_381_g1_msm point not on curve".to_string(),
            )),
            4 => Err(CryptoError::Other(
                "bls12_381_g1_msm point not in subgroup".to_string(),
            )),
            _ => Err(CryptoError::Other("bls12_381_g1_msm failed".to_string())),
        }
    }

    fn bls12_381_g2_add(
        &self,
        a: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    ) -> Result<[u8; 192], CryptoError> {
        // G2Point is ([u8; 48], [u8; 48], [u8; 48], [u8; 48])
        let mut a_bytes = [0u8; 192];
        a_bytes[..48].copy_from_slice(&a.0);
        a_bytes[48..96].copy_from_slice(&a.1);
        a_bytes[96..144].copy_from_slice(&a.2);
        a_bytes[144..].copy_from_slice(&a.3);

        let mut b_bytes = [0u8; 192];
        b_bytes[..48].copy_from_slice(&b.0);
        b_bytes[48..96].copy_from_slice(&b.1);
        b_bytes[96..144].copy_from_slice(&b.2);
        b_bytes[144..].copy_from_slice(&b.3);

        let mut result = [0u8; 192];
        let ret_code =
            unsafe { bls12_381_g2_add_c(result.as_mut_ptr(), a_bytes.as_ptr(), b_bytes.as_ptr()) };
        match ret_code {
            0 | 1 => Ok(result),
            2 => Err(CryptoError::Other(
                "bls12_381_g2_add inputs not in field".to_string(),
            )),
            3 => Err(CryptoError::Other(
                "bls12_381_g2_add point not on curve".to_string(),
            )),
            _ => Err(CryptoError::Other("bls12_381_g2_add failed".to_string())),
        }
    }

    fn bls12_381_g2_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 192], CryptoError> {
        // Input is (G2Point, [u8; 32]) = (([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])
        // Each pair is 192 + 32 = 224 bytes
        let mut pairs_bytes = Vec::new();
        let mut num_pairs = 0usize;
        for (point, scalar) in pairs {
            pairs_bytes.extend_from_slice(&point.0);
            pairs_bytes.extend_from_slice(&point.1);
            pairs_bytes.extend_from_slice(&point.2);
            pairs_bytes.extend_from_slice(&point.3);
            pairs_bytes.extend_from_slice(scalar);
            num_pairs += 1;
        }

        let mut result = [0u8; 192];
        let ret_code =
            unsafe { bls12_381_g2_msm_c(result.as_mut_ptr(), pairs_bytes.as_ptr(), num_pairs) };
        match ret_code {
            0 | 1 => Ok(result),
            2 => Err(CryptoError::Other(
                "bls12_381_g2_msm inputs not in field".to_string(),
            )),
            3 => Err(CryptoError::Other(
                "bls12_381_g2_msm point not on curve".to_string(),
            )),
            4 => Err(CryptoError::Other(
                "bls12_381_g2_msm point not in subgroup".to_string(),
            )),
            _ => Err(CryptoError::Other("bls12_381_g2_msm failed".to_string())),
        }
    }

    fn bls12_381_pairing_check(
        &self,
        pairs: &[(
            ([u8; 48], [u8; 48]),
            ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        )],
    ) -> Result<bool, CryptoError> {
        // Each pair is G1Point (96 bytes) + G2Point (192 bytes) = 288 bytes
        let mut pairs_bytes = Vec::new();
        for (g1, g2) in pairs {
            // G1Point: ([u8; 48], [u8; 48])
            pairs_bytes.extend_from_slice(&g1.0);
            pairs_bytes.extend_from_slice(&g1.1);
            // G2Point: ([u8; 48], [u8; 48], [u8; 48], [u8; 48])
            pairs_bytes.extend_from_slice(&g2.0);
            pairs_bytes.extend_from_slice(&g2.1);
            pairs_bytes.extend_from_slice(&g2.2);
            pairs_bytes.extend_from_slice(&g2.3);
        }

        let ret_code = unsafe { bls12_381_pairing_check_c(pairs_bytes.as_ptr(), pairs.len()) };
        match ret_code {
            0 => Ok(true),
            1 => Ok(false),
            2 => Err(CryptoError::Other(
                "bls12_381_pairing_check G1 inputs not in group".to_string(),
            )),
            3 => Err(CryptoError::Other(
                "bls12_381_pairing_check G1 inputs not in subgroup".to_string(),
            )),
            4 => Err(CryptoError::Other(
                "bls12_381_pairing_check G2 inputs not in group".to_string(),
            )),
            5 => Err(CryptoError::Other(
                "bls12_381_pairing_check G2 inputs not in subgroup".to_string(),
            )),
            _ => Err(CryptoError::Other(
                "bls12_381_pairing_check failed".to_string(),
            )),
        }
    }

    fn bls12_381_fp_to_g1(&self, fp: &[u8; 48]) -> Result<[u8; 96], CryptoError> {
        let mut result = [0u8; 96];
        let ret_code = unsafe { bls12_381_fp_to_g1_c(result.as_mut_ptr(), fp.as_ptr()) };
        match ret_code {
            0 => Ok(result),
            1 => Err(CryptoError::Other(
                "bls12_381_fp_to_g1 input not in field".to_string(),
            )),
            _ => Err(CryptoError::Other("bls12_381_fp_to_g1 failed".to_string())),
        }
    }

    fn bls12_381_fp2_to_g2(&self, fp2: ([u8; 48], [u8; 48])) -> Result<[u8; 192], CryptoError> {
        let mut fp2_bytes = [0u8; 96];
        fp2_bytes[..48].copy_from_slice(&fp2.0);
        fp2_bytes[48..].copy_from_slice(&fp2.1);

        let mut result = [0u8; 192];
        let ret_code = unsafe { bls12_381_fp2_to_g2_c(result.as_mut_ptr(), fp2_bytes.as_ptr()) };
        match ret_code {
            0 => Ok(result),
            1 => Err(CryptoError::Other(
                "bls12_381_fp2_to_g2 input not in field".to_string(),
            )),
            _ => Err(CryptoError::Other("bls12_381_fp2_to_g2 failed".to_string())),
        }
    }
}
