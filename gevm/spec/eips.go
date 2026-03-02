package spec

import "math/bits"

// EIP-170: Contract code size limit
const MaxCodeSize = 0x6000 // 24576 bytes

// EIP-3860: Limit and meter initcode (Shanghai)
const MaxInitcodeSize = 2 * MaxCodeSize // 49152 bytes

// EIP-4844: Shard Blob Transactions (Cancun)
const (
	VersionedHashVersionKZG = 0x01
	GasPerBlob              = 1 << 17 // 131072
	MinBlobGasPrice  uint64 = 1

	TargetBlobNumberPerBlockCancun     = 3
	MaxBlobNumberPerBlockCancun        = 6
	MaxBlobGasPerBlockCancun           = MaxBlobNumberPerBlockCancun * GasPerBlob
	TargetBlobGasPerBlockCancun        = TargetBlobNumberPerBlockCancun * GasPerBlob
	BlobBaseFeeUpdateFractionCancun    = 3_338_477

	TargetBlobNumberPerBlockPrague     = 6
	MaxBlobNumberPerBlockPrague        = 9
	MaxBlobGasPerBlockPrague           = MaxBlobNumberPerBlockPrague * GasPerBlob
	TargetBlobGasPerBlockPrague        = TargetBlobNumberPerBlockPrague * GasPerBlob
	BlobBaseFeeUpdateFractionPrague    = 5_007_716
)

// CalcBlobGasPrice calculates the blob gas price from excess blob gas.
func CalcBlobGasPrice(excessBlobGas uint64, forkID ForkID) uint64 {
	var fraction uint64
	if forkID.IsEnabledIn(Prague) {
		fraction = BlobBaseFeeUpdateFractionPrague
	} else {
		fraction = BlobBaseFeeUpdateFractionCancun
	}
	return FakeExponential(MinBlobGasPrice, excessBlobGas, fraction)
}

// FakeExponential approximates factor * e ** (numerator / denominator)
// using Taylor expansion.
// Uses math/bits for 128-bit intermediate calculations.
func FakeExponential(factor, numerator, denominator uint64) uint64 {
	if denominator == 0 {
		panic("fake_exponential: denominator is zero")
	}
	// Use [2]uint64 for 128-bit arithmetic: [0]=low, [1]=high
	f := u128from(factor)
	n := u128from(numerator)
	d := u128from(denominator)

	i := u128from(1)
	output := u128from(0)
	numeratorAccum := u128mul(f, d) // factor * denominator
	for u128gt(numeratorAccum, u128from(0)) {
		output = u128add(output, numeratorAccum)
		// numeratorAccum = numeratorAccum * numerator / (denominator * i)
		num := u128mul(numeratorAccum, n)
		den := u128mul(d, i)
		numeratorAccum = u128div(num, den)
		i = u128add(i, u128from(1))
	}
	result := u128div(output, d)
	// Saturate to uint64
	if result[1] > 0 {
		return ^uint64(0)
	}
	return result[0]
}

// 128-bit unsigned integer arithmetic using [2]uint64 (little-endian: [0]=low, [1]=high)
type u128 [2]uint64

func u128from(v uint64) u128 { return u128{v, 0} }

func u128gt(a, b u128) bool {
	if a[1] != b[1] {
		return a[1] > b[1]
	}
	return a[0] > b[0]
}

func u128add(a, b u128) u128 {
	lo := a[0] + b[0]
	carry := uint64(0)
	if lo < a[0] {
		carry = 1
	}
	return u128{lo, a[1] + b[1] + carry}
}

func u128mul(a, b u128) u128 {
	// (aH*2^64 + aL) * (bH*2^64 + bL)
	// = aH*bH*2^128 + (aH*bL + aL*bH)*2^64 + aL*bL
	// We only keep 128 bits (ignore aH*bH overflow).
	hi, lo := bits.Mul64(a[0], b[0])
	hi += a[1]*b[0] + a[0]*b[1]
	return u128{lo, hi}
}

func u128div(a, b u128) u128 {
	// Simple long division for u128 / u128
	if b[1] == 0 && b[0] == 0 {
		panic("u128div: division by zero")
	}
	// Fast path: both fit in uint64
	if a[1] == 0 && b[1] == 0 {
		return u128{a[0] / b[0], 0}
	}
	// Fast path: divisor is single uint64
	if b[1] == 0 {
		return u128divU64(a, b[0])
	}
	// General case: binary long division
	return u128divGeneral(a, b)
}

func u128divU64(a u128, b uint64) u128 {
	var q u128
	q[1], a[1] = bits.Div64(0, a[1], b)
	q[0], _ = bits.Div64(a[1], a[0], b)
	return q
}

func u128divGeneral(a, b u128) u128 {
	if u128gt(b, a) {
		return u128{}
	}
	// Count leading zeros to normalize
	var q u128
	for !u128gt(b, a) {
		// Find shift amount
		shift := u128clz(b) - u128clz(a)
		bs := u128shl(b, uint(shift))
		if u128gt(bs, a) {
			shift--
			bs = u128shl(b, uint(shift))
		}
		a = u128sub(a, bs)
		if shift >= 64 {
			q[1] |= 1 << (uint(shift) - 64)
		} else {
			q[0] |= 1 << uint(shift)
		}
	}
	return q
}

func u128sub(a, b u128) u128 {
	lo := a[0] - b[0]
	borrow := uint64(0)
	if a[0] < b[0] {
		borrow = 1
	}
	return u128{lo, a[1] - b[1] - borrow}
}

func u128shl(a u128, n uint) u128 {
	if n >= 128 {
		return u128{}
	}
	if n >= 64 {
		return u128{0, a[0] << (n - 64)}
	}
	if n == 0 {
		return a
	}
	return u128{a[0] << n, a[1]<<n | a[0]>>(64-n)}
}

func u128clz(a u128) int {
	if a[1] != 0 {
		return bits.LeadingZeros64(a[1])
	}
	return 64 + bits.LeadingZeros64(a[0])
}

// EIP-7702: Set EOA account code (Prague)
const (
	EIP7702PerAuthBaseCost       uint64 = 12500
	EIP7702PerEmptyAccountCost   uint64 = 25000
)

// EIP-7823: Set upper bounds for MODEXP (Prague)
const EIP7823InputSizeLimit = 1024 // bytes (8192 bits)

// EIP-7825: Transaction gas limit cap (Prague)
const TxGasLimitCap uint64 = 16_777_216 // 2^24

// EIP-7907: Code size meter and limit (Prague)
const (
	EIP7907MaxCodeSize     = 0xC000  // 49152 bytes
	EIP7907MaxInitcodeSize = 0x12000 // 73728 bytes
)
