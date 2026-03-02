package precompiles

const ecrecoverGas uint64 = 3000

// EcRecoverRun implements the ECRECOVER precompile (address 0x01).
// Recovers the signer address from an ECDSA signature.
//
// Input (128 bytes, right-padded):
//
//	[0..32]   message hash
//	[32..64]  v (recovery id: 27 or 28, as 32-byte big-endian)
//	[64..128] signature (r || s, each 32 bytes)
//
// Output: 32 bytes (address left-padded with zeros), or empty on failure.
func EcRecoverRun(input []byte, gasLimit uint64) PrecompileResult {
	if ecrecoverGas > gasLimit {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}

	// Right-pad input to 128 bytes
	input = RightPad(input, 128)

	// v must be a 32-byte big-endian integer equal to 27 or 28.
	// Bytes [32..63] must all be zero, and byte [63] must be 27 or 28.
	for i := 32; i < 63; i++ {
		if input[i] != 0 {
			return PrecompileOk(NewPrecompileOutput(ecrecoverGas, nil))
		}
	}
	if input[63] != 27 && input[63] != 28 {
		return PrecompileOk(NewPrecompileOutput(ecrecoverGas, nil))
	}

	recid := input[63] - 27 // 0 or 1

	// Extract message hash and signature
	var msgHash [32]byte
	copy(msgHash[:], input[0:32])

	var sig [64]byte
	copy(sig[:], input[64:128])

	// Recover the public key
	addr, ok := Ecrecover(sig, recid, msgHash)
	if !ok {
		return PrecompileOk(NewPrecompileOutput(ecrecoverGas, nil))
	}

	return PrecompileOk(NewPrecompileOutput(ecrecoverGas, addr[:]))
}
