package precompiles

import (
	keccak "github.com/Giulio2002/fastkeccak"
	"github.com/decred/dcrd/dcrec/secp256k1/v4"
	"github.com/decred/dcrd/dcrec/secp256k1/v4/ecdsa"
)

// Ecrecover performs secp256k1 ECDSA recovery using pure Go (dcrd).
// Returns the 32-byte padded address (12 zero bytes + 20-byte address) and true on success.
func Ecrecover(sig [64]byte, recid byte, msgHash [32]byte) ([32]byte, bool) {
	var result [32]byte

	// Parse R and S from the signature
	var r, s secp256k1.ModNScalar
	if overflow := r.SetByteSlice(sig[0:32]); overflow {
		return result, false
	}
	if overflow := s.SetByteSlice(sig[32:64]); overflow {
		return result, false
	}

	// R and S must not be zero
	if r.IsZero() || s.IsZero() {
		return result, false
	}

	// Recover the public key using dcrd's compact signature recovery
	// dcrd's RecoverCompact expects a 65-byte signature: [recoveryFlag, R(32), S(32)]
	// where recoveryFlag encodes the recovery ID and compression flag
	var compactSig [65]byte
	// The recovery flag for uncompressed key with recid 0 = 27, recid 1 = 28
	compactSig[0] = 27 + recid
	copy(compactSig[1:33], sig[0:32])
	copy(compactSig[33:65], sig[32:64])

	pubKey, _, err := ecdsa.RecoverCompact(compactSig[:], msgHash[:])
	if err != nil {
		return result, false
	}

	// Serialize uncompressed key (65 bytes: 0x04 || X(32) || Y(32))
	// Keccak256 of the 64 bytes after the 0x04 prefix
	uncompressed := pubKey.SerializeUncompressed()
	hash := keccak.Sum256(uncompressed[1:])

	// Address is the last 20 bytes, left-padded with zeros to 32 bytes
	copy(result[12:], hash[12:])
	return result, true
}
