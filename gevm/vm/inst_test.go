package vm

import (
	"testing"

	"github.com/Giulio2002/gevm/opcode"
	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
)

// runBytecode creates an interpreter, loads bytecode, and runs it to completion.
func runBytecode(code []byte, gasLimit uint64) *Interpreter {
	return runBytecodeWithSpec(code, gasLimit, spec.LatestForkID)
}

func runBytecodeWithSpec(code []byte, gasLimit uint64, forkID spec.ForkID) *Interpreter {
	interp := NewInterpreter(
		NewMemory(),
		NewBytecode(code),
		Inputs{},
		false,
		forkID,
		gasLimit,
	)
	DefaultRunner{}.Run(interp, nil)
	return interp
}

func u(v uint64) types.Uint256 {
	return types.U256From(v)
}

// push2Add pushes two values and adds them
func push2Op(a, b types.Uint256, op byte) []byte {
	code := make([]byte, 0, 67)
	// PUSH32 a
	code = append(code, opcode.PUSH32)
	ab := a.ToBytes32()
	code = append(code, ab[:]...)
	// PUSH32 b
	code = append(code, opcode.PUSH32)
	bb := b.ToBytes32()
	code = append(code, bb[:]...)
	// OP
	code = append(code, op)
	return code
}

func push3Op(a, b, c types.Uint256, op byte) []byte {
	code := make([]byte, 0, 100)
	code = append(code, opcode.PUSH32)
	ab := a.ToBytes32()
	code = append(code, ab[:]...)
	code = append(code, opcode.PUSH32)
	bb := b.ToBytes32()
	code = append(code, bb[:]...)
	code = append(code, opcode.PUSH32)
	cb := c.ToBytes32()
	code = append(code, cb[:]...)
	code = append(code, op)
	return code
}

// --- Arithmetic Tests ---

func TestOpAdd(t *testing.T) {
	interp := runBytecode(push2Op(u(10), u(20), opcode.ADD), 100000)
	if interp.Stack.Len() != 1 {
		t.Fatalf("stack len: got %d, want 1", interp.Stack.Len())
	}
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(30)) {
		t.Errorf("10+20: got %s, want 30", val.Hex())
	}
}

func TestOpAddOverflow(t *testing.T) {
	interp := runBytecode(push2Op(types.U256Max, u(1), opcode.ADD), 100000)
	val, _ := interp.Stack.Pop()
	if !val.IsZero() {
		t.Errorf("MAX+1 should wrap to 0: got %s", val.Hex())
	}
}

func TestOpMul(t *testing.T) {
	interp := runBytecode(push2Op(u(7), u(6), opcode.MUL), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(42)) {
		t.Errorf("7*6: got %s, want 42", val.Hex())
	}
}

func TestOpSub(t *testing.T) {
	// push2Op(a,b,op) => stack [a,b], op pops b (top) then accesses a => result = b - a
	interp := runBytecode(push2Op(u(10), u(30), opcode.SUB), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(20)) {
		t.Errorf("30-10: got %s, want 20", val.Hex())
	}
}

