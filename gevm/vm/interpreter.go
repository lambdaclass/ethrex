package vm

import (
	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
	"github.com/Giulio2002/gevm/state"
)

// ForkGas holds pre-computed static gas costs for the few opcodes whose
// base cost varies across hard forks. 6 fields (48 bytes) replaces the
// former 256-entry [256]uint64 table (2048 bytes).
type ForkGas struct {
	Balance      uint64 // BALANCE (0x31)
	ExtCodeSize  uint64 // EXTCODESIZE (0x3B), EXTCODECOPY (0x3C)
	ExtCodeHash  uint64 // EXTCODEHASH (0x3F)
	Sload        uint64 // SLOAD (0x54)
	Call         uint64 // CALL (0xF1), CALLCODE (0xF2), DELEGATECALL (0xF4), STATICCALL (0xFA)
	Selfdestruct uint64 // SELFDESTRUCT (0xFF)
}

// forkGasCache is pre-computed at init time for every ForkID.
var forkGasCache [20]ForkGas

func init() {
	for i := spec.ForkID(0); i <= spec.Osaka; i++ {
		forkGasCache[i] = buildForkGas(i)
	}
}

// NewForkGas returns the cached ForkGas for the given spec.
func NewForkGas(forkID spec.ForkID) ForkGas {
	return forkGasCache[forkID]
}

// buildForkGas computes fork-varying gas costs matching the former
// buildStaticGasTableForSpec logic in opcode/gas_table.go.
func buildForkGas(forkID spec.ForkID) ForkGas {
	fg := ForkGas{
		Balance:      20,
		ExtCodeSize:  20,
		ExtCodeHash:  400,
		Sload:        50,
		Call:         40,
		Selfdestruct: 0,
	}
	if forkID.IsEnabledIn(spec.Tangerine) {
		fg.Balance = 400
		fg.ExtCodeSize = 700
		fg.Sload = 200
		fg.Call = 700
		fg.Selfdestruct = 5000
	}
	if forkID.IsEnabledIn(spec.Istanbul) {
		fg.Balance = 700
		fg.ExtCodeHash = 700
		fg.Sload = spec.GasIstanbulSloadGas // 800
	}
	if forkID.IsEnabledIn(spec.Berlin) {
		fg.Balance = spec.GasWarmStorageReadCost      // 100
		fg.ExtCodeSize = spec.GasWarmStorageReadCost  // 100
		fg.ExtCodeHash = spec.GasWarmStorageReadCost  // 100
		fg.Sload = spec.GasWarmStorageReadCost        // 100
		fg.Call = spec.GasWarmStorageReadCost          // 100
	}
	return fg
}

// InterpreterResult holds the result of interpreter execution.
type InterpreterResult struct {
	Result InstructionResult
	Output types.Bytes
	Gas    Gas
}

// NewInterpreterResult creates a new InterpreterResult.
func NewInterpreterResult(result InstructionResult, output types.Bytes, gas Gas) InterpreterResult {
	return InterpreterResult{
		Result: result,
		Output: output,
		Gas:    gas,
	}
}

// NewInterpreterResultOOG creates an out-of-gas result with zero remaining gas.
func NewInterpreterResultOOG(gasLimit uint64) InterpreterResult {
	return InterpreterResult{
		Result: InstructionResultOutOfGas,
		Output: nil,
		Gas:    NewGasSpent(gasLimit),
	}
}

// IsOk returns true if the result is a success.
func (r *InterpreterResult) IsOk() bool {
	return r.Result.IsOk()
}

// IsRevert returns true if the result is a revert.
func (r *InterpreterResult) IsRevert() bool {
	return r.Result.IsRevert()
}

// IsError returns true if the result is an error.
func (r *InterpreterResult) IsError() bool {
	return r.Result.IsError()
}

// Inputs holds the input data for the current interpreter execution context.
type Inputs struct {
	TargetAddress   types.Address
	BytecodeAddress types.Address
	CallerAddress   types.Address
	Input           types.Bytes
	CallValue       types.Uint256
}

// RuntimeFlags controls interpreter execution behavior.
type RuntimeFlags struct {
	IsStatic bool
	ForkID   spec.ForkID
}

// Interpreter is the main EVM interpreter that ties together
// bytecode, gas, stack, memory, inputs, and execution control.
type Interpreter struct {
	Bytecode       *Bytecode
	Gas            Gas
	Stack          *Stack
	ReturnData     types.Bytes
	Memory         *Memory
	Input          Inputs
	RuntimeFlag    RuntimeFlags
	GasParams *spec.GasParams
	ForkGas   ForkGas

	// ActionData holds a pending frame action (CALL/CREATE).
	// HasAction == true means the interpreter wants a sub-frame.
	// Embedded to avoid heap allocation on every CALL/CREATE.
	ActionData FrameInput
	HasAction  bool

	// HaltResult stores the InstructionResult when the interpreter halts.
	HaltResult InstructionResult

	// SStoreScratch is reusable scratch space for SSTORE results.
	// Passed by pointer to Host.SStore to avoid 97-byte struct return copy.
	SStoreScratch SStoreResult

	// TopicsScratch is reusable scratch for LOG topics.
	// Passed by pointer to Host.Log to avoid 128-byte array copy.
	TopicsScratch [4]types.B256

	// ReturnAlloc is an optional arena for pooling RETURN/REVERT output buffers.
	// When non-nil, returnInner uses it instead of make([]byte, length).
	ReturnAlloc *ReturnDataArena

	// Journal is a direct reference to the state journal, bypassing Host
	// interface dispatch for SLOAD/SSTORE/LOG hot paths.
	// Set by the handler; nil when using Host interface fallback.
	Journal *state.Journal

	// Depth is the current call depth, set by the handler before Run().
	Depth int
}

