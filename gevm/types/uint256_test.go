package types

import (
	"math/big"
	"testing"
)

// --- Helpers ---

func u(v uint64) Uint256 { return U256From(v) }

func u256FromBig(b *big.Int) Uint256 {
	var buf [32]byte
	bytes := b.Bytes()
	copy(buf[32-len(bytes):], bytes)
	return U256FromBytes32(buf)
}

func assertEq(t *testing.T, label string, got, want Uint256) {
	t.Helper()
	if !got.Eq(want) {
		t.Errorf("%s: got %s, want %s", label, got.Hex(), want.Hex())
	}
}

func assertBool(t *testing.T, label string, got, want bool) {
	t.Helper()
	if got != want {
		t.Errorf("%s: got %v, want %v", label, got, want)
	}
}

// --- Zero/One/Max ---

func TestConstants(t *testing.T) {
	assertBool(t, "zero is zero", U256Zero.IsZero(), true)
	assertBool(t, "one is not zero", U256One.IsZero(), false)
	assertBool(t, "one is one", U256One.IsOne(), true)
	assertBool(t, "max is not zero", U256Max.IsZero(), false)

	for i := 0; i < 4; i++ {
		if U256Max[i] != 0xffffffffffffffff {
			t.Errorf("Max limb %d: got %x", i, U256Max[i])
		}
	}
}

// --- Comparison ---

func TestCmp(t *testing.T) {
	a := u(10)
	b := u(20)
	assertBool(t, "10 < 20", a.Lt(b), true)
	assertBool(t, "20 > 10", b.Gt(a), true)
	assertBool(t, "10 == 10", a.Eq(a), true)
	assertBool(t, "10 != 20", a.Eq(b), false)

	x := Uint256{0, 0, 0, 1}
	y := Uint256{0xffffffffffffffff, 0xffffffffffffffff, 0xffffffffffffffff, 0}
	assertBool(t, "high > low", x.Gt(y), true)
}

// --- Add ---

func TestAdd(t *testing.T) {
	assertEq(t, "0+0", U256Zero.Add(U256Zero), U256Zero)
	assertEq(t, "1+1", U256One.Add(U256One), u(2))
	assertEq(t, "100+200", u(100).Add(u(200)), u(300))

	a := Uint256{0xffffffffffffffff, 0, 0, 0}
	assertEq(t, "carry to limb1", a.Add(U256One), Uint256{0, 1, 0, 0})

	a = Uint256{0xffffffffffffffff, 0xffffffffffffffff, 0xffffffffffffffff, 0}
	assertEq(t, "carry to limb3", a.Add(U256One), Uint256{0, 0, 0, 1})

	// Wrapping (MAX + 1 = 0)
	assertEq(t, "MAX+1=0", U256Max.Add(U256One), U256Zero)

	_, overflow := U256Max.OverflowingAdd(U256One)
	assertBool(t, "MAX+1 overflows", overflow, true)

	_, overflow = u(1).OverflowingAdd(u(2))
	assertBool(t, "1+2 no overflow", overflow, false)
}

// --- Sub ---

func TestSub(t *testing.T) {
	assertEq(t, "5-3", u(5).Sub(u(3)), u(2))
	assertEq(t, "0-0", U256Zero.Sub(U256Zero), U256Zero)

	// Wrapping (0 - 1 = MAX)
	assertEq(t, "0-1=MAX", U256Zero.Sub(U256One), U256Max)

	a := Uint256{0, 1, 0, 0}
	assertEq(t, "borrow", a.Sub(U256One), Uint256{0xffffffffffffffff, 0, 0, 0})

	_, underflow := U256Zero.OverflowingSub(U256One)
	assertBool(t, "0-1 underflows", underflow, true)
}

// --- Mul ---

func TestMul(t *testing.T) {
	assertEq(t, "0*x", U256Zero.Mul(u(42)), U256Zero)
	assertEq(t, "1*x", U256One.Mul(u(42)), u(42))
	assertEq(t, "2*3", u(2).Mul(u(3)), u(6))
	assertEq(t, "7*11", u(7).Mul(u(11)), u(77))

	a := Uint256{0xffffffffffffffff, 0, 0, 0}
	assertEq(t, "large*2", a.Mul(u(2)), Uint256{0xfffffffffffffffe, 1, 0, 0})

	// MAX * 2 = MAX - 1 (mod 2^256)
	expected := U256Max.Sub(U256One)
	assertEq(t, "MAX*2", U256Max.Mul(u(2)), expected)

	// MAX * MAX = 1 (mod 2^256)
	assertEq(t, "MAX*MAX", U256Max.Mul(U256Max), U256One)
}

