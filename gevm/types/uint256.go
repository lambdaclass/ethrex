//
// Uint256 is a 256-bit unsigned integer stored as 4 little-endian uint64 limbs.
// Limb ordering: [0] = least significant, [3] = most significant.
package types

import (
	"encoding/binary"
	"fmt"
	"math/big"
	"math/bits"
)

// Uint256 represents a 256-bit unsigned integer as 4 little-endian uint64 limbs.
type Uint256 [4]uint64

// Zero/max/special constants.
var (
	U256Zero = Uint256{}
	U256One  = Uint256{1, 0, 0, 0}
	U256Max  = Uint256{
		0xffffffffffffffff,
		0xffffffffffffffff,
		0xffffffffffffffff,
		0xffffffffffffffff,
	}
	// MAX_POSITIVE_VALUE is the maximum positive signed 256-bit integer (2^255 - 1).
	U256MaxPositiveI256 = Uint256{
		0xffffffffffffffff,
		0xffffffffffffffff,
		0xffffffffffffffff,
		0x7fffffffffffffff,
	}
	// MIN_NEGATIVE_VALUE is -2^255 in two's complement.
	U256MinNegativeI256 = Uint256{
		0x0000000000000000,
		0x0000000000000000,
		0x0000000000000000,
		0x8000000000000000,
	}
	u256Thirty1 = Uint256{31, 0, 0, 0}
)

// U256From creates a Uint256 from a uint64.
func U256From(v uint64) Uint256 {
	return Uint256{v, 0, 0, 0}
}

// U256FromLimbs creates a Uint256 from 4 little-endian uint64 limbs.
func U256FromLimbs(a, b, c, d uint64) Uint256 {
	return Uint256{a, b, c, d}
}

// IsZero returns true if the value is zero.
func (u Uint256) IsZero() bool {
	return u[0] == 0 && u[1] == 0 && u[2] == 0 && u[3] == 0
}

// IsOne returns true if the value is one.
func (u Uint256) IsOne() bool {
	return u[0] == 1 && u[1] == 0 && u[2] == 0 && u[3] == 0
}

// Bit returns the bit value at position n (0-indexed from LSB).
func (u Uint256) Bit(n uint) bool {
	if n >= 256 {
		return false
	}
	limb := n / 64
	bit := n % 64
	return (u[limb]>>bit)&1 == 1
}

// Byte returns the byte at position n (0=least significant byte, 31=most significant byte).
// Returns the byte at the given little-endian byte index.
func (u Uint256) Byte(n uint) byte {
	if n >= 32 {
		return 0
	}
	limb := n / 8
	byteIdx := n % 8
	return byte(u[limb] >> (byteIdx * 8))
}

// ByteBE returns the byte at big-endian position n (0=MSB, 31=LSB).
// This is what the EVM BYTE opcode uses.
func (u Uint256) ByteBE(n uint) byte {
	if n >= 32 {
		return 0
	}
	return u.Byte(31 - n)
}

// LeadingZeros returns the number of leading zero bits.
func (u Uint256) LeadingZeros() uint {
	if u[3] != 0 {
		return uint(bits.LeadingZeros64(u[3]))
	}
	if u[2] != 0 {
		return 64 + uint(bits.LeadingZeros64(u[2]))
	}
	if u[1] != 0 {
		return 128 + uint(bits.LeadingZeros64(u[1]))
	}
	if u[0] != 0 {
		return 192 + uint(bits.LeadingZeros64(u[0]))
	}
	return 256
}

// Cmp compares two Uint256 values. Returns -1, 0, or 1.
func (u Uint256) Cmp(other Uint256) int {
	if u[3] != other[3] {
		if u[3] < other[3] {
			return -1
		}
		return 1
	}
	if u[2] != other[2] {
		if u[2] < other[2] {
			return -1
		}
		return 1
	}
	if u[1] != other[1] {
		if u[1] < other[1] {
			return -1
		}
		return 1
	}
	if u[0] != other[0] {
		if u[0] < other[0] {
			return -1
		}
		return 1
	}
	return 0
}

// Eq returns true if both values are equal.
func (u Uint256) Eq(other Uint256) bool {
	return u[0] == other[0] && u[1] == other[1] && u[2] == other[2] && u[3] == other[3]
}

// Lt returns true if u < other (unsigned).
func (u Uint256) Lt(other Uint256) bool {
	if u[3] != other[3] {
		return u[3] < other[3]
	}
	if u[2] != other[2] {
		return u[2] < other[2]
	}
	if u[1] != other[1] {
		return u[1] < other[1]
	}
	return u[0] < other[0]
}

