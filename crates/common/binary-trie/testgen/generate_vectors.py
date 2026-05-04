"""
Generate test vectors for ethrex binary trie from EIP-7864 reference implementation.

Deterministic PRNG seed: 0xDEADBEEF42
All random data in this file uses `random.Random(SEED)` so byte-identical output
is guaranteed on every run.

Run from crates/common/binary-trie/:
    python3 testgen/generate_vectors.py

Outputs:
    testgen/test_vectors.json           -- original 10 raw-insert vectors
    testgen/vectors_accounts.json       -- account update sequences
    testgen/vectors_storage.json        -- storage slot sequences
    testgen/vectors_codechunk.json      -- code-chunk boundary sequences
    testgen/vectors_negative.json       -- absence-proof + selfdestruct cases

Requires: pip install blake3
"""

import json
import os
import random
import struct
from typing import Optional
from blake3 import blake3

# ---------------------------------------------------------------------------
# Deterministic PRNG seed (documented per §2 requirement)
# ---------------------------------------------------------------------------
SEED = 0xDEADBEEF42


# ---------------------------------------------------------------------------
# EIP-7864 constants (must match key_mapping.rs exactly)
# ---------------------------------------------------------------------------
BASIC_DATA_LEAF_KEY = 0
CODE_HASH_LEAF_KEY = 1
HEADER_STORAGE_OFFSET = 64
CODE_OFFSET = 128
STEM_SUBTREE_WIDTH = 256
MAIN_STORAGE_OFFSET = 1 << 248


# ---------------------------------------------------------------------------
# Core binary trie implementation (reference; used for root computation)
# ---------------------------------------------------------------------------

class StemNode:
    def __init__(self, stem: bytes):
        assert len(stem) == 31, "stem must be 31 bytes"
        self.stem = stem
        self.values: list[Optional[bytes]] = [None] * 256

    def set_value(self, index: int, value: bytes):
        self.values[index] = value

    def clear_value(self, index: int):
        self.values[index] = None


class InternalNode:
    def __init__(self):
        self.left = None
        self.right = None


class BinaryTree:
    def __init__(self):
        self.root = None

    def insert(self, key: bytes, value: bytes):
        assert len(key) == 32, "key must be 32 bytes"
        assert len(value) == 32, "value must be 32 bytes"
        stem = key[:31]
        subindex = key[31]

        if self.root is None:
            self.root = StemNode(stem)
            self.root.set_value(subindex, value)
            return

        self.root = self._insert(self.root, stem, subindex, value, 0)

    def remove(self, key: bytes):
        """Remove a leaf. If the StemNode ends up with all-None values it is
        pruned (removed from the tree), matching Rust's behavior where empty
        StemNodes are freed and collapsed by their parent InternalNode.
        """
        assert len(key) == 32, "key must be 32 bytes"
        stem = key[:31]
        subindex = key[31]
        if self.root is None:
            return
        self.root = self._remove(self.root, stem, subindex, 0)

    def _remove(self, node, stem, subindex, depth):
        if node is None:
            return None
        if isinstance(node, StemNode):
            if node.stem == stem:
                node.clear_value(subindex)
                # Prune the StemNode if all values are now None.
                if all(v is None for v in node.values):
                    return None
            return node
        bits = self._bytes_to_bits(stem)
        bit = bits[depth]
        if bit == 0:
            node.left = self._remove(node.left, stem, subindex, depth + 1)
        else:
            node.right = self._remove(node.right, stem, subindex, depth + 1)
        # Collapse InternalNode if both children are None.
        if node.left is None and node.right is None:
            return None
        # If one child is None and the other is a StemNode, promote it
        # (matches Rust's collapse logic in remove_node_at_depth).
        if node.left is None and isinstance(node.right, StemNode):
            return node.right
        if node.right is None and isinstance(node.left, StemNode):
            return node.left
        return node

    def _insert(self, node, stem, subindex, value, depth):
        assert depth < 248, "depth must be less than 248"

        if node is None:
            node = StemNode(stem)
            node.set_value(subindex, value)
            return node

        stem_bits = self._bytes_to_bits(stem)
        if isinstance(node, StemNode):
            if node.stem == stem:
                node.set_value(subindex, value)
                return node
            existing_stem_bits = self._bytes_to_bits(node.stem)
            return self._split_leaf(
                node, stem_bits, existing_stem_bits, subindex, value, depth
            )

        bit = stem_bits[depth]
        if bit == 0:
            node.left = self._insert(node.left, stem, subindex, value, depth + 1)
        else:
            node.right = self._insert(node.right, stem, subindex, value, depth + 1)
        return node

    def _split_leaf(self, leaf, stem_bits, existing_stem_bits, subindex, value, depth):
        if stem_bits[depth] == existing_stem_bits[depth]:
            new_internal = InternalNode()
            bit = stem_bits[depth]
            if bit == 0:
                new_internal.left = self._split_leaf(
                    leaf, stem_bits, existing_stem_bits, subindex, value, depth + 1
                )
            else:
                new_internal.right = self._split_leaf(
                    leaf, stem_bits, existing_stem_bits, subindex, value, depth + 1
                )
            return new_internal
        else:
            new_internal = InternalNode()
            bit = stem_bits[depth]
            stem = self._bits_to_bytes(stem_bits)
            if bit == 0:
                new_internal.left = StemNode(stem)
                new_internal.left.set_value(subindex, value)
                new_internal.right = leaf
            else:
                new_internal.right = StemNode(stem)
                new_internal.right.set_value(subindex, value)
                new_internal.left = leaf
            return new_internal

    def _bytes_to_bits(self, data: bytes) -> list[int]:
        bits = []
        for byte in data:
            for i in range(7, -1, -1):
                bits.append((byte >> i) & 1)
        return bits

    def _bits_to_bytes(self, bits: list[int]) -> bytes:
        result = bytearray()
        for i in range(0, len(bits), 8):
            byte = 0
            for j in range(8):
                if i + j < len(bits):
                    byte = (byte << 1) | bits[i + j]
                else:
                    byte <<= 1
            result.append(byte)
        return bytes(result)

    def _hash(self, data):
        if data in (None, b"\x00" * 64):
            return b"\x00" * 32
        assert len(data) == 64 or len(data) == 32, f"data must be 32 or 64 bytes, got {len(data)}"
        return blake3(data).digest()

    def merkelize(self):
        def _merkelize(node):
            if node is None:
                return b"\x00" * 32
            if isinstance(node, InternalNode):
                left_hash = _merkelize(node.left)
                right_hash = _merkelize(node.right)
                return self._hash(left_hash + right_hash)

            level = [self._hash(x) for x in node.values]
            while len(level) > 1:
                new_level = []
                for i in range(0, len(level), 2):
                    new_level.append(self._hash(level[i] + level[i + 1]))
                level = new_level

            return self._hash(node.stem + b"\0" + level[0])

        return _merkelize(self.root)