// --- Div ---

func TestDiv(t *testing.T) {
	assertEq(t, "6/3", u(6).Div(u(3)), u(2))
	assertEq(t, "7/3", u(7).Div(u(3)), u(2))
	assertEq(t, "x/1", u(42).Div(U256One), u(42))
	assertEq(t, "0/x", U256Zero.Div(u(42)), U256Zero)

	// Division by zero returns 0
	assertEq(t, "x/0", u(42).Div(U256Zero), U256Zero)

	a := Uint256{0, 1, 0, 0} // 2^64
	assertEq(t, "2^64/2^32", a.Div(u(1<<32)), u(1<<32))

	assertEq(t, "MAX/MAX", U256Max.Div(U256Max), U256One)
}

// --- Mod ---

func TestMod(t *testing.T) {
	assertEq(t, "7%3", u(7).Mod(u(3)), u(1))
	assertEq(t, "6%3", u(6).Mod(u(3)), u(0))
	assertEq(t, "x%1", u(42).Mod(U256One), U256Zero)

	// Mod by zero returns 0
	assertEq(t, "x%0", u(42).Mod(U256Zero), U256Zero)
}

// --- Exp ---

func TestExp(t *testing.T) {
	assertEq(t, "0^0", U256Zero.Exp(U256Zero), U256One)
	assertEq(t, "0^1", U256Zero.Exp(U256One), U256Zero)
	assertEq(t, "1^0", U256One.Exp(U256Zero), U256One)
	assertEq(t, "2^10", u(2).Exp(u(10)), u(1024))
	assertEq(t, "3^3", u(3).Exp(u(3)), u(27))
	assertEq(t, "2^255", u(2).Exp(u(255)), U256MinNegativeI256)
	assertEq(t, "2^256=0", u(2).Exp(u(256)), U256Zero)
}

// --- AddMod / MulMod ---

func TestAddMod(t *testing.T) {
	assertEq(t, "(2+3)%5", u(2).AddMod(u(3), u(5)), U256Zero)
	assertEq(t, "(2+3)%4", u(2).AddMod(u(3), u(4)), u(1))
	assertEq(t, "addmod n=0", u(2).AddMod(u(3), U256Zero), U256Zero)

	// MAX + MAX mod 3 = 0
	assertEq(t, "MAX+MAX mod 3", U256Max.AddMod(U256Max, u(3)), U256Zero)

	// Validate against big.Int for overflow cases.
	cases := [][3]Uint256{
		{U256Max, U256One, u(7)},                                          // 257-bit sum, small modulus
		{U256Max, U256Max, u(5)},                                          // 257-bit sum
		{U256Max, U256Max, U256Max},                                       // MAX+MAX mod MAX = 0
		{U256Max, u(0), u(3)},                                             // no overflow
		{Uint256{0, 0, 0, 1 << 63}, Uint256{0, 0, 0, 1 << 63}, u(1000000007)}, // two huge values
		{Uint256{^uint64(0), ^uint64(0), 0, 0}, Uint256{^uint64(0), ^uint64(0), 0, 0}, Uint256{0, 0, 1, 0}}, // mid-range overflow
	}
	for _, tc := range cases {
		a, b, m := tc[0], tc[1], tc[2]
		got := a.AddMod(b, m)
		// big.Int reference
		ba, bb, bm := new(big.Int), new(big.Int), new(big.Int)
		ab := a.ToBytes32(); ba.SetBytes(ab[:])
		bb2 := b.ToBytes32(); bb.SetBytes(bb2[:])
		mb := m.ToBytes32(); bm.SetBytes(mb[:])
		want := u256FromBig(new(big.Int).Mod(new(big.Int).Add(ba, bb), bm))
		assertEq(t, "addmod-bigint", got, want)
	}
}

