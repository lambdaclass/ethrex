// Implements the frame execution handler that processes CALL/CREATE frames.
package host

import (
	"github.com/Giulio2002/gevm/opcode"
	"github.com/Giulio2002/gevm/precompiles"
	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
	"github.com/Giulio2002/gevm/state"
	"github.com/Giulio2002/gevm/vm"
)

// MaxCodeSize is the maximum contract code size (EIP-170).
const MaxCodeSize = 24576

// CallStackLimit is the maximum call depth.
const CallStackLimit = 1024

// Handler orchestrates EVM frame execution.
// It manages the frame stack, checkpoints, and result propagation.
type Handler struct {
	Host           *EvmHost
	Precompiles    *precompiles.PrecompileSet
	RootMemory     *vm.Memory                      // Root memory for the transaction, shared across frames via child contexts
	ReturnAlloc    *vm.ReturnDataArena              // Arena for pooling RETURN/REVERT output buffers (owned by Evm)
	JumpTableCache map[types.B256][]byte       // Cached JUMPDEST bitmaps keyed by code hash (persists across pooled Evm reuse)
	RootInterp     *vm.Interpreter                  // Embedded root interpreter for depth-0 calls (nil = use pool)
	RootBytecode   *vm.Bytecode                     // Embedded root bytecode for depth-0 calls (nil = use pool)
	Runner         vm.Runner                        // Opcode loop runner (DefaultRunner or TracingRunner)
	hooks          *vm.Hooks                        // Lifecycle hooks (OnEnter/OnExit/OnTxStart/OnTxEnd), extracted from Runner
}

// NewHandler creates a new Handler.
func NewHandler(host *EvmHost, rootMemory *vm.Memory) *Handler {
	return &Handler{
		Host:        host,
		Precompiles: precompiles.ForSpec(host.Journal.Cfg.Spec),
		RootMemory:  rootMemory,
	}
}

// ExecuteFrame executes a single FrameInput (either CALL or CREATE) at the given depth.
// parentMem is the parent frame's Memory (or root memory for depth 0); child contexts
// are created from it to share a single underlying buffer across the call stack.
// Returns the full FrameResult preserving both CallOutcome and CreateOutcome (with address).
func (h *Handler) ExecuteFrame(input *vm.FrameInput, depth int, parentMem *vm.Memory) vm.FrameResult {
	switch input.Kind {
	case vm.FrameInputCall:
		return vm.NewFrameResultCall(h.executeCall(&input.Call, depth, parentMem))
	case vm.FrameInputCreate:
		return vm.NewFrameResultCreate(h.executeCreate(&input.Create, depth, parentMem))
	default:
		return vm.NewFrameResultCall(vm.NewCallOutcome(
			vm.NewInterpreterResult(vm.InstructionResultFatalExternalError, nil, vm.NewGas(0)),
			vm.MemoryRange{},
		))
	}
}

