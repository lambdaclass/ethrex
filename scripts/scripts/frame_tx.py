"""Frame transaction (EIP-8141, type 0x06) construction, signing, and RLP encoding.


Frame transaction RLP field order:
  [chain_id, nonce, sender, frames, max_priority_fee, max_fee, max_blob_fee, blob_hashes]

Each frame is RLP-encoded as:
  [mode, target, gas_limit, data]
  where target is empty bytes for None (DEFAULT mode with no target).
"""

from typing import List

import rlp
from eth_utils import keccak
from dataclasses import dataclass, field


# Frame modes
FRAME_MODE_DEFAULT = 0
FRAME_MODE_VERIFY = 1
FRAME_MODE_SENDER = 2

TX_TYPE = 0x06


@dataclass
class Frame:
    mode: int
    target: bytes  # 20-byte address, or b'' for None
    gas_limit: int
    data: bytes

    def to_rlp_list(self):
        return [
            self.mode,
            self.target,
            self.gas_limit,
            self.data,
        ]


@dataclass
class FrameTransaction:
    chain_id: int
    nonce: int
    sender: bytes  # 20-byte address
    frames: List[Frame]
    max_priority_fee_per_gas: int
    max_fee_per_gas: int
    max_fee_per_blob_gas: int = 0
    blob_versioned_hashes: List[bytes] = field(default_factory=list)

    def to_rlp_list(self):
        """Convert to RLP-serializable list."""
        frames_rlp = [f.to_rlp_list() for f in self.frames]
        return [
            self.chain_id,
            self.nonce,
            self.sender,
            frames_rlp,
            self.max_priority_fee_per_gas,
            self.max_fee_per_gas,
            self.max_fee_per_blob_gas,
            self.blob_versioned_hashes,
        ]

    def encode_rlp(self) -> bytes:
        """RLP encode the transaction (without type prefix)."""
        return rlp.encode(self.to_rlp_list())

    def encode_canonical(self) -> bytes:
        """Encode with 0x06 type prefix for sending via RPC."""
        return bytes([TX_TYPE]) + self.encode_rlp()

    def compute_sig_hash(self) -> bytes:
        """Compute the signature hash: keccak256(0x06 || rlp(tx_with_elided_verify_data)).

        For sig_hash computation, VERIFY frame data is replaced with empty bytes.
        """
        # Clone frames with VERIFY data elided
        elided_frames = []
        for f in self.frames:
            if f.mode == FRAME_MODE_VERIFY:
                elided_frames.append(Frame(
                    mode=f.mode,
                    target=f.target,
                    gas_limit=f.gas_limit,
                    data=b'',  # elided
                ))
            else:
                elided_frames.append(f)

        elided_tx = FrameTransaction(
            chain_id=self.chain_id,
            nonce=self.nonce,
            sender=self.sender,
            frames=elided_frames,
            max_priority_fee_per_gas=self.max_priority_fee_per_gas,
            max_fee_per_gas=self.max_fee_per_gas,
            max_fee_per_blob_gas=self.max_fee_per_blob_gas,
            blob_versioned_hashes=self.blob_versioned_hashes,
        )

        payload = bytes([TX_TYPE]) + elided_tx.encode_rlp()
        return keccak(payload)

    def tx_hash(self) -> bytes:
        """Compute the transaction hash: keccak256(0x06 || rlp(tx))."""
        return keccak(self.encode_canonical())


def build_verify_frame(target: bytes, gas_limit: int, r: int, s: int) -> Frame:
    """Build a VERIFY frame with P256 signature (r, s) as calldata.

    Calldata format: selector(4) || r(32) || s(32) = 68 bytes
    Selector 0x00000000 for verify().
    """
    data = (
        b'\x00\x00\x00\x00' +  # selector
        r.to_bytes(32, 'big') +
        s.to_bytes(32, 'big')
    )
    return Frame(
        mode=FRAME_MODE_VERIFY,
        target=target,
        gas_limit=gas_limit,
        data=data,
    )


def build_execute_frame(target: bytes, gas_limit: int,
                         dest: bytes, value: int, call_data: bytes = b'') -> Frame:
    """Build a SENDER frame that calls execute() on the account.

    Calldata format: selector(4) || dest(32) || value(32) || data(variable)
    Selector 0x00000001 for execute().
    """
    data = (
        b'\x00\x00\x00\x01' +              # selector
        dest.rjust(32, b'\x00') +           # dest right-aligned in 32 bytes
        value.to_bytes(32, 'big') +         # value
        call_data                           # data
    )
    return Frame(
        mode=FRAME_MODE_SENDER,
        target=target,
        gas_limit=gas_limit,
        data=data,
    )


def build_transfer_frame(target: bytes, gas_limit: int,
                          dest: bytes, amount: int) -> Frame:
    """Build a SENDER frame that calls transfer() on the account.

    Calldata format: selector(4) || dest(32) || amount(32)
    Selector 0x00000002 for transfer().
    """
    data = (
        b'\x00\x00\x00\x02' +              # selector
        dest.rjust(32, b'\x00') +           # dest right-aligned
        amount.to_bytes(32, 'big')          # amount
    )
    return Frame(
        mode=FRAME_MODE_SENDER,
        target=target,
        gas_limit=gas_limit,
        data=data,
    )


if __name__ == '__main__':
    # Quick test: build a frame tx and compute sig_hash
    sender = bytes.fromhex('aa' * 20)
    tx = FrameTransaction(
        chain_id=1,
        nonce=0,
        sender=sender,
        frames=[
            Frame(mode=FRAME_MODE_VERIFY, target=sender, gas_limit=100000,
                  data=b'\x00' * 68),
            Frame(mode=FRAME_MODE_SENDER, target=sender, gas_limit=50000,
                  data=b'\x00\x00\x00\x02' + b'\xbb' * 32 + b'\x00' * 31 + b'\x01'),
        ],
        max_priority_fee_per_gas=1_000_000_000,
        max_fee_per_gas=50_000_000_000,
    )

    canonical = tx.encode_canonical()
    sig_hash = tx.compute_sig_hash()
    tx_hash = tx.tx_hash()

    print(f"Canonical encoding: 0x{canonical.hex()}")
    print(f"Canonical length: {len(canonical)} bytes")
    print(f"Sig hash: 0x{sig_hash.hex()}")
    print(f"Tx hash: 0x{tx_hash.hex()}")