func TestOpSubUnderflow(t *testing.T) {
	interp := runBytecode(push2Op(u(1), u(0), opcode.SUB), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(types.U256Max) {
		t.Errorf("0-1 should wrap to MAX: got %s", val.Hex())
	}
}

func TestOpDiv(t *testing.T) {
	// push a=3, push b=10, DIV => b/a = 10/3 = 3
	interp := runBytecode(push2Op(u(3), u(10), opcode.DIV), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(3)) {
		t.Errorf("10/3: got %s, want 3", val.Hex())
	}
}

func TestOpDivByZero(t *testing.T) {
	interp := runBytecode(push2Op(u(0), u(10), opcode.DIV), 100000)
	val, _ := interp.Stack.Pop()
	if !val.IsZero() {
		t.Errorf("10/0: got %s, want 0", val.Hex())
	}
}

func TestOpSdiv(t *testing.T) {
	// -10 / 3 = -3 (in two's complement)
	neg10 := u(0).Sub(u(10)) // wrapping: MAX - 9
	// push a=3, push b=neg10, SDIV => b/a = (-10)/3 = -3
	interp := runBytecode(push2Op(u(3), neg10, opcode.SDIV), 100000)
	val, _ := interp.Stack.Pop()
	neg3 := u(0).Sub(u(3))
	if !val.Eq(neg3) {
		t.Errorf("-10/3: got %s, want %s", val.Hex(), neg3.Hex())
	}
}

func TestOpMod(t *testing.T) {
	// push a=3, push b=10, MOD => b%a = 10%3 = 1
	interp := runBytecode(push2Op(u(3), u(10), opcode.MOD), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(1)) {
		t.Errorf("10%%3: got %s, want 1", val.Hex())
	}
}

func TestOpModByZero(t *testing.T) {
	interp := runBytecode(push2Op(u(0), u(10), opcode.MOD), 100000)
	val, _ := interp.Stack.Pop()
	if !val.IsZero() {
		t.Errorf("10%%0: got %s, want 0", val.Hex())
	}
}

func TestOpAddmod(t *testing.T) {
	// push3Op(a,b,c, ADDMOD): stack [a,b,c], pops c (top), then b, then replaces with (c+b)%a
	// Want (10+10)%8 = 4: c=10, b=10, a=8
	interp := runBytecode(push3Op(u(8), u(10), u(10), opcode.ADDMOD), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(4)) {
		t.Errorf("(10+10)%%8: got %s, want 4", val.Hex())
	}
}

func TestOpMulmod(t *testing.T) {
	// Want (10*10)%8 = 4: c=10, b=10, a=8
	interp := runBytecode(push3Op(u(8), u(10), u(10), opcode.MULMOD), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(4)) {
		t.Errorf("(10*10)%%8: got %s, want 4", val.Hex())
	}
}

func TestOpExp(t *testing.T) {
	// push a=10, push b=2, EXP => b^a = 2^10 = 1024
	interp := runBytecode(push2Op(u(10), u(2), opcode.EXP), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(1024)) {
		t.Errorf("2^10: got %s, want 1024", val.Hex())
	}
}

func TestOpSignextend(t *testing.T) {
	// push a=0xFF, push b=0 (byte index), SIGNEXTEND => signextend(b, a)
	// Sign-extend byte 0: value 0xFF -> should extend to all 1s
	interp := runBytecode(push2Op(u(0xFF), u(0), opcode.SIGNEXTEND), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(types.U256Max) {
		t.Errorf("signextend(0, 0xFF): got %s, want MAX", val.Hex())
	}
}

func TestOpSignextendPositive(t *testing.T) {
	// Sign-extend byte 0: value 0x7F -> should stay positive
	interp := runBytecode(push2Op(u(0x7F), u(0), opcode.SIGNEXTEND), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(0x7F)) {
		t.Errorf("signextend(0, 0x7F): got %s, want 0x7F", val.Hex())
	}
}

// --- Bitwise Tests ---

func TestOpLt(t *testing.T) {
	// push a=2, push b=1, LT => b < a? => 1 < 2 = true
	interp := runBytecode(push2Op(u(2), u(1), opcode.LT), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(1)) {
		t.Errorf("1<2: got %s, want 1", val.Hex())
	}
}

func TestOpGt(t *testing.T) {
	// push a=1, push b=2, GT => b > a? => 2 > 1 = true
	interp := runBytecode(push2Op(u(1), u(2), opcode.GT), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(1)) {
		t.Errorf("2>1: got %s, want 1", val.Hex())
	}
}

func TestOpEq(t *testing.T) {
	interp := runBytecode(push2Op(u(42), u(42), opcode.EQ), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(1)) {
		t.Errorf("42==42: got %s, want 1", val.Hex())
	}
}

func TestOpIszero(t *testing.T) {
	code := make([]byte, 0, 35)
	code = append(code, opcode.PUSH32)
	z := types.U256Zero.ToBytes32()
	code = append(code, z[:]...)
	code = append(code, opcode.ISZERO)
	interp := runBytecode(code, 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(1)) {
		t.Errorf("ISZERO(0): got %s, want 1", val.Hex())
	}
}

func TestOpAnd(t *testing.T) {
	interp := runBytecode(push2Op(u(0xFF), u(0x0F), opcode.AND), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(0x0F)) {
		t.Errorf("0xFF&0x0F: got %s, want 0x0F", val.Hex())
	}
}

func TestOpOr(t *testing.T) {
	interp := runBytecode(push2Op(u(0xF0), u(0x0F), opcode.OR), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(0xFF)) {
		t.Errorf("0xF0|0x0F: got %s, want 0xFF", val.Hex())
	}
}

func TestOpXor(t *testing.T) {
	interp := runBytecode(push2Op(u(0xFF), u(0x0F), opcode.XOR), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(0xF0)) {
		t.Errorf("0xFF^0x0F: got %s, want 0xF0", val.Hex())
	}
}

func TestOpNot(t *testing.T) {
	code := make([]byte, 0, 35)
	code = append(code, opcode.PUSH32)
	z := types.U256Zero.ToBytes32()
	code = append(code, z[:]...)
	code = append(code, opcode.NOT)
	interp := runBytecode(code, 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(types.U256Max) {
		t.Errorf("NOT(0): got %s, want MAX", val.Hex())
	}
}

func TestOpShl(t *testing.T) {
	// SHL: shift value 1 left by 255 positions
	interp := runBytecode(push2Op(u(1), u(255), opcode.SHL), 100000)
	val, _ := interp.Stack.Pop()
	expected := types.U256MinNegativeI256 // 1 << 255
	if !val.Eq(expected) {
		t.Errorf("1<<255: got %s, want %s", val.Hex(), expected.Hex())
	}
}

func TestOpShlOvershift(t *testing.T) {
	interp := runBytecode(push2Op(u(1), u(256), opcode.SHL), 100000)
	val, _ := interp.Stack.Pop()
	if !val.IsZero() {
		t.Errorf("1<<256: got %s, want 0", val.Hex())
	}
}

func TestOpShr(t *testing.T) {
	interp := runBytecode(push2Op(types.U256MinNegativeI256, u(255), opcode.SHR), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(1)) {
		t.Errorf("(1<<255)>>255: got %s, want 1", val.Hex())
	}
}

func TestOpSar(t *testing.T) {
	// SAR of negative number: -1 >> 1 should still be -1
	interp := runBytecode(push2Op(types.U256Max, u(1), opcode.SAR), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(types.U256Max) {
		t.Errorf("(-1)>>1: got %s, want MAX", val.Hex())
	}
}

func TestOpByte(t *testing.T) {
	// BYTE(31, 0xFF) should return 0xFF (least significant byte)
	interp := runBytecode(push2Op(u(0xFF), u(31), opcode.BYTE), 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(0xFF)) {
		t.Errorf("BYTE(31, 0xFF): got %s, want 0xFF", val.Hex())
	}
}

func TestOpByteOutOfRange(t *testing.T) {
	interp := runBytecode(push2Op(u(0xFF), u(32), opcode.BYTE), 100000)
	val, _ := interp.Stack.Pop()
	if !val.IsZero() {
		t.Errorf("BYTE(32, 0xFF): got %s, want 0", val.Hex())
	}
}

// --- Stack Tests ---

func TestOpPush1(t *testing.T) {
	code := []byte{opcode.PUSH1, 0x42}
	interp := runBytecode(code, 100000)
	if interp.Stack.Len() != 1 {
		t.Fatalf("stack len: got %d, want 1", interp.Stack.Len())
	}
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(0x42)) {
		t.Errorf("PUSH1 0x42: got %s", val.Hex())
	}
}

func TestOpPush32(t *testing.T) {
	code := make([]byte, 33)
	code[0] = opcode.PUSH32
	for i := 1; i <= 32; i++ {
		code[i] = byte(i)
	}
	interp := runBytecode(code, 100000)
	val, _ := interp.Stack.Pop()
	b := val.ToBytes32()
	for i := 0; i < 32; i++ {
		if b[i] != byte(i+1) {
			t.Errorf("PUSH32 byte[%d]: got %x, want %x", i, b[i], i+1)
		}
	}
}

func TestOpDup1(t *testing.T) {
	code := []byte{opcode.PUSH1, 0x42, opcode.DUP1}
	interp := runBytecode(code, 100000)
	if interp.Stack.Len() != 2 {
		t.Fatalf("stack len: got %d, want 2", interp.Stack.Len())
	}
	a, _ := interp.Stack.Pop()
	b, _ := interp.Stack.Pop()
	if !a.Eq(u(0x42)) || !b.Eq(u(0x42)) {
		t.Errorf("DUP1: got %s, %s", a.Hex(), b.Hex())
	}
}

func TestOpSwap1(t *testing.T) {
	code := []byte{opcode.PUSH1, 0x01, opcode.PUSH1, 0x02, opcode.SWAP1}
	interp := runBytecode(code, 100000)
	a, _ := interp.Stack.Pop()
	b, _ := interp.Stack.Pop()
	if !a.Eq(u(1)) || !b.Eq(u(2)) {
		t.Errorf("SWAP1: got top=%s, second=%s", a.Hex(), b.Hex())
	}
}

func TestOpPop(t *testing.T) {
	code := []byte{opcode.PUSH1, 0x42, opcode.PUSH1, 0x01, opcode.POP}
	interp := runBytecode(code, 100000)
	if interp.Stack.Len() != 1 {
		t.Fatalf("stack len after POP: got %d, want 1", interp.Stack.Len())
	}
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(0x42)) {
		t.Errorf("remaining after POP: got %s, want 0x42", val.Hex())
	}
}

// --- Memory Tests ---

func TestOpMloadMstore(t *testing.T) {
	code := []byte{
		opcode.PUSH1, 0x42, // value
		opcode.PUSH1, 0x00, // offset
		opcode.MSTORE,      // store at offset 0
		opcode.PUSH1, 0x00, // offset
		opcode.MLOAD, // load from offset 0
	}
	interp := runBytecode(code, 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(0x42)) {
		t.Errorf("MLOAD after MSTORE: got %s, want 0x42", val.Hex())
	}
}

func TestOpMstore8(t *testing.T) {
	code := []byte{
		opcode.PUSH1, 0xAB, // value (low byte = 0xAB)
		opcode.PUSH1, 0x00, // offset
		opcode.MSTORE8,     // store byte at offset 0
		opcode.PUSH1, 0x00, // offset
		opcode.MLOAD, // load 32 bytes from offset 0
	}
	interp := runBytecode(code, 100000)
	val, _ := interp.Stack.Pop()
	// 0xAB stored at byte 0, loaded as 32-byte big-endian => 0xAB << (31*8)
	expected := types.U256From(0xAB).Shl(248)
	if !val.Eq(expected) {
		t.Errorf("MSTORE8: got %s, want %s", val.Hex(), expected.Hex())
	}
}

func TestOpMsize(t *testing.T) {
	code := []byte{
		opcode.PUSH1, 0x42, // value
		opcode.PUSH1, 0x00, // offset
		opcode.MSTORE, // expand to 32 bytes
		opcode.MSIZE,  // should be 32
	}
	interp := runBytecode(code, 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(32)) {
		t.Errorf("MSIZE: got %s, want 32", val.Hex())
	}
}

// --- Control Flow Tests ---

func TestOpJump(t *testing.T) {
	code := []byte{
		opcode.PUSH1, 0x04, // target = 4
		opcode.JUMP,
		opcode.INVALID,  // should be skipped
		opcode.JUMPDEST, // position 4
		opcode.PUSH1, 0x42,
	}
	interp := runBytecode(code, 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(0x42)) {
		t.Errorf("JUMP: got %s, want 0x42", val.Hex())
	}
}

func TestOpJumpi(t *testing.T) {
	// JUMPI with condition=1 should jump
	code := []byte{
		opcode.PUSH1, 0x01, // condition (true)
		opcode.PUSH1, 0x06, // target = 6
		opcode.JUMPI,
		opcode.INVALID,  // should be skipped
		opcode.JUMPDEST, // position 6
		opcode.PUSH1, 0x42,
	}
	interp := runBytecode(code, 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(0x42)) {
		t.Errorf("JUMPI true: got %s, want 0x42", val.Hex())
	}
}

func TestOpJumpiNoJump(t *testing.T) {
	// JUMPI with condition=0 should not jump
	code := []byte{
		opcode.PUSH1, 0x00, // condition (false)
		opcode.PUSH1, 0x06, // target
		opcode.JUMPI,
		opcode.PUSH1, 0x42, // should execute this
	}
	interp := runBytecode(code, 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(0x42)) {
		t.Errorf("JUMPI false: got %s, want 0x42", val.Hex())
	}
}

func TestOpPc(t *testing.T) {
	code := []byte{
		opcode.PUSH1, 0x00, // positions 0,1
		opcode.POP, // position 2
		opcode.PC,  // position 3
	}
	interp := runBytecode(code, 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(3)) {
		t.Errorf("PC: got %s, want 3", val.Hex())
	}
}

func TestOpGas(t *testing.T) {
	// GAS opcode should push remaining gas after static gas deductions
	code := []byte{opcode.GAS}
	interp := runBytecode(code, 100000)
	val, _ := interp.Stack.Pop()
	// GAS costs 2 static gas, so remaining should be 100000 - 2
	if !val.Eq(u(100000 - 2)) {
		t.Errorf("GAS: got %s, want %d", val.Hex(), 100000-2)
	}
}

func TestOpReturn(t *testing.T) {
	code := []byte{
		opcode.PUSH1, 0x42, // value
		opcode.PUSH1, 0x00, // offset
		opcode.MSTORE,      // store at offset 0
		opcode.PUSH1, 0x20, // length = 32
		opcode.PUSH1, 0x00, // offset = 0
		opcode.RETURN,
	}
	interp := runBytecode(code, 100000)
	if len(interp.ReturnData) != 32 {
		t.Fatalf("return data len: got %d, want 32", len(interp.ReturnData))
	}
	// Check that return data contains the stored value (big-endian 0x42)
	if interp.ReturnData[31] != 0x42 {
		t.Errorf("return data[31]: got %x, want 0x42", interp.ReturnData[31])
	}
}

func TestOpStop(t *testing.T) {
	code := []byte{opcode.PUSH1, 0x42, opcode.STOP, opcode.INVALID}
	interp := runBytecode(code, 100000)
	if interp.Stack.Len() != 1 {
		t.Fatalf("stack after STOP: got %d, want 1", interp.Stack.Len())
	}
}

func TestOpInvalid(t *testing.T) {
	code := []byte{opcode.INVALID}
	interp := runBytecode(code, 100000)
	if interp.Bytecode.IsRunning() {
		t.Error("should be halted after INVALID")
	}
}

// --- System Tests ---

func TestOpCalldataload(t *testing.T) {
	input := make([]byte, 32)
	input[31] = 0x42
	interp := NewInterpreter(
		NewMemory(),
		NewBytecode([]byte{opcode.PUSH1, 0x00, opcode.CALLDATALOAD}),
		Inputs{Input: input},
		false,
		spec.LatestForkID,
		100000,
	)
	DefaultRunner{}.Run(interp, nil)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(0x42)) {
		t.Errorf("CALLDATALOAD: got %s, want 0x42", val.Hex())
	}
}

func TestOpCalldatasize(t *testing.T) {
	input := make([]byte, 100)
	interp := NewInterpreter(
		NewMemory(),
		NewBytecode([]byte{opcode.CALLDATASIZE}),
		Inputs{Input: input},
		false,
		spec.LatestForkID,
		100000,
	)
	DefaultRunner{}.Run(interp, nil)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(100)) {
		t.Errorf("CALLDATASIZE: got %s, want 100", val.Hex())
	}
}