def to_hex(b: bytes) -> str:
    return b.hex()


# ---------------------------------------------------------------------------
# EIP-7864 key derivation (must match key_mapping.rs exactly)
# ---------------------------------------------------------------------------

def tree_hash(data: bytes) -> bytes:
    """BLAKE3 hash — no special-case for all-zero input (key derivation only)."""
    return blake3(data).digest()


def old_style_address_to_address32(address: bytes) -> bytes:
    """Zero-pad 20-byte address to 32 bytes (12 zero prefix + 20 address)."""
    assert len(address) == 20
    return b"\x00" * 12 + address


def get_tree_key(address: bytes, tree_index: int, sub_index: int) -> bytes:
    """Derive 32-byte tree key for address + tree_index + sub_index."""
    addr32 = old_style_address_to_address32(address)
    tree_index_bytes = tree_index.to_bytes(32, "big")
    input_data = addr32 + tree_index_bytes
    h = tree_hash(input_data)
    return h[:31] + bytes([sub_index])


def get_stem_for_base(address: bytes) -> bytes:
    """31-byte stem for tree_index=0 (basic_data, code_hash, header storage, code chunks 0-127)."""
    return get_tree_key(address, 0, 0)[:31]


def get_tree_key_for_basic_data(address: bytes) -> bytes:
    stem = get_stem_for_base(address)
    return stem + bytes([BASIC_DATA_LEAF_KEY])


def get_tree_key_for_code_hash(address: bytes) -> bytes:
    stem = get_stem_for_base(address)
    return stem + bytes([CODE_HASH_LEAF_KEY])


def get_tree_key_for_storage_slot(address: bytes, storage_key: int) -> bytes:
    """Map storage slot to EIP-7864 tree key.

    Slots 0-63 map to header range (sub_indices 64-127 at tree_index=0).
    Slots >= 64 map to main storage range (MAIN_STORAGE_OFFSET + slot).
    """
    header_capacity = CODE_OFFSET - HEADER_STORAGE_OFFSET  # 64
    if storage_key < header_capacity:
        pos = HEADER_STORAGE_OFFSET + storage_key
        tree_index = pos // STEM_SUBTREE_WIDTH
        sub_index = pos % STEM_SUBTREE_WIDTH
        return get_tree_key(address, tree_index, sub_index)
    else:
        # pos = MAIN_STORAGE_OFFSET + storage_key
        # sub_index = pos % 256 = storage_key % 256 (since 2^248 % 256 == 0)
        # tree_index = pos // 256 = 2^240 + storage_key // 256
        sub_index = (MAIN_STORAGE_OFFSET + storage_key) % STEM_SUBTREE_WIDTH
        tree_index = (MAIN_STORAGE_OFFSET + storage_key) // STEM_SUBTREE_WIDTH
        return get_tree_key(address, tree_index, sub_index)


def get_tree_key_for_code_chunk(address: bytes, chunk_id: int) -> bytes:
    """Map code chunk ID to EIP-7864 tree key.

    pos = CODE_OFFSET + chunk_id
    tree_index = pos // 256
    sub_index = pos % 256
    """
    pos = CODE_OFFSET + chunk_id
    tree_index = pos // STEM_SUBTREE_WIDTH
    sub_index = pos % STEM_SUBTREE_WIDTH
    return get_tree_key(address, tree_index, sub_index)


# ---------------------------------------------------------------------------
# basic_data packing (must match pack_basic_data in key_mapping.rs)
# ---------------------------------------------------------------------------

def pack_basic_data(version: int, code_size: int, nonce: int, balance: int) -> bytes:
    """Pack account header fields into 32-byte basic_data leaf layout.

    Layout (big-endian):
    byte 0:       version
    bytes 1-4:    reserved (zeros)
    bytes 5-7:    code_size (3 bytes)
    bytes 8-15:   nonce (8 bytes)
    bytes 16-31:  balance (low 16 bytes of U256, big-endian)
    """
    data = bytearray(32)
    data[0] = version & 0xFF
    # bytes 1-4 reserved
    data[5] = (code_size >> 16) & 0xFF
    data[6] = (code_size >> 8) & 0xFF
    data[7] = code_size & 0xFF
    data[8:16] = nonce.to_bytes(8, "big")
    # balance: low 16 bytes (U256 is 32 bytes big-endian; we take bytes 16-31)
    balance_bytes = balance.to_bytes(32, "big")
    data[16:32] = balance_bytes[16:32]
    return bytes(data)


ZERO_KECCAK = bytes.fromhex(
    "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
)


# ---------------------------------------------------------------------------
# Original test_vectors.json generator (unchanged)
# ---------------------------------------------------------------------------