// executeCall handles a CALL-family frame.
// inputs is passed by pointer to avoid ~170 byte struct copy per call.
func (h *Handler) executeCall(inputs *vm.CallInputs, depth int, parentMem *vm.Memory) vm.CallOutcome {
	forkID := h.Host.Journal.Cfg.Spec

	// Depth limit check
	if depth > CallStackLimit {
		return vm.NewCallOutcome(
			vm.NewInterpreterResult(vm.InstructionResultCallTooDeep, nil, vm.NewGas(inputs.GasLimit)),
			inputs.ReturnMemoryOffset,
		)
	}

	// Create journal checkpoint
	checkpoint := h.Host.Journal.Checkpoint()

	// Value transfer (uses Transfer which loads both accounts internally)
	if tv := inputs.Value.TransferValue(); tv != nil && !tv.IsZero() {
		transferErr, _ := h.Host.Journal.Transfer(inputs.Caller, inputs.TargetAddress, *tv)
		if transferErr != nil {
			h.Host.Journal.CheckpointRevert(checkpoint)
			var result vm.InstructionResult
			switch *transferErr {
			case state.TransferErrorOutOfFunds:
				result = vm.InstructionResultOutOfFunds
			case state.TransferErrorOverflowPayment:
				result = vm.InstructionResultOverflowPayment
			default:
				result = vm.InstructionResultFatalExternalError
			}
			return vm.NewCallOutcome(
				vm.NewInterpreterResult(result, nil, vm.NewGas(inputs.GasLimit)),
				inputs.ReturnMemoryOffset,
			)
		}
	}

	// Check if the bytecode address is a precompile
	if precompile := h.Precompiles.Get(inputs.BytecodeAddress); precompile != nil {
		// OnEnter/OnExit for precompile calls
		if h.hooks != nil && h.hooks.OnEnter != nil {
			h.hooks.OnEnter(depth, callSchemeToOpcode(inputs.Scheme), inputs.Caller,
				inputs.BytecodeAddress, inputs.Input, inputs.GasLimit, inputs.Value.Value)
		}
		outcome := h.executePrecompile(precompile, inputs.Input, inputs.GasLimit, inputs.ReturnMemoryOffset, checkpoint)
		if h.hooks != nil && h.hooks.OnExit != nil {
			gasUsed := inputs.GasLimit - outcome.Result.Gas.Remaining()
			h.hooks.OnExit(depth, outcome.Result.Output, gasUsed,
				resultToError(outcome.Result.Result), outcome.Result.Result.IsRevert())
		}
		return outcome
	}

	// Load bytecode for the call target
	acl := h.Host.LoadAccountCode(inputs.BytecodeAddress)
	code := acl.Code

	// EIP-7702: if the code is a delegation designator, follow it
	if delegateAddr, ok := EIP7702Address(code); ok {
		delegateACL := h.Host.LoadAccountCode(delegateAddr)
		code = delegateACL.Code
		acl.CodeHash = delegateACL.CodeHash
	}

	// Empty code: return immediately with success
	if len(code) == 0 {
		// OnEnter/OnExit for empty-code calls (e.g. value transfers)
		if h.hooks != nil && h.hooks.OnEnter != nil {
			h.hooks.OnEnter(depth, callSchemeToOpcode(inputs.Scheme), inputs.Caller,
				inputs.TargetAddress, inputs.Input, inputs.GasLimit, inputs.Value.Value)
		}
		h.Host.Journal.CheckpointCommit()
		outcome := vm.NewCallOutcome(
			vm.NewInterpreterResult(vm.InstructionResultStop, nil, vm.NewGas(inputs.GasLimit)),
			inputs.ReturnMemoryOffset,
		)
		if h.hooks != nil && h.hooks.OnExit != nil {
			h.hooks.OnExit(depth, nil, 0, nil, false)
		}
		return outcome
	}

	// Create child memory context from parent (shares underlying buffer)
	childMem := parentMem.NewChildContext()

	interpInput := vm.Inputs{
		TargetAddress:   inputs.TargetAddress,
		BytecodeAddress: inputs.BytecodeAddress,
		CallerAddress:   inputs.Caller,
		Input:           inputs.Input,
		CallValue:       inputs.Value.Value,
	}

	// Look up cached jump table for this code hash
	var cachedJT []byte
	if h.JumpTableCache != nil {
		cachedJT = h.JumpTableCache[acl.CodeHash]
	}

	// Use embedded objects at depth 0 (avoids 3 pool Get/Put round-trips).
	var interp *vm.Interpreter
	var bc *vm.Bytecode
	var pooled bool
	if depth == 0 && h.RootInterp != nil {
		bc = h.RootBytecode
		vm.ResetEmbeddedBytecodeWithHash(bc, code, acl.CodeHash, cachedJT)
		interp = h.RootInterp
		interp.Clear(childMem, bc, interpInput, inputs.IsStatic, forkID, inputs.GasLimit)
		interp.ReturnAlloc = h.ReturnAlloc
		interp.Journal = h.Host.Journal
	} else {
		bc = vm.AcquireBytecodeWithHash(code, acl.CodeHash, cachedJT)
		interp = vm.AcquireInterpreter(
			childMem,
			bc,
			interpInput,
			inputs.IsStatic,
			forkID,
			inputs.GasLimit,
		)
		interp.ReturnAlloc = h.ReturnAlloc
		interp.Journal = h.Host.Journal
		pooled = true
	}

	// Set depth for tracer hooks
	interp.Depth = depth

	// OnEnter hook
	if h.hooks != nil && h.hooks.OnEnter != nil {
		h.hooks.OnEnter(depth, callSchemeToOpcode(inputs.Scheme), inputs.Caller,
			inputs.TargetAddress, inputs.Input, inputs.GasLimit, inputs.Value.Value)
	}

	// Run with frame handling
	h.runInterpreterLoop(interp, depth)

	// Collect result
	result := interp.InterpreterResultFromHalt()

	// Cache the jump table for reuse by future calls to the same contract
	if h.JumpTableCache != nil {
		if jt := bc.GetJumpTable(); jt != nil {
			h.JumpTableCache[acl.CodeHash] = jt
		}
	}

	// Return pooled objects; embedded objects are owned by Evm.
	if pooled {
		vm.ReleaseInterpreter(interp)
		vm.ReleaseBytecode(bc)
	}
	parentMem.ReleaseChildContext(childMem)

	// Handle checkpoint
	if result.Result.IsOk() {
		h.Host.Journal.CheckpointCommit()
	} else {
		h.Host.Journal.CheckpointRevert(checkpoint)
	}

	// OnExit hook
	if h.hooks != nil && h.hooks.OnExit != nil {
		gasUsed := inputs.GasLimit - result.Gas.Remaining()
		h.hooks.OnExit(depth, result.Output, gasUsed, resultToError(result.Result), result.Result.IsRevert())
	}

	return vm.NewCallOutcome(result, inputs.ReturnMemoryOffset)
}