// Gt returns true if u > other (unsigned).
func (u Uint256) Gt(other Uint256) bool {
	if u[3] != other[3] {
		return u[3] > other[3]
	}
	if u[2] != other[2] {
		return u[2] > other[2]
	}
	if u[1] != other[1] {
		return u[1] > other[1]
	}
	return u[0] > other[0]
}

// --- Conversions ---

// ToBytes32 encodes as big-endian 32-byte array.
func (u Uint256) ToBytes32() [32]byte {
	var b [32]byte
	binary.BigEndian.PutUint64(b[0:8], u[3])
	binary.BigEndian.PutUint64(b[8:16], u[2])
	binary.BigEndian.PutUint64(b[16:24], u[1])
	binary.BigEndian.PutUint64(b[24:32], u[0])
	return b
}

// PutBytes32 writes the Uint256 as big-endian into dst (must be >= 32 bytes).
func (u Uint256) PutBytes32(dst []byte) {
	_ = dst[31] // bounds check hint
	binary.BigEndian.PutUint64(dst[0:8], u[3])
	binary.BigEndian.PutUint64(dst[8:16], u[2])
	binary.BigEndian.PutUint64(dst[16:24], u[1])
	binary.BigEndian.PutUint64(dst[24:32], u[0])
}

// U256FromBytes32 decodes from a big-endian 32-byte array.
func U256FromBytes32(b [32]byte) Uint256 {
	return Uint256{
		binary.BigEndian.Uint64(b[24:32]),
		binary.BigEndian.Uint64(b[16:24]),
		binary.BigEndian.Uint64(b[8:16]),
		binary.BigEndian.Uint64(b[0:8]),
	}
}

// U256FromBytes decodes from a big-endian byte slice (up to 32 bytes, zero-padded on left).
func U256FromBytes(b []byte) Uint256 {
	n := len(b)
	if n == 0 {
		return U256Zero
	}
	// Fast path: fits in single limb (PUSH3-PUSH8)
	if n <= 8 {
		var v uint64
		for _, x := range b {
			v = v<<8 | uint64(x)
		}
		return Uint256{v, 0, 0, 0}
	}
	if n > 32 {
		b = b[n-32:]
		n = 32
	}
	var padded [32]byte
	copy(padded[32-n:], b)
	return U256FromBytes32(padded)
}

// ToBig converts to *big.Int. Not for hot paths.
func (u Uint256) ToBig() *big.Int {
	b := u.ToBytes32()
	return new(big.Int).SetBytes(b[:])
}

// U256FromBig converts from *big.Int. Not for hot paths. Truncates to 256 bits.
func U256FromBig(v *big.Int) Uint256 {
	if v.Sign() == 0 {
		return U256Zero
	}
	b := v.Bytes()
	if len(b) > 32 {
		b = b[len(b)-32:]
	}
	return U256FromBytes(b)
}

// ToAddress extracts the low 20 bytes as an Address.
func (u Uint256) ToAddress() Address {
	b := u.ToBytes32()
	var addr Address
	copy(addr[:], b[12:32])
	return addr
}

// AsUsize returns the low 64-bit limb as a usize-equivalent.
// Saturates to max uint64 if value exceeds uint64 range.
func (u Uint256) AsUsize() uint64 {
	if u[1] != 0 || u[2] != 0 || u[3] != 0 {
		return ^uint64(0) // saturate
	}
	return u[0]
}

// AsUsizeSaturated is an alias for AsUsize for clarity at call sites.
func (u Uint256) AsUsizeSaturated() uint64 {
	return u.AsUsize()
}

// LowU64 returns the least significant 64-bit limb.
func (u Uint256) LowU64() uint64 {
	return u[0]
}

// String returns the decimal representation.
func (u Uint256) String() string {
	if u.IsZero() {
		return "0"
	}
	return u.ToBig().String()
}

// Hex returns the hexadecimal representation with 0x prefix.
func (u Uint256) Hex() string {
	if u.IsZero() {
		return "0x0"
	}
	return fmt.Sprintf("0x%x", u.ToBig())
}

// --- Arithmetic (wrapping, modular 2^256) ---

// Add returns u + v (wrapping).
func (u Uint256) Add(v Uint256) Uint256 {
	var result Uint256
	var carry uint64
	result[0], carry = bits.Add64(u[0], v[0], 0)
	result[1], carry = bits.Add64(u[1], v[1], carry)
	result[2], carry = bits.Add64(u[2], v[2], carry)
	result[3], _ = bits.Add64(u[3], v[3], carry)
	return result
}