def generate_vectors():
    vectors = []

    # Vector 1: Empty tree
    t = BinaryTree()
    vectors.append({
        "name": "empty_tree",
        "inserts": [],
        "expected_root": to_hex(t.merkelize()),
    })

    # Vector 2: Single key-value
    t = BinaryTree()
    key = bytes(32)
    val = bytes([1] * 32)
    t.insert(key, val)
    vectors.append({
        "name": "single_key_value",
        "inserts": [{"key": to_hex(key), "value": to_hex(val)}],
        "expected_root": to_hex(t.merkelize()),
    })

    # Vector 3: Two keys, same stem, different sub_index
    t = BinaryTree()
    k1 = bytes([0xAA] * 31 + [0])
    v1 = bytes([0x11] * 32)
    k2 = bytes([0xAA] * 31 + [1])
    v2 = bytes([0x22] * 32)
    t.insert(k1, v1)
    t.insert(k2, v2)
    vectors.append({
        "name": "same_stem_different_sub_index",
        "inserts": [
            {"key": to_hex(k1), "value": to_hex(v1)},
            {"key": to_hex(k2), "value": to_hex(v2)},
        ],
        "expected_root": to_hex(t.merkelize()),
    })

    # Vector 4: Two keys, stems diverge at bit 0
    t = BinaryTree()
    k1 = bytes([0x00] * 31 + [0])
    v1 = bytes([0x01] * 32)
    k2 = bytes([0x80] + [0x00] * 30 + [0])
    v2 = bytes([0x02] * 32)
    t.insert(k1, v1)
    t.insert(k2, v2)
    vectors.append({
        "name": "stems_diverge_at_bit_0",
        "inserts": [
            {"key": to_hex(k1), "value": to_hex(v1)},
            {"key": to_hex(k2), "value": to_hex(v2)},
        ],
        "expected_root": to_hex(t.merkelize()),
    })

    # Vector 5: Two keys, stems diverge deep (bit 8 — second byte differs)
    t = BinaryTree()
    k1 = bytes([0x00] * 31 + [0])
    v1 = bytes([0x0A] * 32)
    k2 = bytes([0x00, 0x80] + [0x00] * 29 + [0])
    v2 = bytes([0x0B] * 32)
    t.insert(k1, v1)
    t.insert(k2, v2)
    vectors.append({
        "name": "stems_diverge_at_bit_8",
        "inserts": [
            {"key": to_hex(k1), "value": to_hex(v1)},
            {"key": to_hex(k2), "value": to_hex(v2)},
        ],
        "expected_root": to_hex(t.merkelize()),
    })

    # Vector 6: Multiple stems (3 different stems)
    t = BinaryTree()
    entries = []
    for i, stem_byte in enumerate([0x20, 0x80, 0xC0]):
        k = bytes([stem_byte] + [0x00] * 30 + [0])
        v = bytes([i + 1] * 32)
        t.insert(k, v)
        entries.append({"key": to_hex(k), "value": to_hex(v)})
    vectors.append({
        "name": "three_different_stems",
        "inserts": entries,
        "expected_root": to_hex(t.merkelize()),
    })

    # Vector 7: Insertion order independence
    entries_data = [
        (bytes([0x20] + [0x00] * 30 + [0]), bytes([1] * 32)),
        (bytes([0x80] + [0x00] * 30 + [0]), bytes([2] * 32)),
        (bytes([0xC0] + [0x00] * 30 + [0]), bytes([3] * 32)),
    ]
    t1 = BinaryTree()
    for k, v in entries_data:
        t1.insert(k, v)
    t2 = BinaryTree()
    for k, v in reversed(entries_data):
        t2.insert(k, v)
    root1 = t1.merkelize()
    root2 = t2.merkelize()
    assert root1 == root2, "insertion order independence failed!"
    vectors.append({
        "name": "insertion_order_independence",
        "inserts": [{"key": to_hex(k), "value": to_hex(v)} for k, v in entries_data],
        "expected_root": to_hex(root1),
        "note": "same root regardless of insertion order",
    })

    # Vector 8: Many keys on same stem (fill sub_indices 0..9)
    t = BinaryTree()
    entries = []
    stem = bytes([0x55] * 31)
    for i in range(10):
        k = stem + bytes([i])
        v = bytes([i * 10 + 1] * 32)
        t.insert(k, v)
        entries.append({"key": to_hex(k), "value": to_hex(v)})
    vectors.append({
        "name": "ten_values_same_stem",
        "inserts": entries,
        "expected_root": to_hex(t.merkelize()),
    })

    # Vector 9: Overwrite value
    t = BinaryTree()
    k = bytes([0xBB] * 31 + [5])
    v1 = bytes([0x11] * 32)
    v2 = bytes([0x22] * 32)
    t.insert(k, v1)
    t.insert(k, v2)  # overwrite
    vectors.append({
        "name": "overwrite_value",
        "inserts": [
            {"key": to_hex(k), "value": to_hex(v1)},
            {"key": to_hex(k), "value": to_hex(v2)},
        ],
        "expected_root": to_hex(t.merkelize()),
        "note": "second insert overwrites the first",
    })

    # Vector 10: Zero key, zero value
    t = BinaryTree()
    k = bytes(32)
    v = bytes(32)
    t.insert(k, v)
    vectors.append({
        "name": "zero_key_zero_value",
        "inserts": [{"key": to_hex(k), "value": to_hex(v)}],
        "expected_root": to_hex(t.merkelize()),
    })

    return vectors


# ---------------------------------------------------------------------------
# Account update generator
#
# Produces sequences over 50 synthetic addresses.
# Each vector is a sequence of ops:
#   { "op": "set_basic_data", "address": "0x...",
#     "version": N, "code_size": N, "nonce": N, "balance": "<hex32>" }
#   { "op": "set_code_hash",  "address": "0x...", "code_hash": "<hex32>" }
#   { "op": "clear_stem",     "address": "0x..." }
#
# Each vector records the final expected_root after all ops applied.
#
# Includes:
#   - Nonce/balance/code_hash cycles
#   - At least 3 selfdestruct (clear_stem) cases
#   - At least 3 post-selfdestruct recreation cases
# ---------------------------------------------------------------------------

def _make_address(rng: random.Random, idx: int) -> bytes:
    """20-byte synthetic address deterministically derived from index."""
    rng_local = random.Random(SEED ^ (idx * 0x1337))
    return bytes([rng_local.randint(0, 255) for _ in range(20)])


def _u256_to_hex32(n: int) -> str:
    return n.to_bytes(32, "big").hex()


