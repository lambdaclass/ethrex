// Implements BLS12-381 precompiles (EIP-2537) for Prague fork.
// Addresses 0x0B-0x11.
package precompiles

import (
	"math/big"

	bls12381 "github.com/consensys/gnark-crypto/ecc/bls12-381"
	"github.com/consensys/gnark-crypto/ecc/bls12-381/fp"
	"github.com/consensys/gnark-crypto/ecc/bls12-381/fr"

	"github.com/consensys/gnark-crypto/ecc"
)

// --- BLS12-381 Constants ---

const (
	blsFpLength      = 48  // Raw Fp element size
	blsPaddedFpLen   = 64  // EVM-padded Fp element size
	blsFpPadBy       = 16  // Number of padding bytes
	blsPaddedG1Len   = 128 // 2 * blsPaddedFpLen
	blsPaddedG2Len   = 256 // 4 * blsPaddedFpLen
	blsScalarLen     = 32  // Fr element size
	blsG1AddInputLen = 256 // 2 * blsPaddedG1Len
	blsG1MsmInputLen = 160 // blsPaddedG1Len + blsScalarLen
	blsG2AddInputLen = 512 // 2 * blsPaddedG2Len
	blsG2MsmInputLen = 288 // blsPaddedG2Len + blsScalarLen
	blsPairingInputLen = 384 // blsPaddedG1Len + blsPaddedG2Len
	blsPaddedFp2Len  = 128 // 2 * blsPaddedFpLen

	// Gas constants
	blsG1AddGas       uint64 = 375
	blsG1MsmBaseGas   uint64 = 12000
	blsG2AddGas       uint64 = 600
	blsG2MsmBaseGas   uint64 = 22500
	blsPairingBaseGas uint64 = 37700
	blsPairingPerPair uint64 = 32600
	blsMapFpToG1Gas   uint64 = 5500
	blsMapFp2ToG2Gas  uint64 = 23800
	blsMsmMultiplier  uint64 = 1000
)

// BLS12-381 error types
const (
	PrecompileErrorBls12381InputLength PrecompileError = iota + 100
	PrecompileErrorBls12381FpPaddingInvalid
	PrecompileErrorBls12381FpNotCanonical
	PrecompileErrorBls12381G1NotOnCurve
	PrecompileErrorBls12381G1NotInSubgroup
	PrecompileErrorBls12381G2NotOnCurve
	PrecompileErrorBls12381G2NotInSubgroup
	PrecompileErrorBls12381PairingError
)

// MSM discount tables

var discountTableG1MSM = [128]uint16{
	1000, 949, 848, 797, 764, 750, 738, 728, 719, 712, 705, 698, 692, 687, 682, 677, 673, 669, 665,
	661, 658, 654, 651, 648, 645, 642, 640, 637, 635, 632, 630, 627, 625, 623, 621, 619, 617, 615,
	613, 611, 609, 608, 606, 604, 603, 601, 599, 598, 596, 595, 593, 592, 591, 589, 588, 586, 585,
	584, 582, 581, 580, 579, 577, 576, 575, 574, 573, 572, 570, 569, 568, 567, 566, 565, 564, 563,
	562, 561, 560, 559, 558, 557, 556, 555, 554, 553, 552, 551, 550, 549, 548, 547, 547, 546, 545,
	544, 543, 542, 541, 540, 540, 539, 538, 537, 536, 536, 535, 534, 533, 532, 532, 531, 530, 529,
	528, 528, 527, 526, 525, 525, 524, 523, 522, 522, 521, 520, 520, 519,
}

var discountTableG2MSM = [128]uint16{
	1000, 1000, 923, 884, 855, 832, 812, 796, 782, 770, 759, 749, 740, 732, 724, 717, 711, 704,
	699, 693, 688, 683, 679, 674, 670, 666, 663, 659, 655, 652, 649, 646, 643, 640, 637, 634, 632,
	629, 627, 624, 622, 620, 618, 615, 613, 611, 609, 607, 606, 604, 602, 600, 598, 597, 595, 593,
	592, 590, 589, 587, 586, 584, 583, 582, 580, 579, 578, 576, 575, 574, 573, 571, 570, 569, 568,
	567, 566, 565, 563, 562, 561, 560, 559, 558, 557, 556, 555, 554, 553, 552, 552, 551, 550, 549,
	548, 547, 546, 545, 545, 544, 543, 542, 541, 541, 540, 539, 538, 537, 537, 536, 535, 535, 534,
	533, 532, 532, 531, 530, 530, 529, 528, 528, 527, 526, 526, 525, 524, 524,
}

// --- Utility functions ---