// NewInterpreter creates a new Interpreter with the given parameters.
func NewInterpreter(
	memory *Memory,
	bytecode *Bytecode,
	input Inputs,
	isStatic bool,
	forkID spec.ForkID,
	gasLimit uint64,
) *Interpreter {
	gp := spec.NewGasParams(forkID)
	return &Interpreter{
		Bytecode:   bytecode,
		Gas:        NewGas(gasLimit),
		Stack:      NewStack(),
		ReturnData: nil,
		Memory:     memory,
		Input:      input,
		RuntimeFlag: RuntimeFlags{
			IsStatic: isStatic,
			ForkID:   forkID,
		},
		GasParams: gp,
		ForkGas:   NewForkGas(forkID),
	}
}

// DefaultInterpreter creates an Interpreter with default values.
func DefaultInterpreter() *Interpreter {
	return NewInterpreter(
		NewMemory(),
		NewBytecode(nil),
		Inputs{},
		false,
		spec.LatestForkID,
		^uint64(0),
	)
}

// Clear reinitializes the interpreter with new parameters.
func (interp *Interpreter) Clear(
	memory *Memory,
	bytecode *Bytecode,
	input Inputs,
	isStatic bool,
	forkID spec.ForkID,
	gasLimit uint64,
) {
	interp.Bytecode = bytecode
	interp.Gas = NewGas(gasLimit)
	interp.Stack.Clear()
	interp.ReturnData = nil
	interp.Memory = memory
	interp.Input = input
	interp.RuntimeFlag = RuntimeFlags{
		IsStatic: isStatic,
		ForkID:   forkID,
	}
	interp.GasParams = spec.NewGasParams(forkID)
	interp.ForkGas = NewForkGas(forkID)
	interp.HasAction = false
	interp.HaltResult = 0
	// ReturnAlloc and Depth are set externally by the handler; not cleared here.
}

// Halt stops execution with the given result.
func (interp *Interpreter) Halt(result InstructionResult) {
	interp.HaltResult = result
	interp.Bytecode.Stop()
}

// SetAction sets a pending frame action and stops the interpreter loop.
// The outer frame handler should check HasAction after Run() returns.
func (interp *Interpreter) SetAction(action FrameInput) {
	interp.ActionData = action
	interp.HasAction = true
	interp.Bytecode.Stop()
}

// SetCallAction sets the action directly for a CALL-family instruction,
// avoiding intermediate FrameInput struct construction and copies.
func (interp *Interpreter) SetCallAction(inputs CallInputs) {
	interp.ActionData.Kind = FrameInputCall
	interp.ActionData.Call = inputs
	interp.HasAction = true
	interp.Bytecode.Stop()
}

// SetCreateAction sets the action directly for a CREATE instruction,
// avoiding intermediate FrameInput struct construction and copies.
func (interp *Interpreter) SetCreateAction(inputs CreateInputs) {
	interp.ActionData.Kind = FrameInputCreate
	interp.ActionData.Create = inputs
	interp.HasAction = true
	interp.Bytecode.Stop()
}

// ResetAction clears the pending action and resumes the interpreter loop.
// Called by the frame handler after processing a sub-frame result.
func (interp *Interpreter) ResetAction() {
	interp.HasAction = false
	interp.Bytecode.Resume()
}

// InterpreterResultFromHalt constructs an InterpreterResult from the
// interpreter's current halt state.
// Only RETURN and REVERT produce output; all other halt reasons (STOP,
// SELFDESTRUCT, OOG, etc.) produce nil output. This is critical because
// interp.ReturnData is dual-purpose: it holds the RETURNDATASIZE buffer
// (set by sub-call returns) AND the RETURN/REVERT output. Without this
// distinction, stale sub-call return data would leak as the frame's output.
func (interp *Interpreter) InterpreterResultFromHalt() InterpreterResult {
	var output types.Bytes
	if interp.HaltResult == InstructionResultReturn || interp.HaltResult == InstructionResultRevert {
		output = interp.ReturnData
	}
	return InterpreterResult{
		Result: interp.HaltResult,
		Output: output,
		Gas:    interp.Gas,
	}
}

// HaltOOG halts with an out-of-gas error after spending all remaining gas.
func (interp *Interpreter) HaltOOG() {
	interp.Gas.SpendAll()
	interp.Halt(InstructionResultOutOfGas)
}

// HaltMemoryOOG halts with a memory out-of-gas error.
func (interp *Interpreter) HaltMemoryOOG() {
	interp.Halt(InstructionResultMemoryOOG)
}