func TestMulMod(t *testing.T) {
	assertEq(t, "(2*3)%5", u(2).MulMod(u(3), u(5)), u(1))
	assertEq(t, "(3*3)%9", u(3).MulMod(u(3), u(9)), U256Zero)
	assertEq(t, "mulmod n=0", u(2).MulMod(u(3), U256Zero), U256Zero)
	assertEq(t, "MAX*MAX mod MAX", U256Max.MulMod(U256Max, U256Max), U256Zero)

	// Validate against big.Int for wide-product cases.
	cases := [][3]Uint256{
		{U256Max, U256Max, u(7)},                                          // 512-bit product, small mod
		{U256Max, u(2), u(5)},                                             // 257-bit product
		{Uint256{0, 0, 0, 1 << 63}, Uint256{0, 0, 0, 1 << 63}, u(1000000007)}, // huge * huge
		{Uint256{^uint64(0), ^uint64(0), 0, 0}, Uint256{^uint64(0), ^uint64(0), 0, 0}, Uint256{0, 0, 0, 1}}, // mid-range
		{U256Max, U256Max, Uint256{0, 0, 0, 1}},                             // big mod
		{U256Max, U256Max, U256One},                                       // mod 1 = 0
	}
	for _, tc := range cases {
		a, b, m := tc[0], tc[1], tc[2]
		got := a.MulMod(b, m)
		// big.Int reference
		ba, bb, bm := new(big.Int), new(big.Int), new(big.Int)
		ab := a.ToBytes32(); ba.SetBytes(ab[:])
		bb2 := b.ToBytes32(); bb.SetBytes(bb2[:])
		mb := m.ToBytes32(); bm.SetBytes(mb[:])
		want := u256FromBig(new(big.Int).Mod(new(big.Int).Mul(ba, bb), bm))
		assertEq(t, "mulmod-bigint", got, want)
	}
}

// --- Neg ---

func TestNeg(t *testing.T) {
	assertEq(t, "neg(0)", U256Zero.Neg(), U256Zero)
	assertEq(t, "neg(1)", U256One.Neg(), U256Max)
	assertEq(t, "neg(MAX)", U256Max.Neg(), U256One)
	assertEq(t, "neg(MIN)", U256MinNegativeI256.Neg(), U256MinNegativeI256)
}

// --- Shifts ---

func TestShl(t *testing.T) {
	assertEq(t, "1<<0", U256One.Shl(0), U256One)
	assertEq(t, "1<<1", U256One.Shl(1), u(2))
	assertEq(t, "1<<64", U256One.Shl(64), Uint256{0, 1, 0, 0})
	assertEq(t, "1<<128", U256One.Shl(128), Uint256{0, 0, 1, 0})
	assertEq(t, "1<<192", U256One.Shl(192), Uint256{0, 0, 0, 1})
	assertEq(t, "1<<255", U256One.Shl(255), U256MinNegativeI256)
	assertEq(t, "1<<256", U256One.Shl(256), U256Zero)
	assertEq(t, "MAX<<256", U256Max.Shl(256), U256Zero)
	assertEq(t, "MAX<<1000", U256Max.Shl(1000), U256Zero)
}

func TestShr(t *testing.T) {
	assertEq(t, "2>>1", u(2).Shr(1), U256One)
	assertEq(t, "1>>0", U256One.Shr(0), U256One)

	a := Uint256{0, 1, 0, 0}
	assertEq(t, "2^64>>64", a.Shr(64), U256One)

	assertEq(t, "MAX>>256", U256Max.Shr(256), U256Zero)
	assertEq(t, "1>>1000", U256One.Shr(1000), U256Zero)
}

func TestSar(t *testing.T) {
	assertEq(t, "+4>>1", u(4).Sar(1), u(2))
	assertEq(t, "-1>>1", U256Max.Sar(1), U256Max)

	expected := Uint256{0, 0, 0, 0xc000000000000000}
	assertEq(t, "MIN>>1", U256MinNegativeI256.Sar(1), expected)

	assertEq(t, "neg>>256=MAX", U256Max.Sar(256), U256Max)
	assertEq(t, "pos>>256=0", U256One.Sar(256), U256Zero)
	assertEq(t, "MIN>>256=MAX", U256MinNegativeI256.Sar(256), U256Max)
}

// --- Bit / Byte access ---

func TestBit(t *testing.T) {
	assertBool(t, "1.bit(0)", U256One.Bit(0), true)
	assertBool(t, "1.bit(1)", U256One.Bit(1), false)
	assertBool(t, "2.bit(1)", u(2).Bit(1), true)
	assertBool(t, "MIN.bit(255)", U256MinNegativeI256.Bit(255), true)
	assertBool(t, "MAX.bit(255)", U256Max.Bit(255), true)
	assertBool(t, "0.bit(0)", U256Zero.Bit(0), false)
	assertBool(t, "1.bit(256)", U256One.Bit(256), false)
}

