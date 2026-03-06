"""P256 (secp256r1) key generation and signing utilities for EIP-8141 demo."""
from __future__ import annotations

from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric.utils import decode_dss_signature
import secrets


def generate_keypair():
    """Generate a P256 keypair. Returns (private_key, pubkey_x, pubkey_y) as (ec.EllipticCurvePrivateKey, int, int)."""
    private_key = ec.generate_private_key(ec.SECP256R1())
    pub_numbers = private_key.public_key().public_numbers()
    return private_key, pub_numbers.x, pub_numbers.y


def sign_hash(private_key, msg_hash: bytes) -> tuple[int, int]:
    """Sign a 32-byte hash with P256. Returns (r, s).

    Uses ECDSA with a prehash (the hash is already computed).
    The P256VERIFY precompile expects the raw hash, not a double-hash,
    so we use Prehashed to avoid hashing again.
    """
    from cryptography.hazmat.primitives.asymmetric.utils import Prehashed
    assert len(msg_hash) == 32, f"Expected 32-byte hash, got {len(msg_hash)}"

    signature = private_key.sign(
        msg_hash,
        ec.ECDSA(Prehashed(hashes.SHA256()))
    )
    r, s = decode_dss_signature(signature)
    return r, s


def pubkey_to_bytes(x: int, y: int) -> tuple[bytes, bytes]:
    """Convert pubkey coordinates to 32-byte big-endian."""
    return x.to_bytes(32, 'big'), y.to_bytes(32, 'big')


if __name__ == '__main__':
    # Quick test
    pk, x, y = generate_keypair()
    print(f"Public key X: 0x{x:064x}")
    print(f"Public key Y: 0x{y:064x}")

    test_hash = secrets.token_bytes(32)
    r, s = sign_hash(pk, test_hash)
    print(f"Signature r: 0x{r:064x}")
    print(f"Signature s: 0x{s:064x}")

    # Verify signature
    from cryptography.hazmat.primitives.asymmetric.utils import Prehashed, encode_dss_signature
    signature = encode_dss_signature(r, s)
    pk.public_key().verify(signature, test_hash, ec.ECDSA(Prehashed(hashes.SHA256())))
    print("Signature verified!")
