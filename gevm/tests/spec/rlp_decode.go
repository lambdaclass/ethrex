// Minimal RLP decoder with strict validation for TransactionTests.
// Implements strict canonical RLP decoding rules required by Ethereum.
package spec

import (
	"fmt"

	"github.com/Giulio2002/gevm/types"
)

// RlpKind distinguishes RLP strings from lists.
type RlpKind int

const (
	RlpString RlpKind = iota
	RlpList
)

// RlpItem represents a decoded RLP item (either a string or a list).
type RlpItem struct {
	Kind  RlpKind
	Data  []byte    // raw bytes for strings
	Items []RlpItem // children for lists
}

// AsBytes returns the string data. Panics if item is a list.
func (r *RlpItem) AsBytes() []byte {
	return r.Data
}

// AsUint64 parses the RLP string as a big-endian uint64.
// Rejects leading zeros and values that overflow uint64.
// For Ethereum numeric fields, zero must be encoded as empty bytes (0x80),
// not as [0x00].
func (r *RlpItem) AsUint64() (uint64, error) {
	b := r.Data
	if len(b) == 0 {
		return 0, nil
	}
	if b[0] == 0 {
		return 0, fmt.Errorf("RLP uint64: leading zeros")
	}
	if len(b) > 8 {
		return 0, fmt.Errorf("RLP uint64: overflow (%d bytes)", len(b))
	}
	var v uint64
	for _, bb := range b {
		v = (v << 8) | uint64(bb)
	}
	return v, nil
}

// AsU256 parses the RLP string as a big-endian Uint256.
// Rejects leading zeros. Zero must be encoded as empty bytes (0x80).
func (r *RlpItem) AsU256() (types.Uint256, error) {
	b := r.Data
	if len(b) == 0 {
		return types.U256Zero, nil
	}
	if b[0] == 0 {
		return types.U256Zero, fmt.Errorf("RLP Uint256: leading zeros")
	}
	if len(b) > 32 {
		return types.U256Zero, fmt.Errorf("RLP Uint256: overflow (%d bytes)", len(b))
	}
	return types.U256FromBytes(b), nil
}

// AsList returns the list items. Returns nil if item is a string.
func (r *RlpItem) AsList() []RlpItem {
	return r.Items
}

// RlpDecodeComplete decodes a single RLP item and rejects trailing bytes.
func RlpDecodeComplete(data []byte) (RlpItem, error) {
	item, consumed, err := RlpDecode(data)
	if err != nil {
		return RlpItem{}, err
	}
	if consumed != len(data) {
		return RlpItem{}, fmt.Errorf("RLP: %d trailing bytes after item", len(data)-consumed)
	}
	return item, nil
}