def generate_account_vectors() -> list[dict]:
    rng = random.Random(SEED ^ 0xA0)
    vectors = []

    # Synthetic addresses (50 total)
    NUM_ADDRESSES = 50
    addresses = [_make_address(rng, i) for i in range(NUM_ADDRESSES)]

    # Helper: insert basic_data + code_hash into a BinaryTree
    def apply_set_basic_data(tree: BinaryTree, addr: bytes, version: int,
                              code_size: int, nonce: int, balance: int):
        packed = pack_basic_data(version, code_size, nonce, balance)
        key = get_tree_key_for_basic_data(addr)
        tree.insert(key, packed)

    def apply_set_code_hash(tree: BinaryTree, addr: bytes, code_hash: bytes):
        key = get_tree_key_for_code_hash(addr)
        tree.insert(key, code_hash)

    def apply_clear_stem(tree: BinaryTree, addr: bytes):
        """Clear all leaves at the base stem (sub_indices 0-255 at tree_index=0)."""
        stem = get_stem_for_base(addr)
        for sub_index in range(STEM_SUBTREE_WIDTH):
            key = stem + bytes([sub_index])
            tree.remove(key)

    # --- Vector A1: basic account insertions for first 10 addresses ---
    t = BinaryTree()
    ops = []
    for i in range(10):
        addr = addresses[i]
        addr_hex = "0x" + addr.hex()
        nonce = i + 1
        balance = (i + 1) * 10 ** 18
        code_size = 0
        version = 0
        code_hash = ZERO_KECCAK

        ops.append({
            "op": "set_basic_data",
            "address": addr_hex,
            "version": version,
            "code_size": code_size,
            "nonce": nonce,
            "balance": _u256_to_hex32(balance),
        })
        ops.append({
            "op": "set_code_hash",
            "address": addr_hex,
            "code_hash": code_hash.hex(),
        })
        apply_set_basic_data(t, addr, version, code_size, nonce, balance)
        apply_set_code_hash(t, addr, code_hash)

    vectors.append({
        "name": "accounts_basic_10",
        "ops": ops,
        "expected_root": to_hex(t.merkelize()),
    })

    # --- Vector A2: nonce/balance cycle updates on 5 addresses ---
    t = BinaryTree()
    ops = []
    for i in range(5):
        addr = addresses[i]
        addr_hex = "0x" + addr.hex()
        # Three rounds of updates
        for round_idx in range(3):
            nonce = (i + 1) * 10 + round_idx
            balance = (round_idx + 1) * 10 ** 15 + i
            ops.append({
                "op": "set_basic_data",
                "address": addr_hex,
                "version": 0,
                "code_size": 0,
                "nonce": nonce,
                "balance": _u256_to_hex32(balance),
            })
            apply_set_basic_data(t, addr, 0, 0, nonce, balance)
        # Set code_hash once
        code_hash = ZERO_KECCAK
        ops.append({
            "op": "set_code_hash",
            "address": addr_hex,
            "code_hash": code_hash.hex(),
        })
        apply_set_code_hash(t, addr, code_hash)

    vectors.append({
        "name": "accounts_nonce_balance_cycle",
        "ops": ops,
        "expected_root": to_hex(t.merkelize()),
    })

    # --- Vector A3: code_hash update cycles ---
    t = BinaryTree()
    ops = []
    fake_hashes = [
        bytes([i] * 32) for i in range(1, 11)
    ]
    for i in range(5):
        addr = addresses[10 + i]
        addr_hex = "0x" + addr.hex()
        ops.append({
            "op": "set_basic_data",
            "address": addr_hex,
            "version": 0,
            "code_size": i * 100,
            "nonce": 1,
            "balance": _u256_to_hex32(10 ** 18),
        })
        apply_set_basic_data(t, addr, 0, i * 100, 1, 10 ** 18)
        # Two code_hash updates
        for h in [fake_hashes[i], fake_hashes[i + 5]]:
            ops.append({
                "op": "set_code_hash",
                "address": addr_hex,
                "code_hash": h.hex(),
            })
            apply_set_code_hash(t, addr, h)

    vectors.append({
        "name": "accounts_code_hash_cycle",
        "ops": ops,
        "expected_root": to_hex(t.merkelize()),
    })

    # --- Vector A4: selfdestruct (clear_stem) on 3 accounts ---
    t = BinaryTree()
    ops = []
    # First insert accounts 20, 21, 22
    for i in range(3):
        addr = addresses[20 + i]
        addr_hex = "0x" + addr.hex()
        ops.append({
            "op": "set_basic_data",
            "address": addr_hex,
            "version": 0,
            "code_size": 0,
            "nonce": i + 1,
            "balance": _u256_to_hex32((i + 1) * 10 ** 18),
        })
        ops.append({
            "op": "set_code_hash",
            "address": addr_hex,
            "code_hash": ZERO_KECCAK.hex(),
        })
        apply_set_basic_data(t, addr, 0, 0, i + 1, (i + 1) * 10 ** 18)
        apply_set_code_hash(t, addr, ZERO_KECCAK)
    # Now selfdestruct all three
    for i in range(3):
        addr = addresses[20 + i]
        addr_hex = "0x" + addr.hex()
        ops.append({"op": "clear_stem", "address": addr_hex})
        apply_clear_stem(t, addr)

    vectors.append({
        "name": "accounts_selfdestruct_3",
        "ops": ops,
        "expected_root": to_hex(t.merkelize()),
    })

    # --- Vector A5: post-selfdestruct recreation (3 accounts) ---
    t = BinaryTree()
    ops = []
    # Insert accounts 30, 31, 32
    for i in range(3):
        addr = addresses[30 + i]
        addr_hex = "0x" + addr.hex()
        ops.append({
            "op": "set_basic_data",
            "address": addr_hex,
            "version": 0,
            "code_size": 0,
            "nonce": 5,
            "balance": _u256_to_hex32(5 * 10 ** 18),
        })
        ops.append({
            "op": "set_code_hash",
            "address": addr_hex,
            "code_hash": ZERO_KECCAK.hex(),
        })
        apply_set_basic_data(t, addr, 0, 0, 5, 5 * 10 ** 18)
        apply_set_code_hash(t, addr, ZERO_KECCAK)
    # Selfdestruct
    for i in range(3):
        addr = addresses[30 + i]
        addr_hex = "0x" + addr.hex()
        ops.append({"op": "clear_stem", "address": addr_hex})
        apply_clear_stem(t, addr)
    # Recreate (new nonce=1, different balance)
    for i in range(3):
        addr = addresses[30 + i]
        addr_hex = "0x" + addr.hex()
        new_balance = (i + 10) * 10 ** 17
        ops.append({
            "op": "set_basic_data",
            "address": addr_hex,
            "version": 0,
            "code_size": 0,
            "nonce": 1,
            "balance": _u256_to_hex32(new_balance),
        })
        ops.append({
            "op": "set_code_hash",
            "address": addr_hex,
            "code_hash": ZERO_KECCAK.hex(),
        })
        apply_set_basic_data(t, addr, 0, 0, 1, new_balance)
        apply_set_code_hash(t, addr, ZERO_KECCAK)

    vectors.append({
        "name": "accounts_selfdestruct_then_recreate_3",
        "ops": ops,
        "expected_root": to_hex(t.merkelize()),
    })

    # --- Vector A6: large account set — 50 accounts, insertion order independence ---
    rng2 = random.Random(SEED ^ 0xB0)
    t1 = BinaryTree()
    t2 = BinaryTree()
    all_ops = []
    for i in range(NUM_ADDRESSES):
        addr = addresses[i]
        addr_hex = "0x" + addr.hex()
        nonce = rng2.randint(1, 10000)
        balance = rng2.randint(0, 10 ** 20)
        code_size = rng2.randint(0, 0xFFFFFF)
        code_hash_val = rng2.randint(0, 2**256 - 1)
        code_hash = code_hash_val.to_bytes(32, "big")
        all_ops.append((addr, addr_hex, nonce, balance, code_size, code_hash))

    for addr, addr_hex, nonce, balance, code_size, code_hash in all_ops:
        apply_set_basic_data(t1, addr, 0, code_size, nonce, balance)
        apply_set_code_hash(t1, addr, code_hash)
    for addr, addr_hex, nonce, balance, code_size, code_hash in reversed(all_ops):
        apply_set_basic_data(t2, addr, 0, code_size, nonce, balance)
        apply_set_code_hash(t2, addr, code_hash)

    root1 = t1.merkelize()
    root2 = t2.merkelize()
    assert root1 == root2, "account insertion order independence failed"

    ops_json = []
    for addr, addr_hex, nonce, balance, code_size, code_hash in all_ops:
        ops_json.append({
            "op": "set_basic_data",
            "address": addr_hex,
            "version": 0,
            "code_size": code_size,
            "nonce": nonce,
            "balance": _u256_to_hex32(balance),
        })
        ops_json.append({
            "op": "set_code_hash",
            "address": addr_hex,
            "code_hash": code_hash.hex(),
        })

    vectors.append({
        "name": "accounts_50_insertion_order_independence",
        "ops": ops_json,
        "expected_root": to_hex(root1),
        "note": "50 accounts; root identical for forward and reverse insertion",
    })

    return vectors


