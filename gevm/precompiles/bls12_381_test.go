// Tests for BLS12-381 precompile implementations (EIP-2537).
package precompiles

import (
	"testing"

	"github.com/Giulio2002/gevm/spec"
	bls12381 "github.com/consensys/gnark-crypto/ecc/bls12-381"
	"github.com/consensys/gnark-crypto/ecc/bls12-381/fp"
)

// --- Helper functions ---

// makeG1Input creates a 128-byte padded G1 point input from x, y fp.Elements.
func makeG1Input(x, y fp.Element) []byte {
	var p bls12381.G1Affine
	p.X = x
	p.Y = y
	encoded := encodeG1(&p)
	return encoded[:]
}

// makeZeroG1Input returns 128 zero bytes (point at infinity).
func makeZeroG1Input() []byte {
	return make([]byte, blsPaddedG1Len)
}

// makeZeroG2Input returns 256 zero bytes (point at infinity).
func makeZeroG2Input() []byte {
	return make([]byte, blsPaddedG2Len)
}

// --- G1ADD tests ---

func TestBls12G1AddInputLength(t *testing.T) {
	r := Bls12G1AddRun(make([]byte, 100), 1000000)
	if r.IsOk() {
		t.Fatal("expected error for wrong input length")
	}
}

func TestBls12G1AddOOG(t *testing.T) {
	r := Bls12G1AddRun(make([]byte, blsG1AddInputLen), 100)
	if r.IsOk() {
		t.Fatal("expected OOG")
	}
	if *r.Err != PrecompileErrorOutOfGas {
		t.Fatalf("expected OOG error, got %d", *r.Err)
	}
}

func TestBls12G1AddInfinityPlusInfinity(t *testing.T) {
	// infinity + infinity = infinity
	input := append(makeZeroG1Input(), makeZeroG1Input()...)
	r := Bls12G1AddRun(input, 1000000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %d", *r.Err)
	}
	if r.Output.GasUsed != blsG1AddGas {
		t.Fatalf("expected gas %d, got %d", blsG1AddGas, r.Output.GasUsed)
	}
	// Result should be all zeros (infinity)
	for _, b := range r.Output.Bytes {
		if b != 0 {
			t.Fatal("expected infinity output (all zeros)")
		}
	}
}

func TestBls12G1AddGeneratorPlusInfinity(t *testing.T) {
	// G + infinity = G
	_, _, g1Aff, _ := bls12381.Generators()
	g1Encoded := encodeG1(&g1Aff)

	input := append(g1Encoded[:], makeZeroG1Input()...)
	r := Bls12G1AddRun(input, 1000000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %d", *r.Err)
	}
	// Result should equal G1 generator
	if len(r.Output.Bytes) != blsPaddedG1Len {
		t.Fatalf("expected %d byte output, got %d", blsPaddedG1Len, len(r.Output.Bytes))
	}
	for i, b := range r.Output.Bytes {
		if b != g1Encoded[i] {
			t.Fatal("G + infinity should equal G")
		}
	}
}

func TestBls12G1AddGeneratorPlusGenerator(t *testing.T) {
	// G + G = 2G
	_, _, g1Aff, _ := bls12381.Generators()
	g1Encoded := encodeG1(&g1Aff)

	input := append(g1Encoded[:], g1Encoded[:]...)
	r := Bls12G1AddRun(input, 1000000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %d", *r.Err)
	}

	// Compute 2G directly
	var doubledG1 bls12381.G1Affine
	doubledG1.Double(&g1Aff)
	expectedEncoded := encodeG1(&doubledG1)
	for i, b := range r.Output.Bytes {
		if b != expectedEncoded[i] {
			t.Fatal("G + G should equal 2G")
		}
	}
}

func TestBls12G1AddInvalidPadding(t *testing.T) {
	// Non-zero padding should fail
	input := make([]byte, blsG1AddInputLen)
	input[0] = 0x01 // invalid padding
	r := Bls12G1AddRun(input, 1000000)
	if r.IsOk() {
		t.Fatal("expected error for invalid padding")
	}
}

// --- G2ADD tests ---

func TestBls12G2AddInputLength(t *testing.T) {
	r := Bls12G2AddRun(make([]byte, 100), 1000000)
	if r.IsOk() {
		t.Fatal("expected error for wrong input length")
	}
}