// HaltOverflow halts with a stack overflow error.
func (interp *Interpreter) HaltOverflow() {
	interp.Halt(InstructionResultStackOverflow)
}

// HaltUnderflow halts with a stack underflow error.
func (interp *Interpreter) HaltUnderflow() {
	interp.Halt(InstructionResultStackUnderflow)
}

// HaltNotActivated halts with a not-activated error.
func (interp *Interpreter) HaltNotActivated() {
	interp.Halt(InstructionResultNotActivated)
}

// ResizeMemory performs EVM memory resize using the interpreter's GasParams.
// Returns true if successful, false if out of gas (and halts the interpreter).
func (interp *Interpreter) ResizeMemory(offset, length int) bool {
	result := ResizeMemory(&interp.Gas, interp.Memory, interp.GasParams, offset, length)
	if result != 0 {
		interp.Halt(result)
		return false
	}
	return true
}

// asUsizeOrFail converts a Uint256 to int, halting with InvalidOperandOOG if it overflows.
// Returns (value, true) on success, (0, false) on failure.
func (interp *Interpreter) asUsizeOrFail(val types.Uint256) (int, bool) {
	if val[1] != 0 || val[2] != 0 || val[3] != 0 || val[0] > uint64(maxInt) {
		interp.Halt(InstructionResultInvalidOperandOOG)
		return 0, false
	}
	return int(val[0]), true
}

// Host is the interface for external environment interactions.
type Host interface {
	// Block info
	Beneficiary() types.Address
	Timestamp() types.Uint256
	BlockNumber() types.Uint256
	Difficulty() types.Uint256
	Prevrandao() *types.Uint256
	GasLimit() types.Uint256
	ChainId() types.Uint256
	BaseFee() types.Uint256
	BlobGasPrice() types.Uint256
	SlotNum() types.Uint256

	// Tx info
	Caller() types.Address // tx origin
	EffectiveGasPrice() types.Uint256
	BlobHash(index int) *types.Uint256

	// Account access
	Balance(addr types.Address) (types.Uint256, bool)  // balance, cold
	CodeSize(addr types.Address) (int, bool)             // size, cold
	CodeHash(addr types.Address) (types.B256, bool) // hash, cold
	Code(addr types.Address) (types.Bytes, bool)    // code, cold
	SelfBalance(addr types.Address) types.Uint256

	// LoadAccountCode loads account for CALL gas calculation.
	// Returns code, code hash, cold flag, and empty flag in one call.
	LoadAccountCode(addr types.Address) AccountCodeLoad

	// Storage access
	// SLoadInto reads a storage value. Key is read by pointer to avoid 32B interface copy.
	// out receives the loaded value (key and out may alias the same stack slot).
	// Returns true if the access was cold.
	SLoadInto(addr types.Address, key *types.Uint256, out *types.Uint256) bool
	// SStore writes a value to storage. Key and value passed by pointer to avoid 64B copy.
	// Results are written into the provided pointer to avoid a 97-byte struct return copy.
	SStore(addr types.Address, key *types.Uint256, value *types.Uint256, out *SStoreResult)
	TLoad(addr types.Address, key types.Uint256) types.Uint256
	TStore(addr types.Address, key types.Uint256, value types.Uint256)

	// Block hash
	BlockHash(number types.Uint256) types.B256

	// Logging — topics passed as fixed array + count to avoid heap allocation.
	Log(addr types.Address, topics *[4]types.B256, numTopics int, data types.Bytes)

	// Self destruct
	SelfDestruct(addr types.Address, target types.Address) SelfDestructResult
}

// AccountCodeLoad holds the result of loading an account for CALL instructions.
type AccountCodeLoad struct {
	Code     types.Bytes
	CodeHash types.B256
	IsCold   bool
	IsEmpty  bool
}

// SStoreResult holds the result of an SSTORE operation.
// Embeds spec.SStoreResult so opSstore can pass &result.SStoreResult
// directly to gas calculation without copying 96 bytes.
type SStoreResult struct {
	spec.SStoreResult
	IsCold bool
}

// SelfDestructResult holds the result of a SELFDESTRUCT operation.
type SelfDestructResult struct {
	HadValue            bool
	TargetExists        bool
	IsCold              bool
	PreviouslyDestroyed bool
}

// maxInt is the maximum value of int on the platform.
const maxInt = int(^uint(0) >> 1)

// --- OpContext interface implementation ---

func (interp *Interpreter) MemoryData() []byte              { return interp.Memory.ContextMemory() }
func (interp *Interpreter) StackData() []types.Uint256  { return interp.Stack.data[:interp.Stack.top] }
func (interp *Interpreter) StackLen() int                    { return interp.Stack.top }
func (interp *Interpreter) CallerAddr() types.Address   { return interp.Input.CallerAddress }
func (interp *Interpreter) ContractAddr() types.Address { return interp.Input.TargetAddress }
func (interp *Interpreter) CallValue() types.Uint256    { return interp.Input.CallValue }
func (interp *Interpreter) CallInput() []byte                { return interp.Input.Input }
