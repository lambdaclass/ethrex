use ethereum_types::Address;

/// Errors from crypto operations. Opaque — does not leak library-specific types.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("invalid signature")]
    InvalidSignature,
    #[error("invalid recovery id")]
    InvalidRecoveryId,
    #[error("recovery failed")]
    RecoveryFailed,
    #[error("invalid point: {0}")]
    InvalidPoint(&'static str),
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
    #[error("verification failed")]
    VerificationFailed,
    #[error("{0}")]
    Other(String),
}

/// All cryptographic operations the EVM needs.
///
/// Implementors provide the actual crypto — native libraries, zkVM circuits,
/// or anything else. ethrex's EVM code depends only on this trait.
///
/// Default implementations call the native (system-library) free functions in
/// [`crate::native`]. Implementors only override methods where they need
/// different behavior (e.g. zkVM-accelerated ECDSA or pairing checks).
///
/// Methods take `&self` to support `&dyn Crypto` (dynamic dispatch).
/// Implementations are typically zero-sized structs.
pub trait Crypto: Send + Sync + core::fmt::Debug {
    // ── ECDSA (secp256k1) ──────────────────────────────────────────────

    /// Recover the Ethereum address from a 64-byte signature + recovery id + 32-byte message hash.
    /// Used by the ECRECOVER precompile (0x01).
    /// Returns the 32-byte keccak hash of the uncompressed public key (address is last 20 bytes).
    fn secp256k1_ecrecover(
        &self,
        sig: &[u8; 64],
        recid: u8,
        msg: &[u8; 32],
    ) -> Result<[u8; 32], CryptoError> {
        crate::native::secp256k1_ecrecover_impl(sig, recid, msg)
    }

    /// Recover the signer address from a 65-byte signature (r||s||v) + 32-byte message hash.
    /// Used by transaction validation (tx.sender()) and EIP-7702 authority recovery.
    fn recover_signer(&self, sig: &[u8; 65], msg: &[u8; 32]) -> Result<Address, CryptoError> {
        crate::native::recover_signer_impl(sig, msg)
    }

    // ── Hashing ────────────────────────────────────────────────────────

    /// SHA-256 hash. Used by SHA2-256 precompile (0x02) and KZG point evaluation.
    fn sha256(&self, input: &[u8]) -> [u8; 32] {
        crate::native::sha256_impl(input)
    }

    /// RIPEMD-160 hash (zero-padded to 32 bytes). Used by RIPEMD-160 precompile (0x03).
    fn ripemd160(&self, input: &[u8]) -> [u8; 32] {
        crate::native::ripemd160_impl(input)
    }

    // ── BN254 (alt_bn128) ──────────────────────────────────────────────

    /// G1 point addition. Used by ECADD precompile (0x06).
    /// Input: two uncompressed G1 points (64 bytes each as big-endian x||y).
    /// Output: uncompressed G1 point (64 bytes).
    fn bn254_g1_add(&self, p1: &[u8], p2: &[u8]) -> Result<[u8; 64], CryptoError> {
        crate::native::bn254_g1_add_impl(p1, p2)
    }

    /// G1 scalar multiplication. Used by ECMUL precompile (0x07).
    /// Input: uncompressed G1 point (64 bytes) + scalar (32 bytes big-endian).
    /// Output: uncompressed G1 point (64 bytes).
    fn bn254_g1_mul(&self, point: &[u8], scalar: &[u8]) -> Result<[u8; 64], CryptoError> {
        crate::native::bn254_g1_mul_impl(point, scalar)
    }

    /// Pairing check. Used by ECPAIRING precompile (0x08).
    /// Input: pairs of (G1 64 bytes, G2 128 bytes) as raw byte slices.
    /// Returns true if the pairing equation holds.
    fn bn254_pairing_check(&self, pairs: &[(&[u8], &[u8])]) -> Result<bool, CryptoError> {
        crate::native::bn254_pairing_check_impl(pairs)
    }

    // ── Modular arithmetic ─────────────────────────────────────────────

    /// Modular exponentiation (arbitrary precision).
    /// Used by MODEXP precompile (0x05).
    fn modexp(&self, base: &[u8], exp: &[u8], modulus: &[u8]) -> Result<Vec<u8>, CryptoError> {
        crate::native::modexp_impl(base, exp, modulus)
    }