func TestBls12G2AddOOG(t *testing.T) {
	r := Bls12G2AddRun(make([]byte, blsG2AddInputLen), 100)
	if r.IsOk() {
		t.Fatal("expected OOG")
	}
}

func TestBls12G2AddInfinityPlusInfinity(t *testing.T) {
	input := append(makeZeroG2Input(), makeZeroG2Input()...)
	r := Bls12G2AddRun(input, 1000000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %d", *r.Err)
	}
	if r.Output.GasUsed != blsG2AddGas {
		t.Fatalf("expected gas %d, got %d", blsG2AddGas, r.Output.GasUsed)
	}
	for _, b := range r.Output.Bytes {
		if b != 0 {
			t.Fatal("expected infinity output")
		}
	}
}

func TestBls12G2AddGeneratorPlusInfinity(t *testing.T) {
	_, _, _, g2Aff := bls12381.Generators()
	g2Encoded := encodeG2(&g2Aff)

	input := append(g2Encoded[:], makeZeroG2Input()...)
	r := Bls12G2AddRun(input, 1000000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %d", *r.Err)
	}
	for i, b := range r.Output.Bytes {
		if b != g2Encoded[i] {
			t.Fatal("G2 + infinity should equal G2")
		}
	}
}

// --- G1 MSM tests ---

func TestBls12G1MsmInputLength(t *testing.T) {
	// Empty input
	r := Bls12G1MsmRun(nil, 1000000)
	if r.IsOk() {
		t.Fatal("expected error for empty input")
	}
	// Wrong length
	r = Bls12G1MsmRun(make([]byte, 100), 1000000)
	if r.IsOk() {
		t.Fatal("expected error for wrong length")
	}
}

func TestBls12G1MsmSinglePairScalarOne(t *testing.T) {
	// G1 * 1 = G1
	_, _, g1Aff, _ := bls12381.Generators()
	g1Encoded := encodeG1(&g1Aff)

	// Scalar = 1 in big-endian 32 bytes
	var scalar [32]byte
	scalar[31] = 1

	input := append(g1Encoded[:], scalar[:]...)
	r := Bls12G1MsmRun(input, 1000000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %d", *r.Err)
	}
	if r.Output.GasUsed != blsG1MsmBaseGas {
		t.Fatalf("expected gas %d, got %d", blsG1MsmBaseGas, r.Output.GasUsed)
	}
	// Result should equal G1 generator
	for i, b := range r.Output.Bytes {
		if b != g1Encoded[i] {
			t.Fatal("G1 * 1 should equal G1")
		}
	}
}

func TestBls12G1MsmSinglePairScalarZero(t *testing.T) {
	// G1 * 0 = infinity
	_, _, g1Aff, _ := bls12381.Generators()
	g1Encoded := encodeG1(&g1Aff)

	var scalar [32]byte // all zeros

	input := append(g1Encoded[:], scalar[:]...)
	r := Bls12G1MsmRun(input, 1000000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %d", *r.Err)
	}
	// Result should be infinity (all zeros)
	for _, b := range r.Output.Bytes {
		if b != 0 {
			t.Fatal("G1 * 0 should be infinity")
		}
	}
}

// --- G2 MSM tests ---

func TestBls12G2MsmSinglePairScalarOne(t *testing.T) {
	// G2 * 1 = G2
	_, _, _, g2Aff := bls12381.Generators()
	g2Encoded := encodeG2(&g2Aff)

	var scalar [32]byte
	scalar[31] = 1

	input := append(g2Encoded[:], scalar[:]...)
	r := Bls12G2MsmRun(input, 1000000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %d", *r.Err)
	}
	if r.Output.GasUsed != blsG2MsmBaseGas {
		t.Fatalf("expected gas %d, got %d", blsG2MsmBaseGas, r.Output.GasUsed)
	}
	for i, b := range r.Output.Bytes {
		if b != g2Encoded[i] {
			t.Fatal("G2 * 1 should equal G2")
		}
	}
}

// --- Pairing tests ---

func TestBls12PairingInputLength(t *testing.T) {
	r := Bls12PairingRun(nil, 1000000)
	if r.IsOk() {
		t.Fatal("expected error for empty input")
	}
	r = Bls12PairingRun(make([]byte, 100), 1000000)
	if r.IsOk() {
		t.Fatal("expected error for wrong length")
	}
}

func TestBls12PairingOOG(t *testing.T) {
	r := Bls12PairingRun(make([]byte, blsPairingInputLen), 1000)
	if r.IsOk() {
		t.Fatal("expected OOG")
	}
}