func TestOpCodesize(t *testing.T) {
	code := []byte{opcode.PUSH1, 0x00, opcode.POP, opcode.CODESIZE}
	interp := runBytecode(code, 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(4)) {
		t.Errorf("CODESIZE: got %s, want 4", val.Hex())
	}
}

func TestOpKeccak256Empty(t *testing.T) {
	// KECCAK256 of empty (offset=0, length=0)
	code := []byte{
		opcode.PUSH1, 0x00, // length = 0
		opcode.PUSH1, 0x00, // offset = 0
		opcode.KECCAK256,
	}
	interp := runBytecode(code, 100000)
	val, _ := interp.Stack.Pop()
	// Known keccak256("") = 0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470
	b := val.ToBytes32()
	if b[0] != 0xc5 || b[1] != 0xd2 {
		t.Errorf("KECCAK256(empty): got %x %x... want c5 d2...", b[0], b[1])
	}
}

// --- Execution Loop Integration Test ---

func TestSimpleProgram(t *testing.T) {
	// Program: push 3, push 5, add => stack should have 8
	code := []byte{
		opcode.PUSH1, 0x03,
		opcode.PUSH1, 0x05,
		opcode.ADD,
	}
	interp := runBytecode(code, 100000)
	if interp.Stack.Len() != 1 {
		t.Fatalf("stack len: got %d, want 1", interp.Stack.Len())
	}
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(8)) {
		t.Errorf("3+5: got %s, want 8", val.Hex())
	}
}

