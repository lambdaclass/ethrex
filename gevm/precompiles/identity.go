package precompiles

const (
	identityBase    uint64 = 15
	identityPerWord uint64 = 3
)

// IdentityRun implements the IDENTITY precompile (address 0x04).
// Copies input directly to output.
func IdentityRun(input []byte, gasLimit uint64) PrecompileResult {
	gasUsed := CalcLinearCost(len(input), identityBase, identityPerWord)
	if gasUsed > gasLimit {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}
	out := make([]byte, len(input))
	copy(out, input)
	return PrecompileOk(NewPrecompileOutput(gasUsed, out))
}