// OverflowingAdd returns u + v and whether it overflowed.
func (u Uint256) OverflowingAdd(v Uint256) (Uint256, bool) {
	var result Uint256
	var carry uint64
	result[0], carry = bits.Add64(u[0], v[0], 0)
	result[1], carry = bits.Add64(u[1], v[1], carry)
	result[2], carry = bits.Add64(u[2], v[2], carry)
	result[3], carry = bits.Add64(u[3], v[3], carry)
	return result, carry != 0
}

// Sub returns u - v (wrapping).
func (u Uint256) Sub(v Uint256) Uint256 {
	var result Uint256
	var borrow uint64
	result[0], borrow = bits.Sub64(u[0], v[0], 0)
	result[1], borrow = bits.Sub64(u[1], v[1], borrow)
	result[2], borrow = bits.Sub64(u[2], v[2], borrow)
	result[3], _ = bits.Sub64(u[3], v[3], borrow)
	return result
}

// OverflowingSub returns u - v and whether it underflowed.
func (u Uint256) OverflowingSub(v Uint256) (Uint256, bool) {
	var result Uint256
	var borrow uint64
	result[0], borrow = bits.Sub64(u[0], v[0], 0)
	result[1], borrow = bits.Sub64(u[1], v[1], borrow)
	result[2], borrow = bits.Sub64(u[2], v[2], borrow)
	result[3], borrow = bits.Sub64(u[3], v[3], borrow)
	return result, borrow != 0
}

// Mul returns u * v (wrapping, lower 256 bits only).
func (u Uint256) Mul(v Uint256) Uint256 {
	// Fast path: both fit in a single limb.
	if u[1]|u[2]|u[3] == 0 && v[1]|v[2]|v[3] == 0 {
		hi, lo := bits.Mul64(u[0], v[0])
		return Uint256{lo, hi, 0, 0}
	}

	// Use a 192-bit accumulator (a0, a1, a2) to handle carry overflow
	// correctly when summing multiple partial products per result limb.
	// Each column accumulates partial products and propagates carry.
	var result Uint256
	var a0, a1, a2 uint64 // 192-bit accumulator
	var hi, lo, c uint64

	// Column 0: u[0]*v[0]
	hi, lo = bits.Mul64(u[0], v[0])
	a0 = lo
	a1 = hi

	result[0] = a0
	a0, a1, a2 = a1, a2, 0

	// Column 1: u[0]*v[1] + u[1]*v[0]
	hi, lo = bits.Mul64(u[0], v[1])
	a0, c = bits.Add64(a0, lo, 0)
	a1, c = bits.Add64(a1, hi, c)
	a2 += c

	hi, lo = bits.Mul64(u[1], v[0])
	a0, c = bits.Add64(a0, lo, 0)
	a1, c = bits.Add64(a1, hi, c)
	a2 += c

	result[1] = a0
	a0, a1, a2 = a1, a2, 0

	// Column 2: u[0]*v[2] + u[1]*v[1] + u[2]*v[0]
	hi, lo = bits.Mul64(u[0], v[2])
	a0, c = bits.Add64(a0, lo, 0)
	a1, c = bits.Add64(a1, hi, c)
	a2 += c

	hi, lo = bits.Mul64(u[1], v[1])
	a0, c = bits.Add64(a0, lo, 0)
	a1, c = bits.Add64(a1, hi, c)
	a2 += c

	hi, lo = bits.Mul64(u[2], v[0])
	a0, c = bits.Add64(a0, lo, 0)
	a1, c = bits.Add64(a1, hi, c)
	a2 += c

	result[2] = a0

	// Column 3: u[0]*v[3] + u[1]*v[2] + u[2]*v[1] + u[3]*v[0] + carry
	// Only need the low 64 bits (everything above is discarded).
	result[3] = a1 + u[0]*v[3] + u[1]*v[2] + u[2]*v[1] + u[3]*v[0]

	return result
}

// addCarry adds a + b + carry, returns (sum, carry_out).
func addCarry(a, b, carry uint64) (uint64, uint64) {
	sum, c1 := bits.Add64(a, b, 0)
	sum2, c2 := bits.Add64(sum, carry, 0)
	return sum2, c1 + c2
}

// Neg returns the two's complement negation (wrapping).
func (u Uint256) Neg() Uint256 {
	inv := Uint256{^u[0], ^u[1], ^u[2], ^u[3]}
	return inv.Add(U256One)
}

