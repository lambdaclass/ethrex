## EIP-7864 Binary Trie: Explained Simply

### The problem with today's Ethereum state

Ethereum stores all account data (balances, contract code, storage) in a **Merkle-Patricia Trie (MPT)**. It's actually a "tree of trees":

```
State Trie (accounts)
├── keccak(alice) → {nonce, balance, code_hash, storage_root}
│                                                    └── Storage Trie (alice's data)
│                                                        ├── keccak(slot_0) → value
│                                                        └── keccak(slot_1) → value
├── keccak(bob) → {nonce, balance, code_hash, storage_root}
│                                                    └── Storage Trie (bob's data)
└── ...
```

Problems:
- **16-way branching**: each node has up to 16 children (hexadecimal), so proofs are large
- **Keccak everywhere**: hard to prove in zero-knowledge circuits
- **Tree of trees**: each account has its own separate storage trie, making everything more complex
- **Code stored separately**: not in the trie at all

### The binary trie solution

EIP-7864 replaces all of this with **one flat binary tree**. Everything goes in the same tree: accounts, storage, code, all of it.

```
One Binary Trie
├── 0 (left)
│   ├── 0
│   │   └── StemNode [alice's stem]
│   │       ├── slot 0: basic_data (nonce, balance packed into 32 bytes)
│   │       ├── slot 1: code_hash
│   │       ├── slot 64: storage slot 0
│   │       ├── slot 65: storage slot 1
│   │       ├── slot 128: code chunk 0 (first 31 bytes of bytecode)
│   │       ├── slot 129: code chunk 1 (next 31 bytes)
│   │       └── ... (256 slots total per stem)
│   └── 1
│       └── StemNode [bob's stem]
│           └── ...
└── 1 (right)
    └── ...
```

### Key concepts

**Binary = two children per node.** Left (bit 0) or right (bit 1). Much simpler than MPT's 16 children. Proofs are smaller.

**Stems group 256 values.** Instead of one value per leaf, a StemNode holds 256 values. An account's basic data, code hash, first 64 storage slots, and first 128 code chunks all share the same stem. This means reading related data (like an account's balance AND its storage) often hits the same part of the tree.

**How keys work:**
```
key = BLAKE3(zero_padded_address + tree_index)[first 31 bytes] + sub_index

Example for alice's balance:
  address = 0x000...alice (padded to 32 bytes)
  tree_index = 0 (basic account data)
  sub_index = 0 (basic_data slot)
  → key = BLAKE3(address + 0)[0:31] + [0]

Example for alice's storage slot 5:
  tree_index = 0
  sub_index = 64 + 5 = 69
  → key = BLAKE3(address + 0)[0:31] + [69]   ← same stem! just different slot
```

**Account data is packed into 32 bytes:**
```
[version(1)] [reserved(4)] [code_size(3)] [nonce(8)] [balance(16)]
```

No more separate `storage_root` field. Storage lives directly in the tree.

**Code is chunked into 31-byte pieces**, each with a 1-byte header saying how many bytes are PUSH data (so you know what's executable vs data when doing code analysis).

**BLAKE3 instead of Keccak.** Much faster to prove in zero-knowledge circuits. That's the whole point: making Ethereum state provable.

### How hashing works

```
Empty node → [0x00 * 32]

Internal node → BLAKE3(left_hash + right_hash)
                (special case: if both are zero → zero, not actual hash)

Stem node → BLAKE3(stem + 0x00 + subtree_root)
            where subtree_root = binary merkle tree of 256 value hashes
```

The root of the whole trie is one 32-byte hash that commits to the entire Ethereum state.

### Why it matters

This lets you **prove** that a specific account has a specific balance (or storage value) by providing a short proof path through the binary tree. And because it uses BLAKE3 (or eventually Poseidon2), these proofs can be verified efficiently inside a ZK circuit.
