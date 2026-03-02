// Implements the MODEXP precompile (address 0x05).
// Gas models: Byzantium (EIP-198), Berlin (EIP-2565), Osaka (EIP-7883).
// Ported from go-ethereum core/vm/contracts.go.
package precompiles

import (
	"math"
	"math/big"
	"math/bits"
)

// modexpModel selects the gas pricing model.
type modexpModel int

const (
	modexpByzantium modexpModel = iota
	modexpBerlin
	modexpOsaka
)

// ModExpByzantiumRun implements MODEXP with Byzantium gas pricing (EIP-198).
func ModExpByzantiumRun(input []byte, gasLimit uint64) PrecompileResult {
	return modexpRun(input, gasLimit, modexpByzantium)
}

// ModExpBerlinRun implements MODEXP with Berlin gas pricing (EIP-2565).
func ModExpBerlinRun(input []byte, gasLimit uint64) PrecompileResult {
	return modexpRun(input, gasLimit, modexpBerlin)
}

// ModExpOsakaRun implements MODEXP with Osaka gas pricing (EIP-7883).
func ModExpOsakaRun(input []byte, gasLimit uint64) PrecompileResult {
	return modexpRun(input, gasLimit, modexpOsaka)
}

func modexpRun(input []byte, gasLimit uint64, model modexpModel) PrecompileResult {
	// Right-pad input to at least 96 bytes for the length headers.
	if len(input) < 96 {
		input = RightPad(input, 96)
	}

	// Parse lengths (32-byte big-endian each).
	baseLen := parseModexpLen(input[0:32])
	expLen := parseModexpLen(input[32:64])
	modLen := parseModexpLen(input[64:96])

	// EIP-7823 (Osaka): reject inputs where any length exceeds 1024 bytes.
	if model == modexpOsaka && (baseLen > 1024 || expLen > 1024 || modLen > 1024) {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}

	// Gas calculation.
	gasUsed := modexpGas(baseLen, expLen, modLen, input, model)
	if gasUsed > gasLimit {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}

	// If baseLen and modLen are both 0, result is empty.
	if baseLen == 0 && modLen == 0 {
		return PrecompileOk(NewPrecompileOutput(gasUsed, nil))
	}
	// If modLen is 0, result is always empty.
	if modLen == 0 {
		return PrecompileOk(NewPrecompileOutput(gasUsed, nil))
	}

	// Strip the 96-byte header; remaining is [base | exp | mod].
	data := input[96:]

	base := new(big.Int).SetBytes(getData(data, 0, baseLen))
	exp := new(big.Int).SetBytes(getData(data, baseLen, expLen))
	mod := new(big.Int).SetBytes(getData(data, baseLen+expLen, modLen))

	// If mod is 0, result is all zeros.
	if mod.BitLen() == 0 {
		return PrecompileOk(NewPrecompileOutput(gasUsed, make([]byte, modLen)))
	}

	var v []byte
	if base.BitLen() == 1 {
		// base is 0 or 1 — Mod is cheaper than Exp.
		v = base.Mod(base, mod).Bytes()
	} else {
		v = base.Exp(base, exp, mod).Bytes()
	}

	return PrecompileOk(NewPrecompileOutput(gasUsed, leftPadBytes(v, int(modLen))))
}

// getData returns input[start:start+size], zero-padded on the right if out of bounds.
// Ported from go-ethereum common/bytes.go getData pattern.
func getData(data []byte, start uint64, size uint64) []byte {
	length := uint64(len(data))
	if start > length {
		start = length
	}
	end := start + size
	if end > length {
		end = length
	}
	return rightPadBytes(data[start:end], int(size))
}

func rightPadBytes(b []byte, l int) []byte {
	if l <= len(b) {
		return b
	}
	padded := make([]byte, l)
	copy(padded, b)
	return padded
}

func leftPadBytes(b []byte, l int) []byte {
	if l <= len(b) {
		return b
	}
	padded := make([]byte, l)
	copy(padded[l-len(b):], b)
	return padded
}

// parseModexpLen parses a 32-byte big-endian length field.
// Returns math.MaxUint64 if the value overflows uint64.
func parseModexpLen(b []byte) uint64 {
	for i := 0; i < 24; i++ {
		if b[i] != 0 {
			return math.MaxUint64
		}
	}
	var v uint64
	for i := 24; i < 32; i++ {
		v = v<<8 | uint64(b[i])
	}
	return v
}

// modexpGas calculates gas cost for modexp.
func modexpGas(baseLen, expLen, modLen uint64, input []byte, model modexpModel) uint64 {
	maxLen := baseLen
	if modLen > maxLen {
		maxLen = modLen
	}

	// Extract the first 32 bytes of the exponent for bit-length calculation.
	var expHead [32]byte
	expStart := 96 + baseLen
	if expStart < uint64(len(input)) {
		remaining := input[expStart:]
		if expLen > 32 {
			copy(expHead[:], remaining)
		} else if expLen > 0 {
			copy(expHead[32-expLen:], remaining)
		}
	}
	expHeadInt := new(big.Int).SetBytes(expHead[:])

	switch model {
	case modexpOsaka:
		return osakaModexpGas(baseLen, expLen, modLen, maxLen, expHeadInt)
	case modexpBerlin:
		return berlinModexpGas(baseLen, expLen, modLen, maxLen, expHeadInt)
	default:
		return byzantiumModexpGas(baseLen, expLen, modLen, maxLen, expHeadInt)
	}
}

