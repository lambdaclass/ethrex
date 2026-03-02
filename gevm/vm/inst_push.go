// Push and EOF stack opcode handlers: PUSH0-PUSH32, DUPN, SWAPN, EXCHANGE.
package vm

import "github.com/Giulio2002/gevm/types"

// opPush0 — PushVal body. Fork gate (Shanghai) handled by generator.
func opPush0(interp *Interpreter) {
	s := interp.Stack
	s.data[s.top] = types.U256Zero
	s.top++
}

// opPush1 — PushVal body. Reads 1 byte from bytecode.
func opPush1(interp *Interpreter) {
	s := interp.Stack
	s.data[s.top] = types.Uint256{uint64(interp.Bytecode.code[interp.Bytecode.pc]), 0, 0, 0}
	interp.Bytecode.pc++
	s.top++
}

// opPush2 — PushVal body. Reads 2 bytes from bytecode.
func opPush2(interp *Interpreter) {
	s := interp.Stack
	v := uint64(interp.Bytecode.code[interp.Bytecode.pc])<<8 | uint64(interp.Bytecode.code[interp.Bytecode.pc+1])
	interp.Bytecode.pc += 2
	s.data[s.top] = types.Uint256{v, 0, 0, 0}
	s.top++
}

// opPush3 — PushVal body. Reads 3 bytes from bytecode.
func opPush3(interp *Interpreter) {
	s := interp.Stack
	c := interp.Bytecode.code
	p := interp.Bytecode.pc
	v := uint64(c[p])<<16 | uint64(c[p+1])<<8 | uint64(c[p+2])
	interp.Bytecode.pc = p + 3
	s.data[s.top] = types.Uint256{v, 0, 0, 0}
	s.top++
}

// opPush4 — PushVal body. Reads 4 bytes from bytecode.
func opPush4(interp *Interpreter) {
	s := interp.Stack
	c := interp.Bytecode.code
	p := interp.Bytecode.pc
	v := uint64(c[p])<<24 | uint64(c[p+1])<<16 | uint64(c[p+2])<<8 | uint64(c[p+3])
	interp.Bytecode.pc = p + 4
	s.data[s.top] = types.Uint256{v, 0, 0, 0}
	s.top++
}

// opPush20 — PushVal body. Reads 20 bytes from bytecode (common for addresses).
func opPush20(interp *Interpreter) {
	s := interp.Stack
	c := interp.Bytecode.code
	p := interp.Bytecode.pc
	// 20 bytes = limb2 (4 bytes) + limb1 (8 bytes) + limb0 (8 bytes)
	l2 := uint64(c[p])<<24 | uint64(c[p+1])<<16 | uint64(c[p+2])<<8 | uint64(c[p+3])
	l1 := uint64(c[p+4])<<56 | uint64(c[p+5])<<48 | uint64(c[p+6])<<40 | uint64(c[p+7])<<32 |
		uint64(c[p+8])<<24 | uint64(c[p+9])<<16 | uint64(c[p+10])<<8 | uint64(c[p+11])
	l0 := uint64(c[p+12])<<56 | uint64(c[p+13])<<48 | uint64(c[p+14])<<40 | uint64(c[p+15])<<32 |
		uint64(c[p+16])<<24 | uint64(c[p+17])<<16 | uint64(c[p+18])<<8 | uint64(c[p+19])
	interp.Bytecode.pc = p + 20
	s.data[s.top] = types.Uint256{l0, l1, l2, 0}
	s.top++
}

// opPush32 — PushVal body. Reads 32 bytes from bytecode.
func opPush32(interp *Interpreter) {
	s := interp.Stack
	c := interp.Bytecode.code
	p := interp.Bytecode.pc
	l3 := uint64(c[p])<<56 | uint64(c[p+1])<<48 | uint64(c[p+2])<<40 | uint64(c[p+3])<<32 |
		uint64(c[p+4])<<24 | uint64(c[p+5])<<16 | uint64(c[p+6])<<8 | uint64(c[p+7])
	l2 := uint64(c[p+8])<<56 | uint64(c[p+9])<<48 | uint64(c[p+10])<<40 | uint64(c[p+11])<<32 |
		uint64(c[p+12])<<24 | uint64(c[p+13])<<16 | uint64(c[p+14])<<8 | uint64(c[p+15])
	l1 := uint64(c[p+16])<<56 | uint64(c[p+17])<<48 | uint64(c[p+18])<<40 | uint64(c[p+19])<<32 |
		uint64(c[p+20])<<24 | uint64(c[p+21])<<16 | uint64(c[p+22])<<8 | uint64(c[p+23])
	l0 := uint64(c[p+24])<<56 | uint64(c[p+25])<<48 | uint64(c[p+26])<<40 | uint64(c[p+27])<<32 |
		uint64(c[p+28])<<24 | uint64(c[p+29])<<16 | uint64(c[p+30])<<8 | uint64(c[p+31])
	interp.Bytecode.pc = p + 32
	s.data[s.top] = types.Uint256{l0, l1, l2, l3}
	s.top++
}

// opPushN — PushVal body for PUSH5-PUSH31 (excluding PUSH20 and PUSH32 which have specialized versions).
// The `op` parameter is the opcode byte (0x64-0x7E).
func opPushN(interp *Interpreter, op byte) {
	s := interp.Stack
	n := int(op - 0x5F) // opcode.PUSH0 = 0x5F, so PUSH5 (0x64) => n=5
	s.data[s.top] = types.U256FromBytes(interp.Bytecode.code[interp.Bytecode.pc : interp.Bytecode.pc+n])
	interp.Bytecode.pc += n
	s.top++
}

// opDupN — Custom handler for EOF DUPN. Fork gate (Amsterdam) handled by generator.
func opDupN(interp *Interpreter) {
	bc := interp.Bytecode
	x := int(bc.code[bc.pc])
	n, ok := decodeSingle(x)
	if !ok {
		interp.Halt(InstructionResultInvalidImmediateEncoding)
		return
	}
	if !interp.Stack.Dup(n) {
		interp.HaltOverflow()
		return
	}
	bc.pc++
}

// opSwapN — Custom handler for EOF SWAPN. Fork gate (Amsterdam) handled by generator.
func opSwapN(interp *Interpreter) {
	bc := interp.Bytecode
	x := int(bc.code[bc.pc])
	n, ok := decodeSingle(x)
	if !ok {
		interp.Halt(InstructionResultInvalidImmediateEncoding)
		return
	}
	if !interp.Stack.Exchange(0, n) {
		interp.HaltOverflow()
		return
	}
	bc.pc++
}

// opExchange — Custom handler for EOF EXCHANGE. Fork gate (Amsterdam) handled by generator.
func opExchange(interp *Interpreter) {
	bc := interp.Bytecode
	x := int(bc.code[bc.pc])
	n, m, ok := decodePair(x)
	if !ok {
		interp.Halt(InstructionResultInvalidImmediateEncoding)
		return
	}
	if !interp.Stack.Exchange(n, m-n) {
		interp.HaltOverflow()
		return
	}
	bc.pc++
}
