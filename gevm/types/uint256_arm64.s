#include "textflag.h"

// ARM64 assembly for hot U256 operations using ABI0 calling convention.
// All functions are NOSPLIT leaf functions with zero local stack frame.
// Go compiler auto-generates ABIInternal→ABI0 wrappers for callers.

// func u256Add(result, a, b *U256)
// result = a + b (wrapping 256-bit addition)
TEXT ·u256Add(SB), NOSPLIT, $0-24
	MOVD	result+0(FP), R0
	MOVD	a+8(FP), R1
	MOVD	b+16(FP), R2
	LDP	0(R1), (R3, R4)
	LDP	16(R1), (R5, R6)
	LDP	0(R2), (R7, R8)
	LDP	16(R2), (R9, R10)
	ADDS	R7, R3, R3
	ADCS	R8, R4, R4
	ADCS	R9, R5, R5
	ADC	R10, R6, R6
	STP	(R3, R4), 0(R0)
	STP	(R5, R6), 16(R0)
	RET

// func u256Sub(result, a, b *U256)
// result = a - b (wrapping 256-bit subtraction)
TEXT ·u256Sub(SB), NOSPLIT, $0-24
	MOVD	result+0(FP), R0
	MOVD	a+8(FP), R1
	MOVD	b+16(FP), R2
	LDP	0(R1), (R3, R4)
	LDP	16(R1), (R5, R6)
	LDP	0(R2), (R7, R8)
	LDP	16(R2), (R9, R10)
	SUBS	R7, R3, R3
	SBCS	R8, R4, R4
	SBCS	R9, R5, R5
	SBC	R10, R6, R6
	STP	(R3, R4), 0(R0)
	STP	(R5, R6), 16(R0)
	RET

// func u256And(result, a, b *U256)
// result = a & b
TEXT ·u256And(SB), NOSPLIT, $0-24
	MOVD	result+0(FP), R0
	MOVD	a+8(FP), R1
	MOVD	b+16(FP), R2
	LDP	0(R1), (R3, R4)
	LDP	16(R1), (R5, R6)
	LDP	0(R2), (R7, R8)
	LDP	16(R2), (R9, R10)
	AND	R7, R3, R3
	AND	R8, R4, R4
	AND	R9, R5, R5
	AND	R10, R6, R6
	STP	(R3, R4), 0(R0)
	STP	(R5, R6), 16(R0)
	RET

// func u256Or(result, a, b *U256)
// result = a | b
TEXT ·u256Or(SB), NOSPLIT, $0-24
	MOVD	result+0(FP), R0
	MOVD	a+8(FP), R1
	MOVD	b+16(FP), R2
	LDP	0(R1), (R3, R4)
	LDP	16(R1), (R5, R6)
	LDP	0(R2), (R7, R8)
	LDP	16(R2), (R9, R10)
	ORR	R7, R3, R3
	ORR	R8, R4, R4
	ORR	R9, R5, R5
	ORR	R10, R6, R6
	STP	(R3, R4), 0(R0)
	STP	(R5, R6), 16(R0)
	RET

// func u256Xor(result, a, b *U256)
// result = a ^ b
TEXT ·u256Xor(SB), NOSPLIT, $0-24
	MOVD	result+0(FP), R0
	MOVD	a+8(FP), R1
	MOVD	b+16(FP), R2
	LDP	0(R1), (R3, R4)
	LDP	16(R1), (R5, R6)
	LDP	0(R2), (R7, R8)
	LDP	16(R2), (R9, R10)
	EOR	R7, R3, R3
	EOR	R8, R4, R4
	EOR	R9, R5, R5
	EOR	R10, R6, R6
	STP	(R3, R4), 0(R0)
	STP	(R5, R6), 16(R0)
	RET

// func u256Not(result, a *U256)
// result = ^a
TEXT ·u256Not(SB), NOSPLIT, $0-16
	MOVD	result+0(FP), R0
	MOVD	a+8(FP), R1
	LDP	0(R1), (R3, R4)
	LDP	16(R1), (R5, R6)
	MVN	R3, R3
	MVN	R4, R4
	MVN	R5, R5
	MVN	R6, R6
	STP	(R3, R4), 0(R0)
	STP	(R5, R6), 16(R0)
	RET

// func u256Eq(a, b *U256) bool
// Returns true if a == b
TEXT ·u256Eq(SB), NOSPLIT, $0-17
	MOVD	a+0(FP), R0
	MOVD	b+8(FP), R1
	LDP	0(R0), (R3, R4)
	LDP	16(R0), (R5, R6)
	LDP	0(R1), (R7, R8)
	LDP	16(R1), (R9, R10)
	EOR	R7, R3, R3
	EOR	R8, R4, R4
	EOR	R9, R5, R5
	EOR	R10, R6, R6
	ORR	R4, R3, R3
	ORR	R5, R3, R3
	ORR	R6, R3, R3
	CMP	$0, R3
	CSET	EQ, R0
	MOVB	R0, ret+16(FP)
	RET

// func u256Lt(a, b *U256) bool
// Returns true if a < b (unsigned)
TEXT ·u256Lt(SB), NOSPLIT, $0-17
	MOVD	a+0(FP), R0
	MOVD	b+8(FP), R1
	LDP	0(R0), (R3, R4)
	LDP	16(R0), (R5, R6)
	LDP	0(R1), (R7, R8)
	LDP	16(R1), (R9, R10)
	SUBS	R7, R3, R3
	SBCS	R8, R4, R4
	SBCS	R9, R5, R5
	SBCS	R10, R6, R6
	CSET	LO, R0
	MOVB	R0, ret+16(FP)
	RET

// func u256Gt(a, b *U256) bool
// Returns true if a > b (unsigned)
// Implemented as b < a: compute b - a and check for borrow
TEXT ·u256Gt(SB), NOSPLIT, $0-17
	MOVD	a+0(FP), R0
	MOVD	b+8(FP), R1
	LDP	0(R1), (R3, R4)
	LDP	16(R1), (R5, R6)
	LDP	0(R0), (R7, R8)
	LDP	16(R0), (R9, R10)
	SUBS	R7, R3, R3
	SBCS	R8, R4, R4
	SBCS	R9, R5, R5
	SBCS	R10, R6, R6
	CSET	LO, R0
	MOVB	R0, ret+16(FP)
	RET

// func u256IsZero(a *U256) bool
// Returns true if a == 0
TEXT ·u256IsZero(SB), NOSPLIT, $0-9
	MOVD	a+0(FP), R0
	LDP	0(R0), (R3, R4)
	LDP	16(R0), (R5, R6)
	ORR	R4, R3, R3
	ORR	R5, R3, R3
	ORR	R6, R3, R3
	CMP	$0, R3
	CSET	EQ, R0
	MOVB	R0, ret+8(FP)
	RET