    /// 256-bit modular multiplication.
    /// Used by the MULMOD opcode. Default impl uses standard bigint arithmetic.
    /// ZisK overrides with a native circuit instruction.
    fn mulmod256(&self, a: &[u8; 32], b: &[u8; 32], m: &[u8; 32]) -> [u8; 32] {
        // Default: U256 big-endian → U512 full_mul → mod → U256 → big-endian
        let a = ethereum_types::U256::from_big_endian(a);
        let b = ethereum_types::U256::from_big_endian(b);
        let m = ethereum_types::U256::from_big_endian(m);

        let result = if m.is_zero() {
            ethereum_types::U256::zero()
        } else {
            let product = a.full_mul(b);
            let (_, rem) = product.div_mod(m.into());
            // rem fits in U256 since m is U256
            rem.try_into().unwrap_or(ethereum_types::U256::zero())
        };

        result.to_big_endian()
    }

    // ── Blake2 ─────────────────────────────────────────────────────────

    /// Blake2b compression function F. Used by BLAKE2F precompile (0x09).
    fn blake2_compress(
        &self,
        rounds: u32,
        h: &mut [u64; 8],
        m: [u64; 16],
        t: [u64; 2],
        f: bool,
    ) {
        crate::native::blake2_compress_impl(rounds, h, m, t, f)
    }

    // ── secp256r1 (P-256) ──────────────────────────────────────────────

    /// P-256 signature verification. Used by P256VERIFY precompile (0x0100, Osaka).
    fn secp256r1_verify(&self, msg: &[u8; 32], sig: &[u8; 64], pk: &[u8; 64]) -> bool {
        crate::native::secp256r1_verify_impl(msg, sig, pk)
    }

    // ── KZG ────────────────────────────────────────────────────────────

    /// KZG point evaluation. Used by POINT_EVALUATION precompile (0x0a, Cancun).
    fn verify_kzg_proof(
        &self,
        z: &[u8; 32],
        y: &[u8; 32],
        commitment: &[u8; 48],
        proof: &[u8; 48],
    ) -> Result<(), CryptoError> {
        crate::native::verify_kzg_proof_impl(z, y, commitment, proof)
    }

    /// Verify blob KZG proof. Used by blob transaction validation.
    fn verify_blob_kzg_proof(
        &self,
        blob: &[u8],
        commitment: &[u8; 48],
        proof: &[u8; 48],
    ) -> Result<bool, CryptoError> {
        crate::native::verify_blob_kzg_proof_impl(blob, commitment, proof)
    }

    // ── BLS12-381 (Prague, EIP-2537) ───────────────────────────────────

    /// G1 addition. Returns 96-byte unpadded G1 point.
    fn bls12_381_g1_add(
        &self,
        a: ([u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48]),
    ) -> Result<[u8; 96], CryptoError> {
        crate::native::bls12_381_g1_add_impl(a, b)
    }

    /// G1 multi-scalar multiplication. Returns 96-byte unpadded G1 point.
    fn bls12_381_g1_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 96], CryptoError> {
        crate::native::bls12_381_g1_msm_impl(pairs)
    }

    /// G2 addition. Returns 192-byte unpadded G2 point.
    fn bls12_381_g2_add(
        &self,
        a: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
        b: ([u8; 48], [u8; 48], [u8; 48], [u8; 48]),
    ) -> Result<[u8; 192], CryptoError> {
        crate::native::bls12_381_g2_add_impl(a, b)
    }

    /// G2 multi-scalar multiplication. Returns 192-byte unpadded G2 point.
    fn bls12_381_g2_msm(
        &self,
        pairs: &[(([u8; 48], [u8; 48], [u8; 48], [u8; 48]), [u8; 32])],
    ) -> Result<[u8; 192], CryptoError> {
        crate::native::bls12_381_g2_msm_impl(pairs)
    }

    /// BLS12-381 pairing check.
    fn bls12_381_pairing_check(
        &self,
        pairs: &[(([u8; 48], [u8; 48]), ([u8; 48], [u8; 48], [u8; 48], [u8; 48]))],
    ) -> Result<bool, CryptoError> {
        crate::native::bls12_381_pairing_check_impl(pairs)
    }

    /// Map field element to G1 point.
    fn bls12_381_fp_to_g1(&self, fp: &[u8; 48]) -> Result<[u8; 96], CryptoError> {
        crate::native::bls12_381_fp_to_g1_impl(fp)
    }

    /// Map field element pair to G2 point.
    fn bls12_381_fp2_to_g2(&self, fp2: ([u8; 48], [u8; 48])) -> Result<[u8; 192], CryptoError> {
        crate::native::bls12_381_fp2_to_g2_impl(fp2)
    }
}
