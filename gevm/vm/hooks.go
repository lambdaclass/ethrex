// Tracing hooks for EVM execution.
// When Hooks is nil (the common case), the only overhead per opcode is a
// single `if debug {` branch, predicted false by the CPU (~0.5-1%).
package vm

import "github.com/Giulio2002/gevm/types"

// Hooks holds optional function pointers for tracing EVM execution.
// nil fields are disabled. A cached `debug` boolean in the Run() loop
// avoids repeated nil checks on the hot path.
type Hooks struct {
	// Transaction lifecycle
	OnTxStart func(gasLimit uint64, from, to types.Address,
		value types.Uint256, input []byte, isCreate bool)
	OnTxEnd func(gasUsed uint64, output []byte, err error)

	// Call/Create frame entry/exit
	OnEnter func(depth int, opType byte, from, to types.Address,
		input []byte, gas uint64, value types.Uint256)
	OnExit func(depth int, output []byte, gasUsed uint64,
		err error, reverted bool)

	// Per-opcode (called BEFORE execution with flushed gas)
	OnOpcode func(pc uint64, op byte, gas, cost uint64,
		scope OpContext, rData []byte, depth int, err error)
	OnFault func(pc uint64, op byte, gas, cost uint64,
		scope OpContext, depth int, err error)
}

// OpContext exposes interpreter state to tracers without copying.
type OpContext interface {
	MemoryData() []byte
	StackData() []types.Uint256
	StackLen() int
	CallerAddr() types.Address
	ContractAddr() types.Address
	CallValue() types.Uint256
	CallInput() []byte
}
