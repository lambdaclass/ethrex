package types

import (
	"encoding/hex"
	"fmt"

	keccak "github.com/Giulio2002/fastkeccak"
)

// Address is a 20-byte Ethereum address.
type Address [20]byte

// AddressZero is the zero address.
var AddressZero Address

// AddressFrom creates an Address from a byte slice (right-aligned, zero-padded left).
func AddressFrom(b []byte) Address {
	var addr Address
	if len(b) >= 20 {
		copy(addr[:], b[len(b)-20:])
	} else {
		copy(addr[20-len(b):], b)
	}
	return addr
}

// IsZero returns true if the address is all zeros.
func (a *Address) IsZero() bool {
	return *a == AddressZero
}

// ToU256 converts an Address to Uint256 (right-aligned, big-endian in the 32-byte representation).
func (a *Address) ToU256() Uint256 {
	var b [32]byte
	copy(b[12:], a[:]) // Address occupies the low 20 bytes (left-padded with zeros)
	return U256FromBytes32(b)
}

// Hex returns the checksumless hex representation with 0x prefix.
func (a *Address) Hex() string {
	return "0x" + hex.EncodeToString(a[:])
}

// String returns the hex representation.
func (a *Address) String() string {
	return a.Hex()
}

// B256 is a 32-byte hash / fixed-size byte array.
type B256 [32]byte

// B256Zero is the zero hash.
var B256Zero B256

// B256From creates a B256 from a byte slice (right-aligned, zero-padded left).
func B256From(b []byte) B256 {
	var h B256
	if len(b) >= 32 {
		copy(h[:], b[len(b)-32:])
	} else {
		copy(h[32-len(b):], b)
	}
	return h
}

// IsZero returns true if all bytes are zero.
func (b *B256) IsZero() bool {
	return *b == B256Zero
}

// Hex returns the hex representation with 0x prefix.
func (b *B256) Hex() string {
	return "0x" + hex.EncodeToString(b[:])
}

// String returns the hex representation.
func (b *B256) String() string {
	return b.Hex()
}

// ToU256 converts a B256 (big-endian) to Uint256.
func (b *B256) ToU256() Uint256 {
	return U256FromBytes32(*b)
}

// B256FromU256 converts a Uint256 to B256 (big-endian).
func B256FromU256(u Uint256) B256 {
	return B256(u.ToBytes32())
}

// Bytes is a byte slice wrapper.
type Bytes []byte

// BytesFrom creates a Bytes from a byte slice (copies).
func BytesFrom(b []byte) Bytes {
	c := make([]byte, len(b))
	copy(c, b)
	return Bytes(c)
}

// Len returns the length.
func (b Bytes) Len() int {
	return len(b)
}

// IsEmpty returns true if the byte slice is empty.
func (b Bytes) IsEmpty() bool {
	return len(b) == 0
}

// Hex returns the hex representation with 0x prefix.
func (b Bytes) Hex() string {
	return "0x" + hex.EncodeToString(b)
}

// String returns the hex representation.
func (b Bytes) String() string {
	return b.Hex()
}

var (
	// KeccakEmpty is the Keccak-256 hash of the empty byte string.
	KeccakEmpty = B256{
		0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c,
		0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03, 0xc0,
		0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b,
		0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85, 0xa4, 0x70,
	}

	// Precompile3 address (used in some gas calculations).
	Precompile3 = Address{0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3}
)

const (
	// BlockHashHistory is the maximum number of block hashes accessible.
	BlockHashHistory uint64 = 256

	// StackLimit is the maximum stack depth.
	StackLimit = 1024

	// CallStackLimit is the maximum call depth.
	CallStackLimit uint64 = 1024

	// OneEther in wei.
	OneEther uint128Val = 1_000_000_000_000_000_000

	// OneGwei in wei.
	OneGwei uint128Val = 1_000_000_000

	// ShortAddressCap is used for short address optimization.
	ShortAddressCap = 300
)

