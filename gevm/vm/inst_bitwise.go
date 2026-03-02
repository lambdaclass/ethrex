package vm

// Comparison and bitwise opcode handlers.

import "github.com/Giulio2002/gevm/types"

// opLt — BinaryOp body.
func opLt(interp *Interpreter) {
	s := interp.Stack
	if types.LtPtr(&s.data[s.top], &s.data[s.top-1]) {
		s.data[s.top-1] = types.U256One
	} else {
		s.data[s.top-1] = types.U256Zero
	}
}

// opGt — BinaryOp body.
func opGt(interp *Interpreter) {
	s := interp.Stack
	if types.GtPtr(&s.data[s.top], &s.data[s.top-1]) {
		s.data[s.top-1] = types.U256One
	} else {
		s.data[s.top-1] = types.U256Zero
	}
}

// opSlt — BinaryOp body.
func opSlt(interp *Interpreter) {
	s := interp.Stack
	a := &s.data[s.top]
	b := &s.data[s.top-1]
	aNeg := a[3] >> 63
	bNeg := b[3] >> 63
	var lt bool
	if aNeg != bNeg {
		lt = aNeg > bNeg
	} else {
		lt = types.LtPtr(a, b)
	}
	if lt {
		s.data[s.top-1] = types.U256One
	} else {
		s.data[s.top-1] = types.U256Zero
	}
}

// opSgt — BinaryOp body.
func opSgt(interp *Interpreter) {
	s := interp.Stack
	a := &s.data[s.top]
	b := &s.data[s.top-1]
	aNeg := a[3] >> 63
	bNeg := b[3] >> 63
	var gt bool
	if aNeg != bNeg {
		gt = bNeg > aNeg
	} else {
		gt = types.GtPtr(a, b)
	}
	if gt {
		s.data[s.top-1] = types.U256One
	} else {
		s.data[s.top-1] = types.U256Zero
	}
}

// opEq — BinaryOp body.
func opEq(interp *Interpreter) {
	s := interp.Stack
	if types.EqPtr(&s.data[s.top], &s.data[s.top-1]) {
		s.data[s.top-1] = types.U256One
	} else {
		s.data[s.top-1] = types.U256Zero
	}
}

// opIszero — UnaryOp body.
func opIszero(interp *Interpreter) {
	s := interp.Stack
	if types.IsZeroPtr(&s.data[s.top-1]) {
		s.data[s.top-1] = types.U256One
	} else {
		s.data[s.top-1] = types.U256Zero
	}
}

// opAnd — BinaryOp body.
func opAnd(interp *Interpreter) {
	s := interp.Stack
	types.AndTo(&s.data[s.top-1], &s.data[s.top], &s.data[s.top-1])
}

// opOr — BinaryOp body.
func opOr(interp *Interpreter) {
	s := interp.Stack
	types.OrTo(&s.data[s.top-1], &s.data[s.top], &s.data[s.top-1])
}

// opXor — BinaryOp body.
func opXor(interp *Interpreter) {
	s := interp.Stack
	types.XorTo(&s.data[s.top-1], &s.data[s.top], &s.data[s.top-1])
}

// opNot — UnaryOp body.
func opNot(interp *Interpreter) {
	s := interp.Stack
	types.NotTo(&s.data[s.top-1], &s.data[s.top-1])
}

// opByte — BinaryOp body.
func opByte(interp *Interpreter) {
	s := interp.Stack
	a := s.data[s.top]
	top := &s.data[s.top-1]
	idx := a.AsUsizeSaturated()
	if idx < 32 {
		*top = types.U256From(uint64(top.ByteBE(uint(idx))))
	} else {
		*top = types.U256Zero
	}
}

// opShl — BinaryOp body. Fork gate (Constantinople) handled by generator.
func opShl(interp *Interpreter) {
	s := interp.Stack
	shift := s.data[s.top]
	top := &s.data[s.top-1]
	sa := shift.AsUsizeSaturated()
	if sa < 256 {
		*top = top.Shl(uint(sa))
	} else {
		*top = types.U256Zero
	}
}

// opShr — BinaryOp body. Fork gate (Constantinople) handled by generator.
func opShr(interp *Interpreter) {
	s := interp.Stack
	shift := s.data[s.top]
	top := &s.data[s.top-1]
	sa := shift.AsUsizeSaturated()
	if sa < 256 {
		*top = top.Shr(uint(sa))
	} else {
		*top = types.U256Zero
	}
}

// opSar — BinaryOp body. Fork gate (Constantinople) handled by generator.
func opSar(interp *Interpreter) {
	s := interp.Stack
	shift := s.data[s.top]
	top := &s.data[s.top-1]
	sa := shift.AsUsizeSaturated()
	if sa < 256 {
		*top = top.Sar(uint(sa))
	} else if top.Bit(255) {
		*top = types.U256Max
	} else {
		*top = types.U256Zero
	}
}

// opClz — UnaryOp body. Fork gate (Osaka) handled by generator.
func opClz(interp *Interpreter) {
	s := interp.Stack
	top := &s.data[s.top-1]
	*top = types.U256From(uint64(top.LeadingZeros()))
}
