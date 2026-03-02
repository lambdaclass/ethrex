// Arithmetic opcode handlers: ADD, MUL, SUB, DIV, SDIV, MOD, SMOD, ADDMOD, MULMOD, EXP, SIGNEXTEND, KECCAK256.
package vm

import (
	keccak "github.com/Giulio2002/fastkeccak"
	"github.com/Giulio2002/gevm/types"
)

// opAdd — BinaryOp body. s.top already decremented.
func opAdd(interp *Interpreter) {
	s := interp.Stack
	types.AddTo(&s.data[s.top-1], &s.data[s.top], &s.data[s.top-1])
}

// opMul — BinaryOp body.
func opMul(interp *Interpreter) {
	s := interp.Stack
	a := s.data[s.top]
	top := &s.data[s.top-1]
	*top = a.Mul(*top)
}

// opSub — BinaryOp body.
func opSub(interp *Interpreter) {
	s := interp.Stack
	types.SubTo(&s.data[s.top-1], &s.data[s.top], &s.data[s.top-1])
}

// opDiv — BinaryOp body.
func opDiv(interp *Interpreter) {
	s := interp.Stack
	a := s.data[s.top]
	top := &s.data[s.top-1]
	if !types.IsZeroPtr(top) {
		*top = a.Div(*top)
	}
}

// opSdiv — BinaryOp body.
func opSdiv(interp *Interpreter) {
	s := interp.Stack
	a := s.data[s.top]
	top := &s.data[s.top-1]
	*top = types.SDiv(a, *top)
}

// opMod — BinaryOp body.
func opMod(interp *Interpreter) {
	s := interp.Stack
	a := s.data[s.top]
	top := &s.data[s.top-1]
	if !types.IsZeroPtr(top) {
		*top = a.Mod(*top)
	}
}

// opSmod — BinaryOp body.
func opSmod(interp *Interpreter) {
	s := interp.Stack
	a := s.data[s.top]
	top := &s.data[s.top-1]
	*top = types.SMod(a, *top)
}

// opAddmod — TernaryOp body. s.top already decremented by 2.
func opAddmod(interp *Interpreter) {
	s := interp.Stack
	a := s.data[s.top+1]
	b := s.data[s.top]
	top := &s.data[s.top-1]
	*top = a.AddMod(b, *top)
}

// opMulmod — TernaryOp body. s.top already decremented by 2.
func opMulmod(interp *Interpreter) {
	s := interp.Stack
	a := s.data[s.top+1]
	b := s.data[s.top]
	top := &s.data[s.top-1]
	*top = a.MulMod(b, *top)
}

// opSignextend — BinaryOp body.
func opSignextend(interp *Interpreter) {
	s := interp.Stack
	ext := s.data[s.top]
	top := &s.data[s.top-1]
	*top = types.SignExtend(ext, *top)
}

// opExp — Custom flush handler. Full body after gas flush.
// EXP has static gas (GasHigh) charged by generator. This handles stack + dynamic gas.
func opExp(interp *Interpreter) {
	s := interp.Stack
	if s.top < 2 {
		interp.HaltUnderflow()
		return
	}
	s.top--
	base := s.data[s.top]
	top := &s.data[s.top-1]
	cost := interp.GasParams.ExpCost(*top)
	if !interp.Gas.RecordCost(cost) {
		interp.HaltOOG()
		return
	}
	*top = base.Exp(*top)
}

// opKeccak256 — Custom flush handler. Full body after gas flush.
func opKeccak256(interp *Interpreter) {
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
	cost := interp.GasParams.Keccak256Cost(uint64(length))
	if !interp.Gas.RecordCost(cost) {
		interp.HaltOOG()
		return
	}
	var hash types.B256
	if length != 0 {
		offset, ok := interp.asUsizeOrFail(offsetVal)
		if !ok {
			return
		}
		if !interp.ResizeMemory(offset, length) {
			return
		}
		data := interp.Memory.Slice(offset, length)
		hash = types.B256(keccak.Sum256(data))
	} else {
		hash = types.KeccakEmpty
	}
	s.data[s.top] = hash.ToU256()
	s.top++
}