// removeFpPadding removes the 16-byte zero padding from a 64-byte padded Fp element.
// Returns the 48-byte unpadded Fp, or error if padding is invalid.
func removeFpPadding(input []byte) ([]byte, PrecompileError, bool) {
	if len(input) != blsPaddedFpLen {
		return nil, PrecompileErrorBls12381InputLength, true
	}
	// Check that first 16 bytes are zero
	for i := 0; i < blsFpPadBy; i++ {
		if input[i] != 0 {
			return nil, PrecompileErrorBls12381FpPaddingInvalid, true
		}
	}
	return input[blsFpPadBy:], 0, false
}

// decodeFp decodes a 64-byte padded Fp element into a gnark-crypto fp.Element.
// Validates padding and canonical representation.
func decodeFp(input []byte) (fp.Element, PrecompileError, bool) {
	var elem fp.Element
	raw, err, isErr := removeFpPadding(input)
	if isErr {
		return elem, err, true
	}
	if e := elem.SetBytesCanonical(raw); e != nil {
		return elem, PrecompileErrorBls12381FpNotCanonical, true
	}
	return elem, 0, false
}

// decodeG1 decodes a 128-byte padded G1 point with on-curve + subgroup check.
// Use for operations that require subgroup membership (MSM, pairing).
func decodeG1(input []byte) (bls12381.G1Affine, PrecompileError, bool) {
	p, err, isErr := decodeG1NoSubgroup(input)
	if isErr {
		return p, err, true
	}
	if !p.IsInfinity() && !p.IsInSubGroup() {
		return p, PrecompileErrorBls12381G1NotInSubgroup, true
	}
	return p, 0, false
}

// decodeG1NoSubgroup decodes a G1 point with on-curve check only (no subgroup).
// Per EIP-2537, G1ADD does not require subgroup checks.
func decodeG1NoSubgroup(input []byte) (bls12381.G1Affine, PrecompileError, bool) {
	var p bls12381.G1Affine
	if len(input) != blsPaddedG1Len {
		return p, PrecompileErrorBls12381InputLength, true
	}

	x, err, isErr := decodeFp(input[:blsPaddedFpLen])
	if isErr {
		return p, err, true
	}
	y, err, isErr := decodeFp(input[blsPaddedFpLen:])
	if isErr {
		return p, err, true
	}

	// Check for point at infinity (both coords zero)
	if x.IsZero() && y.IsZero() {
		p.SetInfinity()
		return p, 0, false
	}

	p.X = x
	p.Y = y

	if !p.IsOnCurve() {
		return p, PrecompileErrorBls12381G1NotOnCurve, true
	}
	return p, 0, false
}

// decodeG2 decodes a G2 point with on-curve + subgroup check.
// Use for operations that require subgroup membership (MSM, pairing).
func decodeG2(input []byte) (bls12381.G2Affine, PrecompileError, bool) {
	p, err, isErr := decodeG2NoSubgroup(input)
	if isErr {
		return p, err, true
	}
	if !p.IsInfinity() && !p.IsInSubGroup() {
		return p, PrecompileErrorBls12381G2NotInSubgroup, true
	}
	return p, 0, false
}

// decodeG2NoSubgroup decodes a G2 point with on-curve check only (no subgroup).
// Per EIP-2537, G2ADD does not require subgroup checks.
func decodeG2NoSubgroup(input []byte) (bls12381.G2Affine, PrecompileError, bool) {
	var p bls12381.G2Affine
	if len(input) != blsPaddedG2Len {
		return p, PrecompileErrorBls12381InputLength, true
	}

	x0, err, isErr := decodeFp(input[0*blsPaddedFpLen : 1*blsPaddedFpLen])
	if isErr {
		return p, err, true
	}
	x1, err, isErr := decodeFp(input[1*blsPaddedFpLen : 2*blsPaddedFpLen])
	if isErr {
		return p, err, true
	}
	y0, err, isErr := decodeFp(input[2*blsPaddedFpLen : 3*blsPaddedFpLen])
	if isErr {
		return p, err, true
	}
	y1, err, isErr := decodeFp(input[3*blsPaddedFpLen : 4*blsPaddedFpLen])
	if isErr {
		return p, err, true
	}

	// Check for point at infinity (all coords zero)
	if x0.IsZero() && x1.IsZero() && y0.IsZero() && y1.IsZero() {
		p.SetInfinity()
		return p, 0, false
	}

	p.X = bls12381.E2{A0: x0, A1: x1}
	p.Y = bls12381.E2{A0: y0, A1: y1}

	if !p.IsOnCurve() {
		return p, PrecompileErrorBls12381G2NotOnCurve, true
	}
	return p, 0, false
}

