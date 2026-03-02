// Minimal Merkle Patricia Trie for computing Ethereum state roots.
// Test-only code: builds trie from sorted entries in one pass, no persistent storage.
package spec

import (
	"slices"

	"github.com/Giulio2002/gevm/types"
)

// emptyTrieRoot is keccak256(RLP("")) = keccak256(0x80).
var emptyTrieRoot = types.Keccak256([]byte{0x80})

// mptEntry is a key-value pair for trie insertion.
// keyNibbles is the 64-nibble representation of a 32-byte keccak hash.
type mptEntry struct {
	keyNibbles [64]byte
	value      []byte // RLP-encoded value
}

// mptRoot computes the Merkle Patricia Trie root hash from a set of entries.
// Returns emptyTrieRoot for empty input.
func mptRoot(entries []mptEntry) types.B256 {
	if len(entries) == 0 {
		return emptyTrieRoot
	}

	// Sort by key nibbles.
	slices.SortFunc(entries, func(a, b mptEntry) int {
		for i := range a.keyNibbles {
			if a.keyNibbles[i] < b.keyNibbles[i] {
				return -1
			}
			if a.keyNibbles[i] > b.keyNibbles[i] {
				return 1
			}
		}
		return 0
	})

	node := buildNode(entries, 0)
	if len(node) < 32 {
		// Root is always hashed, even if < 32 bytes.
		return types.Keccak256(node)
	}
	return types.Keccak256(node)
}

// buildNode recursively builds a trie node from sorted entries starting at the given nibble depth.
// Returns the RLP encoding of the node.
func buildNode(entries []mptEntry, depth int) []byte {
	if len(entries) == 0 {
		return []byte{0x80} // RLP empty string
	}

	if len(entries) == 1 {
		// Leaf node: [hp_encode(remaining_key, leaf=true), value]
		remaining := entries[0].keyNibbles[depth:]
		return RlpEncodeList([][]byte{
			RlpEncodeBytes(hexPrefixEncode(remaining, true)),
			entries[0].value,
		})
	}

	// Find common prefix length among all entries at current depth.
	first := entries[0].keyNibbles[depth:]
	last := entries[len(entries)-1].keyNibbles[depth:]
	cpLen := commonPrefixLen(first, last)

	if cpLen > 0 {
		// Extension node: [hp_encode(shared_prefix, leaf=false), child_ref]
		child := buildNode(entries, depth+cpLen)
		return RlpEncodeList([][]byte{
			RlpEncodeBytes(hexPrefixEncode(first[:cpLen], false)),
			nodeRef(child),
		})
	}

	// Branch node: 17 items [child0..child15, value]
	// Partition entries by the nibble at `depth`.
	items := make([][]byte, 17)

	start := 0
	for start < len(entries) {
		nibble := entries[start].keyNibbles[depth]
		// Find the end of entries with this nibble.
		end := start + 1
		for end < len(entries) && entries[end].keyNibbles[depth] == nibble {
			end++
		}
		child := buildNode(entries[start:end], depth+1)
		items[nibble] = nodeRef(child)
		start = end
	}

	// Fill empty slots with 0x80.
	for i := 0; i < 16; i++ {
		if items[i] == nil {
			items[i] = []byte{0x80}
		}
	}
	// 17th item: value at this branch (always empty for keccak-keyed tries).
	items[16] = []byte{0x80}

	return RlpEncodeList(items)
}

// hexPrefixEncode applies compact hex-prefix encoding to a nibble path.
// isLeaf controls the flag bit (bit 1 of the first nibble).
func hexPrefixEncode(nibbles []byte, isLeaf bool) []byte {
	var flag byte
	if isLeaf {
		flag = 2
	}

	odd := len(nibbles) % 2
	if odd == 1 {
		// Odd: first byte = (flag|1) << 4 | first_nibble
		out := make([]byte, 1+len(nibbles)/2)
		out[0] = (flag|1)<<4 | nibbles[0]
		for i := 1; i < len(nibbles); i += 2 {
			out[1+i/2] = nibbles[i]<<4 | nibbles[i+1]
		}
		return out
	}

	// Even: first byte = flag << 4 | 0, then pairs
	out := make([]byte, 1+len(nibbles)/2)
	out[0] = flag << 4
	for i := 0; i < len(nibbles); i += 2 {
		out[1+i/2] = nibbles[i]<<4 | nibbles[i+1]
	}
	return out
}

// nodeRef returns the reference to a node for embedding in a parent.
// If the RLP encoding is < 32 bytes, it is inlined as-is.
// Otherwise, it is replaced by RlpEncodeBytes(keccak256(rlp)).
func nodeRef(rlpEncoded []byte) []byte {
	if len(rlpEncoded) < 32 {
		return rlpEncoded
	}
	h := types.Keccak256(rlpEncoded)
	return RlpEncodeBytes(h[:])
}

// keyToNibbles converts a 32-byte hash to a 64-nibble array.
func keyToNibbles(key types.B256) [64]byte {
	var nibbles [64]byte
	for i, b := range key {
		nibbles[i*2] = b >> 4
		nibbles[i*2+1] = b & 0x0f
	}
	return nibbles
}

// commonPrefixLen returns the length of the shared prefix between two nibble slices.
func commonPrefixLen(a, b []byte) int {
	n := len(a)
	if len(b) < n {
		n = len(b)
	}
	for i := 0; i < n; i++ {
		if a[i] != b[i] {
			return i
		}
	}
	return n
}
