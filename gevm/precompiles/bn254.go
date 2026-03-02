// Implements BN254 (alt_bn128) precompiles: ADD (0x06), MUL (0x07), PAIRING (0x08).
package precompiles

import (
	"math/big"

	"github.com/consensys/gnark-crypto/ecc/bn254"
)

// Gas costs by fork.
const (
	bn254AddByzantiumGas  uint64 = 500
	bn254AddIstanbulGas   uint64 = 150
	bn254MulByzantiumGas  uint64 = 40000
	bn254MulIstanbulGas   uint64 = 6000
	bn254PairBaseByzantium  uint64 = 100000
	bn254PairPerPointByzantium uint64 = 80000
	bn254PairBaseIstanbul   uint64 = 45000
	bn254PairPerPointIstanbul  uint64 = 34000
)

// --- BN254 ADD ---

// Bn254AddByzantiumRun implements BN254 point addition with Byzantium gas pricing.
func Bn254AddByzantiumRun(input []byte, gasLimit uint64) PrecompileResult {
	return bn254AddRun(input, gasLimit, bn254AddByzantiumGas)
}

// Bn254AddIstanbulRun implements BN254 point addition with Istanbul gas pricing.
func Bn254AddIstanbulRun(input []byte, gasLimit uint64) PrecompileResult {
	return bn254AddRun(input, gasLimit, bn254AddIstanbulGas)
}

func bn254AddRun(input []byte, gasLimit uint64, gasCost uint64) PrecompileResult {
	if gasCost > gasLimit {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}

	input = RightPad(input, 128)

	// Parse two G1 points
	p1, ok := decodeG1Point(input[0:64])
	if !ok {
		return PrecompileErr(PrecompileErrorBn254FieldPointNotAMember)
	}
	p2, ok := decodeG1Point(input[64:128])
	if !ok {
		return PrecompileErr(PrecompileErrorBn254FieldPointNotAMember)
	}

	// Add points
	var result bn254.G1Affine
	result.Add(&p1, &p2)

	return PrecompileOk(NewPrecompileOutput(gasCost, encodeG1Point(&result)))
}

// --- BN254 MUL ---

// Bn254MulByzantiumRun implements BN254 scalar multiplication with Byzantium gas pricing.
func Bn254MulByzantiumRun(input []byte, gasLimit uint64) PrecompileResult {
	return bn254MulRun(input, gasLimit, bn254MulByzantiumGas)
}

// Bn254MulIstanbulRun implements BN254 scalar multiplication with Istanbul gas pricing.
func Bn254MulIstanbulRun(input []byte, gasLimit uint64) PrecompileResult {
	return bn254MulRun(input, gasLimit, bn254MulIstanbulGas)
}

func bn254MulRun(input []byte, gasLimit uint64, gasCost uint64) PrecompileResult {
	if gasCost > gasLimit {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}

	input = RightPad(input, 96)

	// Parse G1 point
	p, ok := decodeG1Point(input[0:64])
	if !ok {
		return PrecompileErr(PrecompileErrorBn254FieldPointNotAMember)
	}

	// Parse scalar (32 bytes, big-endian)
	scalar := new(big.Int).SetBytes(input[64:96])

	// Scalar multiplication
	var result bn254.G1Affine
	result.ScalarMultiplication(&p, scalar)

	return PrecompileOk(NewPrecompileOutput(gasCost, encodeG1Point(&result)))
}

// --- BN254 PAIRING ---

// Bn254PairingByzantiumRun implements BN254 pairing check with Byzantium gas pricing.
func Bn254PairingByzantiumRun(input []byte, gasLimit uint64) PrecompileResult {
	return bn254PairingRun(input, gasLimit, bn254PairBaseByzantium, bn254PairPerPointByzantium)
}

// Bn254PairingIstanbulRun implements BN254 pairing check with Istanbul gas pricing.
func Bn254PairingIstanbulRun(input []byte, gasLimit uint64) PrecompileResult {
	return bn254PairingRun(input, gasLimit, bn254PairBaseIstanbul, bn254PairPerPointIstanbul)
}

