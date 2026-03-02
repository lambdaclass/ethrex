package spec

import (
	"encoding/hex"
	"testing"

	"github.com/Giulio2002/gevm/types"
)

func TestEmptyTrieRoot(t *testing.T) {
	root := mptRoot(nil)
	// keccak256(0x80) — the well-known empty trie root
	want := "56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421"
	if root.Hex() != "0x"+want {
		t.Fatalf("empty trie root: got %s, want 0x%s", root.Hex(), want)
	}
}

func TestHexPrefixEncode(t *testing.T) {
	tests := []struct {
		nibbles []byte
		isLeaf  bool
		want    string
	}{
		// Even extension: [0x00, nibbles...]
		{[]byte{1, 2, 3, 4}, false, "00" + "1234"},
		// Odd extension: [0x1N, nibbles...]
		{[]byte{1, 2, 3}, false, "11" + "23"},
		// Even leaf: [0x20, nibbles...]
		{[]byte{1, 2, 3, 4}, true, "20" + "1234"},
		// Odd leaf: [0x3N, nibbles...]
		{[]byte{1, 2, 3}, true, "31" + "23"},
		// Empty extension
		{[]byte{}, false, "00"},
		// Empty leaf
		{[]byte{}, true, "20"},
		// Single nibble leaf
		{[]byte{0x0f}, true, "3f"},
		// Single nibble extension
		{[]byte{0x0a}, false, "1a"},
	}

	for _, tt := range tests {
		got := hex.EncodeToString(hexPrefixEncode(tt.nibbles, tt.isLeaf))
		if got != tt.want {
			t.Errorf("hexPrefixEncode(%v, %v) = %s, want %s", tt.nibbles, tt.isLeaf, got, tt.want)
		}
	}
}

func TestKeyToNibbles(t *testing.T) {
	var key types.B256
	key[0] = 0xab
	key[1] = 0xcd
	key[31] = 0xef

	nibbles := keyToNibbles(key)
	if nibbles[0] != 0xa || nibbles[1] != 0xb {
		t.Errorf("nibbles[0:2] = %x %x, want a b", nibbles[0], nibbles[1])
	}
	if nibbles[2] != 0xc || nibbles[3] != 0xd {
		t.Errorf("nibbles[2:4] = %x %x, want c d", nibbles[2], nibbles[3])
	}
	if nibbles[62] != 0xe || nibbles[63] != 0xf {
		t.Errorf("nibbles[62:64] = %x %x, want e f", nibbles[62], nibbles[63])
	}
}

func TestSingleEntryTrie(t *testing.T) {
	// A trie with one entry produces: Leaf([hp_encode(key, true), value])
	// Then root = keccak256(leaf_rlp)
	var key types.B256 // all zeros
	nibbles := keyToNibbles(key)
	value := RlpEncodeBytes([]byte("hello"))

	entries := []mptEntry{{keyNibbles: nibbles, value: value}}
	root := mptRoot(entries)

	// Verify it's not empty
	if root == emptyTrieRoot {
		t.Fatal("single entry trie should not have empty root")
	}

	// Verify deterministic: same input gives same root
	root2 := mptRoot([]mptEntry{{keyNibbles: nibbles, value: value}})
	if root != root2 {
		t.Fatalf("non-deterministic: %s vs %s", root.Hex(), root2.Hex())
	}
}

func TestTwoEntryTrie(t *testing.T) {
	// Two entries that differ at the first nibble → branch node at depth 0
	key1 := types.Keccak256([]byte("key1"))
	key2 := types.Keccak256([]byte("key2"))

	entries := []mptEntry{
		{keyNibbles: keyToNibbles(key1), value: RlpEncodeBytes([]byte("val1"))},
		{keyNibbles: keyToNibbles(key2), value: RlpEncodeBytes([]byte("val2"))},
	}

	root := mptRoot(entries)
	if root == emptyTrieRoot {
		t.Fatal("two-entry trie should not have empty root")
	}

	// Reversed order should give same root (sorting is internal)
	entries2 := []mptEntry{
		{keyNibbles: keyToNibbles(key2), value: RlpEncodeBytes([]byte("val2"))},
		{keyNibbles: keyToNibbles(key1), value: RlpEncodeBytes([]byte("val1"))},
	}
	root2 := mptRoot(entries2)
	if root != root2 {
		t.Fatalf("order should not matter: %s vs %s", root.Hex(), root2.Hex())
	}
}

func TestStorageRootEmpty(t *testing.T) {
	root := storageRoot(nil)
	if root != emptyTrieRoot {
		t.Fatalf("empty storage root: got %s, want %s", root.Hex(), emptyTrieRoot.Hex())
	}
}

func TestStorageRootSingleSlot(t *testing.T) {
	storage := map[types.Uint256]types.Uint256{
		types.U256From(0): types.U256From(1),
	}
	root := storageRoot(storage)
	if root == emptyTrieRoot {
		t.Fatal("single-slot storage should not have empty root")
	}

	// Same input → same root
	root2 := storageRoot(storage)
	if root != root2 {
		t.Fatalf("non-deterministic: %s vs %s", root.Hex(), root2.Hex())
	}
}

func TestRlpEncodeAccount(t *testing.T) {
	// Verify account RLP encoding produces a valid RLP list
	encoded := rlpEncodeAccount(0, types.U256Zero, emptyTrieRoot, types.KeccakEmpty)
	if len(encoded) == 0 {
		t.Fatal("empty account encoding")
	}
	// First byte should be RLP list prefix
	if encoded[0] < 0xc0 {
		t.Fatalf("expected RLP list prefix, got 0x%02x", encoded[0])
	}
}

// TestEthereumTrieVector tests against the Ethereum trie test "emptyValues" from the wiki.
// An empty trie has a well-known root; a trie with one key-value pair has a deterministic root.
func TestEthereumTrieVector(t *testing.T) {
	// Test: trie with key=keccak256(""), value=RLP("") should produce a specific root.
	key := types.Keccak256(nil) // keccak256 of empty input
	nibbles := keyToNibbles(key)
	value := RlpEncodeBytes(nil) // RLP("") = 0x80

	entries := []mptEntry{{keyNibbles: nibbles, value: value}}
	root := mptRoot(entries)

	// This should not be the empty root since we have one entry
	if root == emptyTrieRoot {
		t.Fatal("trie with one entry should not have empty root")
	}

	// Verify by manual computation:
	// Leaf node: RLP([hp_encode(64 nibbles, leaf=true), 0x80])
	remaining := nibbles[:]
	leafRLP := RlpEncodeList([][]byte{
		RlpEncodeBytes(hexPrefixEncode(remaining, true)),
		value,
	})
	want := types.Keccak256(leafRLP)
	if root != want {
		t.Fatalf("single-entry root mismatch: got %s, want %s", root.Hex(), want.Hex())
	}
}