func TestBls12PairingInfinityPair(t *testing.T) {
	// e(infinity_G1, infinity_G2) should return 1 (identity)
	input := make([]byte, blsPairingInputLen)
	r := Bls12PairingRun(input, 1000000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %d", *r.Err)
	}
	expectedGas := blsPairingPerPair + blsPairingBaseGas
	if r.Output.GasUsed != expectedGas {
		t.Fatalf("expected gas %d, got %d", expectedGas, r.Output.GasUsed)
	}
	if len(r.Output.Bytes) != 32 {
		t.Fatalf("expected 32-byte output, got %d", len(r.Output.Bytes))
	}
	if r.Output.Bytes[31] != 1 {
		t.Fatal("pairing of infinity should return 1")
	}
}

func TestBls12PairingValid(t *testing.T) {
	// e(G1, G2) * e(-G1, G2) = 1 (identity)
	_, _, g1Aff, g2Aff := bls12381.Generators()
	g1Encoded := encodeG1(&g1Aff)
	g2Encoded := encodeG2(&g2Aff)

	var negG1 bls12381.G1Affine
	negG1.Neg(&g1Aff)
	negG1Encoded := encodeG1(&negG1)

	// First pair: (G1, G2)
	input := append(g1Encoded[:], g2Encoded[:]...)
	// Second pair: (-G1, G2)
	input = append(input, negG1Encoded[:]...)
	input = append(input, g2Encoded[:]...)

	r := Bls12PairingRun(input, 1000000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %d", *r.Err)
	}
	if r.Output.Bytes[31] != 1 {
		t.Fatal("e(G1, G2) * e(-G1, G2) should be identity")
	}
}

// --- MAP_FP_TO_G1 tests ---

func TestBls12MapFpToG1InputLength(t *testing.T) {
	r := Bls12MapFpToG1Run(make([]byte, 32), 1000000)
	if r.IsOk() {
		t.Fatal("expected error for wrong input length")
	}
}

func TestBls12MapFpToG1OOG(t *testing.T) {
	r := Bls12MapFpToG1Run(make([]byte, blsPaddedFpLen), 100)
	if r.IsOk() {
		t.Fatal("expected OOG")
	}
}

func TestBls12MapFpToG1Zero(t *testing.T) {
	// Map zero element to G1
	input := make([]byte, blsPaddedFpLen)
	r := Bls12MapFpToG1Run(input, 1000000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %d", *r.Err)
	}
	if r.Output.GasUsed != blsMapFpToG1Gas {
		t.Fatalf("expected gas %d, got %d", blsMapFpToG1Gas, r.Output.GasUsed)
	}
	if len(r.Output.Bytes) != blsPaddedG1Len {
		t.Fatalf("expected %d-byte output, got %d", blsPaddedG1Len, len(r.Output.Bytes))
	}
	// Verify result is on curve by decoding it
	_, err, isErr := decodeG1(r.Output.Bytes)
	if isErr {
		t.Fatalf("result should be a valid G1 point, got error %d", err)
	}
}

func TestBls12MapFpToG1One(t *testing.T) {
	// Map field element 1 to G1
	input := make([]byte, blsPaddedFpLen)
	input[blsPaddedFpLen-1] = 1

	r := Bls12MapFpToG1Run(input, 1000000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %d", *r.Err)
	}
	// Verify result is on curve
	p, err, isErr := decodeG1(r.Output.Bytes)
	if isErr {
		t.Fatalf("result should be a valid G1 point, got error %d", err)
	}
	if p.IsInfinity() {
		t.Fatal("mapping of 1 should not be infinity")
	}
}

func TestBls12MapFpToG1InvalidPadding(t *testing.T) {
	input := make([]byte, blsPaddedFpLen)
	input[0] = 0xFF // invalid padding
	r := Bls12MapFpToG1Run(input, 1000000)
	if r.IsOk() {
		t.Fatal("expected error for invalid padding")
	}
}

// --- MAP_FP2_TO_G2 tests ---

func TestBls12MapFp2ToG2InputLength(t *testing.T) {
	r := Bls12MapFp2ToG2Run(make([]byte, 64), 1000000)
	if r.IsOk() {
		t.Fatal("expected error for wrong input length")
	}
}