// encodeG1 encodes a G1Affine point into 128 bytes (padded format).
func encodeG1(p *bls12381.G1Affine) [blsPaddedG1Len]byte {
	var out [blsPaddedG1Len]byte
	if p.IsInfinity() {
		return out // all zeros
	}
	xBytes := p.X.Bytes()
	yBytes := p.Y.Bytes()
	// Copy with padding: 16 zero bytes + 48 data bytes for each coordinate
	copy(out[blsFpPadBy:blsPaddedFpLen], xBytes[:])
	copy(out[blsPaddedFpLen+blsFpPadBy:], yBytes[:])
	return out
}

// encodeG2 encodes a G2Affine point into 256 bytes (padded format).
func encodeG2(p *bls12381.G2Affine) [blsPaddedG2Len]byte {
	var out [blsPaddedG2Len]byte
	if p.IsInfinity() {
		return out // all zeros
	}
	x0Bytes := p.X.A0.Bytes()
	x1Bytes := p.X.A1.Bytes()
	y0Bytes := p.Y.A0.Bytes()
	y1Bytes := p.Y.A1.Bytes()
	copy(out[0*blsPaddedFpLen+blsFpPadBy:1*blsPaddedFpLen], x0Bytes[:])
	copy(out[1*blsPaddedFpLen+blsFpPadBy:2*blsPaddedFpLen], x1Bytes[:])
	copy(out[2*blsPaddedFpLen+blsFpPadBy:3*blsPaddedFpLen], y0Bytes[:])
	copy(out[3*blsPaddedFpLen+blsFpPadBy:4*blsPaddedFpLen], y1Bytes[:])
	return out
}

// msmRequiredGas computes the gas cost for an MSM operation.
func msmRequiredGas(k int, discountTable []uint16, baseCost uint64) uint64 {
	if k == 0 {
		return 0
	}
	idx := k - 1
	if idx >= len(discountTable) {
		idx = len(discountTable) - 1
	}
	discount := uint64(discountTable[idx])
	return (uint64(k) * discount * baseCost) / blsMsmMultiplier
}

// --- G1 ADD (0x0B) ---

func Bls12G1AddRun(input []byte, gasLimit uint64) PrecompileResult {
	if blsG1AddGas > gasLimit {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}
	if len(input) != blsG1AddInputLen {
		return PrecompileErr(PrecompileErrorBls12381InputLength)
	}

	// Per EIP-2537: G1ADD does not require subgroup checks.
	a, err, isErr := decodeG1NoSubgroup(input[:blsPaddedG1Len])
	if isErr {
		return PrecompileErr(err)
	}
	b, err, isErr := decodeG1NoSubgroup(input[blsPaddedG1Len:])
	if isErr {
		return PrecompileErr(err)
	}

	var result bls12381.G1Affine
	result.Add(&a, &b)

	encoded := encodeG1(&result)
	return PrecompileOk(NewPrecompileOutput(blsG1AddGas, encoded[:]))
}

// --- G1 MSM (0x0C) ---

func Bls12G1MsmRun(input []byte, gasLimit uint64) PrecompileResult {
	inputLen := len(input)
	if inputLen == 0 || inputLen%blsG1MsmInputLen != 0 {
		return PrecompileErr(PrecompileErrorBls12381InputLength)
	}

	k := inputLen / blsG1MsmInputLen
	requiredGas := msmRequiredGas(k, discountTableG1MSM[:], blsG1MsmBaseGas)
	if requiredGas > gasLimit {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}

	points := make([]bls12381.G1Affine, k)
	scalars := make([]fr.Element, k)

	for i := 0; i < k; i++ {
		start := i * blsG1MsmInputLen
		p, err, isErr := decodeG1(input[start : start+blsPaddedG1Len])
		if isErr {
			return PrecompileErr(err)
		}
		points[i] = p

		scalarBytes := input[start+blsPaddedG1Len : start+blsG1MsmInputLen]
		// Scalars are big-endian 32-byte values
		scalars[i].SetBigInt(new(big.Int).SetBytes(scalarBytes))
	}

	var result bls12381.G1Affine
	_, msmerr := result.MultiExp(points, scalars, ecc.MultiExpConfig{})
	if msmerr != nil {
		return PrecompileErr(PrecompileErrorFatal)
	}

	encoded := encodeG1(&result)
	return PrecompileOk(NewPrecompileOutput(requiredGas, encoded[:]))
}

// --- G2 ADD (0x0D) ---

func Bls12G2AddRun(input []byte, gasLimit uint64) PrecompileResult {
	if blsG2AddGas > gasLimit {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}
	if len(input) != blsG2AddInputLen {
		return PrecompileErr(PrecompileErrorBls12381InputLength)
	}

	// Per EIP-2537: G2ADD does not require subgroup checks.
	a, err, isErr := decodeG2NoSubgroup(input[:blsPaddedG2Len])
	if isErr {
		return PrecompileErr(err)
	}
	b, err, isErr := decodeG2NoSubgroup(input[blsPaddedG2Len:])
	if isErr {
		return PrecompileErr(err)
	}

	var result bls12381.G2Affine
	result.Add(&a, &b)

	encoded := encodeG2(&result)
	return PrecompileOk(NewPrecompileOutput(blsG2AddGas, encoded[:]))
}

