//go:build !arm64 && !amd64

package types

import "math/bits"

func AddTo(result, a, b *Uint256) {
	var c uint64
	result[0], c = bits.Add64(a[0], b[0], 0)
	result[1], c = bits.Add64(a[1], b[1], c)
	result[2], c = bits.Add64(a[2], b[2], c)
	result[3], _ = bits.Add64(a[3], b[3], c)
}

func SubTo(result, a, b *Uint256) {
	var c uint64
	result[0], c = bits.Sub64(a[0], b[0], 0)
	result[1], c = bits.Sub64(a[1], b[1], c)
	result[2], c = bits.Sub64(a[2], b[2], c)
	result[3], _ = bits.Sub64(a[3], b[3], c)
}

func AndTo(result, a, b *Uint256) {
	result[0] = a[0] & b[0]
	result[1] = a[1] & b[1]
	result[2] = a[2] & b[2]
	result[3] = a[3] & b[3]
}

func OrTo(result, a, b *Uint256) {
	result[0] = a[0] | b[0]
	result[1] = a[1] | b[1]
	result[2] = a[2] | b[2]
	result[3] = a[3] | b[3]
}

func XorTo(result, a, b *Uint256) {
	result[0] = a[0] ^ b[0]
	result[1] = a[1] ^ b[1]
	result[2] = a[2] ^ b[2]
	result[3] = a[3] ^ b[3]
}

func NotTo(result, a *Uint256) {
	result[0] = ^a[0]
	result[1] = ^a[1]
	result[2] = ^a[2]
	result[3] = ^a[3]
}

func EqPtr(a, b *Uint256) bool {
	return a[0] == b[0] && a[1] == b[1] && a[2] == b[2] && a[3] == b[3]
}

func LtPtr(a, b *Uint256) bool {
	if a[3] != b[3] {
		return a[3] < b[3]
	}
	if a[2] != b[2] {
		return a[2] < b[2]
	}
	if a[1] != b[1] {
		return a[1] < b[1]
	}
	return a[0] < b[0]
}

func GtPtr(a, b *Uint256) bool {
	if a[3] != b[3] {
		return a[3] > b[3]
	}
	if a[2] != b[2] {
		return a[2] > b[2]
	}
	if a[1] != b[1] {
		return a[1] > b[1]
	}
	return a[0] > b[0]
}

func IsZeroPtr(a *Uint256) bool {
	return a[0] == 0 && a[1] == 0 && a[2] == 0 && a[3] == 0
}