// modexpIterationCount computes the iteration count for gas purposes.
// Always returns at least 1. Uses overflow-safe arithmetic.
// multiplier is 8 for both Byzantium and Berlin.
func modexpIterationCount(expLen uint64, expHead *big.Int, multiplier uint64) uint64 {
	var iterationCount uint64

	if expLen > 32 {
		carry, count := bits.Mul64(expLen-32, multiplier)
		if carry > 0 {
			return math.MaxUint64
		}
		iterationCount = count
	}

	if bitLen := expHead.BitLen(); bitLen > 0 {
		count, carry := bits.Add64(iterationCount, uint64(bitLen-1), 0)
		if carry > 0 {
			return math.MaxUint64
		}
		iterationCount = count
	}

	if iterationCount < 1 {
		iterationCount = 1
	}
	return iterationCount
}

// byzantiumModexpGas implements EIP-198 gas formula.
// GQUADDIVISOR = 20, multiplier = 8.
func byzantiumModexpGas(baseLen, expLen, modLen, maxLen uint64, expHead *big.Int) uint64 {
	multComplexity := byzantiumMultComplexity(maxLen)
	if multComplexity == math.MaxUint64 {
		return math.MaxUint64
	}
	iterationCount := modexpIterationCount(expLen, expHead, 8)

	carry, gas := bits.Mul64(iterationCount, multComplexity)
	gas /= 20
	if carry != 0 {
		return math.MaxUint64
	}
	return gas
}

// berlinModexpGas implements EIP-2565 gas formula.
// divisor = 3, multiplier = 8, minGas = 200.
func berlinModexpGas(baseLen, expLen, modLen, maxLen uint64, expHead *big.Int) uint64 {
	multComplexity := berlinMultComplexity(maxLen)
	if multComplexity == math.MaxUint64 {
		return math.MaxUint64
	}
	iterationCount := modexpIterationCount(expLen, expHead, 8)

	carry, gas := bits.Mul64(iterationCount, multComplexity)
	gas /= 3
	if carry != 0 {
		return math.MaxUint64
	}
	if gas < 200 {
		gas = 200
	}
	return gas
}

// byzantiumMultComplexity implements the EIP-198 piecewise complexity formula.
func byzantiumMultComplexity(x uint64) uint64 {
	switch {
	case x <= 64:
		return x * x
	case x <= 1024:
		return x*x/4 + 96*x - 3072
	default:
		carry, xSqr := bits.Mul64(x, x)
		if carry != 0 {
			return math.MaxUint64
		}
		xSqr = xSqr >> 4           // x*x/16
		x480 := x*480 - 199680     // 480*x - 199680
		sum, carry := bits.Add64(xSqr, x480, 0)
		if carry != 0 {
			return math.MaxUint64
		}
		return sum
	}
}

// berlinMultComplexity implements the EIP-2565 complexity formula: ceil(x/8)^2.
func berlinMultComplexity(x uint64) uint64 {
	x, carry := bits.Add64(x, 7, 0)
	if carry != 0 {
		return math.MaxUint64
	}
	x /= 8
	carry, x = bits.Mul64(x, x)
	if carry != 0 {
		return math.MaxUint64
	}
	return x
}

// osakaModexpGas implements EIP-7883 gas formula.
// No divisor, multiplier = 16, minGas = 500.
func osakaModexpGas(baseLen, expLen, modLen, maxLen uint64, expHead *big.Int) uint64 {
	multComplexity := osakaMultComplexity(maxLen)
	if multComplexity == math.MaxUint64 {
		return math.MaxUint64
	}
	iterationCount := modexpIterationCount(expLen, expHead, 16)

	carry, gas := bits.Mul64(iterationCount, multComplexity)
	if carry != 0 {
		return math.MaxUint64
	}
	if gas < 500 {
		gas = 500
	}
	return gas
}

// osakaMultComplexity implements the EIP-7883 complexity formula.
// maxLen <= 32: 16; maxLen > 32: 2 * ceil(maxLen/8)^2.
func osakaMultComplexity(x uint64) uint64 {
	if x <= 32 {
		return 16
	}
	words, carry := bits.Add64(x, 7, 0)
	if carry != 0 {
		return math.MaxUint64
	}
	words /= 8
	carry, wSqr := bits.Mul64(words, words)
	if carry != 0 {
		return math.MaxUint64
	}
	result, carry := bits.Add64(wSqr, wSqr, 0)
	if carry != 0 {
		return math.MaxUint64
	}
	return result
}