// Div returns u / v. Returns 0 if v is zero.
func (u Uint256) Div(v Uint256) Uint256 {
	if v.IsZero() {
		return U256Zero
	}
	if v.Gt(u) {
		return U256Zero
	}
	if v.Eq(u) {
		return U256One
	}
	// Fast path: both fit in a single limb
	if u[1]|u[2]|u[3] == 0 && v[1]|v[2]|v[3] == 0 {
		return Uint256{u[0] / v[0], 0, 0, 0}
	}
	var quot Uint256
	udivrem(quot[:], u[:], &v, nil)
	return quot
}

// Mod returns u % v. Returns 0 if v is zero.
func (u Uint256) Mod(v Uint256) Uint256 {
	if v.IsZero() {
		return U256Zero
	}
	if v.Gt(u) {
		return u
	}
	if v.Eq(u) {
		return U256Zero
	}
	// Fast path: both fit in a single limb.
	if u[1]|u[2]|u[3] == 0 && v[1]|v[2]|v[3] == 0 {
		return Uint256{u[0] % v[0], 0, 0, 0}
	}
	var quot, rem Uint256
	udivrem(quot[:], u[:], &v, &rem)
	return rem
}

// divmod performs unsigned division and returns (quotient, remainder).
// Precondition: d is not zero.
func (u Uint256) divmod(d Uint256) (Uint256, Uint256) {
	if d.Gt(u) {
		return U256Zero, u
	}
	if d.Eq(u) {
		return U256One, U256Zero
	}
	var quot, rem Uint256
	udivrem(quot[:], u[:], &d, &rem)
	return quot, rem
}

// divmodSmall divides by a single uint64.
func (u Uint256) divmodSmall(d uint64) (Uint256, Uint256) {
	var q Uint256
	var rem uint64
	for i := 3; i >= 0; i-- {
		combined := uint128{rem, u[uint(i)]}
		q[i], rem = combined.divmod64(d)
	}
	return q, Uint256{rem, 0, 0, 0}
}

// uint128 helper for 128-bit / 64-bit division.
type uint128 struct {
	hi, lo uint64
}

func (u uint128) divmod64(d uint64) (uint64, uint64) {
	q, r := bits.Div64(u.hi, u.lo, d)
	return q, r
}

// divmodBig performs division for multi-limb divisors using Knuth's Algorithm D.
// Zero allocations.
func (u Uint256) divmodBig(d Uint256) (Uint256, Uint256) {
	var quot Uint256
	var rem Uint256
	udivrem(quot[:], u[:], &d, &rem)
	return quot, rem
}

// reciprocal2by1 computes <^d, ^0> / d.
func reciprocal2by1(d uint64) uint64 {
	reciprocal, _ := bits.Div64(^d, ^uint64(0), d)
	return reciprocal
}

// udivrem2by1 divides <uh, ul> / d using precomputed reciprocal.
// Based on "Improved division by invariant integers", Algorithm 4.
func udivrem2by1(uh, ul, d, reciprocal uint64) (uint64, uint64) {
	qh, ql := bits.Mul64(reciprocal, uh)
	ql, carry := bits.Add64(ql, ul, 0)
	qh, _ = bits.Add64(qh, uh, carry)
	qh++
	r := ul - qh*d
	if r > ql {
		qh--
		r += d
	}
	if r >= d {
		qh++
		r -= d
	}
	return qh, r
}

// udivremBy1 divides u by single normalized word d.
func udivremBy1(quot, u []uint64, d uint64) uint64 {
	reciprocal := reciprocal2by1(d)
	rem := u[len(u)-1]
	for j := len(u) - 2; j >= 0; j-- {
		quot[j], rem = udivrem2by1(rem, u[j], d, reciprocal)
	}
	return rem
}

// subMulTo computes x -= y * multiplier.
func subMulTo(x, y []uint64, multiplier uint64) uint64 {
	var borrow uint64
	_ = x[len(y)-1]
	for i := 0; i < len(y); i++ {
		s, carry1 := bits.Sub64(x[i], borrow, 0)
		ph, pl := bits.Mul64(y[i], multiplier)
		t, carry2 := bits.Sub64(s, pl, 0)
		x[i] = t
		borrow = ph + carry1 + carry2
	}
	return borrow
}

// addTo computes x += y with carry propagation.
func addTo(x, y []uint64) uint64 {
	var carry uint64
	_ = x[len(y)-1]
	for i := 0; i < len(y); i++ {
		x[i], carry = bits.Add64(x[i], y[i], carry)
	}
	return carry
}

