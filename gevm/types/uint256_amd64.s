#include "textflag.h"

// AMD64 assembly for hot U256 operations using ABI0 calling convention.
// All functions are NOSPLIT leaf functions with zero local stack frame.
// Uses memory-source ALU instructions for compact encoding.
// Bool results use SBB+NEG pattern (avoids SETcc portability issues).

// func u256Add(result, a, b *U256)
// result = a + b (wrapping 256-bit addition)
TEXT ·u256Add(SB), NOSPLIT, $0-24
	MOVQ	result+0(FP), DI
	MOVQ	a+8(FP), SI
	MOVQ	b+16(FP), DX
	MOVQ	0(SI), AX
	MOVQ	8(SI), BX
	MOVQ	16(SI), CX
	MOVQ	24(SI), R8
	ADDQ	0(DX), AX
	ADCQ	8(DX), BX
	ADCQ	16(DX), CX
	ADCQ	24(DX), R8
	MOVQ	AX, 0(DI)
	MOVQ	BX, 8(DI)
	MOVQ	CX, 16(DI)
	MOVQ	R8, 24(DI)
	RET

// func u256Sub(result, a, b *U256)
// result = a - b (wrapping 256-bit subtraction)
TEXT ·u256Sub(SB), NOSPLIT, $0-24
	MOVQ	result+0(FP), DI
	MOVQ	a+8(FP), SI
	MOVQ	b+16(FP), DX
	MOVQ	0(SI), AX
	MOVQ	8(SI), BX
	MOVQ	16(SI), CX
	MOVQ	24(SI), R8
	SUBQ	0(DX), AX
	SBBQ	8(DX), BX
	SBBQ	16(DX), CX
	SBBQ	24(DX), R8
	MOVQ	AX, 0(DI)
	MOVQ	BX, 8(DI)
	MOVQ	CX, 16(DI)
	MOVQ	R8, 24(DI)
	RET

// func u256And(result, a, b *U256)
// result = a & b
TEXT ·u256And(SB), NOSPLIT, $0-24
	MOVQ	result+0(FP), DI
	MOVQ	a+8(FP), SI
	MOVQ	b+16(FP), DX
	MOVQ	0(SI), AX
	MOVQ	8(SI), BX
	MOVQ	16(SI), CX
	MOVQ	24(SI), R8
	ANDQ	0(DX), AX
	ANDQ	8(DX), BX
	ANDQ	16(DX), CX
	ANDQ	24(DX), R8
	MOVQ	AX, 0(DI)
	MOVQ	BX, 8(DI)
	MOVQ	CX, 16(DI)
	MOVQ	R8, 24(DI)
	RET

// func u256Or(result, a, b *U256)
// result = a | b
TEXT ·u256Or(SB), NOSPLIT, $0-24
	MOVQ	result+0(FP), DI
	MOVQ	a+8(FP), SI
	MOVQ	b+16(FP), DX
	MOVQ	0(SI), AX
	MOVQ	8(SI), BX
	MOVQ	16(SI), CX
	MOVQ	24(SI), R8
	ORQ	0(DX), AX
	ORQ	8(DX), BX
	ORQ	16(DX), CX
	ORQ	24(DX), R8
	MOVQ	AX, 0(DI)
	MOVQ	BX, 8(DI)
	MOVQ	CX, 16(DI)
	MOVQ	R8, 24(DI)
	RET

// func u256Xor(result, a, b *U256)
// result = a ^ b
TEXT ·u256Xor(SB), NOSPLIT, $0-24
	MOVQ	result+0(FP), DI
	MOVQ	a+8(FP), SI
	MOVQ	b+16(FP), DX
	MOVQ	0(SI), AX
	MOVQ	8(SI), BX
	MOVQ	16(SI), CX
	MOVQ	24(SI), R8
	XORQ	0(DX), AX
	XORQ	8(DX), BX
	XORQ	16(DX), CX
	XORQ	24(DX), R8
	MOVQ	AX, 0(DI)
	MOVQ	BX, 8(DI)
	MOVQ	CX, 16(DI)
	MOVQ	R8, 24(DI)
	RET

// func u256Not(result, a *U256)
// result = ^a
TEXT ·u256Not(SB), NOSPLIT, $0-16
	MOVQ	result+0(FP), DI
	MOVQ	a+8(FP), SI
	MOVQ	0(SI), AX
	MOVQ	8(SI), BX
	MOVQ	16(SI), CX
	MOVQ	24(SI), DX
	NOTQ	AX
	NOTQ	BX
	NOTQ	CX
	NOTQ	DX
	MOVQ	AX, 0(DI)
	MOVQ	BX, 8(DI)
	MOVQ	CX, 16(DI)
	MOVQ	DX, 24(DI)
	RET

// func u256Eq(a, b *U256) bool
// Returns true if a == b
// XOR corresponding limbs, OR results; zero means equal.
// Convert: CMPQ $1 sets CF when AX==0, then SBB+NEG captures CF.
TEXT ·u256Eq(SB), NOSPLIT, $0-17
	MOVQ	a+0(FP), SI
	MOVQ	b+8(FP), DI
	MOVQ	0(SI), AX
	MOVQ	8(SI), BX
	MOVQ	16(SI), CX
	MOVQ	24(SI), DX
	XORQ	0(DI), AX
	XORQ	8(DI), BX
	XORQ	16(DI), CX
	XORQ	24(DI), DX
	ORQ	BX, AX
	ORQ	CX, AX
	ORQ	DX, AX
	CMPQ	AX, $1
	SBBQ	AX, AX
	NEGQ	AX
	MOVB	AX, ret+16(FP)
	RET

// func u256Lt(a, b *U256) bool
// Returns true if a < b (unsigned)
// SUB/SBB chain; CF=1 means borrow (a < b). SBB+NEG captures CF.
TEXT ·u256Lt(SB), NOSPLIT, $0-17
	MOVQ	a+0(FP), SI
	MOVQ	b+8(FP), DI
	MOVQ	0(SI), AX
	MOVQ	8(SI), BX
	MOVQ	16(SI), CX
	MOVQ	24(SI), DX
	SUBQ	0(DI), AX
	SBBQ	8(DI), BX
	SBBQ	16(DI), CX
	SBBQ	24(DI), DX
	SBBQ	AX, AX
	NEGQ	AX
	MOVB	AX, ret+16(FP)
	RET

// func u256Gt(a, b *U256) bool
// Returns true if a > b (unsigned)
// Implemented as b < a: load b, subtract a, check CF.
TEXT ·u256Gt(SB), NOSPLIT, $0-17
	MOVQ	a+0(FP), DI
	MOVQ	b+8(FP), SI
	MOVQ	0(SI), AX
	MOVQ	8(SI), BX
	MOVQ	16(SI), CX
	MOVQ	24(SI), DX
	SUBQ	0(DI), AX
	SBBQ	8(DI), BX
	SBBQ	16(DI), CX
	SBBQ	24(DI), DX
	SBBQ	AX, AX
	NEGQ	AX
	MOVB	AX, ret+16(FP)
	RET

// func u256IsZero(a *U256) bool
// Returns true if a == 0
// OR all limbs; zero means all zero.
TEXT ·u256IsZero(SB), NOSPLIT, $0-9
	MOVQ	a+0(FP), SI
	MOVQ	0(SI), AX
	ORQ	8(SI), AX
	ORQ	16(SI), AX
	ORQ	24(SI), AX
	CMPQ	AX, $1
	SBBQ	AX, AX
	NEGQ	AX
	MOVB	AX, ret+8(FP)
	RET