# ---------------------------------------------------------------------------
# Storage generator
#
# 10 synthetic accounts, 100 storage slots total (varied distribution).
# Slot distribution:
#   accounts 0-3: many slots (accounts 0 and 1 get 15 slots each,
#                              accounts 2 and 3 get 10 slots each)
#   accounts 4-9: few slots (2-4 slots each; total 15)
#
# Storage keys span:
#   - Header range: slots 0-63 (sub_indices 64-127 at tree_index=0)
#   - Main range:   slots >= 64 (MAIN_STORAGE_OFFSET + slot)
#
# Zero-writes (writing zero bytes = deletion) form at least 15% of all ops.
#
# Each op: { "op": "set_storage", "address": "0x...", "slot": "<hex32>",
#            "value": "<hex32>" }
# value = "0000...0000" means deletion (zero-write).
# ---------------------------------------------------------------------------

def generate_storage_vectors() -> list[dict]:
    rng = random.Random(SEED ^ 0xC0)
    vectors = []

    NUM_ACCOUNTS = 10
    addresses = [_make_address(rng, 200 + i) for i in range(NUM_ACCOUNTS)]

    def apply_storage(tree: BinaryTree, addr: bytes, slot: int, value: int):
        key = get_tree_key_for_storage_slot(addr, slot)
        if value == 0:
            tree.remove(key)
        else:
            val_bytes = value.to_bytes(32, "big")
            tree.insert(key, val_bytes)

    # Slot distribution:
    # acc 0,1: 15 slots each (header + main range)
    # acc 2,3: 10 slots each (header range only)
    # acc 4-9: 2-4 slots each (main range or header)
    slot_plans = [
        # (account_idx, slots_list)
        (0, list(range(0, 10)) + [64, 100, 200, 512, 1000]),  # 15: 10 header + 5 main
        (1, list(range(5, 15)) + [65, 256, 300, 700, 900]),   # 15: 10 header + 5 main
        (2, list(range(0, 10))),                               # 10 header
        (3, list(range(54, 64))),                              # 10 header (slots 54-63)
        (4, [0, 64]),                                          # 2: 1 header + 1 main
        (5, [0, 64, 128]),                                     # 3: 1 header + 2 main
        (6, [63, 65]),                                         # 2: boundary header + main
        (7, [0, 256, 512]),                                    # 3
        (8, [1, 65, 1000, 4096]),                              # 4
        (9, [63, 64]),                                         # 2: last header + first main
    ]

    # --- Vector S1: all slots written once ---
    t = BinaryTree()
    ops = []
    for acc_idx, slots in slot_plans:
        addr = addresses[acc_idx]
        addr_hex = "0x" + addr.hex()
        for slot in slots:
            val = rng.randint(1, 2**256 - 1)  # non-zero first
            ops.append({
                "op": "set_storage",
                "address": addr_hex,
                "slot": _u256_to_hex32(slot),
                "value": _u256_to_hex32(val),
            })
            apply_storage(t, addr, slot, val)

    vectors.append({
        "name": "storage_initial_write_100_slots",
        "ops": ops,
        "expected_root": to_hex(t.merkelize()),
    })

    # --- Vector S2: zero-writes (deletions) — at least 15% of total ops ---
    # We'll write 20 non-zero + 4 zero-writes (4/24 = 16.7%)
    t = BinaryTree()
    ops = []
    non_zero_slots = [(0, [0, 1, 64, 65, 100]), (1, [2, 3, 66, 67, 200]),
                      (2, [4, 5, 6, 7, 8])]
    all_written = {}  # (acc_idx, slot) -> True
    for acc_idx, slots in non_zero_slots:
        addr = addresses[acc_idx]
        addr_hex = "0x" + addr.hex()
        for slot in slots:
            val = rng.randint(1, 2**256 - 1)
            ops.append({
                "op": "set_storage",
                "address": addr_hex,
                "slot": _u256_to_hex32(slot),
                "value": _u256_to_hex32(val),
            })
            apply_storage(t, addr, slot, val)
            all_written[(acc_idx, slot)] = True

    # Now zero-write (delete) 4 slots across different accounts
    zero_targets = [(0, 1), (0, 65), (1, 2), (2, 5)]
    for acc_idx, slot in zero_targets:
        addr = addresses[acc_idx]
        addr_hex = "0x" + addr.hex()
        ops.append({
            "op": "set_storage",
            "address": addr_hex,
            "slot": _u256_to_hex32(slot),
            "value": _u256_to_hex32(0),  # zero-write = deletion
        })
        apply_storage(t, addr, slot, 0)

    vectors.append({
        "name": "storage_zero_writes_delete",
        "ops": ops,
        "expected_root": to_hex(t.merkelize()),
    })

    # --- Vector S3: header range only (slots 0-63) ---
    t = BinaryTree()
    ops = []
    for i in range(4):
        addr = addresses[i]
        addr_hex = "0x" + addr.hex()
        for slot in range(0, 16):
            val = (i * 16 + slot + 1) * 10 ** 15
            ops.append({
                "op": "set_storage",
                "address": addr_hex,
                "slot": _u256_to_hex32(slot),
                "value": _u256_to_hex32(val),
            })
            apply_storage(t, addr, slot, val)

    vectors.append({
        "name": "storage_header_range_only",
        "ops": ops,
        "expected_root": to_hex(t.merkelize()),
    })

    # --- Vector S4: main storage range only (slots >= 64) ---
    t = BinaryTree()
    ops = []
    main_slots = [64, 65, 100, 128, 255, 256, 512, 1000]
    for i in range(3):
        addr = addresses[i]
        addr_hex = "0x" + addr.hex()
        for slot in main_slots:
            val = rng.randint(1, 2**256 - 1)
            ops.append({
                "op": "set_storage",
                "address": addr_hex,
                "slot": _u256_to_hex32(slot),
                "value": _u256_to_hex32(val),
            })
            apply_storage(t, addr, slot, val)

    vectors.append({
        "name": "storage_main_range_only",
        "ops": ops,
        "expected_root": to_hex(t.merkelize()),
    })

    # --- Vector S5: boundary slots (63 last header, 64 first main) ---
    t = BinaryTree()
    ops = []
    for acc_idx in [0, 1, 2]:
        addr = addresses[acc_idx]
        addr_hex = "0x" + addr.hex()
        for slot in [62, 63, 64, 65]:  # straddle boundary
            val = rng.randint(1, 2**255)
            ops.append({
                "op": "set_storage",
                "address": addr_hex,
                "slot": _u256_to_hex32(slot),
                "value": _u256_to_hex32(val),
            })
            apply_storage(t, addr, slot, val)

    vectors.append({
        "name": "storage_boundary_header_main",
        "ops": ops,
        "expected_root": to_hex(t.merkelize()),
    })

    # --- Vector S6: overwrite then zero-write (write then delete) ---
    t = BinaryTree()
    ops = []
    addr = addresses[0]
    addr_hex = "0x" + addr.hex()
    for slot in [0, 10, 64, 200]:
        # Write non-zero
        val_first = rng.randint(1, 2**256 - 1)
        ops.append({
            "op": "set_storage",
            "address": addr_hex,
            "slot": _u256_to_hex32(slot),
            "value": _u256_to_hex32(val_first),
        })
        apply_storage(t, addr, slot, val_first)
        # Overwrite with different non-zero
        val_second = rng.randint(1, 2**256 - 1)
        ops.append({
            "op": "set_storage",
            "address": addr_hex,
            "slot": _u256_to_hex32(slot),
            "value": _u256_to_hex32(val_second),
        })
        apply_storage(t, addr, slot, val_second)
        # Zero-write (delete)
        ops.append({
            "op": "set_storage",
            "address": addr_hex,
            "slot": _u256_to_hex32(slot),
            "value": _u256_to_hex32(0),
        })
        apply_storage(t, addr, slot, 0)

    vectors.append({
        "name": "storage_overwrite_then_zero_write",
        "ops": ops,
        "expected_root": to_hex(t.merkelize()),
    })

    return vectors


