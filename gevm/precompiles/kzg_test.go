// Tests for KZG point evaluation precompile (EIP-4844).
package precompiles

import (
	"crypto/sha256"
	"testing"

	"github.com/Giulio2002/gevm/spec"
)

// --- KZG tests ---

func TestKzgInputLength(t *testing.T) {
	r := KzgPointEvaluationRun(make([]byte, 100), 100000)
	if r.IsOk() {
		t.Fatal("expected error for wrong input length")
	}
	if *r.Err != PrecompileErrorBlobInvalidInputLength {
		t.Fatalf("expected BlobInvalidInputLength, got %d", *r.Err)
	}
}

func TestKzgOOG(t *testing.T) {
	r := KzgPointEvaluationRun(make([]byte, kzgInputLength), 10000)
	if r.IsOk() {
		t.Fatal("expected OOG")
	}
	if *r.Err != PrecompileErrorOutOfGas {
		t.Fatalf("expected OOG, got %d", *r.Err)
	}
}

func TestKzgVersionedHashMismatch(t *testing.T) {
	// Create input with wrong versioned hash
	input := make([]byte, kzgInputLength)
	// Set some arbitrary commitment bytes
	input[96] = 0x01
	// versioned_hash is all zeros, which won't match sha256(commitment)
	r := KzgPointEvaluationRun(input, 100000)
	if r.IsOk() {
		t.Fatal("expected error for mismatched versioned hash")
	}
	if *r.Err != PrecompileErrorBlobMismatchedVersion {
		t.Fatalf("expected BlobMismatchedVersion, got %d", *r.Err)
	}
}

func TestKzgToVersionedHash(t *testing.T) {
	commitment := make([]byte, 48)
	commitment[0] = 0xC0 // compressed infinity point

	hash := kzgToVersionedHash(commitment)

	// First byte should be version byte
	if hash[0] != kzgVersionedHashKZG {
		t.Fatalf("expected version byte 0x%02x, got 0x%02x", kzgVersionedHashKZG, hash[0])
	}

	// Rest should be sha256[1:]
	expected := sha256.Sum256(commitment)
	for i := 1; i < 32; i++ {
		if hash[i] != expected[i] {
			t.Fatalf("hash[%d] mismatch: got 0x%02x, want 0x%02x", i, hash[i], expected[i])
		}
	}
}

func TestKzgValidProof(t *testing.T) {
	// Test data from c-kzg-4844 verify_kzg_proof_case_correct_proof_4_4
	commitment := hexDecode("8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7")
	z := hexDecode("73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000")
	y := hexDecode("1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9")
	proof := hexDecode("a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c")

	// Compute versioned hash
	versionedHash := kzgToVersionedHash(commitment)

	// Build input
	input := make([]byte, 0, kzgInputLength)
	input = append(input, versionedHash[:]...)
	input = append(input, z...)
	input = append(input, y...)
	input = append(input, commitment...)
	input = append(input, proof...)

	r := KzgPointEvaluationRun(input, 100000)
	if !r.IsOk() {
		t.Fatalf("expected success, got error %d", *r.Err)
	}
	if r.Output.GasUsed != kzgGasCost {
		t.Fatalf("expected gas %d, got %d", kzgGasCost, r.Output.GasUsed)
	}
	if len(r.Output.Bytes) != 64 {
		t.Fatalf("expected 64-byte output, got %d", len(r.Output.Bytes))
	}

	// Verify return value matches expected constant
	expectedOutput := hexDecode("000000000000000000000000000000000000000000000000000000000000100073eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000001")
	for i, b := range r.Output.Bytes {
		if b != expectedOutput[i] {
			t.Fatalf("output byte %d: got 0x%02x, want 0x%02x", i, b, expectedOutput[i])
		}
	}
}

func TestKzgInvalidProof(t *testing.T) {
	// Use valid commitment but invalid proof
	commitment := hexDecode("8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7")
	z := hexDecode("73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000")
	y := hexDecode("0000000000000000000000000000000000000000000000000000000000000000") // wrong y
	proof := hexDecode("a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c")

	versionedHash := kzgToVersionedHash(commitment)

	input := make([]byte, 0, kzgInputLength)
	input = append(input, versionedHash[:]...)
	input = append(input, z...)
	input = append(input, y...)
	input = append(input, commitment...)
	input = append(input, proof...)

	r := KzgPointEvaluationRun(input, 100000)
	if r.IsOk() {
		t.Fatal("expected error for invalid proof")
	}
	if *r.Err != PrecompileErrorBlobVerifyKZGProofFailed {
		t.Fatalf("expected BlobVerifyKZGProofFailed, got %d", *r.Err)
	}
}

// --- Cancun registration tests ---

func TestCancunHasKZG(t *testing.T) {
	ps := ForSpec(spec.Cancun)
	var addr [20]byte
	addr[19] = 0x0A
	if !ps.Contains(addr) {
		t.Fatal("Cancun should have KZG precompile at 0x0A")
	}
}

func TestBerlinDoesNotHaveKZG(t *testing.T) {
	ps := ForSpec(spec.Berlin)
	var addr [20]byte
	addr[19] = 0x0A
	if ps.Contains(addr) {
		t.Fatal("Berlin should not have KZG precompile")
	}
}

func TestCancunWarmAddresses(t *testing.T) {
	ps := ForSpec(spec.Cancun)
	addrs := ps.WarmAddresses()
	// Should have 0x01-0x09 (Berlin) + 0x0A (KZG) = 10
	if len(addrs) != 10 {
		t.Fatalf("expected 10 warm addresses for Cancun, got %d", len(addrs))
	}
}