// udivremKnuth implements multi-word division (Knuth's Algorithm D).
func udivremKnuth(quot, u, d []uint64) {
	dh := d[len(d)-1]
	dl := d[len(d)-2]
	reciprocal := reciprocal2by1(dh)
	for j := len(u) - len(d) - 1; j >= 0; j-- {
		u2 := u[j+len(d)]
		u1 := u[j+len(d)-1]
		u0 := u[j+len(d)-2]
		var qhat uint64
		if u2 >= dh {
			qhat = ^uint64(0)
		} else {
			var rhat uint64
			qhat, rhat = udivrem2by1(u2, u1, dh, reciprocal)
			ph, pl := bits.Mul64(qhat, dl)
			if ph > rhat || (ph == rhat && pl > u0) {
				qhat--
			}
		}
		borrow := subMulTo(u[j:], d, qhat)
		u[j+len(d)] = u2 - borrow
		if u2 < borrow {
			qhat--
			u[j+len(d)] += addTo(u[j:], d)
		}
		quot[j] = qhat
	}
}

// udivrem divides u by d producing quotient and remainder.
// Knuth's Algorithm D with normalization. Zero allocations.
func udivrem(quot []uint64, u []uint64, d *Uint256, rem *Uint256) {
	var dLen int
	for i := 3; i >= 0; i-- {
		if d[i] != 0 {
			dLen = i + 1
			break
		}
	}

	shift := uint(bits.LeadingZeros64(d[dLen-1]))

	var dnStorage Uint256
	dn := dnStorage[:dLen]
	for i := dLen - 1; i > 0; i-- {
		dn[i] = (d[i] << shift) | (d[i-1] >> (64 - shift))
	}
	dn[0] = d[0] << shift

	var uLen int
	for i := len(u) - 1; i >= 0; i-- {
		if u[i] != 0 {
			uLen = i + 1
			break
		}
	}

	if uLen < dLen {
		if rem != nil {
			copy(rem[:], u)
		}
		return
	}

	var unStorage [9]uint64
	un := unStorage[:uLen+1]
	un[uLen] = u[uLen-1] >> (64 - shift)
	for i := uLen - 1; i > 0; i-- {
		un[i] = (u[i] << shift) | (u[i-1] >> (64 - shift))
	}
	un[0] = u[0] << shift

	if dLen == 1 {
		r := udivremBy1(quot, un, dn[0])
		if rem != nil {
			*rem = Uint256{}
			rem[0] = r >> shift
		}
		return
	}

	udivremKnuth(quot, un, dn)

	if rem != nil {
		*rem = Uint256{}
		for i := 0; i < dLen-1; i++ {
			rem[i] = (un[i] >> shift) | (un[i+1] << (64 - shift))
		}
		rem[dLen-1] = un[dLen-1] >> shift
	}
}

// Exp returns u^exp (wrapping). 0^0 = 1.
func (u Uint256) Exp(exp Uint256) Uint256 {
	if exp.IsZero() {
		return U256One
	}
	if u.IsZero() {
		return U256Zero
	}
	if exp.IsOne() {
		return u
	}

	// Process exponent word-by-word to avoid shifting the entire Uint256
	// each iteration. Each limb is processed with simple word >>= 1.
	// The last squaring is skipped since `base` is not used after.
	result := U256One
	base := u
	expBitLen := 0
	for i := 3; i >= 0; i-- {
		if exp[i] != 0 {
			expBitLen = i*64 + bits.Len64(exp[i])
			break
		}
	}

	curBit := 0
	word := exp[0]
	for ; curBit < expBitLen && curBit < 64; curBit++ {
		if word&1 == 1 {
			result = result.Mul(base)
		}
		word >>= 1
		if curBit+1 < expBitLen {
			base = base.Mul(base)
		}
	}
	word = exp[1]
	for ; curBit < expBitLen && curBit < 128; curBit++ {
		if word&1 == 1 {
			result = result.Mul(base)
		}
		word >>= 1
		if curBit+1 < expBitLen {
			base = base.Mul(base)
		}
	}
	word = exp[2]
	for ; curBit < expBitLen && curBit < 192; curBit++ {
		if word&1 == 1 {
			result = result.Mul(base)
		}
		word >>= 1
		if curBit+1 < expBitLen {
			base = base.Mul(base)
		}
	}
	word = exp[3]
	for ; curBit < expBitLen; curBit++ {
		if word&1 == 1 {
			result = result.Mul(base)
		}
		word >>= 1
		if curBit+1 < expBitLen {
			base = base.Mul(base)
		}
	}
	return result
}