# ---------------------------------------------------------------------------
# Code-chunk boundary generator
#
# Produces keys at chunk_ids: 0, 1, 127, 128, 1000, 1023 explicitly.
# chunk_id=0:    pos=128, tree_index=0, sub_index=128
# chunk_id=1:    pos=129, tree_index=0, sub_index=129
# chunk_id=127:  pos=255, tree_index=0, sub_index=255  (last chunk in subtree 0)
# chunk_id=128:  pos=256, tree_index=1, sub_index=0    (first chunk in subtree 1)
# chunk_id=1000: pos=1128, tree_index=4, sub_index=104
# chunk_id=1023: pos=1151, tree_index=4, sub_index=127
#
# Also includes one contract spanning chunk_ids 0-1023 contiguously.
# ---------------------------------------------------------------------------

def _make_chunk_value(chunk_id: int, addr_idx: int) -> bytes:
    """Deterministic 32-byte chunk value (1 leading byte + 31 code bytes)."""
    leading = min(chunk_id % 32, 31)
    code_byte = (chunk_id ^ addr_idx) & 0xFF
    return bytes([leading]) + bytes([code_byte] * 31)


def generate_codechunk_vectors() -> list[dict]:
    rng = random.Random(SEED ^ 0xD0)
    vectors = []

    addresses_cc = [_make_address(rng, 400 + i) for i in range(5)]

    BOUNDARY_CHUNKS = [0, 1, 127, 128, 1000, 1023]

    def apply_code_chunk(tree: BinaryTree, addr: bytes, chunk_id: int, value: bytes):
        key = get_tree_key_for_code_chunk(addr, chunk_id)
        tree.insert(key, value)

    # --- Vector C1: single contract with all boundary chunk_ids ---
    addr = addresses_cc[0]
    addr_hex = "0x" + addr.hex()
    t = BinaryTree()
    ops = []
    for chunk_id in BOUNDARY_CHUNKS:
        val = _make_chunk_value(chunk_id, 0)
        ops.append({
            "op": "set_code_chunk",
            "address": addr_hex,
            "chunk_id": chunk_id,
            "value": val.hex(),
        })
        apply_code_chunk(t, addr, chunk_id, val)

    vectors.append({
        "name": "codechunk_boundary_ids_single_contract",
        "ops": ops,
        "expected_root": to_hex(t.merkelize()),
    })

    # --- Vector C2: boundary splits — chunk 127 and 128 on same address,
    #     verifying different stems (tree_index 0 vs 1) ---
    addr = addresses_cc[1]
    addr_hex = "0x" + addr.hex()
    t = BinaryTree()
    ops = []
    for chunk_id in [126, 127, 128, 129]:
        val = _make_chunk_value(chunk_id, 1)
        ops.append({
            "op": "set_code_chunk",
            "address": addr_hex,
            "chunk_id": chunk_id,
            "value": val.hex(),
        })
        apply_code_chunk(t, addr, chunk_id, val)

    vectors.append({
        "name": "codechunk_split_at_128",
        "ops": ops,
        "expected_root": to_hex(t.merkelize()),
    })

    # --- Vector C3: high-index boundary (chunk_ids 999, 1000, 1022, 1023) ---
    addr = addresses_cc[2]
    addr_hex = "0x" + addr.hex()
    t = BinaryTree()
    ops = []
    for chunk_id in [999, 1000, 1022, 1023]:
        val = _make_chunk_value(chunk_id, 2)
        ops.append({
            "op": "set_code_chunk",
            "address": addr_hex,
            "chunk_id": chunk_id,
            "value": val.hex(),
        })
        apply_code_chunk(t, addr, chunk_id, val)

    vectors.append({
        "name": "codechunk_high_index_boundary",
        "ops": ops,
        "expected_root": to_hex(t.merkelize()),
    })

    # --- Vector C4: full contract 0-1023 contiguous (1024 chunks on one address) ---
    addr = addresses_cc[3]
    addr_hex = "0x" + addr.hex()
    t = BinaryTree()
    ops = []
    for chunk_id in range(1024):
        val = _make_chunk_value(chunk_id, 3)
        ops.append({
            "op": "set_code_chunk",
            "address": addr_hex,
            "chunk_id": chunk_id,
            "value": val.hex(),
        })
        apply_code_chunk(t, addr, chunk_id, val)

    vectors.append({
        "name": "codechunk_full_contract_0_to_1023",
        "ops": ops,
        "expected_root": to_hex(t.merkelize()),
    })

    # --- Vector C5: multiple contracts each with boundary chunks ---
    t = BinaryTree()
    ops = []
    for addr_idx in range(3):
        addr = addresses_cc[addr_idx]
        addr_hex = "0x" + addr.hex()
        for chunk_id in BOUNDARY_CHUNKS:
            val = _make_chunk_value(chunk_id, addr_idx)
            ops.append({
                "op": "set_code_chunk",
                "address": addr_hex,
                "chunk_id": chunk_id,
                "value": val.hex(),
            })
            apply_code_chunk(t, addr, chunk_id, val)

    vectors.append({
        "name": "codechunk_multi_contract_boundaries",
        "ops": ops,
        "expected_root": to_hex(t.merkelize()),
    })

    return vectors


