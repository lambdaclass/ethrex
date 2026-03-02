// Hex-encoded JSON types for Ethereum test fixture deserialization.
// These types parse hex strings (e.g. "0x01", "0xff") from test JSON into
// their corresponding Go primitives types.
package spec

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"strings"

	"github.com/Giulio2002/gevm/types"
)

// hexToBytes decodes a hex string (with or without 0x prefix) to bytes.
func hexToBytes(s string) ([]byte, error) {
	s = strings.TrimPrefix(s, "0x")
	s = strings.TrimPrefix(s, "0X")
	if len(s)%2 != 0 {
		s = "0" + s
	}
	return hex.DecodeString(s)
}

// --- HexU256 ---

// HexU256 wraps types.Uint256 with JSON hex string deserialization.
type HexU256 struct {
	V types.Uint256
}

func (h *HexU256) UnmarshalJSON(data []byte) error {
	var s string
	if err := json.Unmarshal(data, &s); err != nil {
		return err
	}
	b, err := hexToBytes(s)
	if err != nil {
		return fmt.Errorf("HexU256: %w", err)
	}
	h.V = types.U256FromBytes(b)
	return nil
}

func (h HexU256) MarshalJSON() ([]byte, error) {
	return json.Marshal(h.V.Hex())
}

// --- HexU64 ---

// HexU64 wraps uint64 with JSON hex string deserialization.
type HexU64 struct {
	V uint64
}

func (h *HexU64) UnmarshalJSON(data []byte) error {
	var s string
	if err := json.Unmarshal(data, &s); err != nil {
		return err
	}
	b, err := hexToBytes(s)
	if err != nil {
		return fmt.Errorf("HexU64: %w", err)
	}
	if len(b) > 8 {
		return fmt.Errorf("HexU64: value too large (%d bytes)", len(b))
	}
	var v uint64
	for _, bb := range b {
		v = (v << 8) | uint64(bb)
	}
	h.V = v
	return nil
}

// --- HexAddr ---

// HexAddr wraps types.Address with JSON hex string deserialization.
// Implements encoding.TextUnmarshaler for use as JSON map keys.
type HexAddr struct {
	V types.Address
}

func (h *HexAddr) UnmarshalText(text []byte) error {
	addr, err := types.HexToAddress(string(text))
	if err != nil {
		return err
	}
	h.V = addr
	return nil
}

func (h *HexAddr) UnmarshalJSON(data []byte) error {
	var s string
	if err := json.Unmarshal(data, &s); err != nil {
		return err
	}
	return h.UnmarshalText([]byte(s))
}

func (h HexAddr) MarshalText() ([]byte, error) {
	return []byte(h.V.Hex()), nil
}

// --- HexB256 ---

// HexB256 wraps types.B256 with JSON hex string deserialization.
// Implements encoding.TextUnmarshaler for use as JSON map keys.
type HexB256 struct {
	V types.B256
}

func (h *HexB256) UnmarshalText(text []byte) error {
	b256, err := types.HexToB256(string(text))
	if err != nil {
		return err
	}
	h.V = b256
	return nil
}

func (h *HexB256) UnmarshalJSON(data []byte) error {
	var s string
	if err := json.Unmarshal(data, &s); err != nil {
		return err
	}
	return h.UnmarshalText([]byte(s))
}

func (h HexB256) MarshalText() ([]byte, error) {
	return []byte(h.V.Hex()), nil
}

// --- HexBytes ---

// HexBytes wraps []byte with JSON hex string deserialization.
type HexBytes struct {
	V []byte
}

func (h *HexBytes) UnmarshalJSON(data []byte) error {
	var s string
	if err := json.Unmarshal(data, &s); err != nil {
		return err
	}
	b, err := hexToBytes(s)
	if err != nil {
		return fmt.Errorf("HexBytes: %w", err)
	}
	h.V = b
	return nil
}
