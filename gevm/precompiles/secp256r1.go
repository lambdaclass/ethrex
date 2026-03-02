// RIP-7212 secp256r1 (P-256) ECDSA signature verification precompile.
package precompiles

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"math/big"

	"github.com/Giulio2002/gevm/types"
)

// p256VerifyBaseGas is the gas cost for P256VERIFY (pre-Osaka).
const p256VerifyBaseGas uint64 = 3450

// p256VerifyBaseGasOsaka is the gas cost for P256VERIFY (Osaka+).
const p256VerifyBaseGasOsaka uint64 = 6900

// P256VerifyRun implements the P256VERIFY precompile (address 0x0100).
// Verifies a secp256r1 (P-256) ECDSA signature with gas cost 3450.
//
// Input (exactly 160 bytes):
//
//	[0..32]    signed message hash
//	[32..64]   r (signature)
//	[64..96]   s (signature)
//	[96..128]  public key x
//	[128..160] public key y
//
// Output: B256 with last byte = 0x01 on success, empty bytes on failure.
func P256VerifyRun(input []byte, gasLimit uint64) PrecompileResult {
	return p256VerifyInner(input, gasLimit, p256VerifyBaseGas)
}

// P256VerifyOsakaRun implements the P256VERIFY precompile with Osaka gas cost (6900).
func P256VerifyOsakaRun(input []byte, gasLimit uint64) PrecompileResult {
	return p256VerifyInner(input, gasLimit, p256VerifyBaseGasOsaka)
}

// p256VerifyInner is the core P256VERIFY logic.
func p256VerifyInner(input []byte, gasLimit uint64, gasCost uint64) PrecompileResult {
	if gasCost > gasLimit {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}
	if p256VerifyImpl(input) {
		var result types.B256
		result[31] = 1
		return PrecompileOk(NewPrecompileOutput(gasCost, result[:]))
	}
	return PrecompileOk(NewPrecompileOutput(gasCost, nil))
}

// p256VerifyImpl validates the secp256r1 ECDSA signature.
// Returns true if the signature is valid, false otherwise.
func p256VerifyImpl(input []byte) bool {
	if len(input) != 160 {
		return false
	}

	// Extract components
	msgHash := input[0:32]
	rBytes := input[32:64]
	sBytes := input[64:96]
	pkxBytes := input[96:128]
	pkyBytes := input[128:160]

	// Parse r, s as big-endian integers
	r := new(big.Int).SetBytes(rBytes)
	s := new(big.Int).SetBytes(sBytes)

	// r and s must be > 0 and < curve order
	curve := elliptic.P256()
	n := curve.Params().N
	if r.Sign() <= 0 || r.Cmp(n) >= 0 {
		return false
	}
	if s.Sign() <= 0 || s.Cmp(n) >= 0 {
		return false
	}

	// Parse public key coordinates
	x := new(big.Int).SetBytes(pkxBytes)
	y := new(big.Int).SetBytes(pkyBytes)

	// Verify the point is on the curve
	if !curve.IsOnCurve(x, y) {
		return false
	}

	// Create ECDSA public key
	pubKey := &ecdsa.PublicKey{
		Curve: curve,
		X:     x,
		Y:     y,
	}

	// Verify the signature using prehash (message is already hashed)
	return ecdsa.Verify(pubKey, msgHash, r, s)
}