// executePrecompile runs a precompile contract and returns the call outcome.
func (h *Handler) executePrecompile(
	precompile *precompiles.Precompile,
	input types.Bytes,
	gasLimit uint64,
	retMemOffset vm.MemoryRange,
	checkpoint state.JournalCheckpoint,
) vm.CallOutcome {
	// Initialize result with full gas limit
	gas := vm.NewGas(gasLimit)

	// Execute the precompile
	execResult := precompile.Execute(input, gasLimit)

	if execResult.IsErr() {
		// Error - determine result code
		h.Host.Journal.CheckpointRevert(checkpoint)
		var resultCode vm.InstructionResult
		if *execResult.Err == precompiles.PrecompileErrorOutOfGas {
			resultCode = vm.InstructionResultPrecompileOOG
		} else {
			resultCode = vm.InstructionResultPrecompileError
		}
		return vm.NewCallOutcome(
			vm.NewInterpreterResult(resultCode, nil, gas),
			retMemOffset,
		)
	}

	// Success - handle gas and output
	output := execResult.Output
	gas.RecordRefund(output.GasRefund)
	gas.RecordCost(output.GasUsed)

	var resultCode vm.InstructionResult
	if output.Reverted {
		resultCode = vm.InstructionResultRevert
		h.Host.Journal.CheckpointRevert(checkpoint)
	} else {
		resultCode = vm.InstructionResultReturn
		h.Host.Journal.CheckpointCommit()
	}

	return vm.NewCallOutcome(
		vm.NewInterpreterResult(resultCode, output.Bytes, gas),
		retMemOffset,
	)
}

