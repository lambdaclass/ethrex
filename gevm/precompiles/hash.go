package precompiles

import (
	"crypto/sha256"

	"golang.org/x/crypto/ripemd160"
)

const (
	sha256Base    uint64 = 60
	sha256PerWord uint64 = 12

	ripemd160Base    uint64 = 600
	ripemd160PerWord uint64 = 120
)

// Sha256Run implements the SHA-256 precompile (address 0x02).
func Sha256Run(input []byte, gasLimit uint64) PrecompileResult {
	gasUsed := CalcLinearCost(len(input), sha256Base, sha256PerWord)
	if gasUsed > gasLimit {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}
	hash := sha256.Sum256(input)
	return PrecompileOk(NewPrecompileOutput(gasUsed, hash[:]))
}

// Ripemd160Run implements the RIPEMD-160 precompile (address 0x03).
// Output is left-padded to 32 bytes (EVM ABI convention).
func Ripemd160Run(input []byte, gasLimit uint64) PrecompileResult {
	gasUsed := CalcLinearCost(len(input), ripemd160Base, ripemd160PerWord)
	if gasUsed > gasLimit {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}
	h := ripemd160.New()
	h.Write(input)
	hash := h.Sum(nil) // 20 bytes

	// Left-pad to 32 bytes (12 zero bytes + 20 hash bytes)
	var out [32]byte
	copy(out[12:], hash)
	return PrecompileOk(NewPrecompileOutput(gasUsed, out[:]))
}
