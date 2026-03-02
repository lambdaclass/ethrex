// Block information opcode handlers: BLOCKHASH through SLOTNUM.
package vm

import (
	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
)

// opBlockhash — UnaryOp body (needs Host). Pops block number, pushes hash.
func opBlockhash(interp *Interpreter, host Host) {
	s := interp.Stack
	top := &s.data[s.top-1]
	hash := host.BlockHash(*top)
	*top = hash.ToU256()
}

// opCoinbase — PushVal body (needs Host).
func opCoinbase(interp *Interpreter, host Host) {
	s := interp.Stack
	addr := host.Beneficiary()
	s.data[s.top] = addr.ToU256()
	s.top++
}

// opTimestamp — PushVal body (needs Host).
func opTimestamp(interp *Interpreter, host Host) {
	s := interp.Stack
	s.data[s.top] = host.Timestamp()
	s.top++
}

// opNumber — PushVal body (needs Host).
func opNumber(interp *Interpreter, host Host) {
	s := interp.Stack
	s.data[s.top] = host.BlockNumber()
	s.top++
}

// opDifficulty — Custom handler (needs Host). Dual-mode: PREVRANDAO post-Merge, DIFFICULTY pre-Merge.
// Fork gate and gas handled by generator. This is a PushVal that needs conditional logic.
func opDifficulty(interp *Interpreter, host Host) {
	if interp.RuntimeFlag.ForkID.IsEnabledIn(spec.Merge) {
		prevrandao := host.Prevrandao()
		if prevrandao != nil {
			s := interp.Stack
			if s.top >= StackLimit {
				interp.HaltOverflow()
				return
			}
			s.data[s.top] = *prevrandao
			s.top++
			return
		}
	}
	s := interp.Stack
	if s.top >= StackLimit {
		interp.HaltOverflow()
		return
	}
	s.data[s.top] = host.Difficulty()
	s.top++
}

// opGaslimit — PushVal body (needs Host).
func opGaslimit(interp *Interpreter, host Host) {
	s := interp.Stack
	s.data[s.top] = host.GasLimit()
	s.top++
}

// opChainid — PushVal body (needs Host). Fork gate (Istanbul) handled by generator.
func opChainid(interp *Interpreter, host Host) {
	s := interp.Stack
	s.data[s.top] = host.ChainId()
	s.top++
}

// opSelfbalance — PushVal body (needs Host). Fork gate (Istanbul) handled by generator.
func opSelfbalance(interp *Interpreter, host Host) {
	s := interp.Stack
	s.data[s.top] = host.SelfBalance(interp.Input.TargetAddress)
	s.top++
}

// opBasefee — PushVal body (needs Host). Fork gate (London) handled by generator.
func opBasefee(interp *Interpreter, host Host) {
	s := interp.Stack
	s.data[s.top] = host.BaseFee()
	s.top++
}

// opBlobhash — Custom handler (needs Host). Fork gate (Cancun) handled by generator.
// UnaryOp-like but uses host call.
func opBlobhash(interp *Interpreter, host Host) {
	s := interp.Stack
	if s.top == 0 {
		interp.HaltUnderflow()
		return
	}
	top := &s.data[s.top-1]
	idx := int(top.AsUsizeSaturated())
	hash := host.BlobHash(idx)
	if hash != nil {
		*top = *hash
	} else {
		*top = types.U256Zero
	}
}

// opBlobbasefee — PushVal body (needs Host). Fork gate (Cancun) handled by generator.
func opBlobbasefee(interp *Interpreter, host Host) {
	s := interp.Stack
	s.data[s.top] = host.BlobGasPrice()
	s.top++
}

// opSlotnum — PushVal body (needs Host). Fork gate (Amsterdam) handled by generator.
func opSlotnum(interp *Interpreter, host Host) {
	s := interp.Stack
	s.data[s.top] = host.SlotNum()
	s.top++
}