// executeCreate handles a CREATE/CREATE2 frame.
// inputs is passed by pointer to avoid ~120 byte struct copy per call.
func (h *Handler) executeCreate(inputs *vm.CreateInputs, depth int, parentMem *vm.Memory) vm.CreateOutcome {
	forkID := h.Host.Journal.Cfg.Spec

	// Depth limit check
	if depth > CallStackLimit {
		return vm.NewCreateOutcome(
			vm.NewInterpreterResult(vm.InstructionResultCallTooDeep, nil, vm.NewGas(inputs.GasLimit)),
			nil,
		)
	}

	// Load caller account to check balance and get nonce
	callerResult, err := h.Host.Journal.LoadAccount(inputs.Caller)
	if err != nil {
		return vm.NewCreateOutcome(
			vm.NewInterpreterResult(vm.InstructionResultFatalExternalError, nil, vm.NewGas(inputs.GasLimit)),
			nil,
		)
	}
	callerAcc := callerResult.Data

	// Check caller balance
	if callerAcc.Info.Balance.Lt(inputs.Value) {
		return vm.NewCreateOutcome(
			vm.NewInterpreterResult(vm.InstructionResultOutOfFunds, nil, vm.NewGas(inputs.GasLimit)),
			nil,
		)
	}

	// Get old nonce and bump it
	oldNonce := callerAcc.Info.Nonce
	if oldNonce == ^uint64(0) {
		// Return (Ok) for nonce overflow:
		// so gas is returned to the parent frame. Address=nil means CREATE "failed"
		// but the parent still gets its gas back.
		return vm.NewCreateOutcome(
			vm.NewInterpreterResult(vm.InstructionResultReturn, nil, vm.NewGas(inputs.GasLimit)),
			nil,
		)
	}
	// Bump caller nonce (before address calculation).
	// Record as a journal entry so it can be reverted if the parent frame reverts.
	callerAcc.Info.Nonce = oldNonce + 1
	h.Host.Journal.Entries = append(h.Host.Journal.Entries, state.JournalEntryNonceChange(inputs.Caller, oldNonce))

	// Calculate created address
	var createdAddress types.Address
	switch inputs.Scheme.Kind {
	case vm.CreateSchemeCreate:
		createdAddress = types.CreateAddress(inputs.Caller, oldNonce)
	case vm.CreateSchemeCreate2:
		codeHash := types.Keccak256(inputs.InitCode)
		createdAddress = types.Create2Address(inputs.Caller, inputs.Scheme.Salt.ToBytes32(), codeHash)
	}

	// Warm-load the created address into the journal (required before CreateAccountCheckpoint)
	_, loadErr := h.Host.Journal.LoadAccount(createdAddress)
	if loadErr != nil {
		return vm.NewCreateOutcome(
			vm.NewInterpreterResult(vm.InstructionResultFatalExternalError, nil, vm.NewGas(inputs.GasLimit)),
			nil,
		)
	}

	// Create account checkpoint (creates account, transfers value)
	checkpoint, transferErr := h.Host.Journal.CreateAccountCheckpoint(
		inputs.Caller, createdAddress, inputs.Value, forkID,
	)
	if transferErr != nil {
		var result vm.InstructionResult
		switch *transferErr {
		case state.TransferErrorCreateCollision:
			result = vm.InstructionResultCreateCollision
		case state.TransferErrorOutOfFunds:
			result = vm.InstructionResultOutOfFunds
		case state.TransferErrorOverflowPayment:
			result = vm.InstructionResultOverflowPayment
		default:
			result = vm.InstructionResultFatalExternalError
		}
		return vm.NewCreateOutcome(
			vm.NewInterpreterResult(result, nil, vm.NewGas(inputs.GasLimit)),
			nil,
		)
	}

	// Empty init code: commit and return the address
	if len(inputs.InitCode) == 0 {
		if h.hooks != nil && h.hooks.OnEnter != nil {
			h.hooks.OnEnter(depth, createSchemeToOpcode(inputs.Scheme), inputs.Caller,
				createdAddress, inputs.InitCode, inputs.GasLimit, inputs.Value)
		}
		h.Host.Journal.CheckpointCommit()
		addr := createdAddress
		outcome := vm.NewCreateOutcome(
			vm.NewInterpreterResult(vm.InstructionResultStop, nil, vm.NewGas(inputs.GasLimit)),
			&addr,
		)
		if h.hooks != nil && h.hooks.OnExit != nil {
			h.hooks.OnExit(depth, nil, 0, nil, false)
		}
		return outcome
	}

	// Create child memory context from parent (shares underlying buffer)
	childMem := parentMem.NewChildContext()

	// Create and run the sub-interpreter with init code (pooled)
	interpInput := vm.Inputs{
		TargetAddress: createdAddress,
		CallerAddress: inputs.Caller,
		CallValue:     inputs.Value,
	}

	bc := vm.AcquireBytecode(inputs.InitCode)
	interp := vm.AcquireInterpreter(
		childMem,
		bc,
		interpInput,
		false, // CREATE is never static
		forkID,
		inputs.GasLimit,
	)
	interp.ReturnAlloc = h.ReturnAlloc

	// Set depth for tracer hooks
	interp.Depth = depth

	// OnEnter hook
	if h.hooks != nil && h.hooks.OnEnter != nil {
		h.hooks.OnEnter(depth, createSchemeToOpcode(inputs.Scheme), inputs.Caller,
			createdAddress, inputs.InitCode, inputs.GasLimit, inputs.Value)
	}

	// Run with frame handling
	h.runInterpreterLoop(interp, depth)

	// Collect result
	result := interp.InterpreterResultFromHalt()

	// Return interpreter and bytecode to pools, free child memory
	vm.ReleaseInterpreter(interp)
	vm.ReleaseBytecode(bc)
	parentMem.ReleaseChildContext(childMem)

	// Validate create result
	outcome := h.returnCreate(result, createdAddress, checkpoint, forkID)

	// OnExit hook
	if h.hooks != nil && h.hooks.OnExit != nil {
		gasUsed := inputs.GasLimit - outcome.Result.Gas.Remaining()
		h.hooks.OnExit(depth, outcome.Result.Output, gasUsed,
			resultToError(outcome.Result.Result), outcome.Result.Result.IsRevert())
	}

	return outcome
}