# ---------------------------------------------------------------------------
# Negative-case vectors
#
# Three distinct absence-proof shapes (per plan):
#   1. same-stem-absent-subindex: stem exists but queried sub_index is absent
#   2. different-stem-absent:     path leads to a StemNode with different stem
#   3. empty-child-absent:        path terminates at a None child
#
# Two selfdestruct-then-recreated cases:
#   4. selfdestructed_then_recreated_basic: recreated account has new state
#   5. selfdestructed_then_recreated_code:  recreated account has new code_hash
#
# Each vector includes:
#   - ops: list of insert ops to build the trie
#   - probe_key: the key to prove absence for (or the key to check recreated)
#   - expected_root: the root after all ops
#   - expected_absent: true if the key should be absent in the final trie
#   - expected_value: the expected leaf value (hex32) if present, or null if absent
# ---------------------------------------------------------------------------

def generate_negative_vectors() -> list[dict]:
    rng = random.Random(SEED ^ 0xE0)
    vectors = []

    addresses_neg = [_make_address(rng, 600 + i) for i in range(10)]

    # --- Case 1: same-stem-absent-subindex ---
    # Insert basic_data (sub_index=0) but NOT code_hash (sub_index=1).
    # Probe sub_index=1 → stem matches but value is None.
    addr = addresses_neg[0]
    addr_hex = "0x" + addr.hex()
    t = BinaryTree()
    basic_key = get_tree_key_for_basic_data(addr)
    basic_val = pack_basic_data(0, 0, 1, 10 ** 18)
    t.insert(basic_key, basic_val)
    root = t.merkelize()

    probe_key = get_tree_key_for_code_hash(addr)  # same stem, sub_index=1

    vectors.append({
        "name": "absence_same_stem_absent_subindex",
        "ops": [
            {
                "op": "insert_raw",
                "key": to_hex(basic_key),
                "value": to_hex(basic_val),
            }
        ],
        "probe_key": to_hex(probe_key),
        "expected_root": to_hex(root),
        "expected_absent": True,
        "expected_value": None,
        "note": "same stem as inserted basic_data; code_hash sub_index was never written",
    })

    # --- Case 2: different-stem-absent ---
    # Insert one key; probe a key whose stem is completely different (different-stem shape).
    addr_a = addresses_neg[1]
    addr_b = addresses_neg[2]
    t = BinaryTree()
    key_a = get_tree_key_for_basic_data(addr_a)
    val_a = pack_basic_data(0, 0, 1, 10 ** 18)
    t.insert(key_a, val_a)
    root = t.merkelize()

    probe_key = get_tree_key_for_basic_data(addr_b)  # different stem entirely

    vectors.append({
        "name": "absence_different_stem",
        "ops": [
            {
                "op": "insert_raw",
                "key": to_hex(key_a),
                "value": to_hex(val_a),
            }
        ],
        "probe_key": to_hex(probe_key),
        "expected_root": to_hex(root),
        "expected_absent": True,
        "expected_value": None,
        "note": "probe stem has never been inserted; path hits addr_a's StemNode (different stem) or a None child",
    })

    # --- Case 3: empty-child-absent ---
    # Insert a key with MSB=1 in stem (routes to right subtree at depth 0).
    # Probe a key whose stem has MSB=0 → routes to left child which is None.
    t = BinaryTree()
    # Force MSB of stem to be 1
    stem_right = bytes([0x80]) + bytes([0x00] * 30)
    key_right = stem_right + bytes([0])
    val_right = bytes([0xAB] * 32)
    t.insert(key_right, val_right)
    root = t.merkelize()

    # Probe key with MSB=0 → left child is None
    stem_left = bytes([0x00] * 31)
    probe_key = stem_left + bytes([0])

    vectors.append({
        "name": "absence_empty_child",
        "ops": [
            {
                "op": "insert_raw",
                "key": to_hex(key_right),
                "value": to_hex(val_right),
            }
        ],
        "probe_key": to_hex(probe_key),
        "expected_root": to_hex(root),
        "expected_absent": True,
        "expected_value": None,
        "note": "left child of root is None; probe routes there",
    })

    # --- Case 4: selfdestructed_then_recreated_basic ---
    # After selfdestruct + recreation, the recreated state (nonce=1, new balance)
    # is what the trie should contain — not the pre-destruct state.
    addr = addresses_neg[3]
    addr_hex = "0x" + addr.hex()

    t = BinaryTree()
    basic_key = get_tree_key_for_basic_data(addr)
    code_hash_key = get_tree_key_for_code_hash(addr)

    # Phase 1: create account with nonce=5, balance=5 ETH
    pre_basic = pack_basic_data(0, 0, 5, 5 * 10 ** 18)
    t.insert(basic_key, pre_basic)
    t.insert(code_hash_key, ZERO_KECCAK)

    # Phase 2: selfdestruct (clear all leaves on this stem)
    stem = get_stem_for_base(addr)
    for sub_index in range(STEM_SUBTREE_WIDTH):
        key = stem + bytes([sub_index])
        t.remove(key)

    # Phase 3: recreate with nonce=1, new balance
    post_basic = pack_basic_data(0, 0, 1, 2 * 10 ** 18)
    t.insert(basic_key, post_basic)
    t.insert(code_hash_key, ZERO_KECCAK)

    root = t.merkelize()

    ops = [
        {"op": "insert_raw", "key": to_hex(basic_key), "value": to_hex(pre_basic)},
        {"op": "insert_raw", "key": to_hex(code_hash_key), "value": to_hex(ZERO_KECCAK)},
        {"op": "clear_stem_raw", "stem": get_stem_for_base(addr).hex()},
        {"op": "insert_raw", "key": to_hex(basic_key), "value": to_hex(post_basic)},
        {"op": "insert_raw", "key": to_hex(code_hash_key), "value": to_hex(ZERO_KECCAK)},
    ]

    vectors.append({
        "name": "selfdestruct_then_recreated_basic",
        "ops": ops,
        "probe_key": to_hex(basic_key),
        "expected_root": to_hex(root),
        "expected_absent": False,
        "expected_value": to_hex(post_basic),
        "note": "recreated account; Rust side must observe post_basic (nonce=1), not pre_basic (nonce=5)",
    })

    # --- Case 5: selfdestructed_then_recreated_code ---
    addr = addresses_neg[4]
    addr_hex = "0x" + addr.hex()

    t = BinaryTree()
    basic_key = get_tree_key_for_basic_data(addr)
    code_hash_key = get_tree_key_for_code_hash(addr)
    pre_code_hash = bytes([0xCC] * 32)
    post_code_hash = bytes([0xDD] * 32)

    pre_basic = pack_basic_data(0, 100, 3, 10 ** 18)
    t.insert(basic_key, pre_basic)
    t.insert(code_hash_key, pre_code_hash)

    # Selfdestruct
    stem = get_stem_for_base(addr)
    for sub_index in range(STEM_SUBTREE_WIDTH):
        key = stem + bytes([sub_index])
        t.remove(key)

    # Recreate with new code_hash
    post_basic = pack_basic_data(0, 200, 1, 5 * 10 ** 17)
    t.insert(basic_key, post_basic)
    t.insert(code_hash_key, post_code_hash)

    root = t.merkelize()

    ops = [
        {"op": "insert_raw", "key": to_hex(basic_key), "value": to_hex(pre_basic)},
        {"op": "insert_raw", "key": to_hex(code_hash_key), "value": to_hex(pre_code_hash)},
        {"op": "clear_stem_raw", "stem": stem.hex()},
        {"op": "insert_raw", "key": to_hex(basic_key), "value": to_hex(post_basic)},
        {"op": "insert_raw", "key": to_hex(code_hash_key), "value": to_hex(post_code_hash)},
    ]

    vectors.append({
        "name": "selfdestruct_then_recreated_code",
        "ops": ops,
        "probe_key": to_hex(code_hash_key),
        "expected_root": to_hex(root),
        "expected_absent": False,
        "expected_value": to_hex(post_code_hash),
        "note": "recreated account has different code_hash; Rust side must see post_code_hash",
    })

    return vectors