func TestComplexProgram(t *testing.T) {
	// Program: 2*3 + 4 = 10
	code := []byte{
		opcode.PUSH1, 0x02, // 2
		opcode.PUSH1, 0x03, // 3
		opcode.MUL,         // 6
		opcode.PUSH1, 0x04, // 4
		opcode.ADD, // 10
	}
	interp := runBytecode(code, 100000)
	val, _ := interp.Stack.Pop()
	if !val.Eq(u(10)) {
		t.Errorf("2*3+4: got %s, want 10", val.Hex())
	}
}

func TestGasAccounting(t *testing.T) {
	// PUSH1 costs 3 gas, ADD costs 3 gas = total 9 gas
	code := []byte{
		opcode.PUSH1, 0x01,
		opcode.PUSH1, 0x02,
		opcode.ADD,
	}
	interp := runBytecode(code, 100)
	if interp.Gas.Spent() != 9 {
		t.Errorf("gas spent: got %d, want 9", interp.Gas.Spent())
	}
}

func TestOutOfGas(t *testing.T) {
	code := []byte{
		opcode.PUSH1, 0x01, // costs 3
		opcode.PUSH1, 0x02, // costs 3 -> total 6, only 5 available
	}
	interp := runBytecode(code, 5)
	// Should have halted due to OOG on second PUSH1
	if interp.Bytecode.IsRunning() {
		t.Error("should be halted after OOG")
	}
}