// returnCreate validates the output of a CREATE frame.
func (h *Handler) returnCreate(
	result vm.InterpreterResult,
	createdAddress types.Address,
	checkpoint state.JournalCheckpoint,
	forkID spec.ForkID,
) vm.CreateOutcome {
	if !result.Result.IsOk() {
		h.Host.Journal.CheckpointRevert(checkpoint)
		return vm.NewCreateOutcome(result, nil)
	}

	output := result.Output

	// EIP-3541: reject code starting with 0xEF (London+)
	if forkID.IsEnabledIn(spec.London) && len(output) > 0 && output[0] == 0xEF {
		h.Host.Journal.CheckpointRevert(checkpoint)
		result.Result = vm.InstructionResultCreateContractStartingWithEF
		return vm.NewCreateOutcome(result, nil)
	}

	// EIP-170: code size limit (Spurious Dragon+)
	maxSize := MaxCodeSize
	if forkID.IsEnabledIn(spec.SpuriousDragon) && len(output) > maxSize {
		h.Host.Journal.CheckpointRevert(checkpoint)
		result.Result = vm.InstructionResultCreateContractSizeLimit
		return vm.NewCreateOutcome(result, nil)
	}

	// Code deposit cost
	gp := spec.NewGasParams(forkID)
	depositCost := gp.CodeDepositCost(uint64(len(output)))
	if !result.Gas.RecordCost(depositCost) {
		if forkID.IsEnabledIn(spec.Homestead) {
			// Homestead+: OOG on code deposit
			h.Host.Journal.CheckpointRevert(checkpoint)
			result.Result = vm.InstructionResultOutOfGas
			return vm.NewCreateOutcome(result, nil)
		}
		// Pre-Homestead: no code stored, but still success
		output = nil
	}

	// Commit and store code
	h.Host.Journal.CheckpointCommit()
	if len(output) > 0 {
		codeHash := types.Keccak256(output)
		h.Host.Journal.SetCodeWithHash(createdAddress, output, codeHash)
	}

	addr := createdAddress
	result.Result = vm.InstructionResultReturn
	result.Output = nil // CREATE doesn't return output data
	return vm.NewCreateOutcome(result, &addr)
}

