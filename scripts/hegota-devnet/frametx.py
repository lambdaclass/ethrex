#!/usr/bin/env python3
"""Current-format EIP-8141/8250/8272/7906 frame-transaction (type 0x06) encoder.

Wire layout (ethrex hegota-devnet), verified against the repo golden vector:
  raw = 0x06 || rlp([chain_id, nonce_keys, nonce_seq, sender, frames, signatures,
                     max_priority_fee, max_fee, max_blob_fee, blob_hashes,
                     recent_root_references])
  frame     = rlp([mode, flags, target_or_empty, gas_limit, value, data])
  signature = rlp([scheme, signer, msg, signature_bytes])
  sig_hash  = keccak256(0x06 || rlp(envelope with empty-msg signatures' bytes elided))
"""
from eth_hash.auto import keccak

# ---------- minimal RLP ----------
def rlp_bytes(b: bytes) -> bytes:
    if len(b) == 1 and b[0] < 0x80:
        return b
    if len(b) < 56:
        return bytes([0x80 + len(b)]) + b
    lb = len(b).to_bytes((len(b).bit_length() + 7) // 8, "big")
    return bytes([0xb7 + len(lb)]) + lb + b

def rlp_list(items) -> bytes:
    body = b"".join(items)
    if len(body) < 56:
        return bytes([0xc0 + len(body)]) + body
    lb = len(body).to_bytes((len(body).bit_length() + 7) // 8, "big")
    return bytes([0xf7 + len(lb)]) + lb + body

def rlp_int(x: int) -> bytes:
    if x == 0:
        return rlp_bytes(b"")
    return rlp_bytes(x.to_bytes((x.bit_length() + 7) // 8, "big"))

def addr20(a: int | bytes) -> bytes:
    if isinstance(a, int):
        return a.to_bytes(20, "big")
    return bytes(a)

# ---------- frame-tx model ----------
class Frame:
    def __init__(self, mode, flags, target, gas_limit, value, data):
        self.mode, self.flags, self.target = mode, flags, target  # target: 20-byte int/bytes or None
        self.gas_limit, self.value, self.data = gas_limit, value, data
    def rlp(self):
        tgt = rlp_bytes(addr20(self.target)) if self.target is not None else rlp_bytes(b"")
        return rlp_list([rlp_int(self.mode), rlp_int(self.flags), tgt,
                         rlp_int(self.gas_limit), rlp_int(self.value), rlp_bytes(self.data)])

class FrameSig:
    SECP256K1 = 0
    P256 = 1
    def __init__(self, scheme, signer, msg, signature):
        self.scheme, self.signer, self.msg, self.signature = scheme, signer, msg, signature
    def rlp(self, elide=False):
        sig = b"" if (elide and len(self.msg) == 0) else self.signature
        return rlp_list([rlp_int(self.scheme), rlp_bytes(addr20(self.signer)),
                         rlp_bytes(self.msg), rlp_bytes(sig)])

class FrameTx:
    def __init__(self, chain_id, nonce_keys, nonce_seq, sender, frames, signatures,
                 max_priority_fee, max_fee, max_blob_fee=0, blob_hashes=None, recent_root_refs=None):
        self.chain_id, self.nonce_keys, self.nonce_seq, self.sender = chain_id, nonce_keys, nonce_seq, sender
        self.frames, self.signatures = frames, signatures
        self.max_priority_fee, self.max_fee, self.max_blob_fee = max_priority_fee, max_fee, max_blob_fee
        self.blob_hashes = blob_hashes or []
        self.recent_root_refs = recent_root_refs or []
    def _envelope(self, elide_sigs):
        return [
            rlp_int(self.chain_id),
            rlp_list([rlp_int(k) for k in self.nonce_keys]),
            rlp_int(self.nonce_seq),
            rlp_bytes(addr20(self.sender)),
            rlp_list([f.rlp() for f in self.frames]),
            rlp_list([s.rlp(elide=elide_sigs) for s in self.signatures]),
            rlp_int(self.max_priority_fee),
            rlp_int(self.max_fee),
            rlp_int(self.max_blob_fee),
            rlp_list([rlp_bytes(h) for h in self.blob_hashes]),
            rlp_list([r for r in self.recent_root_refs]),  # entries pre-encoded if any
        ]
    def encode(self) -> bytes:
        return rlp_list(self._envelope(elide_sigs=False))
    def raw(self) -> bytes:
        return bytes([0x06]) + self.encode()
    def sig_hash(self) -> bytes:
        return keccak(bytes([0x06]) + rlp_list(self._envelope(elide_sigs=True)))

# ---------- golden-vector validation ----------
if __name__ == "__main__":
    golden = FrameTx(
        chain_id=1,
        nonce_keys=[0],
        nonce_seq=7,
        sender=0xABCD,
        frames=[
            Frame(mode=1, flags=3, target=None, gas_limit=0x5208, value=0, data=bytes([0x11, 0x22])),
            Frame(mode=2, flags=0, target=0x1234, gas_limit=0x9c40, value=0, data=b""),
        ],
        signatures=[FrameSig(FrameSig.SECP256K1, 0xABCD, b"", bytes([0x01] * 65))],
        max_priority_fee=0x3b9aca00,
        max_fee=0x6fc23ac00,
    )
    EXPECT_RLP = "f8ae01c1800794000000000000000000000000000000000000abcde8ca01038082520880821122dc0280940000000000000000000000000000000000001234829c408080f85cf85a8094000000000000000000000000000000000000abcd80b8410101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101843b9aca008506fc23ac0080c0c0"
    EXPECT_SIGHASH = "0x78ad972cb33b083d46ec78db62ffb45e0e53a9cb5eba1414bc1def77ed223fb3"
    got_rlp = golden.encode().hex()
    got_sh = "0x" + golden.sig_hash().hex()
    print("RLP match:     ", got_rlp == EXPECT_RLP)
    if got_rlp != EXPECT_RLP:
        print("  expected:", EXPECT_RLP)
        print("  got:     ", got_rlp)
    print("sig_hash match:", got_sh == EXPECT_SIGHASH)
    if got_sh != EXPECT_SIGHASH:
        print("  expected:", EXPECT_SIGHASH)
        print("  got:     ", got_sh)
