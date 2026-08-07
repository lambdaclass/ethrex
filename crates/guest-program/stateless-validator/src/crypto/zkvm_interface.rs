//! [`ethrex_crypto::Crypto`] implementation using [`zkvm_interface`].

use alloc::{string::ToString, sync::Arc, vec, vec::Vec};
use core::mem::transmute;

use ethrex_crypto::{Crypto, CryptoError};
use zkvm_interface::{
    zkvm_blake2f, zkvm_blake2f_message, zkvm_blake2f_offset, zkvm_blake2f_state, zkvm_bls12_381_fp,
    zkvm_bls12_381_fp2, zkvm_bls12_381_g1_msm_pair, zkvm_bls12_381_g1_point,
    zkvm_bls12_381_g2_msm_pair, zkvm_bls12_381_g2_point, zkvm_bls12_381_pairing_pair,
    zkvm_bls12_381_scalar, zkvm_bls12_g1_add, zkvm_bls12_g1_msm, zkvm_bls12_g2_add,
    zkvm_bls12_g2_msm, zkvm_bls12_map_fp_to_g1, zkvm_bls12_map_fp2_to_g2, zkvm_bls12_pairing,
    zkvm_bn254_g1_add, zkvm_bn254_g1_mul, zkvm_bn254_g1_point, zkvm_bn254_g2_point,
    zkvm_bn254_pairing, zkvm_bn254_pairing_pair, zkvm_bn254_scalar, zkvm_keccak256,
    zkvm_keccak256_hash, zkvm_kzg_commitment, zkvm_kzg_field_element, zkvm_kzg_point_eval,
    zkvm_kzg_proof, zkvm_modexp, zkvm_ripemd160, zkvm_ripemd160_hash, zkvm_secp256k1_ecrecover,
    zkvm_secp256k1_hash, zkvm_secp256k1_pubkey, zkvm_secp256k1_signature, zkvm_secp256r1_hash,
    zkvm_secp256r1_pubkey, zkvm_secp256r1_signature, zkvm_secp256r1_verify, zkvm_sha256,
    zkvm_sha256_hash,
};

/// Returns a [`Crypto`] implementation backed by [`zkvm_interface`] syscalls.
#[inline]
pub(super) fn crypto() -> Arc<dyn Crypto> {
    Arc::new(ZkVMInterfaceCrypto)
}

#[derive(Debug, Default)]
struct ZkVMInterfaceCrypto;

impl Crypto for ZkVMInterfaceCrypto {
    #[inline]
    fn secp256k1_ecrecover(
        &self,
        sig: &[u8; 64],
        recid: u8,
        msg: &[u8; 32],
    ) -> Result<[u8; 32], CryptoError> {
        let msg = zkvm_secp256k1_hash { data: *msg };
        let sig = zkvm_secp256k1_signature { data: *sig };
        let mut pubkey = zkvm_secp256k1_pubkey { data: [0; 64] };
        let ret = unsafe { zkvm_secp256k1_ecrecover(&msg, &sig, recid, &mut pubkey) };
        if ret != 0 {
            return Err(CryptoError::RecoveryFailed);
        }
        Ok(keccak256(&pubkey.data))
    }

    #[inline]
    fn keccak256(&self, input: &[u8]) -> [u8; 32] {
        keccak256(input)
    }

    #[inline]
    fn sha256(&self, input: &[u8]) -> [u8; 32] {
        let mut output = zkvm_sha256_hash { data: [0; 32] };
        let ret = unsafe { zkvm_sha256(input.as_ptr(), input.len(), &mut output) };
        assert_eq!(ret, 0, "sha256 failed");
        output.data
    }

    #[inline]
    fn ripemd160(&self, input: &[u8]) -> [u8; 32] {
        let mut output = zkvm_ripemd160_hash { data: [0; 32] };
        let ret = unsafe { zkvm_ripemd160(input.as_ptr(), input.len(), &mut output) };
        assert_eq!(ret, 0, "ripemd160 failed");
        output.data
    }

