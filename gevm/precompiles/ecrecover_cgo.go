//go:build ignore

package precompiles

import (
	keccak "github.com/Giulio2002/fastkeccak"
	"github.com/Giulio2002/gevm/internal/secp256k1"
)

// Ecrecover performs secp256k1 ECDSA recovery using libsecp256k1 (CGO).
// Returns the 32-byte padded address (12 zero bytes + 20-byte address) and true on success.
func Ecrecover(sig [64]byte, recid byte, msgHash [32]byte) ([32]byte, bool) {
	var result [32]byte

	// Build 65-byte signature: R(32) || S(32) || V(1)
	var sig65 [65]byte
	copy(sig65[0:64], sig[:])
	sig65[64] = recid

	pubkey, err := secp256k1.RecoverPubkey(msgHash[:], sig65[:])
	if err != nil {
		return result, false
	}

	// pubkey is 65 bytes: 0x04 || X(32) || Y(32)
	// Keccak256 of the 64 bytes after the 0x04 prefix
	hash := keccak.Sum256(pubkey[1:])

	// Address is the last 20 bytes, left-padded with zeros to 32 bytes
	copy(result[12:], hash[12:])
	return result, true
}