// uint128Val is a type alias for large constants that fit in uint128 but not uint64.
// In Go we use a plain uint64 since these constants are under 2^64.
type uint128Val = uint64

// CreateAddress computes the address for a CREATE operation.
// address = keccak256(RLP([sender, nonce]))[12:]
func CreateAddress(sender Address, nonce uint64) Address {
	// RLP encoding of [sender (20 bytes), nonce]:
	// - sender is 20 bytes, so RLP = 0x94 || sender (short string, len=20)
	// - nonce RLP: 0 = 0x80, 1-0x7F = single byte, else length-prefixed
	nonceRlp := rlpEncodeUint64(nonce)
	listLen := 1 + 20 + len(nonceRlp) // 0x94 + sender + nonce_rlp

	var buf []byte
	if listLen <= 55 {
		buf = make([]byte, 0, 1+listLen)
		buf = append(buf, byte(0xC0+listLen))
	} else {
		// For very large list lengths (won't happen with 20+9 max bytes)
		lenBytes := uintBytes(uint64(listLen))
		buf = make([]byte, 0, 1+len(lenBytes)+listLen)
		buf = append(buf, byte(0xF7+len(lenBytes)))
		buf = append(buf, lenBytes...)
	}
	buf = append(buf, 0x94) // RLP prefix for 20-byte string
	buf = append(buf, sender[:]...)
	buf = append(buf, nonceRlp...)

	hash := keccak.Sum256(buf)
	var addr Address
	copy(addr[:], hash[12:])
	return addr
}

// Create2Address computes the address for a CREATE2 operation.
// address = keccak256(0xFF || sender || salt || keccak256(code))[12:]
func Create2Address(sender Address, salt [32]byte, codeHash B256) Address {
	var buf [1 + 20 + 32 + 32]byte
	buf[0] = 0xFF
	copy(buf[1:21], sender[:])
	copy(buf[21:53], salt[:])
	copy(buf[53:85], codeHash[:])

	hash := keccak.Sum256(buf[:])
	var addr Address
	copy(addr[:], hash[12:])
	return addr
}

// Keccak256 computes the keccak256 hash of data.
func Keccak256(data []byte) B256 {
	return B256(keccak.Sum256(data))
}

// rlpEncodeUint64 returns the RLP encoding of a uint64.
func rlpEncodeUint64(v uint64) []byte {
	if v == 0 {
		return []byte{0x80}
	}
	if v <= 0x7F {
		return []byte{byte(v)}
	}
	b := uintBytes(v)
	return append([]byte{byte(0x80 + len(b))}, b...)
}

// uintBytes returns the big-endian byte representation of v with no leading zeros.
func uintBytes(v uint64) []byte {
	var buf [8]byte
	n := 0
	for tmp := v; tmp > 0; tmp >>= 8 {
		n++
	}
	for i := n - 1; i >= 0; i-- {
		buf[i] = byte(v)
		v >>= 8
	}
	return buf[:n]
}

// HexToAddress parses a hex string (with or without 0x prefix) into an Address.
func HexToAddress(s string) (Address, error) {
	if len(s) >= 2 && s[0] == '0' && (s[1] == 'x' || s[1] == 'X') {
		s = s[2:]
	}
	if len(s)%2 != 0 {
		s = "0" + s
	}
	b, err := hex.DecodeString(s)
	if err != nil {
		return AddressZero, fmt.Errorf("invalid hex address: %w", err)
	}
	return AddressFrom(b), nil
}

// HexToB256 parses a hex string (with or without 0x prefix) into a B256.
func HexToB256(s string) (B256, error) {
	if len(s) >= 2 && s[0] == '0' && (s[1] == 'x' || s[1] == 'X') {
		s = s[2:]
	}
	if len(s)%2 != 0 {
		s = "0" + s
	}
	b, err := hex.DecodeString(s)
	if err != nil {
		return B256Zero, fmt.Errorf("invalid hex B256: %w", err)
	}
	return B256From(b), nil
}
