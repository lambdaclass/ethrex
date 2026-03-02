// Tests for precompile implementations.
package precompiles

import (
	"bytes"
	"encoding/hex"
	"testing"

	"github.com/Giulio2002/gevm/spec"
)

// hexDecode decodes a hex string, panicking on error.
func hexDecode(s string) []byte {
	b, err := hex.DecodeString(s)
	if err != nil {
		panic(err)
	}
	return b
}

// --- Identity tests ---

func TestIdentityEmpty(t *testing.T) {
	r := IdentityRun(nil, 100)
	if !r.IsOk() {
		t.Fatal("expected success")
	}
	if len(r.Output.Bytes) != 0 {
		t.Fatal("expected empty output")
	}
	if r.Output.GasUsed != 15 { // base only
		t.Fatalf("expected gas 15, got %d", r.Output.GasUsed)
	}
}

func TestIdentityCopy(t *testing.T) {
	input := []byte{1, 2, 3, 4, 5}
	r := IdentityRun(input, 100)
	if !r.IsOk() {
		t.Fatal("expected success")
	}
	if !bytes.Equal(r.Output.Bytes, input) {
		t.Fatal("output should match input")
	}
	// gas = 15 + ceil(5/32) * 3 = 15 + 3 = 18
	if r.Output.GasUsed != 18 {
		t.Fatalf("expected gas 18, got %d", r.Output.GasUsed)
	}
}

func TestIdentityOOG(t *testing.T) {
	r := IdentityRun([]byte{1, 2, 3}, 10) // needs 18 gas
	if !r.IsErr() {
		t.Fatal("expected OOG error")
	}
	if *r.Err != PrecompileErrorOutOfGas {
		t.Fatal("expected OutOfGas")
	}
}

func TestIdentityLargeInput(t *testing.T) {
	input := make([]byte, 100)
	r := IdentityRun(input, 1000)
	if !r.IsOk() {
		t.Fatal("expected success")
	}
	// gas = 15 + ceil(100/32) * 3 = 15 + 4*3 = 27
	if r.Output.GasUsed != 27 {
		t.Fatalf("expected gas 27, got %d", r.Output.GasUsed)
	}
}

// --- SHA256 tests ---

