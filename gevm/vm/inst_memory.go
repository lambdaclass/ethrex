// Memory and storage opcode handlers: POP, MLOAD, MSTORE, MSTORE8, SLOAD, SSTORE, MCOPY, TLOAD, TSTORE.
package vm

import (
	"github.com/Giulio2002/gevm/spec"
)

// opPop — Pop1 body. Stack check done by boilerplate.
func opPop(interp *Interpreter) {
	interp.Stack.top--
}

// opMload — Custom flush handler. Memory read with fast path.
func opMload(interp *Interpreter) {
	s := interp.Stack
	if s.top == 0 {
		interp.HaltUnderflow()
		return
	}
	top := &s.data[s.top-1]
	if top[1]|top[2]|top[3] == 0 {
		offset := int(top[0])
		if offset >= 0 && offset+32 <= interp.Memory.Len() {
			*top = interp.Memory.GetU256(offset)
		} else {
			if interp.ResizeMemory(offset, 32) {
				*top = interp.Memory.GetU256(offset)
			}
		}
	} else {
		interp.Halt(InstructionResultInvalidOperandOOG)
	}
}

// opMstore — Custom flush handler. Memory write with fast path.
func opMstore(interp *Interpreter) {
	s := interp.Stack
	if s.top < 2 {
		interp.HaltUnderflow()
		return
	}
	s.top -= 2
	offsetVal := s.data[s.top+1]
	value := s.data[s.top]
	if offsetVal[1]|offsetVal[2]|offsetVal[3] == 0 {
		offset := int(offsetVal[0])
		if offset >= 0 && offset+32 <= interp.Memory.Len() {
			interp.Memory.SetU256(offset, value)
		} else {
			if interp.ResizeMemory(offset, 32) {
				interp.Memory.SetU256(offset, value)
			}
		}
	} else {
		interp.Halt(InstructionResultInvalidOperandOOG)
	}
}

// opMstore8 — Custom flush handler.
func opMstore8(interp *Interpreter) {
	s := interp.Stack
	if s.top < 2 {
		interp.HaltUnderflow()
		return
	}
	s.top -= 2
	offsetVal := s.data[s.top+1]
	value := s.data[s.top]
	offset, ok := interp.asUsizeOrFail(offsetVal)
	if !ok {
		return
	}
	if !interp.ResizeMemory(offset, 1) {
		return
	}
	interp.Memory.SetByte(offset, byte(value.LowU64()))
}

// opSload — Custom flush handler (needs Host).
func opSload(interp *Interpreter, host Host) {
	s := interp.Stack
	if s.top == 0 {
		interp.HaltUnderflow()
		return
	}
	top := &s.data[s.top-1]
	var isCold bool
	if interp.Journal != nil {
		isCold, _ = interp.Journal.SLoadInto(interp.Input.TargetAddress, top, top)
	} else {
		isCold = host.SLoadInto(interp.Input.TargetAddress, top, top)
	}
	if interp.RuntimeFlag.ForkID.IsEnabledIn(spec.Berlin) && isCold {
		cost := interp.GasParams.ColdStorageAdditionalCost()
		if !interp.Gas.RecordCost(cost) {
			interp.HaltOOG()
			return
		}
	}
}

// opSstore — Custom flush handler (needs Host). No static gas (handled specially).
func opSstore(interp *Interpreter, host Host) {
	if interp.RuntimeFlag.IsStatic {
		interp.Halt(InstructionResultStateChangeDuringStaticCall)
		return
	}
	s := interp.Stack
	if s.top < 2 {
		interp.HaltUnderflow()
		return
	}
	s.top -= 2
	if interp.RuntimeFlag.ForkID.IsEnabledIn(spec.Istanbul) &&
		interp.Gas.Remaining() <= interp.GasParams.CallStipend() {
		interp.Halt(InstructionResultReentrancySentryOOG)
		return
	}
	staticGas := interp.GasParams.SstoreStaticGas()
	if !interp.Gas.RecordCost(staticGas) {
		interp.HaltOOG()
		return
	}
	result := &interp.SStoreScratch
	if interp.Journal != nil {
		interp.Journal.SStoreInto(interp.Input.TargetAddress,
			&s.data[s.top+1], &s.data[s.top],
			&result.OriginalValue, &result.PresentValue, &result.NewValue, &result.IsCold)
	} else {
		host.SStore(interp.Input.TargetAddress, &s.data[s.top+1], &s.data[s.top], result)
	}
	isIstanbul := interp.RuntimeFlag.ForkID.IsEnabledIn(spec.Istanbul)
	dynamicGas := interp.GasParams.SstoreDynamicGas(isIstanbul, &result.SStoreResult, result.IsCold)
	if !interp.Gas.RecordCost(dynamicGas) {
		interp.HaltOOG()
		return
	}
	refund := interp.GasParams.SstoreRefund(isIstanbul, &result.SStoreResult)
	interp.Gas.RecordRefund(refund)
}

// opMcopy — Custom flush handler. Fork gate (Cancun) checked by generator.
func opMcopy(interp *Interpreter) {
	s := interp.Stack
	if s.top < 3 {
		interp.HaltUnderflow()
		return
	}
	s.top -= 3
	dstVal := s.data[s.top+2]
	srcVal := s.data[s.top+1]
	lenVal := s.data[s.top]
	length, ok := interp.asUsizeOrFail(lenVal)
	if !ok {
		return
	}
	cost := interp.GasParams.McopyGas(uint64(length))
	if !interp.Gas.RecordCost(cost) {
		interp.HaltOOG()
		return
	}
	if length == 0 {
		return
	}
	dst, ok := interp.asUsizeOrFail(dstVal)
	if !ok {
		return
	}
	src, ok := interp.asUsizeOrFail(srcVal)
	if !ok {
		return
	}
	maxOffset := dst
	if src > maxOffset {
		maxOffset = src
	}
	if !interp.ResizeMemory(maxOffset, length) {
		return
	}
	interp.Memory.Copy(dst, src, length)
}

// opTload — Custom handler (needs Host). Fork gate (Cancun) checked by generator.
// UnaryOp-like: pops key, pushes value.
func opTload(interp *Interpreter, host Host) {
	s := interp.Stack
	if s.top == 0 {
		interp.HaltUnderflow()
		return
	}
	top := &s.data[s.top-1]
	*top = host.TLoad(interp.Input.TargetAddress, *top)
}

// opTstore — Custom handler (needs Host). Fork gate (Cancun) checked by generator.
func opTstore(interp *Interpreter, host Host) {
	if interp.RuntimeFlag.IsStatic {
		interp.Halt(InstructionResultStateChangeDuringStaticCall)
		return
	}
	s := interp.Stack
	if s.top < 2 {
		interp.HaltUnderflow()
		return
	}
	s.top -= 2
	key := s.data[s.top+1]
	value := s.data[s.top]
	host.TStore(interp.Input.TargetAddress, key, value)
}