# ---------------------------------------------------------------------------
# Main — emit all four JSON files
# ---------------------------------------------------------------------------

def main():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    testgen_dir = script_dir

    # test_vectors.json (original 10 vectors, kept as-is)
    vectors = generate_vectors()
    path = os.path.join(testgen_dir, "test_vectors.json")
    with open(path, "w") as f:
        json.dump(vectors, f, indent=2)
    print(f"Wrote {len(vectors)} vectors to {path}")

    # vectors_accounts.json
    account_vecs = generate_account_vectors()
    path = os.path.join(testgen_dir, "vectors_accounts.json")
    with open(path, "w") as f:
        json.dump(account_vecs, f, indent=2)
    print(f"Wrote {len(account_vecs)} vectors to {path}")

    # vectors_storage.json
    storage_vecs = generate_storage_vectors()
    path = os.path.join(testgen_dir, "vectors_storage.json")
    with open(path, "w") as f:
        json.dump(storage_vecs, f, indent=2)
    print(f"Wrote {len(storage_vecs)} vectors to {path}")

    # vectors_codechunk.json
    codechunk_vecs = generate_codechunk_vectors()
    path = os.path.join(testgen_dir, "vectors_codechunk.json")
    with open(path, "w") as f:
        json.dump(codechunk_vecs, f, indent=2)
    print(f"Wrote {len(codechunk_vecs)} vectors to {path}")

    # vectors_negative.json
    negative_vecs = generate_negative_vectors()
    path = os.path.join(testgen_dir, "vectors_negative.json")
    with open(path, "w") as f:
        json.dump(negative_vecs, f, indent=2)
    print(f"Wrote {len(negative_vecs)} vectors to {path}")


if __name__ == "__main__":
    main()
