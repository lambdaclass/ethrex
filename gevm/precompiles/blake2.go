// Implements the BLAKE2F compression function precompile (address 0x09, EIP-152).
package precompiles

import (
	"encoding/binary"
	"math/bits"
)

const (
	blake2InputLength = 213
	blake2FRound      = 1 // gas per round
)

// BLAKE2b initialization vectors.
var blake2IV = [8]uint64{
	0x6a09e667f3bcc908, 0xbb67ae8584caa73b,
	0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1,
	0x510e527fade682d1, 0x9b05688c2b3e6c1f,
	0x1f83d9abfb41bd6b, 0x5be0cd19137e2179,
}

// Precomputed BLAKE2b sigma schedule, reordered for the inlined G pattern.
// Each round's 16 entries are grouped as:
//
//	[0..3]  = 1st message index for column G0-G3
//	[4..7]  = 2nd message index for column G0-G3
//	[8..11] = 1st message index for diagonal G4-G7
//	[12..15]= 2nd message index for diagonal G4-G7
var blake2Precomputed = [10][16]byte{
	{0, 2, 4, 6, 1, 3, 5, 7, 8, 10, 12, 14, 9, 11, 13, 15},
	{14, 4, 9, 13, 10, 8, 15, 6, 1, 0, 11, 5, 12, 2, 7, 3},
	{11, 12, 5, 15, 8, 0, 2, 13, 10, 3, 7, 9, 14, 6, 1, 4},
	{7, 3, 13, 11, 9, 1, 12, 14, 2, 5, 4, 15, 6, 10, 0, 8},
	{9, 5, 2, 10, 0, 7, 4, 15, 14, 11, 6, 3, 1, 12, 8, 13},
	{2, 6, 0, 8, 12, 10, 11, 3, 4, 7, 15, 1, 13, 5, 14, 9},
	{12, 1, 14, 4, 5, 15, 13, 10, 0, 6, 9, 8, 7, 3, 2, 11},
	{13, 7, 12, 3, 11, 14, 1, 9, 5, 15, 8, 2, 0, 4, 6, 10},
	{6, 14, 11, 0, 15, 9, 3, 8, 12, 13, 1, 10, 2, 7, 4, 5},
	{10, 8, 7, 1, 2, 4, 6, 5, 15, 9, 3, 13, 11, 14, 12, 0},
}

// Blake2FRun implements the BLAKE2F precompile (address 0x09).
func Blake2FRun(input []byte, gasLimit uint64) PrecompileResult {
	if len(input) != blake2InputLength {
		return PrecompileErr(PrecompileErrorBlake2WrongLength)
	}

	// Parse number of rounds (4 bytes, big-endian)
	rounds := binary.BigEndian.Uint32(input[0:4])
	gasUsed := uint64(rounds) * blake2FRound
	if gasUsed > gasLimit {
		return PrecompileErr(PrecompileErrorOutOfGas)
	}

	// Parse final block flag
	switch input[212] {
	case 0:
		// not final
	case 1:
		// final
	default:
		return PrecompileErr(PrecompileErrorBlake2WrongFinalIndicatorFlag)
	}
	final_ := input[212] == 1

	// Parse state vector h (8 × uint64, little-endian)
	var h [8]uint64
	for i := 0; i < 8; i++ {
		h[i] = binary.LittleEndian.Uint64(input[4+i*8 : 4+(i+1)*8])
	}

	// Parse message block m (16 × uint64, little-endian)
	var m [16]uint64
	for i := 0; i < 16; i++ {
		m[i] = binary.LittleEndian.Uint64(input[68+i*8 : 68+(i+1)*8])
	}

	// Parse offset counters
	t0 := binary.LittleEndian.Uint64(input[196:204])
	t1 := binary.LittleEndian.Uint64(input[204:212])

	// Perform compression
	blake2Compress(&h, &m, t0, t1, final_, rounds)

	// Encode output (8 × uint64, little-endian → 64 bytes)
	var out [64]byte
	for i := 0; i < 8; i++ {
		binary.LittleEndian.PutUint64(out[i*8:(i+1)*8], h[i])
	}

	return PrecompileOk(NewPrecompileOutput(gasUsed, out[:]))
}