// --- G2 MSM (0x0E) ---

func Bls12G2MsmRun(input []byte, gasLimit uint64) PrecompileResult {
	inputLen := len(input)
	if inputLen == 0 || inputLen%blsG2MsmInputLen != 0 {
		return PrecompileErr(PrecompileErrorBls12381InputLength)
	}

	k := inputLen / blsG2MsmInputLen
	requiredGas := msmRequiredGas(k, discountTableG2MSM[:], blsG2MsmBaseGas)
	if requiredGas > gasLimit {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}

	points := make([]bls12381.G2Affine, k)
	scalars := make([]fr.Element, k)

	for i := 0; i < k; i++ {
		start := i * blsG2MsmInputLen
		p, err, isErr := decodeG2(input[start : start+blsPaddedG2Len])
		if isErr {
			return PrecompileErr(err)
		}
		points[i] = p

		scalarBytes := input[start+blsPaddedG2Len : start+blsG2MsmInputLen]
		scalars[i].SetBigInt(new(big.Int).SetBytes(scalarBytes))
	}

	var result bls12381.G2Affine
	_, msmerr := result.MultiExp(points, scalars, ecc.MultiExpConfig{})
	if msmerr != nil {
		return PrecompileErr(PrecompileErrorFatal)
	}

	encoded := encodeG2(&result)
	return PrecompileOk(NewPrecompileOutput(requiredGas, encoded[:]))
}

// --- PAIRING (0x0F) ---

func Bls12PairingRun(input []byte, gasLimit uint64) PrecompileResult {
	inputLen := len(input)
	if inputLen == 0 || inputLen%blsPairingInputLen != 0 {
		return PrecompileErr(PrecompileErrorBls12381InputLength)
	}

	k := inputLen / blsPairingInputLen
	requiredGas := blsPairingPerPair*uint64(k) + blsPairingBaseGas
	if requiredGas > gasLimit {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}

	g1Points := make([]bls12381.G1Affine, k)
	g2Points := make([]bls12381.G2Affine, k)

	for i := 0; i < k; i++ {
		start := i * blsPairingInputLen
		g1, err, isErr := decodeG1(input[start : start+blsPaddedG1Len])
		if isErr {
			return PrecompileErr(err)
		}
		g2, err, isErr := decodeG2(input[start+blsPaddedG1Len : start+blsPairingInputLen])
		if isErr {
			return PrecompileErr(err)
		}
		g1Points[i] = g1
		g2Points[i] = g2
	}

	ok, pairingErr := bls12381.PairingCheck(g1Points, g2Points)
	if pairingErr != nil {
		return PrecompileErr(PrecompileErrorBls12381PairingError)
	}

	var out [32]byte
	if ok {
		out[31] = 1
	}
	return PrecompileOk(NewPrecompileOutput(requiredGas, out[:]))
}

// --- MAP_FP_TO_G1 (0x10) ---

func Bls12MapFpToG1Run(input []byte, gasLimit uint64) PrecompileResult {
	if blsMapFpToG1Gas > gasLimit {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}
	if len(input) != blsPaddedFpLen {
		return PrecompileErr(PrecompileErrorBls12381InputLength)
	}

	elem, err, isErr := decodeFp(input)
	if isErr {
		return PrecompileErr(err)
	}

	// Map field element to G1 curve point
	result := bls12381.MapToG1(elem)

	encoded := encodeG1(&result)
	return PrecompileOk(NewPrecompileOutput(blsMapFpToG1Gas, encoded[:]))
}

// --- MAP_FP2_TO_G2 (0x11) ---

func Bls12MapFp2ToG2Run(input []byte, gasLimit uint64) PrecompileResult {
	if blsMapFp2ToG2Gas > gasLimit {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}
	if len(input) != blsPaddedFp2Len {
		return PrecompileErr(PrecompileErrorBls12381InputLength)
	}

	e0, err, isErr := decodeFp(input[:blsPaddedFpLen])
	if isErr {
		return PrecompileErr(err)
	}
	e1, err, isErr := decodeFp(input[blsPaddedFpLen:])
	if isErr {
		return PrecompileErr(err)
	}

	// Map Fp2 element to G2 curve point
	u := bls12381.E2{A0: e0, A1: e1}
	result := bls12381.MapToG2(u)

	encoded := encodeG2(&result)
	return PrecompileOk(NewPrecompileOutput(blsMapFp2ToG2Gas, encoded[:]))
}