// AddMod returns (u + v) % m. Returns 0 if m is zero.
// Zero allocations.
func (u Uint256) AddMod(v Uint256, m Uint256) Uint256 {
	if m.IsZero() {
		return U256Zero
	}
	// 257-bit addition: sum = u + v, with possible carry into a 5th limb.
	var sum [5]uint64
	var c uint64
	sum[0], c = bits.Add64(u[0], v[0], 0)
	sum[1], c = bits.Add64(u[1], v[1], c)
	sum[2], c = bits.Add64(u[2], v[2], c)
	sum[3], c = bits.Add64(u[3], v[3], c)
	sum[4] = c

	// If no overflow and sum < m, return sum directly.
	if sum[4] == 0 {
		s := Uint256{sum[0], sum[1], sum[2], sum[3]}
		if s.Lt(m) {
			return s
		}
		// sum fits in 256 bits, use standard Mod.
		return s.Mod(m)
	}

	// Overflow: 257-bit sum needs widened division.
	var quot [5]uint64
	var rem Uint256
	udivrem(quot[:], sum[:], &m, &rem)
	return rem
}

// MulMod returns (u * v) % m. Returns 0 if m is zero.
// Zero allocations.
func (u Uint256) MulMod(v Uint256, m Uint256) Uint256 {
	if m.IsZero() {
		return U256Zero
	}
	if u.IsZero() || v.IsZero() {
		return U256Zero
	}
	// Full 512-bit multiplication.
	var p [8]uint64
	umul(&u, &v, &p)

	// If high half is zero, result fits in 256 bits.
	if p[4]|p[5]|p[6]|p[7] == 0 {
		r := Uint256{p[0], p[1], p[2], p[3]}
		return r.Mod(m)
	}

	// 512-bit dividend mod 256-bit divisor.
	var quot [8]uint64
	var rem Uint256
	udivrem(quot[:], p[:], &m, &rem)
	return rem
}

// umul computes full 256x256 -> 512-bit multiplication.
// Result is stored in 8 uint64 limbs (little-endian).
func umul(x, y *Uint256, res *[8]uint64) {
	var carry uint64

	// Column 0: x[0]*y[0]
	carry, res[0] = bits.Mul64(x[0], y[0])

	// Column 1: x[1]*y[0] + x[0]*y[1]
	carry, res[1] = umulHop(carry, x[1], y[0])
	var carry2 uint64
	carry2, res[2] = umulHop(carry, x[2], y[0])
	var carry3 uint64
	carry3, res[3] = umulHop(carry2, x[3], y[0])

	carry, res[1] = umulHop(res[1], x[0], y[1])
	carry, res[2] = umulStep(res[2], x[1], y[1], carry)
	carry, res[3] = umulStep(res[3], x[2], y[1], carry)
	var carry4 uint64
	carry4, res[4] = umulStep(carry3, x[3], y[1], carry)

	carry, res[2] = umulHop(res[2], x[0], y[2])
	carry, res[3] = umulStep(res[3], x[1], y[2], carry)
	carry, res[4] = umulStep(res[4], x[2], y[2], carry)
	var carry5 uint64
	carry5, res[5] = umulStep(carry4, x[3], y[2], carry)

	carry, res[3] = umulHop(res[3], x[0], y[3])
	carry, res[4] = umulStep(res[4], x[1], y[3], carry)
	carry, res[5] = umulStep(res[5], x[2], y[3], carry)
	res[7], res[6] = umulStep(carry5, x[3], y[3], carry)
}

// umulHop computes (hi, lo) = z + x*y.
func umulHop(z, x, y uint64) (hi, lo uint64) {
	hi, lo = bits.Mul64(x, y)
	lo, carry := bits.Add64(lo, z, 0)
	hi, _ = bits.Add64(hi, 0, carry)
	return
}

// umulStep computes (hi, lo) = z + x*y + carry.
func umulStep(z, x, y, c uint64) (hi, lo uint64) {
	hi, lo = bits.Mul64(x, y)
	lo, c = bits.Add64(lo, c, 0)
	hi, _ = bits.Add64(hi, 0, c)
	lo, c = bits.Add64(lo, z, 0)
	hi, _ = bits.Add64(hi, 0, c)
	return
}

// --- Bitwise ---

// And returns u & v.
func (u Uint256) And(v Uint256) Uint256 {
	return Uint256{u[0] & v[0], u[1] & v[1], u[2] & v[2], u[3] & v[3]}
}

// Or returns u | v.
func (u Uint256) Or(v Uint256) Uint256 {
	return Uint256{u[0] | v[0], u[1] | v[1], u[2] | v[2], u[3] | v[3]}
}

