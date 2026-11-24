//! leanSPHINCS signature verification (EIP-8504).
//!
//! leanSPHINCS is SLH-DSA (FIPS 205) instantiated with Keccak-256 and with
//! WOTS+C as its one-time signature. The scheme is specified by the
//! Cryptography/Sphincs module of ethereum/EthLib; the `lean_sphincs` crate this
//! module wraps reproduces that module's test vectors byte for byte.

use lean_sphincs::{
    LeanSphincsPublicKey, LeanSphincsSignature, MESSAGE_LEN, PUB_KEY_SIZE, SIG_SIZE, verify,
};

/// Size in bytes of a leanSPHINCS public key: `PK.seed` followed by `PK.root`.
pub const LEANSPHINCS_PUBLIC_KEY_SIZE: usize = 32;
/// Size in bytes of the message a leanSPHINCS signature signs.
pub const LEANSPHINCS_MESSAGE_SIZE: usize = 32;
/// Size in bytes of a leanSPHINCS signature.
pub const LEANSPHINCS_SIGNATURE_SIZE: usize = 7740;

// The sizes above are consensus constants. A bump of the verifier crate that
// changes its parameter set must fail the build here, not change them silently.
const _: () = assert!(PUB_KEY_SIZE == LEANSPHINCS_PUBLIC_KEY_SIZE);
const _: () = assert!(MESSAGE_LEN == LEANSPHINCS_MESSAGE_SIZE);
const _: () = assert!(SIG_SIZE == LEANSPHINCS_SIGNATURE_SIZE);

/// Returns whether `signature` is a valid leanSPHINCS signature of `msg` under
/// the public key `pk`.
///
/// Returns `false`, without failing, when `signature` is not
/// [`LEANSPHINCS_SIGNATURE_SIZE`] bytes long.
pub fn leansphincs_verify(
    pk: &[u8; LEANSPHINCS_PUBLIC_KEY_SIZE],
    msg: &[u8; LEANSPHINCS_MESSAGE_SIZE],
    signature: &[u8],
) -> bool {
    let Ok(signature) = <&[u8; LEANSPHINCS_SIGNATURE_SIZE]>::try_from(signature) else {
        return false;
    };
    verify(
        &LeanSphincsPublicKey::from_bytes(pk),
        msg,
        &LeanSphincsSignature::from_bytes(signature),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verification cases taken from EthLib's `tests/cryptography/sphincs/verify`
    /// at commit c4474ed422d83b6b735ef802ee43960abe1b788f, one per line:
    /// `name public_key message signature expected`, bytes in hex.
    const VECTORS: &str = include_str!("testdata/leansphincs_verify.txt");

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("vector holds valid hex"))
            .collect()
    }

    #[test]
    fn verifies_the_reference_vectors() {
        let mut cases = 0;
        for line in VECTORS.lines().filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let fields: Vec<&str> = line.split(' ').collect();
            let [name, pk, msg, signature, expected] = fields[..] else {
                panic!("malformed vector line: {line}");
            };
            let pk: [u8; 32] = hex(pk).try_into().expect("public key is 32 bytes");
            let msg: [u8; 32] = hex(msg).try_into().expect("message is 32 bytes");
            let expected = expected == "true";
            assert_eq!(leansphincs_verify(&pk, &msg, &hex(signature)), expected, "{name}");
            cases += 1;
        }
        assert_eq!(cases, 8);
    }

    #[test]
    fn rejects_a_signature_of_the_wrong_length() {
        let pk = [0u8; 32];
        let msg = [0u8; 32];
        assert!(!leansphincs_verify(&pk, &msg, &[]));
        assert!(!leansphincs_verify(&pk, &msg, &[0u8; LEANSPHINCS_SIGNATURE_SIZE - 1]));
        assert!(!leansphincs_verify(&pk, &msg, &[0u8; LEANSPHINCS_SIGNATURE_SIZE + 1]));
    }
}