// runInterpreterLoop runs the interpreter, handling sub-frame requests recursively.
func (h *Handler) runInterpreterLoop(interp *vm.Interpreter, depth int) {
	for {
		h.Runner.Run(interp, h.Host)

		// Check if interpreter wants to create a sub-frame
		if !interp.HasAction {
			// Normal halt - done
			return
		}

		// Process the sub-frame (pass pointer to avoid 304-byte copy)
		frameResult := h.ExecuteFrame(&interp.ActionData, depth+1, interp.Memory)
		interp.ResetAction()

		// Handle the sub-frame result and update parent interpreter
		switch frameResult.Kind {
		case vm.FrameResultCall:
			h.handleCallReturn(interp, frameResult.Call)
		case vm.FrameResultCreate:
			h.handleCreateReturn(interp, frameResult.Create)
		}
	}
}

// handleCallReturn processes a call sub-frame result and updates the parent interpreter.
func (h *Handler) handleCallReturn(interp *vm.Interpreter, outcome vm.CallOutcome) {
	result := outcome.Result
	memOffset := outcome.MemoryOffset

	// Set return data buffer on parent
	interp.ReturnData = result.Output

	// Push success (1) or failure (0) to parent stack
	if result.Result.IsOk() {
		interp.Stack.Push(types.U256From(1))
	} else {
		interp.Stack.Push(types.U256Zero)
	}

	// Copy return data to parent memory (if success or revert)
	if result.Result.IsOkOrRevert() && memOffset.Length > 0 {
		copyLen := memOffset.Length
		if len(result.Output) < copyLen {
			copyLen = len(result.Output)
		}
		if copyLen > 0 {
			interp.Memory.Set(memOffset.Offset, result.Output[:copyLen])
		}
	}

	// Return remaining gas to parent
	if result.Result.IsOkOrRevert() {
		interp.Gas.EraseCost(result.Gas.Remaining())
	}

	// Record refunds only on success
	if result.Result.IsOk() {
		interp.Gas.RecordRefund(result.Gas.Refunded())
	}
}

// handleCreateReturn processes a create sub-frame result and updates the parent interpreter.
func (h *Handler) handleCreateReturn(interp *vm.Interpreter, outcome vm.CreateOutcome) {
	result := outcome.Result

	// For revert, set return data to the output; for success, clear it
	if result.Result.IsRevert() {
		interp.ReturnData = result.Output
	} else {
		interp.ReturnData = nil
	}

	// Push created address (on success) or 0 (on failure) to parent stack
	if result.Result.IsOk() && outcome.Address != nil {
		interp.Stack.Push(outcome.Address.ToU256())
	} else {
		interp.Stack.Push(types.U256Zero)
	}

	// Return remaining gas to parent
	if result.Result.IsOkOrRevert() {
		interp.Gas.EraseCost(result.Gas.Remaining())
	}

	// Record refunds only on success
	if result.Result.IsOk() {
		interp.Gas.RecordRefund(result.Gas.Refunded())
	}
}

// callSchemeToOpcode maps a CallScheme to the corresponding opcode byte.
func callSchemeToOpcode(scheme vm.CallScheme) byte {
	switch scheme {
	case vm.CallSchemeCall:
		return opcode.CALL
	case vm.CallSchemeCallCode:
		return opcode.CALLCODE
	case vm.CallSchemeDelegateCall:
		return opcode.DELEGATECALL
	case vm.CallSchemeStaticCall:
		return opcode.STATICCALL
	default:
		return opcode.CALL
	}
}

// createSchemeToOpcode maps a CreateScheme to the corresponding opcode byte.
func createSchemeToOpcode(scheme vm.CreateScheme) byte {
	switch scheme.Kind {
	case vm.CreateSchemeCreate:
		return opcode.CREATE
	case vm.CreateSchemeCreate2:
		return opcode.CREATE2
	default:
		return opcode.CREATE
	}
}

// resultToError converts an InstructionResult to an error for tracer callbacks.
// Returns nil for success results.
func resultToError(result vm.InstructionResult) error {
	if result.IsOk() {
		return nil
	}
	return result
}