// Xor returns u ^ v.
func (u Uint256) Xor(v Uint256) Uint256 {
	return Uint256{u[0] ^ v[0], u[1] ^ v[1], u[2] ^ v[2], u[3] ^ v[3]}
}

// Not returns ^u (bitwise complement).
func (u Uint256) Not() Uint256 {
	return Uint256{^u[0], ^u[1], ^u[2], ^u[3]}
}

// Shl returns u << n. Returns 0 if n >= 256.
func (u Uint256) Shl(n uint) Uint256 {
	switch {
	case n == 0:
		return u
	case n >= 256:
		return U256Zero
	case n >= 192:
		n -= 192
		return Uint256{0, 0, 0, u[0] << n}
	case n >= 128:
		n -= 128
		return Uint256{0, 0, u[0] << n, (u[1] << n) | (u[0] >> (64 - n))}
	case n >= 64:
		n -= 64
		return Uint256{0, u[0] << n, (u[1] << n) | (u[0] >> (64 - n)), (u[2] << n) | (u[1] >> (64 - n))}
	default:
		return Uint256{
			u[0] << n,
			(u[1] << n) | (u[0] >> (64 - n)),
			(u[2] << n) | (u[1] >> (64 - n)),
			(u[3] << n) | (u[2] >> (64 - n)),
		}
	}
}

// Shr returns u >> n (logical). Returns 0 if n >= 256.
func (u Uint256) Shr(n uint) Uint256 {
	switch {
	case n == 0:
		return u
	case n >= 256:
		return U256Zero
	case n >= 192:
		n -= 192
		return Uint256{u[3] >> n, 0, 0, 0}
	case n >= 128:
		n -= 128
		return Uint256{(u[2] >> n) | (u[3] << (64 - n)), u[3] >> n, 0, 0}
	case n >= 64:
		n -= 64
		return Uint256{(u[1] >> n) | (u[2] << (64 - n)), (u[2] >> n) | (u[3] << (64 - n)), u[3] >> n, 0}
	default:
		return Uint256{
			(u[0] >> n) | (u[1] << (64 - n)),
			(u[1] >> n) | (u[2] << (64 - n)),
			(u[2] >> n) | (u[3] << (64 - n)),
			u[3] >> n,
		}
	}
}

// Sar returns arithmetic right shift of u by n bits.
// Sign-extends from bit 255 if n >= 256.
func (u Uint256) Sar(n uint) Uint256 {
	negative := u[3]&(1<<63) != 0
	if n >= 256 {
		if negative {
			return U256Max
		}
		return U256Zero
	}
	if n == 0 {
		return u
	}
	// Logical shift right, then OR in sign bits for negative values.
	r := u.Shr(n)
	if negative {
		// Set the top n bits. Equivalent to OR with (U256Max << (256 - n)).
		topBits := 256 - n
		limbIdx := topBits / 64
		bitIdx := topBits % 64
		// Fill all limbs above limbIdx with 0xFF..FF.
		for i := limbIdx + 1; i < 4; i++ {
			r[i] = ^uint64(0)
		}
		// Set the upper bits in the boundary limb.
		if bitIdx > 0 {
			r[limbIdx] |= ^uint64(0) << bitIdx
		} else {
			r[limbIdx] = ^uint64(0)
		}
	}
	return r
}

// SignExtend performs sign extension of byte b in value x.
// b is the byte index (0-30) to extend from. If b >= 31, x is unchanged.
func SignExtend(b Uint256, x Uint256) Uint256 {
	if b.Cmp(u256Thirty1) >= 0 {
		return x
	}
	ext := b[0]
	bitIndex := 8*ext + 7
	limbIdx := bitIndex / 64
	bitIdx := bitIndex % 64

	result := x
	signBit := (x[limbIdx] >> bitIdx) & 1

	if signBit != 0 {
		// Negative: set all bits above bitIndex to 1.
		result[limbIdx] |= ^uint64(0) << bitIdx
		for i := limbIdx + 1; i < 4; i++ {
			result[i] = ^uint64(0)
		}
	} else {
		// Positive: clear all bits above bitIndex to 0.
		if bitIdx < 63 {
			result[limbIdx] &= (uint64(1) << (bitIdx + 1)) - 1
		}
		for i := limbIdx + 1; i < 4; i++ {
			result[i] = 0
		}
	}
	return result
}

// --- Signed arithmetic helpers (i256 two's complement) ---

// Sign represents the sign of a signed 256-bit integer.
type Sign int8

const (
	SignMinus Sign = -1
	SignZero  Sign = 0
	SignPlus  Sign = 1
)