// RlpDecode decodes one RLP item from data. Returns the item and bytes consumed.
func RlpDecode(data []byte) (RlpItem, int, error) {
	if len(data) == 0 {
		return RlpItem{}, 0, fmt.Errorf("RLP: unexpected end of input")
	}

	prefix := data[0]

	switch {
	case prefix <= 0x7f:
		// Single byte
		return RlpItem{Kind: RlpString, Data: data[0:1]}, 1, nil

	case prefix <= 0xb7:
		// Short string (0-55 bytes)
		strLen := int(prefix - 0x80)
		if 1+strLen > len(data) {
			return RlpItem{}, 0, fmt.Errorf("RLP: short string truncated: need %d, have %d", 1+strLen, len(data))
		}
		strData := data[1 : 1+strLen]
		// Canonical check: single byte [0x00-0x7f] must not use 0x81 prefix
		if strLen == 1 && strData[0] < 0x80 {
			return RlpItem{}, 0, fmt.Errorf("RLP: non-canonical single byte encoding")
		}
		return RlpItem{Kind: RlpString, Data: strData}, 1 + strLen, nil

	case prefix <= 0xbf:
		// Long string (>55 bytes)
		lenOfLen := int(prefix - 0xb7)
		if 1+lenOfLen > len(data) {
			return RlpItem{}, 0, fmt.Errorf("RLP: long string length truncated")
		}
		strLen, err := decodeRlpLength(data[1:1+lenOfLen], lenOfLen)
		if err != nil {
			return RlpItem{}, 0, err
		}
		// Must be > 55 bytes (otherwise short form should be used)
		if strLen < 56 {
			return RlpItem{}, 0, fmt.Errorf("RLP: non-canonical long string length %d < 56", strLen)
		}
		totalLen := 1 + lenOfLen + strLen
		if totalLen > len(data) {
			return RlpItem{}, 0, fmt.Errorf("RLP: long string truncated: need %d, have %d", totalLen, len(data))
		}
		return RlpItem{Kind: RlpString, Data: data[1+lenOfLen : totalLen]}, totalLen, nil

	case prefix <= 0xf7:
		// Short list (0-55 bytes of content)
		listLen := int(prefix - 0xc0)
		if 1+listLen > len(data) {
			return RlpItem{}, 0, fmt.Errorf("RLP: short list truncated: need %d, have %d", 1+listLen, len(data))
		}
		items, err := decodeRlpListItems(data[1:1+listLen], listLen)
		if err != nil {
			return RlpItem{}, 0, err
		}
		return RlpItem{Kind: RlpList, Items: items}, 1 + listLen, nil

	default:
		// Long list (>55 bytes of content)
		lenOfLen := int(prefix - 0xf7)
		if 1+lenOfLen > len(data) {
			return RlpItem{}, 0, fmt.Errorf("RLP: long list length truncated")
		}
		listLen, err := decodeRlpLength(data[1:1+lenOfLen], lenOfLen)
		if err != nil {
			return RlpItem{}, 0, err
		}
		// Must be > 55 bytes (otherwise short form should be used)
		if listLen < 56 {
			return RlpItem{}, 0, fmt.Errorf("RLP: non-canonical long list length %d < 56", listLen)
		}
		totalLen := 1 + lenOfLen + listLen
		if totalLen > len(data) {
			return RlpItem{}, 0, fmt.Errorf("RLP: long list truncated: need %d, have %d", totalLen, len(data))
		}
		items, err := decodeRlpListItems(data[1+lenOfLen:totalLen], listLen)
		if err != nil {
			return RlpItem{}, 0, err
		}
		return RlpItem{Kind: RlpList, Items: items}, totalLen, nil
	}
}

// decodeRlpLength decodes a big-endian length from lenOfLen bytes.
// Validates no leading zeros and no empty length.
func decodeRlpLength(data []byte, lenOfLen int) (int, error) {
	if lenOfLen == 0 {
		return 0, fmt.Errorf("RLP: empty length-of-length")
	}
	if data[0] == 0 {
		return 0, fmt.Errorf("RLP: leading zeros in length")
	}
	if lenOfLen > 8 {
		return 0, fmt.Errorf("RLP: length-of-length too large (%d bytes)", lenOfLen)
	}
	var n uint64
	for i := 0; i < lenOfLen; i++ {
		n = (n << 8) | uint64(data[i])
	}
	if n > uint64(^uint(0)>>1) {
		return 0, fmt.Errorf("RLP: length overflow")
	}
	return int(n), nil
}

// decodeRlpListItems decodes all items within an RLP list payload.
func decodeRlpListItems(data []byte, dataLen int) ([]RlpItem, error) {
	var items []RlpItem
	offset := 0
	for offset < dataLen {
		item, consumed, err := RlpDecode(data[offset:])
		if err != nil {
			return nil, fmt.Errorf("RLP list item at offset %d: %w", offset, err)
		}
		items = append(items, item)
		offset += consumed
	}
	if offset != dataLen {
		return nil, fmt.Errorf("RLP: list items consumed %d bytes but payload is %d", offset, dataLen)
	}
	return items, nil
}