func TestBls12MapFp2ToG2Zero(t *testing.T) {
	input := make([]byte, blsPaddedFp2Len)
	r := Bls12MapFp2ToG2Run(input, 1000000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %d", *r.Err)
	}
	if r.Output.GasUsed != blsMapFp2ToG2Gas {
		t.Fatalf("expected gas %d, got %d", blsMapFp2ToG2Gas, r.Output.GasUsed)
	}
	if len(r.Output.Bytes) != blsPaddedG2Len {
		t.Fatalf("expected %d-byte output, got %d", blsPaddedG2Len, len(r.Output.Bytes))
	}
	// Verify result is on curve
	_, err, isErr := decodeG2(r.Output.Bytes)
	if isErr {
		t.Fatalf("result should be a valid G2 point, got error %d", err)
	}
}

// --- MSM gas calculation tests ---

func TestMsmRequiredGas(t *testing.T) {
	// k=1: gas = 1 * 1000 * 12000 / 1000 = 12000
	gas := msmRequiredGas(1, discountTableG1MSM[:], blsG1MsmBaseGas)
	if gas != 12000 {
		t.Fatalf("expected 12000, got %d", gas)
	}

	// k=2: gas = 2 * 949 * 12000 / 1000 = 22776
	gas = msmRequiredGas(2, discountTableG1MSM[:], blsG1MsmBaseGas)
	if gas != 22776 {
		t.Fatalf("expected 22776, got %d", gas)
	}

	// k=0: 0
	gas = msmRequiredGas(0, discountTableG1MSM[:], blsG1MsmBaseGas)
	if gas != 0 {
		t.Fatalf("expected 0, got %d", gas)
	}

	// k > table size: use last entry (519)
	gas = msmRequiredGas(200, discountTableG1MSM[:], blsG1MsmBaseGas)
	expected := uint64(200) * 519 * 12000 / 1000
	if gas != expected {
		t.Fatalf("expected %d, got %d", expected, gas)
	}
}

// --- PrecompileSet Prague registration tests ---

func TestPragueHasBls12Precompiles(t *testing.T) {
	ps := ForSpec(spec.Prague)

	// Should have all BLS12-381 precompiles
	addrs := []byte{0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11}
	for _, b := range addrs {
		var addr [20]byte
		addr[19] = b
		if !ps.Contains(addr) {
			t.Fatalf("Prague should have precompile at 0x%02X", b)
		}
	}
}

func TestCancunDoesNotHaveBls12(t *testing.T) {
	ps := ForSpec(spec.Cancun)

	// Should NOT have BLS12-381 precompiles
	var addr [20]byte
	addr[19] = 0x0B
	if ps.Contains(addr) {
		t.Fatal("Cancun should not have BLS12-381 precompiles")
	}
}

func TestPragueWarmAddresses(t *testing.T) {
	ps := ForSpec(spec.Prague)
	addrs := ps.WarmAddresses()
	// Should have 0x01-0x09 (Berlin) + 0x0A (KZG) + 0x0B-0x11 (BLS12-381) = 17
	// P256VERIFY (0x0100) is only added in Osaka, not Prague.
	if len(addrs) != 17 {
		t.Fatalf("expected 17 warm addresses for Prague, got %d", len(addrs))
	}
}

// --- Encode/decode roundtrip tests ---

func TestG1EncodeDecodeRoundtrip(t *testing.T) {
	_, _, g1Aff, _ := bls12381.Generators()
	encoded := encodeG1(&g1Aff)
	decoded, err, isErr := decodeG1(encoded[:])
	if isErr {
		t.Fatalf("decode failed: %d", err)
	}
	if !decoded.Equal(&g1Aff) {
		t.Fatal("roundtrip failed for G1 generator")
	}
}

func TestG2EncodeDecodeRoundtrip(t *testing.T) {
	_, _, _, g2Aff := bls12381.Generators()
	encoded := encodeG2(&g2Aff)
	decoded, err, isErr := decodeG2(encoded[:])
	if isErr {
		t.Fatalf("decode failed: %d", err)
	}
	if !decoded.Equal(&g2Aff) {
		t.Fatal("roundtrip failed for G2 generator")
	}
}

func TestG1InfinityRoundtrip(t *testing.T) {
	var inf bls12381.G1Affine
	inf.SetInfinity()
	encoded := encodeG1(&inf)
	decoded, _, isErr := decodeG1(encoded[:])
	if isErr {
		t.Fatal("decode infinity failed")
	}
	if !decoded.IsInfinity() {
		t.Fatal("decoded should be infinity")
	}
}