func TestByteBE(t *testing.T) {
	a := Uint256{0, 0, 0, 0xff00000000000000}
	if a.ByteBE(0) != 0xff {
		t.Errorf("MSB byte: got %x, want ff", a.ByteBE(0))
	}

	b := u(0x42)
	if b.ByteBE(31) != 0x42 {
		t.Errorf("LSB byte: got %x, want 42", b.ByteBE(31))
	}

	if b.ByteBE(32) != 0 {
		t.Errorf("out of range byte: got %d, want 0", b.ByteBE(32))
	}
}

func TestLeadingZeros(t *testing.T) {
	if U256Zero.LeadingZeros() != 256 {
		t.Errorf("zero: got %d, want 256", U256Zero.LeadingZeros())
	}
	if U256One.LeadingZeros() != 255 {
		t.Errorf("one: got %d, want 255", U256One.LeadingZeros())
	}
	if U256Max.LeadingZeros() != 0 {
		t.Errorf("max: got %d, want 0", U256Max.LeadingZeros())
	}
	if U256MinNegativeI256.LeadingZeros() != 0 {
		t.Errorf("min: got %d, want 0", U256MinNegativeI256.LeadingZeros())
	}
}

// --- Bitwise ---

func TestBitwise(t *testing.T) {
	a := Uint256{0xff, 0, 0, 0}
	b := Uint256{0x0f, 0, 0, 0}
	assertEq(t, "and", a.And(b), Uint256{0x0f, 0, 0, 0})
	assertEq(t, "or", a.Or(b), Uint256{0xff, 0, 0, 0})
	assertEq(t, "xor", a.Xor(b), Uint256{0xf0, 0, 0, 0})
	assertEq(t, "not(0)", U256Zero.Not(), U256Max)
	assertEq(t, "not(max)", U256Max.Not(), U256Zero)
}

// --- SignExtend ---

func TestSignExtend(t *testing.T) {
	result := SignExtend(u(0), u(0xff))
	assertEq(t, "signext(0,0xff)=MAX", result, U256Max)

	result = SignExtend(u(0), u(0x7f))
	assertEq(t, "signext(0,0x7f)=0x7f", result, u(0x7f))

	result = SignExtend(u(1), u(0x80ff))
	expected := U256Max
	expected[0] = 0xffffffffffff80ff
	assertEq(t, "signext(1,0x80ff)", result, expected)

	assertEq(t, "signext(31,x)=x", SignExtend(u(31), u(42)), u(42))
	assertEq(t, "signext(100,x)=x", SignExtend(u(100), u(42)), u(42))
}

// --- Signed arithmetic ---

func TestI256Sign(t *testing.T) {
	if U256Zero.I256Sign() != SignZero {
		t.Error("zero sign should be SignZero")
	}
	if U256One.I256Sign() != SignPlus {
		t.Error("one sign should be SignPlus")
	}
	if U256Max.I256Sign() != SignMinus {
		t.Error("max (= -1) sign should be SignMinus")
	}
	if U256MinNegativeI256.I256Sign() != SignMinus {
		t.Error("min negative sign should be SignMinus")
	}
}

func TestI256Cmp(t *testing.T) {
	if I256Cmp(U256Zero, U256One) >= 0 {
		t.Error("0 should be < 1 (signed)")
	}
	if I256Cmp(U256Max, U256Zero) >= 0 {
		t.Error("-1 should be < 0 (signed)")
	}
	if I256Cmp(U256MinNegativeI256, U256Max) >= 0 {
		t.Error("MIN should be < -1 (signed)")
	}
	if I256Cmp(U256One, U256Max) <= 0 {
		t.Error("1 should be > -1 (signed)")
	}
}

func TestSDiv(t *testing.T) {
	assertEq(t, "6/3", SDiv(u(6), u(3)), u(2))

	neg6 := u(6).Neg()
	assertEq(t, "(-6)/3=-2", SDiv(neg6, u(3)), u(2).Neg())

	neg3 := u(3).Neg()
	assertEq(t, "6/(-3)=-2", SDiv(u(6), neg3), u(2).Neg())

	assertEq(t, "(-6)/(-3)=2", SDiv(neg6, neg3), u(2))

	assertEq(t, "sdiv x/0=0", SDiv(u(42), U256Zero), U256Zero)

	// MIN / -1 = MIN (overflow case)
	assertEq(t, "MIN/-1=MIN", SDiv(U256MinNegativeI256, U256Max), U256MinNegativeI256)
}

