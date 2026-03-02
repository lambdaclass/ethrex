// Implements the KZG point evaluation precompile (EIP-4844) at address 0x0A.
package precompiles

import (
	"crypto/sha256"
	"sync"

	gokzg4844 "github.com/crate-crypto/go-kzg-4844"
)

// KZG constants
const (
	kzgGasCost             uint64 = 50_000
	kzgInputLength                = 192
	kzgVersionedHashKZG    byte   = 0x01
)

// kzgReturnValue is the fixed output of the KZG precompile:
// FIELD_ELEMENTS_PER_BLOB (0x1000 = 4096) ++ BLS_MODULUS
var kzgReturnValue = [64]byte{
	0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
	0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
	0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
	0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00,
	0x73, 0xed, 0xa7, 0x53, 0x29, 0x9d, 0x7d, 0x48,
	0x33, 0x39, 0xd8, 0x08, 0x09, 0xa1, 0xd8, 0x05,
	0x53, 0xbd, 0xa4, 0x02, 0xff, 0xfe, 0x5b, 0xfe,
	0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x01,
}

// KZG error types
const (
	PrecompileErrorBlobInvalidInputLength PrecompileError = iota + 200
	PrecompileErrorBlobMismatchedVersion
	PrecompileErrorBlobVerifyKZGProofFailed
)

// kzgContext is the lazily-initialized KZG trusted setup context.
var (
	kzgContext     *gokzg4844.Context
	kzgContextOnce sync.Once
	kzgContextErr  error
)

// getKZGContext returns the shared KZG context, initializing it on first use.
func getKZGContext() (*gokzg4844.Context, error) {
	kzgContextOnce.Do(func() {
		kzgContext, kzgContextErr = gokzg4844.NewContext4096Secure()
	})
	return kzgContext, kzgContextErr
}

// kzgToVersionedHash computes the versioned hash from a KZG commitment.
// versioned_hash = VERSIONED_HASH_VERSION_KZG || sha256(commitment)[1:]
func kzgToVersionedHash(commitment []byte) [32]byte {
	hash := sha256.Sum256(commitment)
	hash[0] = kzgVersionedHashKZG
	return hash
}

// KzgPointEvaluationRun implements the KZG point evaluation precompile (address 0x0A).
//
// Input (192 bytes):
//   | versioned_hash (32) | z (32) | y (32) | commitment (48) | proof (48) |
//
// Output (64 bytes):
//   | FIELD_ELEMENTS_PER_BLOB (32) | BLS_MODULUS (32) |
func KzgPointEvaluationRun(input []byte, gasLimit uint64) PrecompileResult {
	if gasLimit < kzgGasCost {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}

	if len(input) != kzgInputLength {
		return PrecompileErr(PrecompileErrorBlobInvalidInputLength)
	}

	// Parse input fields
	versionedHash := input[0:32]
	z := input[32:64]
	y := input[64:96]
	commitment := input[96:144]
	proof := input[144:192]

	// Verify commitment matches versioned_hash
	computedHash := kzgToVersionedHash(commitment)
	for i := 0; i < 32; i++ {
		if versionedHash[i] != computedHash[i] {
			return PrecompileErr(PrecompileErrorBlobMismatchedVersion)
		}
	}

	// Get KZG context
	ctx, err := getKZGContext()
	if err != nil {
		return PrecompileErr(PrecompileErrorFatal)
	}

	// Convert to KZG types
	var kzgCommitment gokzg4844.KZGCommitment
	copy(kzgCommitment[:], commitment)

	var kzgProof gokzg4844.KZGProof
	copy(kzgProof[:], proof)

	var zScalar gokzg4844.Scalar
	copy(zScalar[:], z)

	var yScalar gokzg4844.Scalar
	copy(yScalar[:], y)

	// Verify KZG proof
	verifyErr := ctx.VerifyKZGProof(kzgCommitment, zScalar, yScalar, kzgProof)
	if verifyErr != nil {
		return PrecompileErr(PrecompileErrorBlobVerifyKZGProofFailed)
	}

	return PrecompileOk(NewPrecompileOutput(kzgGasCost, kzgReturnValue[:]))
}
