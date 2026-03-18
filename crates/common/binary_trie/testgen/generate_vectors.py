"""
Generate test vectors for ethrex binary trie from EIP-7864 reference implementation.

Run: python3 generate_vectors.py > ../test_vectors.json
Requires: pip install blake3
"""

import json
from typing import Optional
from blake3 import blake3


class StemNode:
    def __init__(self, stem: bytes):
        assert len(stem) == 31, "stem must be 31 bytes"
        self.stem = stem
        self.values: list[Optional[bytes]] = [None] * 256

    def set_value(self, index: int, value: bytes):
        self.values[index] = value


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
        """Convert bytes to list of bits (MSB first)."""
        bits = []
        for byte in data:
            for i in range(7, -1, -1):
                bits.append((byte >> i) & 1)
        return bits

    def _bits_to_bytes(self, bits: list[int]) -> bytes:
        """Convert list of bits back to bytes (MSB first)."""
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

    print(json.dumps(vectors, indent=2))


if __name__ == "__main__":
    generate_vectors()