// blake2Compress performs the BLAKE2b compression function.
// Uses local variables instead of array indexing to eliminate bounds checks,
// and math/bits.RotateLeft64 which compiles to a single ROR instruction.
func blake2Compress(h *[8]uint64, m *[16]uint64, c0, c1 uint64, final_ bool, rounds uint32) {
	v0, v1, v2, v3, v4, v5, v6, v7 := h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]
	v8, v9, v10, v11, v12, v13, v14, v15 := blake2IV[0], blake2IV[1], blake2IV[2], blake2IV[3], blake2IV[4], blake2IV[5], blake2IV[6], blake2IV[7]

	v12 ^= c0
	v13 ^= c1
	if final_ {
		v14 = ^v14
	}

	for i := uint32(0); i < rounds; i++ {
		s := &blake2Precomputed[i%10]

		// Column step: G(0,4,8,12), G(1,5,9,13), G(2,6,10,14), G(3,7,11,15)
		v0 += m[s[0]]
		v0 += v4
		v12 ^= v0
		v12 = bits.RotateLeft64(v12, -32)
		v8 += v12
		v4 ^= v8
		v4 = bits.RotateLeft64(v4, -24)
		v1 += m[s[1]]
		v1 += v5
		v13 ^= v1
		v13 = bits.RotateLeft64(v13, -32)
		v9 += v13
		v5 ^= v9
		v5 = bits.RotateLeft64(v5, -24)
		v2 += m[s[2]]
		v2 += v6
		v14 ^= v2
		v14 = bits.RotateLeft64(v14, -32)
		v10 += v14
		v6 ^= v10
		v6 = bits.RotateLeft64(v6, -24)
		v3 += m[s[3]]
		v3 += v7
		v15 ^= v3
		v15 = bits.RotateLeft64(v15, -32)
		v11 += v15
		v7 ^= v11
		v7 = bits.RotateLeft64(v7, -24)

		v0 += m[s[4]]
		v0 += v4
		v12 ^= v0
		v12 = bits.RotateLeft64(v12, -16)
		v8 += v12
		v4 ^= v8
		v4 = bits.RotateLeft64(v4, -63)
		v1 += m[s[5]]
		v1 += v5
		v13 ^= v1
		v13 = bits.RotateLeft64(v13, -16)
		v9 += v13
		v5 ^= v9
		v5 = bits.RotateLeft64(v5, -63)
		v2 += m[s[6]]
		v2 += v6
		v14 ^= v2
		v14 = bits.RotateLeft64(v14, -16)
		v10 += v14
		v6 ^= v10
		v6 = bits.RotateLeft64(v6, -63)
		v3 += m[s[7]]
		v3 += v7
		v15 ^= v3
		v15 = bits.RotateLeft64(v15, -16)
		v11 += v15
		v7 ^= v11
		v7 = bits.RotateLeft64(v7, -63)

		// Diagonal step: G(0,5,10,15), G(1,6,11,12), G(2,7,8,13), G(3,4,9,14)
		v0 += m[s[8]]
		v0 += v5
		v15 ^= v0
		v15 = bits.RotateLeft64(v15, -32)
		v10 += v15
		v5 ^= v10
		v5 = bits.RotateLeft64(v5, -24)
		v1 += m[s[9]]
		v1 += v6
		v12 ^= v1
		v12 = bits.RotateLeft64(v12, -32)
		v11 += v12
		v6 ^= v11
		v6 = bits.RotateLeft64(v6, -24)
		v2 += m[s[10]]
		v2 += v7
		v13 ^= v2
		v13 = bits.RotateLeft64(v13, -32)
		v8 += v13
		v7 ^= v8
		v7 = bits.RotateLeft64(v7, -24)
		v3 += m[s[11]]
		v3 += v4
		v14 ^= v3
		v14 = bits.RotateLeft64(v14, -32)
		v9 += v14
		v4 ^= v9
		v4 = bits.RotateLeft64(v4, -24)

		v0 += m[s[12]]
		v0 += v5
		v15 ^= v0
		v15 = bits.RotateLeft64(v15, -16)
		v10 += v15
		v5 ^= v10
		v5 = bits.RotateLeft64(v5, -63)
		v1 += m[s[13]]
		v1 += v6
		v12 ^= v1
		v12 = bits.RotateLeft64(v12, -16)
		v11 += v12
		v6 ^= v11
		v6 = bits.RotateLeft64(v6, -63)
		v2 += m[s[14]]
		v2 += v7
		v13 ^= v2
		v13 = bits.RotateLeft64(v13, -16)
		v8 += v13
		v7 ^= v8
		v7 = bits.RotateLeft64(v7, -63)
		v3 += m[s[15]]
		v3 += v4
		v14 ^= v3
		v14 = bits.RotateLeft64(v14, -16)
		v9 += v14
		v4 ^= v9
		v4 = bits.RotateLeft64(v4, -63)
	}

	h[0] ^= v0 ^ v8
	h[1] ^= v1 ^ v9
	h[2] ^= v2 ^ v10
	h[3] ^= v3 ^ v11
	h[4] ^= v4 ^ v12
	h[5] ^= v5 ^ v13
	h[6] ^= v6 ^ v14
	h[7] ^= v7 ^ v15
}
