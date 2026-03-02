// Minimal RLP encoding for computing logs root hash.
// Only the subset needed for encoding Log entries.
package spec

import (
	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/state"
)

// LogsRoot computes the keccak256 hash of RLP-encoded logs.
// Computes the logs hash for test fixture validation.
func LogsRoot(logs []state.Log) types.B256 {
	encoded := rlpEncodeLogs(logs)
	return types.Keccak256(encoded)
}

// rlpEncodeLogs encodes a list of logs as RLP.
func rlpEncodeLogs(logs []state.Log) []byte {
	var items [][]byte
	for _, log := range logs {
		items = append(items, rlpEncodeLog(&log))
	}
	return RlpEncodeList(items)
}

// rlpEncodeLog encodes a single log entry as RLP.
// Log = [address, [topic0, topic1, ...], data]
func rlpEncodeLog(log *state.Log) []byte {
	var items [][]byte

	// Address (20 bytes)
	items = append(items, RlpEncodeBytes(log.Address[:]))

	// Topics as a list of B256
	var topicItems [][]byte
	for _, t := range log.TopicSlice() {
		topicItems = append(topicItems, RlpEncodeBytes(t[:]))
	}
	items = append(items, RlpEncodeList(topicItems))

	// Data
	items = append(items, RlpEncodeBytes(log.Data))

	return RlpEncodeList(items)
}

// RlpEncodeBytes encodes a byte string in RLP.
func RlpEncodeBytes(b []byte) []byte {
	if len(b) == 0 {
		return []byte{0x80}
	}
	if len(b) == 1 && b[0] < 0x80 {
		return []byte{b[0]}
	}
	if len(b) < 56 {
		prefix := byte(0x80 + len(b))
		result := make([]byte, 1+len(b))
		result[0] = prefix
		copy(result[1:], b)
		return result
	}
	// Long string
	lenBytes := encodeLength(uint64(len(b)))
	prefix := byte(0xb7 + len(lenBytes))
	result := make([]byte, 1+len(lenBytes)+len(b))
	result[0] = prefix
	copy(result[1:], lenBytes)
	copy(result[1+len(lenBytes):], b)
	return result
}

// RlpEncodeList encodes a list of already-RLP-encoded items.
func RlpEncodeList(items [][]byte) []byte {
	var totalLen int
	for _, item := range items {
		totalLen += len(item)
	}

	var result []byte
	if totalLen < 56 {
		result = make([]byte, 1+totalLen)
		result[0] = byte(0xc0 + totalLen)
		offset := 1
		for _, item := range items {
			copy(result[offset:], item)
			offset += len(item)
		}
	} else {
		lenBytes := encodeLength(uint64(totalLen))
		result = make([]byte, 1+len(lenBytes)+totalLen)
		result[0] = byte(0xf7 + len(lenBytes))
		copy(result[1:], lenBytes)
		offset := 1 + len(lenBytes)
		for _, item := range items {
			copy(result[offset:], item)
			offset += len(item)
		}
	}
	return result
}

// RlpEncodeUint64 encodes a uint64 as an RLP byte string (big-endian, no leading zeros).
// Zero is encoded as empty byte string (0x80).
func RlpEncodeUint64(v uint64) []byte {
	if v == 0 {
		return []byte{0x80}
	}
	// Encode as big-endian bytes without leading zeros
	var buf [8]byte
	buf[0] = byte(v >> 56)
	buf[1] = byte(v >> 48)
	buf[2] = byte(v >> 40)
	buf[3] = byte(v >> 32)
	buf[4] = byte(v >> 24)
	buf[5] = byte(v >> 16)
	buf[6] = byte(v >> 8)
	buf[7] = byte(v)
	for i := 0; i < 8; i++ {
		if buf[i] != 0 {
			return RlpEncodeBytes(buf[i:])
		}
	}
	return []byte{0x80}
}

// RlpEncodeU256 encodes a Uint256 as an RLP byte string (big-endian, no leading zeros).
// Zero is encoded as empty byte string (0x80).
func RlpEncodeU256(v types.Uint256) []byte {
	if v.IsZero() {
		return []byte{0x80}
	}
	b32 := v.ToBytes32()
	// Skip leading zeros
	for i := 0; i < 32; i++ {
		if b32[i] != 0 {
			return RlpEncodeBytes(b32[i:])
		}
	}
	return []byte{0x80}
}

// encodeLength encodes a length as big-endian bytes without leading zeros.
func encodeLength(n uint64) []byte {
	if n == 0 {
		return []byte{0}
	}
	var buf [8]byte
	buf[0] = byte(n >> 56)
	buf[1] = byte(n >> 48)
	buf[2] = byte(n >> 40)
	buf[3] = byte(n >> 32)
	buf[4] = byte(n >> 24)
	buf[5] = byte(n >> 16)
	buf[6] = byte(n >> 8)
	buf[7] = byte(n)
	// Find first non-zero byte
	for i := 0; i < 8; i++ {
		if buf[i] != 0 {
			return buf[i:]
		}
	}
	return []byte{0}
}