func bn254PairingRun(input []byte, gasLimit uint64, baseCost, perPairCost uint64) PrecompileResult {
	// Input must be a multiple of 192 bytes (each pair is 192 bytes)
	if len(input)%192 != 0 {
		return PrecompileErr(PrecompileErrorBn254PairLength)
	}

	numPairs := uint64(len(input)) / 192
	gasCost := baseCost + numPairs*perPairCost
	if gasCost > gasLimit {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}

	// If no pairs, pairing check trivially succeeds
	if numPairs == 0 {
		return PrecompileOk(NewPrecompileOutput(gasCost, pairingTrue()))
	}

	// Parse all pairs
	g1Points := make([]bn254.G1Affine, numPairs)
	g2Points := make([]bn254.G2Affine, numPairs)

	for i := uint64(0); i < numPairs; i++ {
		offset := i * 192

		g1, ok := decodeG1Point(input[offset : offset+64])
		if !ok {
			return PrecompileErr(PrecompileErrorBn254FieldPointNotAMember)
		}
		g1Points[i] = g1

		g2, ok := decodeG2Point(input[offset+64 : offset+192])
		if !ok {
			return PrecompileErr(PrecompileErrorBn254FieldPointNotAMember)
		}
		g2Points[i] = g2
	}

	// Check pairing: e(P1,Q1) * e(P2,Q2) * ... == 1
	ok, err := bn254.PairingCheck(g1Points, g2Points)
	if err != nil {
		return PrecompileErr(PrecompileErrorBn254AffineGFailedToCreate)
	}

	if ok {
		return PrecompileOk(NewPrecompileOutput(gasCost, pairingTrue()))
	}
	return PrecompileOk(NewPrecompileOutput(gasCost, pairingFalse()))
}

// --- Encoding/Decoding helpers ---

// decodeG1Point decodes a 64-byte big-endian G1 point (x || y).
func decodeG1Point(data []byte) (bn254.G1Affine, bool) {
	var p bn254.G1Affine

	// Both coordinates zero means the point at infinity
	allZero := true
	for _, b := range data {
		if b != 0 {
			allZero = false
			break
		}
	}
	if allZero {
		p.X.SetZero()
		p.Y.SetZero()
		return p, true
	}

	// Use SetBytesCanonical to reject non-canonical inputs (>= field modulus).
	if err := p.X.SetBytesCanonical(data[0:32]); err != nil {
		return p, false
	}
	if err := p.Y.SetBytesCanonical(data[32:64]); err != nil {
		return p, false
	}

	// Validate that the point is on the curve
	if !p.IsOnCurve() {
		return p, false
	}

	return p, true
}

// encodeG1Point encodes a G1 affine point to 64 bytes (x || y, big-endian).
func encodeG1Point(p *bn254.G1Affine) []byte {
	var out [64]byte
	xBytes := p.X.Bytes()
	yBytes := p.Y.Bytes()
	copy(out[0:32], xBytes[:])
	copy(out[32:64], yBytes[:])
	return out[:]
}

// decodeG2Point decodes a 128-byte big-endian G2 point.
// G2 coordinates are in Fp2 = a + b*u, encoded as (b, a) for each coordinate.
// Input layout: [x_imaginary(32) | x_real(32) | y_imaginary(32) | y_real(32)]
func decodeG2Point(data []byte) (bn254.G2Affine, bool) {
	var p bn254.G2Affine

	// All zero means point at infinity
	allZero := true
	for _, b := range data {
		if b != 0 {
			allZero = false
			break
		}
	}
	if allZero {
		p.X.SetZero()
		p.Y.SetZero()
		return p, true
	}

	// x = x_imaginary * u + x_real (EVM encoding: imaginary first)
	// Use SetBytesCanonical to reject non-canonical inputs (>= field modulus).
	if err := p.X.A1.SetBytesCanonical(data[0:32]); err != nil {
		return p, false
	}
	if err := p.X.A0.SetBytesCanonical(data[32:64]); err != nil {
		return p, false
	}
	if err := p.Y.A1.SetBytesCanonical(data[64:96]); err != nil {
		return p, false
	}
	if err := p.Y.A0.SetBytesCanonical(data[96:128]); err != nil {
		return p, false
	}

	// Validate that the point is on the curve and in the correct subgroup
	if !p.IsOnCurve() {
		return p, false
	}
	// Subgroup check
	if !p.IsInSubGroup() {
		return p, false
	}

	return p, true
}

// pairingTrue returns the 32-byte encoding of pairing check success (1).
func pairingTrue() []byte {
	var out [32]byte
	out[31] = 1
	return out[:]
}

// pairingFalse returns the 32-byte encoding of pairing check failure (0).
func pairingFalse() []byte {
	return make([]byte, 32)
}