func TestSMod(t *testing.T) {
	assertEq(t, "7%3=1", SMod(u(7), u(3)), u(1))

	neg7 := u(7).Neg()
	assertEq(t, "(-7)%3=-1", SMod(neg7, u(3)), u(1).Neg())

	neg3 := u(3).Neg()
	assertEq(t, "7%(-3)=1", SMod(u(7), neg3), u(1))

	assertEq(t, "(-7)%(-3)=-1", SMod(neg7, neg3), u(1).Neg())

	assertEq(t, "smod x%0=0", SMod(u(42), U256Zero), U256Zero)
	assertEq(t, "smod 0%x=0", SMod(U256Zero, u(3)), U256Zero)
}

// --- Conversions ---

func TestBytes32Roundtrip(t *testing.T) {
	cases := []Uint256{U256Zero, U256One, U256Max, U256MinNegativeI256, u(0xdeadbeef)}
	for _, c := range cases {
		b := c.ToBytes32()
		got := U256FromBytes32(b)
		assertEq(t, "roundtrip", got, c)
	}
}

func TestBigIntRoundtrip(t *testing.T) {
	cases := []Uint256{U256Zero, U256One, U256Max, u(0xdeadbeef)}
	for _, c := range cases {
		b := c.ToBig()
		got := U256FromBig(b)
		assertEq(t, "big roundtrip", got, c)
	}
}

func TestToAddress(t *testing.T) {
	val := u(0xdeadbeef)
	addr := val.ToAddress()
	expected := Address{0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xde, 0xad, 0xbe, 0xef}
	if addr != expected {
		t.Errorf("ToAddress: got %s, want %s", addr.Hex(), expected.Hex())
	}
}

func TestByteLen(t *testing.T) {
	if U256Zero.ByteLen() != 0 {
		t.Errorf("0 bytelen: got %d, want 0", U256Zero.ByteLen())
	}
	if u(0xff).ByteLen() != 1 {
		t.Errorf("0xff bytelen: got %d, want 1", u(0xff).ByteLen())
	}
	if u(0x100).ByteLen() != 2 {
		t.Errorf("0x100 bytelen: got %d, want 2", u(0x100).ByteLen())
	}
	if U256Max.ByteLen() != 32 {
		t.Errorf("MAX bytelen: got %d, want 32", U256Max.ByteLen())
	}
}

// --- Cross-validation with big.Int ---

func TestCrossValidateArithmetic(t *testing.T) {
	mod256 := new(big.Int).Lsh(big.NewInt(1), 256)

	cases := []struct {
		a, b Uint256
	}{
		{u(0), u(0)},
		{u(1), u(1)},
		{u(0xdeadbeef), u(0xcafebabe)},
		{U256Max, U256One},
		{U256Max, U256Max},
		{Uint256{0xffffffffffffffff, 0, 0, 0}, Uint256{0, 0xffffffffffffffff, 0, 0}},
		{U256MinNegativeI256, u(2)},
	}

	for _, tc := range cases {
		aBig := tc.a.ToBig()
		bBig := tc.b.ToBig()

		// Add
		expected := new(big.Int).Add(aBig, bBig)
		expected.Mod(expected, mod256)
		got := tc.a.Add(tc.b)
		assertEq(t, "cross-add", got, U256FromBig(expected))

		// Sub
		expected = new(big.Int).Sub(aBig, bBig)
		expected.Mod(expected, mod256)
		if expected.Sign() < 0 {
			expected.Add(expected, mod256)
		}
		got = tc.a.Sub(tc.b)
		assertEq(t, "cross-sub", got, U256FromBig(expected))

		// Mul
		expected = new(big.Int).Mul(aBig, bBig)
		expected.Mod(expected, mod256)
		got = tc.a.Mul(tc.b)
		assertEq(t, "cross-mul", got, U256FromBig(expected))

		// Div
		if !tc.b.IsZero() {
			expected = new(big.Int).Div(aBig, bBig)
			got = tc.a.Div(tc.b)
			assertEq(t, "cross-div", got, U256FromBig(expected))
		}

		// Mod
		if !tc.b.IsZero() {
			expected = new(big.Int).Mod(aBig, bBig)
			got = tc.a.Mod(tc.b)
			assertEq(t, "cross-mod", got, U256FromBig(expected))
		}
	}
}