// I256Sign returns the sign of u interpreted as two's complement.
func (u Uint256) I256Sign() Sign {
	if u[3]>>63 != 0 {
		return SignMinus
	}
	if u[0]|u[1]|u[2]|u[3] == 0 {
		return SignZero
	}
	return SignPlus
}

// I256SignCompl returns the sign and converts u to its absolute value in-place.
func I256SignCompl(val *Uint256) Sign {
	if val[3]>>63 != 0 {
		*val = val.Neg()
		return SignMinus
	}
	if val[0]|val[1]|val[2]|val[3] == 0 {
		return SignZero
	}
	return SignPlus
}

// U256RemoveSign clears bit 255 (the sign bit) in-place.
func U256RemoveSign(val *Uint256) {
	val[3] &= 0x7fffffffffffffff
}

// I256Cmp compares two Uint256 values as two's complement signed integers.
func I256Cmp(a, b Uint256) int {
	aNeg := a[3] >> 63 // 1 if negative, 0 if non-negative
	bNeg := b[3] >> 63
	if aNeg == bNeg {
		// Same sign: unsigned compare gives correct result for both
		// positive-positive and negative-negative two's complement.
		return a.Cmp(b)
	}
	if aNeg > bNeg {
		return -1 // a is negative, b is non-negative
	}
	return 1 // a is non-negative, b is negative
}

// I256Lt returns true if a < b as two's complement signed integers.
// Avoids the 3-way Cmp overhead when only a boolean is needed.
func I256Lt(a, b Uint256) bool {
	aNeg := a[3] >> 63
	bNeg := b[3] >> 63
	if aNeg != bNeg {
		return aNeg > bNeg // a is negative, b is non-negative
	}
	return a.Lt(b)
}

// I256Gt returns true if a > b as two's complement signed integers.
func I256Gt(a, b Uint256) bool {
	aNeg := a[3] >> 63
	bNeg := b[3] >> 63
	if aNeg != bNeg {
		return bNeg > aNeg // b is negative, a is non-negative
	}
	return a.Gt(b)
}

// sdivFitsInt64 returns true if the 256-bit two's complement value fits in int64.
// Positive: limbs [1..3] == 0 and bit 63 clear.
// Negative: limbs [1..3] == max and bit 63 set.
func sdivFitsInt64(v Uint256) bool {
	upper := v[1] | v[2] | v[3]
	if upper == 0 {
		return v[0]>>63 == 0 // positive, fits in int64
	}
	return v[1]&v[2]&v[3] == ^uint64(0) && v[0]>>63 == 1 // negative, fits in int64
}

// SDiv returns signed division. Returns 0 if divisor is zero.
func SDiv(a, b Uint256) Uint256 {
	// Fast path: both values fit in int64 range.
	if sdivFitsInt64(a) && sdivFitsInt64(b) {
		bi := int64(b[0])
		if bi == 0 {
			return U256Zero
		}
		ai := int64(a[0])
		// Guard against int64 min / -1 overflow (result doesn't fit in int64)
		if ai != -0x8000000000000000 || bi != -1 {
			return I256ToU256(ai / bi)
		}
	}

	bSign := I256SignCompl(&b)
	if bSign == SignZero {
		return U256Zero
	}
	aSign := I256SignCompl(&a)

	// Special case: MIN / -1 = MIN (overflow)
	if a.Eq(U256MinNegativeI256) && b.IsOne() {
		return U256MinNegativeI256
	}

	q := a.Div(b)
	U256RemoveSign(&q)

	if aSign != bSign {
		q = q.Neg()
	}
	return q
}

// I256ToU256 converts an int64 to its two's complement Uint256 representation.
func I256ToU256(v int64) Uint256 {
	if v >= 0 {
		return Uint256{uint64(v), 0, 0, 0}
	}
	return Uint256{uint64(v), ^uint64(0), ^uint64(0), ^uint64(0)}
}

// SMod returns signed modulo. Result sign follows the dividend (a).
func SMod(a, b Uint256) Uint256 {
	aSign := I256SignCompl(&a)
	if aSign == SignZero {
		return U256Zero
	}
	bSign := I256SignCompl(&b)
	if bSign == SignZero {
		return U256Zero
	}

	r := a.Mod(b)
	U256RemoveSign(&r)

	if aSign == SignMinus {
		r = r.Neg()
	}
	return r
}

// --- Byte count helpers ---

// ByteLen returns the number of bytes needed to represent the value.
func (u Uint256) ByteLen() uint {
	lz := u.LeadingZeros()
	return (256 - lz + 7) / 8
}