    #[inline]
    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
        let p1: &[u8; 64] = p1
            .try_into()
            .map_err(|_| CryptoError::InvalidInput("bn254_g1_add: p1 must be 64 bytes"))?;
        let p2: &[u8; 64] = p2
            .try_into()
            .map_err(|_| CryptoError::InvalidInput("bn254_g1_add: p2 must be 64 bytes"))?;
        let p1 = zkvm_bn254_g1_point { data: *p1 };
        let p2 = zkvm_bn254_g1_point { data: *p2 };
        let mut result = zkvm_bn254_g1_point { data: [0; 64] };
        let ret = unsafe { zkvm_bn254_g1_add(&p1, &p2, &mut result) };
        (ret == 0)
            .then_some(result.data)
            .ok_or_else(|| CryptoError::Other("bn254_g1_add failed".to_string()))
    }

    #[inline]
    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
        let point: &[u8; 64] = point
            .try_into()
            .map_err(|_| CryptoError::InvalidInput("bn254_g1_mul: point must be 64 bytes"))?;
        let scalar: &[u8; 32] = scalar
            .try_into()
            .map_err(|_| CryptoError::InvalidInput("bn254_g1_mul: scalar must be 32 bytes"))?;
        let point = zkvm_bn254_g1_point { data: *point };
        let scalar = zkvm_bn254_scalar { data: *scalar };
        let mut result = zkvm_bn254_g1_point { data: [0; 64] };
        let ret = unsafe { zkvm_bn254_g1_mul(&point, &scalar, &mut result) };
        (ret == 0)
            .then_some(result.data)
            .ok_or_else(|| CryptoError::Other("bn254_g1_mul failed".to_string()))
    }

    #[inline]
    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
        let pairs: Vec<zkvm_bn254_pairing_pair> = pairs
            .iter()
            .map(|(g1, g2)| {
                let g1: [u8; 64] = (*g1)
                    .try_into()
                    .map_err(|_| CryptoError::InvalidInput("bn254_pairing: G1 must be 64 bytes"))?;
                let g2: [u8; 128] = (*g2).try_into().map_err(|_| {
                    CryptoError::InvalidInput("bn254_pairing: G2 must be 128 bytes")
                })?;
                Ok(zkvm_bn254_pairing_pair {
                    g1: zkvm_bn254_g1_point { data: g1 },
                    g2: zkvm_bn254_g2_point { data: g2 },
                })
            })
            .collect::<Result<_, CryptoError>>()?;
        let mut verified = false;
        let ret = unsafe { zkvm_bn254_pairing(pairs.as_ptr(), pairs.len(), &mut verified) };
        (ret == 0)
            .then_some(verified)
            .ok_or_else(|| CryptoError::Other("bn254_pairing failed".to_string()))
    }

    #[inline]
    fn modexp(&self, base: &[u8], exp: &[u8], modulus: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut output = vec![0u8; modulus.len()];
        let ret = unsafe {
            zkvm_modexp(
                base.as_ptr(),
                base.len(),
                exp.as_ptr(),
                exp.len(),
                modulus.as_ptr(),
                modulus.len(),
                output.as_mut_ptr(),
            )
        };
        (ret == 0)
            .then_some(output)
            .ok_or_else(|| CryptoError::Other("modexp failed".to_string()))
    }

    #[cfg(feature = "zisk")]
    #[inline]
    fn mulmod256(&self, a: &[u8; 32], b: &[u8; 32], m: &[u8; 32]) -> [u8; 32] {
        // `mul_mod_bytes256_c` is exported by ziskos but not declared in `zkvm_interface`.
        unsafe extern "C" {
            fn mul_mod_bytes256_c(
                a_ptr: *const u8,
                b_ptr: *const u8,
                m_ptr: *const u8,
                result_ptr: *mut u8,
            );
        }

        let mut result = [0u8; 32];
        unsafe { mul_mod_bytes256_c(a.as_ptr(), b.as_ptr(), m.as_ptr(), result.as_mut_ptr()) };
        result
    }

    #[inline]
    fn blake2_compress(&self, rounds: u32, h: &mut [u64; 8], m: [u64; 16], t: [u64; 2], f: bool) {
        let mut state = zkvm_blake2f_state {
            data: unsafe { transmute::<[u64; 8], [u8; 64]>(*h) },
        };
        let m = zkvm_blake2f_message {
            data: unsafe { transmute::<[u64; 16], [u8; 128]>(m) },
        };
        let t = zkvm_blake2f_offset {
            data: unsafe { transmute::<[u64; 2], [u8; 16]>(t) },
        };
        let ret = unsafe { zkvm_blake2f(rounds, &mut state, &m, &t, f as u8) };
        assert_eq!(ret, 0, "blake2f failed");
        *h = unsafe { transmute::<[u8; 64], [u64; 8]>(state.data) };
    }

    #[inline]
    fn secp256r1_verify(&self, msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
        let msg = zkvm_secp256r1_hash { data: *msg };
        let sig = zkvm_secp256r1_signature { data: *sig };
        let pk = zkvm_secp256r1_pubkey { data: *pk };
        let mut verified = false;
        let ret = unsafe { zkvm_secp256r1_verify(&msg, &sig, &pk, &mut verified) };
        ret == 0 && verified
    }

    #[inline]
    fn verify_kzg_proof(
        &self,
        z: &[u8; 32],
        y: &[u8; 32],
        commitment: &[u8; 48],
        proof: &[u8; 48],
    ) -> Result<(), CryptoError> {
        let commitment = zkvm_kzg_commitment { data: *commitment };
        let z = zkvm_kzg_field_element { data: *z };
        let y = zkvm_kzg_field_element { data: *y };
        let proof = zkvm_kzg_proof { data: *proof };
        let mut verified = false;
        let ret = unsafe { zkvm_kzg_point_eval(&commitment, &z, &y, &proof, &mut verified) };
        if ret != 0 {
            return Err(CryptoError::Other(
                "KZG point eval syscall failed".to_string(),
            ));
        }
        if !verified {
            return Err(CryptoError::VerificationFailed);
        }
        Ok(())
    }

    #[inline]
    fn bls12_381_g1_add(
        &self,
        a: ([u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48]),
    ) -> Result<[u8; 96], CryptoError> {
        let a = pack_bls12_381_g1(a);
        let b = pack_bls12_381_g1(b);
        let mut result = zkvm_bls12_381_g1_point { data: [0; 96] };
        let ret = unsafe { zkvm_bls12_g1_add(&a, &b, &mut result) };
        (ret == 0)
            .then_some(result.data)
            .ok_or_else(|| CryptoError::Other("bls12_g1_add failed".to_string()))
    }

    #[inline]
    fn bls12_381_g1_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 96], CryptoError> {
        let pairs: Vec<zkvm_bls12_381_g1_msm_pair> = pairs
            .iter()
            .map(|(point, scalar)| zkvm_bls12_381_g1_msm_pair {
                point: pack_bls12_381_g1(*point),
                scalar: zkvm_bls12_381_scalar { data: *scalar },
            })
            .collect();
        let mut result = zkvm_bls12_381_g1_point { data: [0; 96] };
        let ret = unsafe { zkvm_bls12_g1_msm(pairs.as_ptr(), pairs.len(), &mut result) };
        (ret == 0)
            .then_some(result.data)
            .ok_or_else(|| CryptoError::Other("bls12_g1_msm failed".to_string()))
    }

    #[inline]
    fn bls12_381_g2_add(
        &self,
        a: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    ) -> Result<[u8; 192], CryptoError> {
        let a = pack_bls12_381_g2(a);
        let b = pack_bls12_381_g2(b);
        let mut result = zkvm_bls12_381_g2_point { data: [0; 192] };
        let ret = unsafe { zkvm_bls12_g2_add(&a, &b, &mut result) };
        (ret == 0)
            .then_some(result.data)
            .ok_or_else(|| CryptoError::Other("bls12_g2_add failed".to_string()))
    }

    #[inline]
    fn bls12_381_g2_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 192], CryptoError> {
        let pairs: Vec<zkvm_bls12_381_g2_msm_pair> = pairs
            .iter()
            .map(|(point, scalar)| zkvm_bls12_381_g2_msm_pair {
                point: pack_bls12_381_g2(*point),
                scalar: zkvm_bls12_381_scalar { data: *scalar },
            })
            .collect();
        let mut result = zkvm_bls12_381_g2_point { data: [0; 192] };
        let ret = unsafe { zkvm_bls12_g2_msm(pairs.as_ptr(), pairs.len(), &mut result) };
        (ret == 0)
            .then_some(result.data)
            .ok_or_else(|| CryptoError::Other("bls12_g2_msm failed".to_string()))
    }

    #[inline]
    fn bls12_381_pairing_check(
        &self,
        pairs: &[(
            ([u8; 48], [u8; 48]),
            ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        )],
    ) -> Result<bool, CryptoError> {
        let pairs: Vec<zkvm_bls12_381_pairing_pair> = pairs
            .iter()
            .map(|(g1, g2)| zkvm_bls12_381_pairing_pair {
                g1: pack_bls12_381_g1(*g1),
                g2: pack_bls12_381_g2(*g2),
            })
            .collect();
        let mut verified = false;
        let ret = unsafe { zkvm_bls12_pairing(pairs.as_ptr(), pairs.len(), &mut verified) };
        (ret == 0)
            .then_some(verified)
            .ok_or_else(|| CryptoError::Other("bls12_pairing failed".to_string()))
    }

    #[inline]
    fn bls12_381_fp_to_g1(&self, fp: &[u8; 48]) -> Result<[u8; 96], CryptoError> {
        let fp = zkvm_bls12_381_fp { data: *fp };
        let mut result = zkvm_bls12_381_g1_point { data: [0; 96] };
        let ret = unsafe { zkvm_bls12_map_fp_to_g1(&fp, &mut result) };
        (ret == 0)
            .then_some(result.data)
            .ok_or_else(|| CryptoError::Other("bls12_map_fp_to_g1 failed".to_string()))
    }

    #[inline]
    fn bls12_381_fp2_to_g2(&self, fp2: ([u8; 48], [u8; 48])) -> Result<[u8; 192], CryptoError> {
        let fp2 = {
            let mut data = [0u8; 96];
            data[..48].copy_from_slice(&fp2.0);
            data[48..].copy_from_slice(&fp2.1);
            zkvm_bls12_381_fp2 { data }
        };
        let mut result = zkvm_bls12_381_g2_point { data: [0; 192] };
        let ret = unsafe { zkvm_bls12_map_fp2_to_g2(&fp2, &mut result) };
        (ret == 0)
            .then_some(result.data)
            .ok_or_else(|| CryptoError::Other("bls12_map_fp2_to_g2 failed".to_string()))
    }
}

#[inline]
fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut output = zkvm_keccak256_hash { data: [0; 32] };
    let ret = unsafe { zkvm_keccak256(data.as_ptr(), data.len(), &mut output) };
    assert_eq!(ret, 0, "keccak256 failed");
    output.data
}

#[inline]
fn pack_bls12_381_g1(p: ([u8; 48], [u8; 48])) -> zkvm_bls12_381_g1_point {
    let mut data = [0u8; 96];
    data[..48].copy_from_slice(&p.0);
    data[48..].copy_from_slice(&p.1);
    zkvm_bls12_381_g1_point { data }
}

#[inline]
fn pack_bls12_381_g2(p: ([u8; 48], [u8; 48], [u8; 48], [u8; 48])) -> zkvm_bls12_381_g2_point {
    let mut data = [0u8; 192];
    data[..48].copy_from_slice(&p.0);
    data[48..96].copy_from_slice(&p.1);
    data[96..144].copy_from_slice(&p.2);
    data[144..].copy_from_slice(&p.3);
    zkvm_bls12_381_g2_point { data }
}