func TestSha256Empty(t *testing.T) {
	r := Sha256Run(nil, 1000)
	if !r.IsOk() {
		t.Fatal("expected success")
	}
	// SHA256 of empty string
	expected := hexDecode("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
	if !bytes.Equal(r.Output.Bytes, expected) {
		t.Fatalf("wrong hash: got %x", r.Output.Bytes)
	}
	if r.Output.GasUsed != 60 { // base only (0 words)
		t.Fatalf("expected gas 60, got %d", r.Output.GasUsed)
	}
}

func TestSha256Hello(t *testing.T) {
	r := Sha256Run([]byte("hello"), 1000)
	if !r.IsOk() {
		t.Fatal("expected success")
	}
	expected := hexDecode("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
	if !bytes.Equal(r.Output.Bytes, expected) {
		t.Fatalf("wrong hash: got %x", r.Output.Bytes)
	}
	// gas = 60 + ceil(5/32) * 12 = 60 + 12 = 72
	if r.Output.GasUsed != 72 {
		t.Fatalf("expected gas 72, got %d", r.Output.GasUsed)
	}
}

func TestSha256OOG(t *testing.T) {
	r := Sha256Run([]byte("test"), 50) // needs 72
	if !r.IsErr() {
		t.Fatal("expected OOG")
	}
}

// --- RIPEMD160 tests ---

func TestRipemd160Empty(t *testing.T) {
	r := Ripemd160Run(nil, 10000)
	if !r.IsOk() {
		t.Fatal("expected success")
	}
	// RIPEMD160 of empty string, left-padded to 32 bytes
	expected := hexDecode("0000000000000000000000009c1185a5c5e9fc54612808977ee8f548b2258d31")
	if !bytes.Equal(r.Output.Bytes, expected) {
		t.Fatalf("wrong hash: got %x", r.Output.Bytes)
	}
	if r.Output.GasUsed != 600 { // base only
		t.Fatalf("expected gas 600, got %d", r.Output.GasUsed)
	}
}

func TestRipemd160Hello(t *testing.T) {
	r := Ripemd160Run([]byte("hello"), 10000)
	if !r.IsOk() {
		t.Fatal("expected success")
	}
	// RIPEMD160 of "hello", left-padded to 32 bytes
	expected := hexDecode("000000000000000000000000108f07b8382412612c048d07d13f814118445acd")
	if !bytes.Equal(r.Output.Bytes, expected) {
		t.Fatalf("wrong hash: got %x", r.Output.Bytes)
	}
	// gas = 600 + ceil(5/32) * 120 = 600 + 120 = 720
	if r.Output.GasUsed != 720 {
		t.Fatalf("expected gas 720, got %d", r.Output.GasUsed)
	}
}

// --- ECRECOVER tests ---

func TestEcrecoverOOG(t *testing.T) {
	r := EcRecoverRun(nil, 2999)
	if !r.IsErr() || *r.Err != PrecompileErrorOutOfGas {
		t.Fatal("expected OOG")
	}
}

func TestEcrecoverInvalidV(t *testing.T) {
	// v = 26 (invalid, must be 27 or 28)
	input := make([]byte, 128)
	input[63] = 26
	r := EcRecoverRun(input, 10000)
	if !r.IsOk() {
		t.Fatal("expected success (invalid v returns empty)")
	}
	if len(r.Output.Bytes) != 0 {
		t.Fatal("invalid v should return empty output")
	}
	if r.Output.GasUsed != 3000 {
		t.Fatalf("gas should be 3000, got %d", r.Output.GasUsed)
	}
}

func TestEcrecoverValidSignature(t *testing.T) {
	// Known test vector from Ethereum tests
	// Message hash, v=28, r, s -> recovered address
	input := hexDecode(
		"456e9aea5e197a1f1af7a3e85a3212fa4049a3ba34c2289b4c860fc0b0c64ef3" + // msg hash
			"000000000000000000000000000000000000000000000000000000000000001c" + // v = 28
			"9242685bf161793cc25603c231bc2f568eb630ea16aa137d2664ac8038825608" + // r
			"4f8ae3bd7535248d0bd448298cc2e2071e56992d0774dc340c368ae950852ada", // s
	)
	r := EcRecoverRun(input, 10000)
	if !r.IsOk() {
		t.Fatal("expected success")
	}
	if len(r.Output.Bytes) != 32 {
		t.Fatalf("expected 32-byte output, got %d", len(r.Output.Bytes))
	}
	// Expected address: 0x7156526fbd7a3c72969b54f64e42c10fbb768c8a
	expectedAddr := hexDecode("0000000000000000000000007156526fbd7a3c72969b54f64e42c10fbb768c8a")
	if !bytes.Equal(r.Output.Bytes, expectedAddr) {
		t.Fatalf("wrong recovered address:\ngot:  %x\nwant: %x", r.Output.Bytes, expectedAddr)
	}
}

func TestEcrecoverShortInput(t *testing.T) {
	// Short input should be right-padded to 128 bytes
	input := make([]byte, 10)
	r := EcRecoverRun(input, 10000)
	if !r.IsOk() {
		t.Fatal("expected success (short input returns empty)")
	}
	// v will be 0 (invalid), so should return empty
	if len(r.Output.Bytes) != 0 {
		t.Fatal("short input with v=0 should return empty")
	}
}

// --- BLAKE2F tests ---

func TestBlake2FWrongLength(t *testing.T) {
	r := Blake2FRun(make([]byte, 100), 10000)
	if !r.IsErr() || *r.Err != PrecompileErrorBlake2WrongLength {
		t.Fatal("expected Blake2WrongLength")
	}
}

func TestBlake2FWrongFinalFlag(t *testing.T) {
	input := make([]byte, 213)
	input[212] = 2 // invalid final flag
	r := Blake2FRun(input, 10000)
	if !r.IsErr() || *r.Err != PrecompileErrorBlake2WrongFinalIndicatorFlag {
		t.Fatal("expected Blake2WrongFinalIndicatorFlag")
	}
}

func TestBlake2FZeroRounds(t *testing.T) {
	input := make([]byte, 213)
	// 0 rounds, final=0
	r := Blake2FRun(input, 10000)
	if !r.IsOk() {
		t.Fatal("expected success")
	}
	if r.Output.GasUsed != 0 {
		t.Fatalf("0 rounds should cost 0 gas, got %d", r.Output.GasUsed)
	}
	if len(r.Output.Bytes) != 64 {
		t.Fatalf("expected 64-byte output, got %d", len(r.Output.Bytes))
	}
}

func TestBlake2FEIP152Vector(t *testing.T) {
	// EIP-152 test vector #1 (from the EIP spec)
	input := hexDecode(
		"0000000c" + // rounds = 12
			"48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5" +
			"d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b" + // h
			"6162630000000000000000000000000000000000000000000000000000000000" +
			"0000000000000000000000000000000000000000000000000000000000000000" +
			"0000000000000000000000000000000000000000000000000000000000000000" +
			"0000000000000000000000000000000000000000000000000000000000000000" + // m
			"0300000000000000" + // t[0]
			"0000000000000000" + // t[1]
			"01") // final = true

	r := Blake2FRun(input, 1000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %v", r.Err)
	}
	if r.Output.GasUsed != 12 {
		t.Fatalf("expected gas 12, got %d", r.Output.GasUsed)
	}

	expected := hexDecode(
		"ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d1" +
			"7d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923")
	if !bytes.Equal(r.Output.Bytes, expected) {
		t.Fatalf("wrong BLAKE2F output:\ngot:  %x\nwant: %x", r.Output.Bytes, expected)
	}
}

// --- MODEXP tests ---

func TestModexpSimple(t *testing.T) {
	// 2^3 mod 5 = 3
	input := hexDecode(
		"0000000000000000000000000000000000000000000000000000000000000001" + // base_len = 1
			"0000000000000000000000000000000000000000000000000000000000000001" + // exp_len = 1
			"0000000000000000000000000000000000000000000000000000000000000001" + // mod_len = 1
			"02" + // base = 2
			"03" + // exp = 3
			"05") // mod = 5

	r := ModExpByzantiumRun(input, 100000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %v", r.Err)
	}
	if len(r.Output.Bytes) != 1 {
		t.Fatalf("expected 1-byte output, got %d", len(r.Output.Bytes))
	}
	if r.Output.Bytes[0] != 3 {
		t.Fatalf("2^3 mod 5 should be 3, got %d", r.Output.Bytes[0])
	}
}

func TestModexpZeroMod(t *testing.T) {
	// anything mod 0 = 0
	input := hexDecode(
		"0000000000000000000000000000000000000000000000000000000000000001" +
			"0000000000000000000000000000000000000000000000000000000000000001" +
			"0000000000000000000000000000000000000000000000000000000000000001" +
			"02" + "03" + "00")

	r := ModExpByzantiumRun(input, 100000)
	if !r.IsOk() {
		t.Fatal("expected success")
	}
	if len(r.Output.Bytes) != 1 || r.Output.Bytes[0] != 0 {
		t.Fatalf("anything mod 0 should be 0, got %v", r.Output.Bytes)
	}
}

func TestModexpBerlinMinGas(t *testing.T) {
	// Berlin has minimum gas of 200
	input := hexDecode(
		"0000000000000000000000000000000000000000000000000000000000000001" +
			"0000000000000000000000000000000000000000000000000000000000000001" +
			"0000000000000000000000000000000000000000000000000000000000000001" +
			"02" + "01" + "03")

	r := ModExpBerlinRun(input, 100000)
	if !r.IsOk() {
		t.Fatal("expected success")
	}
	if r.Output.GasUsed < 200 {
		t.Fatalf("Berlin modexp should have minimum gas 200, got %d", r.Output.GasUsed)
	}
}

// --- BN254 ADD tests ---

func TestBn254AddZeroPoints(t *testing.T) {
	// Adding point at infinity to itself should give infinity
	input := make([]byte, 128)
	r := Bn254AddIstanbulRun(input, 10000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %v", r.Err)
	}
	if r.Output.GasUsed != bn254AddIstanbulGas {
		t.Fatalf("expected gas %d, got %d", bn254AddIstanbulGas, r.Output.GasUsed)
	}
	// Infinity encoded as 64 zero bytes
	if len(r.Output.Bytes) != 64 {
		t.Fatalf("expected 64-byte output, got %d", len(r.Output.Bytes))
	}
	for _, b := range r.Output.Bytes {
		if b != 0 {
			t.Fatal("infinity + infinity should be infinity")
		}
	}
}

func TestBn254AddGenerator(t *testing.T) {
	// G1 generator point: (1, 2)
	// Adding G1 + infinity should give G1
	gen := hexDecode(
		"0000000000000000000000000000000000000000000000000000000000000001" + // x = 1
			"0000000000000000000000000000000000000000000000000000000000000002") // y = 2
	input := make([]byte, 128)
	copy(input[0:64], gen)
	// Second point is zero (infinity)

	r := Bn254AddIstanbulRun(input, 10000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %v", r.Err)
	}
	if !bytes.Equal(r.Output.Bytes[:64], gen) {
		t.Fatalf("G + 0 should equal G")
	}
}

func TestBn254AddOOG(t *testing.T) {
	r := Bn254AddByzantiumRun(nil, 100) // needs 500
	if !r.IsErr() || *r.Err != PrecompileErrorOutOfGas {
		t.Fatal("expected OOG")
	}
}

// --- BN254 MUL tests ---

func TestBn254MulZeroScalar(t *testing.T) {
	// G1 * 0 = infinity
	input := hexDecode(
		"0000000000000000000000000000000000000000000000000000000000000001" + // x = 1
			"0000000000000000000000000000000000000000000000000000000000000002" + // y = 2
			"0000000000000000000000000000000000000000000000000000000000000000") // scalar = 0

	r := Bn254MulIstanbulRun(input, 100000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %v", r.Err)
	}
	// Result should be point at infinity (all zeros)
	for _, b := range r.Output.Bytes {
		if b != 0 {
			t.Fatal("G * 0 should be infinity")
		}
	}
}

func TestBn254MulOneScalar(t *testing.T) {
	// G1 * 1 = G1
	gen := hexDecode(
		"0000000000000000000000000000000000000000000000000000000000000001" + // x = 1
			"0000000000000000000000000000000000000000000000000000000000000002") // y = 2
	input := make([]byte, 96)
	copy(input[0:64], gen)
	input[95] = 1 // scalar = 1

	r := Bn254MulIstanbulRun(input, 100000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %v", r.Err)
	}
	if !bytes.Equal(r.Output.Bytes[:64], gen) {
		t.Fatalf("G * 1 should equal G\ngot:  %x\nwant: %x", r.Output.Bytes[:64], gen)
	}
}

// --- BN254 PAIRING tests ---

func TestBn254PairingEmptyInput(t *testing.T) {
	// Empty input: trivially true
	r := Bn254PairingIstanbulRun(nil, 100000)
	if !r.IsOk() {
		t.Fatal("expected success")
	}
	if r.Output.GasUsed != bn254PairBaseIstanbul {
		t.Fatalf("expected gas %d, got %d", bn254PairBaseIstanbul, r.Output.GasUsed)
	}
	// Should return 1 (true)
	expected := make([]byte, 32)
	expected[31] = 1
	if !bytes.Equal(r.Output.Bytes, expected) {
		t.Fatalf("empty pairing should be true, got %x", r.Output.Bytes)
	}
}

func TestBn254PairingWrongLength(t *testing.T) {
	// Input not a multiple of 192
	r := Bn254PairingIstanbulRun(make([]byte, 100), 1000000)
	if !r.IsErr() || *r.Err != PrecompileErrorBn254PairLength {
		t.Fatal("expected Bn254PairLength error")
	}
}

// --- Precompile set tests ---

func TestPrecompileSetHomestead(t *testing.T) {
	ps := Homestead()
	// Should have 4 precompiles: 0x01-0x04
	for i := byte(1); i <= 4; i++ {
		var addr [20]byte
		addr[19] = i
		if !ps.Contains(addr) {
			t.Fatalf("Homestead should contain precompile 0x%02x", i)
		}
	}
	// 0x05 should not exist
	var addr5 [20]byte
	addr5[19] = 5
	if ps.Contains(addr5) {
		t.Fatal("Homestead should not contain 0x05")
	}
}

func TestPrecompileSetIstanbul(t *testing.T) {
	ps := Istanbul()
	for i := byte(1); i <= 9; i++ {
		var addr [20]byte
		addr[19] = i
		if !ps.Contains(addr) {
			t.Fatalf("Istanbul should contain precompile 0x%02x", i)
		}
	}
}

func TestForSpec(t *testing.T) {
	ps := ForSpec(spec.Shanghai)
	// Shanghai is Berlin-level (9 precompiles)
	for i := byte(1); i <= 9; i++ {
		var addr [20]byte
		addr[19] = i
		if !ps.Contains(addr) {
			t.Fatalf("Shanghai should contain precompile 0x%02x", i)
		}
	}
}

func TestWarmAddresses(t *testing.T) {
	ps := Homestead()
	addrs := ps.WarmAddresses()
	if len(addrs) != 4 {
		t.Fatalf("expected 4 warm addresses, got %d", len(addrs))
	}
}

func TestCalcLinearCost(t *testing.T) {
	tests := []struct {
		dataLen  int
		base     uint64
		word     uint64
		expected uint64
	}{
		{0, 15, 3, 15},     // 0 words
		{1, 15, 3, 18},     // 1 word
		{32, 15, 3, 18},    // 1 word
		{33, 15, 3, 21},    // 2 words
		{100, 60, 12, 108}, // SHA256: 60 + 4*12
		{0, 600, 120, 600}, // RIPEMD160 empty
	}
	for _, tt := range tests {
		got := CalcLinearCost(tt.dataLen, tt.base, tt.word)
		if got != tt.expected {
			t.Errorf("CalcLinearCost(%d, %d, %d) = %d, want %d",
				tt.dataLen, tt.base, tt.word, got, tt.expected)
		}
	}
}
