// Control flow and halting opcode handlers.
package vm

import (
	"github.com/Giulio2002/gevm/opcode"
	"github.com/Giulio2002/gevm/types"
)

// opJump — Custom flush handler. Validates jump destination.
func opJump(interp *Interpreter) {
	s := interp.Stack
	if s.top == 0 {
		interp.HaltUnderflow()
		return
	}
	s.top--
	target := s.data[s.top]
	if target[1]|target[2]|target[3] != 0 {
		interp.Halt(InstructionResultInvalidJump)
		return
	}
	dest := int(target[0])
	bc := interp.Bytecode
	if dest >= bc.originalLen || bc.code[dest] != opcode.JUMPDEST {
		interp.Halt(InstructionResultInvalidJump)
		return
	}
	bc.ensureJumpTable()
	if bc.jumpTable[dest/8]&(1<<(uint(dest)%8)) == 0 {
		interp.Halt(InstructionResultInvalidJump)
		return
	}
	bc.pc = dest
}

// opJumpi — Custom flush handler. Conditional jump.
func opJumpi(interp *Interpreter) {
	s := interp.Stack
	if s.top < 2 {
		interp.HaltUnderflow()
		return
	}
	s.top -= 2
	cond := s.data[s.top]
	target := s.data[s.top+1]
	if !cond.IsZero() {
		if target[1]|target[2]|target[3] != 0 {
			interp.Halt(InstructionResultInvalidJump)
			return
		}
		dest := int(target[0])
		bc := interp.Bytecode
		if dest >= bc.originalLen || bc.code[dest] != opcode.JUMPDEST {
			interp.Halt(InstructionResultInvalidJump)
			return
		}
		bc.ensureJumpTable()
		if bc.jumpTable[dest/8]&(1<<(uint(dest)%8)) == 0 {
			interp.Halt(InstructionResultInvalidJump)
			return
		}
		bc.pc = dest
	}
}

// opPc — PushVal body. Pushes current PC (before this instruction).
func opPc(interp *Interpreter) {
	s := interp.Stack
	s.data[s.top] = types.U256From(uint64(interp.Bytecode.pc - 1))
	s.top++
}

// opMsize — PushVal body. Pushes current memory size.
func opMsize(interp *Interpreter) {
	s := interp.Stack
	s.data[s.top] = types.U256From(uint64(interp.Memory.Len()))
	s.top++
}

// opGas — Custom flush handler. Must see correct gas.remaining after flush.
func opGas(interp *Interpreter) {
	s := interp.Stack
	if s.top >= StackLimit {
		interp.HaltOverflow()
		return
	}
	s.data[s.top] = types.U256From(interp.Gas.remaining)
	s.top++
}

// opReturn — Custom flush handler. Halting + memory resize.
func opReturn(interp *Interpreter) {
	s := interp.Stack
	if s.top < 2 {
		interp.HaltUnderflow()
		return
	}
	s.top -= 2
	offsetVal := s.data[s.top+1]
	lenVal := s.data[s.top]
	length, ok := interp.asUsizeOrFail(lenVal)
	if !ok {
		return
	}
	var output types.Bytes
	if length != 0 {
		offset, ok := interp.asUsizeOrFail(offsetVal)
		if !ok {
			return
		}
		if !interp.ResizeMemory(offset, length) {
			return
		}
		if interp.ReturnAlloc != nil {
			output = interp.ReturnAlloc.Alloc(length)
		} else {
			output = make([]byte, length)
		}
		copy(output, interp.Memory.Slice(offset, length))
	}
	interp.ReturnData = output
	interp.Halt(InstructionResultReturn)
}

// opRevert — Custom flush handler. Fork gate (Byzantium) checked by generator.
func opRevert(interp *Interpreter) {
	s := interp.Stack
	if s.top < 2 {
		interp.HaltUnderflow()
		return
	}
	s.top -= 2
	offsetVal := s.data[s.top+1]
	lenVal := s.data[s.top]
	length, ok := interp.asUsizeOrFail(lenVal)
	if !ok {
		return
	}
	var output types.Bytes
	if length != 0 {
		offset, ok := interp.asUsizeOrFail(offsetVal)
		if !ok {
			return
		}
		if !interp.ResizeMemory(offset, length) {
			return
		}
		if interp.ReturnAlloc != nil {
			output = interp.ReturnAlloc.Alloc(length)
		} else {
			output = make([]byte, length)
		}
		copy(output, interp.Memory.Slice(offset, length))
	}
	interp.ReturnData = output
	interp.Halt(InstructionResultRevert)
}

// opSelfdestruct — Custom flush handler (needs Host).
func opSelfdestruct(interp *Interpreter, host Host) {
	if interp.RuntimeFlag.IsStatic {
		interp.Halt(InstructionResultStateChangeDuringStaticCall)
		return
	}
	s := interp.Stack
	if s.top == 0 {
		interp.HaltUnderflow()
		return
	}
	s.top--
	target := s.data[s.top].ToAddress()
	addr := interp.Input.TargetAddress
	result := host.SelfDestruct(addr, target)
	cost := interp.GasParams.SelfdestructCost(result.HadValue && !result.TargetExists, result.IsCold)
	if !interp.Gas.RecordCost(cost) {
		interp.HaltOOG()
		return
	}
	if !result.PreviouslyDestroyed {
		refund := interp.GasParams.SelfdestructRefund()
		interp.Gas.RecordRefund(refund)
	}
	interp.Halt(InstructionResultSelfDestruct)
}

// opStop — Custom handler. Just halts.
func opStop(interp *Interpreter) {
	interp.Halt(InstructionResultStop)
}

// opInvalid — Custom handler. Just halts with invalid opcode.
func opInvalid(interp *Interpreter) {
	interp.Halt(InstructionResultInvalidFEOpcode)
}
