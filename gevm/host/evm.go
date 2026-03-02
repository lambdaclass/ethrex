// Top-level EVM executor that orchestrates the full transaction lifecycle:
// validation, pre-execution, execution, and post-execution.
package host

import (
	"sync"

	"github.com/Giulio2002/gevm/precompiles"
	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
	"github.com/Giulio2002/gevm/state"
	"github.com/Giulio2002/gevm/vm"
)

// TxKind distinguishes CALL from CREATE transactions.
type TxKind int

const (
	TxKindCall   TxKind = iota // CALL transaction (with target address)
	TxKindCreate               // CREATE transaction (deploy new contract)
)

// TxType distinguishes transaction formats.
type TxType int

const (
	TxTypeLegacy  TxType = iota // Legacy (pre-EIP-2930)
	TxTypeEIP2930               // EIP-2930 access list
	TxTypeEIP1559               // EIP-1559 dynamic fee
	TxTypeEIP4844               // EIP-4844 blob
	TxTypeEIP7702               // EIP-7702 code delegation
)

// AccessListItem describes a single access list entry.
type AccessListItem struct {
	Address     types.Address
	StorageKeys []types.Uint256
}

// Authorization is an EIP-7702 authorization tuple.
type Authorization struct {
	ChainId types.Uint256
	Address types.Address
	Nonce   uint64
	YParity uint8
	R       types.B256
	S       types.B256
}

// Transaction holds all transaction parameters for EVM execution.
type Transaction struct {
	Kind                 TxKind
	TxType               TxType
	Caller               types.Address
	To                   types.Address // Only valid for TxKindCall
	Value                types.Uint256
	Input                types.Bytes // Calldata for CALL, initcode for CREATE
	GasLimit             uint64
	GasPrice             types.Uint256 // Legacy gas price
	MaxFeePerGas         types.Uint256 // EIP-1559: max total fee per gas
	MaxPriorityFeePerGas types.Uint256 // EIP-1559: max tip per gas
	MaxFeePerBlobGas     types.Uint256 // EIP-4844: max fee per blob gas
	Nonce                uint64
	AccessList           []AccessListItem     // EIP-2930 access list
	BlobHashes           []types.Uint256 // EIP-4844 blob versioned hashes
	AuthorizationList    []Authorization      // EIP-7702 authorization list
}

// ResultKind distinguishes execution outcomes.
type ResultKind int

const (
	ResultSuccess ResultKind = iota
	ResultRevert
	ResultHalt
)

// ExecutionResult holds the final result of EVM transaction execution.
type ExecutionResult struct {
	Kind            ResultKind
	Reason          vm.InstructionResult
	GasUsed         uint64
	GasRefund       int64
	Output          types.Bytes
	Logs            []state.Log
	CreatedAddr     *types.Address // Only for successful CREATE
	ValidationError bool                // True if failure was during tx validation (before execution)
}

// IsSuccess returns true if execution succeeded.
func (r *ExecutionResult) IsSuccess() bool { return r.Kind == ResultSuccess }

// IsRevert returns true if execution reverted.
func (r *ExecutionResult) IsRevert() bool { return r.Kind == ResultRevert }

// IsHalt returns true if execution halted with an error.
func (r *ExecutionResult) IsHalt() bool { return r.Kind == ResultHalt }

// Gas constants for intrinsic gas calculation.
const (
	txBaseGas           uint64 = 21000
	txCreateGas         uint64 = 32000 // Additional gas for CREATE transactions
	txDataZeroGas       uint64 = 4     // Gas per zero byte in calldata
	txDataNonZeroGas    uint64 = 16    // Gas per non-zero byte in calldata (post-Istanbul: 16, pre: 68)
	txDataNonZeroGasOld uint64 = 68    // Pre-Istanbul non-zero byte cost
	initcodeWordGas     uint64 = 2     // EIP-3860: gas per 32-byte word of initcode

	// EIP-7623: calldata gas floor constants
	totalCostFloorPerToken        uint64 = 10 // Floor cost per token
	nonZeroByteTokenMultiplier    uint64 = 4  // Istanbul+ non-zero byte multiplier (16/4)
	nonZeroByteTokenMultiplierOld uint64 = 17 // Pre-Istanbul non-zero byte multiplier (68/4)

	// EIP-7702: authorization list gas constants
	eip7702PerEmptyAccountCost uint64 = 25000 // Per authorization in intrinsic gas
	eip7702PerAuthBaseCost     uint64 = 12500 // Base cost per auth (refund = empty - base)
)

// InitialAndFloorGas holds both the initial intrinsic gas and the EIP-7623 floor gas.
type InitialAndFloorGas struct {
	InitialGas uint64
	FloorGas   uint64
}

// Evm is the top-level EVM execution engine.
type Evm struct {
	Journal        *state.Journal
	Block          BlockEnv
	TxEnv          TxEnv
	Cfg            CfgEnv
	ForkID         spec.ForkID
	Runner         vm.Runner                       // pluggable interpreter (nil = DefaultRunner)
	ReturnAlloc    vm.ReturnDataArena              // arena for pooling RETURN/REVERT output buffers
	host           EvmHost                         // reusable host embedded in pooled Evm (avoids heap escape)
	JumpTableCache map[types.B256][]byte      // cached JUMPDEST bitmaps, persists across pooled reuse

	// Embedded root frame objects: avoid pool Get/Put for depth-0 calls.
	// Initialized lazily on first use, then reused across pooled Evm reuse.
	rootMemory   vm.Memory
	rootStack    vm.Stack
	rootInterp   vm.Interpreter
	rootBytecode vm.Bytecode
}

// evmPool reuses Evm objects across transactions.
var evmPool = sync.Pool{
	New: func() any { return &Evm{} },
}

// NewEvm creates a new Evm instance with pooled Journal and Evm struct.
// Call ReleaseEvm() when done to return objects to pools.
func NewEvm(db state.Database, forkID spec.ForkID, block BlockEnv, cfg CfgEnv) *Evm {
	journal := state.AcquireJournal(db)
	journal.SetForkID(forkID)
	evm := evmPool.Get().(*Evm)
	evm.Journal = journal
	evm.Block = block
	evm.Cfg = cfg
	evm.ForkID = forkID
	return evm
}

// Set sets the Runner on the Evm. Use vm.NewTracingRunner(hooks, forkID)
// for tracing, or nil / vm.DefaultRunner{} for the fast path.
func (evm *Evm) Set(runner vm.Runner) {
	evm.Runner = runner
}

// ReleaseEvm returns the Evm and its Journal to their pools.
func (evm *Evm) ReleaseEvm() {
	evm.ReturnAlloc.Reset()
	evm.Runner = nil
	if evm.Journal != nil {
		state.ReleaseJournal(evm.Journal)
		evm.Journal = nil
	}
	evmPool.Put(evm)
}

// MaxInitCodeSize is the maximum initcode size (EIP-3860): 2 * MaxCodeSize.
const MaxInitCodeSize = 2 * MaxCodeSize

// Transact executes a transaction and returns the result.
func (evm *Evm) Transact(tx *Transaction) ExecutionResult {
	haltResult := func(reason vm.InstructionResult) ExecutionResult {
		return ExecutionResult{
			Kind:            ResultHalt,
			Reason:          reason,
			GasUsed:         tx.GasLimit,
			ValidationError: true, // Pre-execution validation failure
		}
	}

	// --- Phase 1: Validation ---

	// Validate tx type is supported at the current fork
	switch tx.TxType {
	case TxTypeEIP2930:
		if !evm.ForkID.IsEnabledIn(spec.Berlin) {
			return haltResult(vm.InstructionResultInvalidTxType)
		}
	case TxTypeEIP1559:
		if !evm.ForkID.IsEnabledIn(spec.London) {
			return haltResult(vm.InstructionResultInvalidTxType)
		}
	case TxTypeEIP4844:
		if !evm.ForkID.IsEnabledIn(spec.Cancun) {
			return haltResult(vm.InstructionResultInvalidTxType)
		}
	case TxTypeEIP7702:
		if !evm.ForkID.IsEnabledIn(spec.Prague) {
			return haltResult(vm.InstructionResultInvalidTxType)
		}
		// EIP-7702: authorization list must not be empty
		if len(tx.AuthorizationList) == 0 {
			return haltResult(vm.InstructionResultEmptyAuthorizationList)
		}
		// EIP-7702: cannot be CREATE
		if tx.Kind == TxKindCreate {
			return haltResult(vm.InstructionResultCreateNotAllowed)
		}
	}

	// Validate gas price / priority fee
	effectiveGasPrice := tx.GasPrice
	switch tx.TxType {
	case TxTypeLegacy, TxTypeEIP2930:
		// Legacy/EIP-2930: gas price must be >= basefee
		if evm.ForkID.IsEnabledIn(spec.London) {
			if tx.GasPrice.Lt(evm.Block.BaseFee) {
				return haltResult(vm.InstructionResultGasPriceBelowBaseFee)
			}
		}
		effectiveGasPrice = tx.GasPrice
	case TxTypeEIP1559, TxTypeEIP4844, TxTypeEIP7702:
		// EIP-1559+: maxPriorityFeePerGas must not exceed maxFeePerGas
		if tx.MaxPriorityFeePerGas.Gt(tx.MaxFeePerGas) {
			return haltResult(vm.InstructionResultPriorityFeeTooHigh)
		}
		// Effective gas price = min(maxFeePerGas, basefee + maxPriorityFeePerGas)
		effectiveGasPrice = evm.Block.BaseFee.Add(tx.MaxPriorityFeePerGas)
		if effectiveGasPrice.Gt(tx.MaxFeePerGas) {
			effectiveGasPrice = tx.MaxFeePerGas
		}
		// Effective gas price must be >= basefee
		if effectiveGasPrice.Lt(evm.Block.BaseFee) {
			return haltResult(vm.InstructionResultGasPriceBelowBaseFee)
		}
	}

	// EIP-4844: validate blob transaction
	if tx.TxType == TxTypeEIP4844 {
		// Blob gas price must not exceed max fee per blob gas
		if evm.Block.BlobGasPrice.Gt(tx.MaxFeePerBlobGas) {
			return haltResult(vm.InstructionResultBlobGasPriceTooHigh)
		}
		// Must have at least one blob
		if len(tx.BlobHashes) == 0 {
			return haltResult(vm.InstructionResultEmptyBlobs)
		}
		// Max blobs per transaction: 6 (Cancun), 9 (Prague+)
		maxBlobs := 6
		if evm.ForkID.IsEnabledIn(spec.Prague) {
			maxBlobs = 9
		}
		if len(tx.BlobHashes) > maxBlobs {
			return haltResult(vm.InstructionResultTooManyBlobs)
		}
		// All blob hashes must be versioned with 0x01
		for _, h := range tx.BlobHashes {
			b32 := h.ToBytes32()
			if b32[0] != 0x01 {
				return haltResult(vm.InstructionResultInvalidBlobVersion)
			}
		}
		// Blob transactions cannot be CREATE
		if tx.Kind == TxKindCreate {
			return haltResult(vm.InstructionResultCreateNotAllowed)
		}
	}

	// EIP-7825 (Osaka): transaction gas limit cap
	if evm.ForkID.IsEnabledIn(spec.Osaka) && tx.GasLimit > spec.TxGasLimitCap {
		return haltResult(vm.InstructionResultGasLimitTooHigh)
	}

	// Check tx gas limit <= block gas limit
	blockGasLimit := evm.Block.GasLimit.AsUsize()
	if tx.GasLimit > blockGasLimit {
		return haltResult(vm.InstructionResultGasLimitTooHigh)
	}

	// EIP-3860: initcode size limit (Shanghai+)
	if evm.ForkID.IsEnabledIn(spec.Shanghai) && tx.Kind == TxKindCreate {
		if uint64(len(tx.Input)) > MaxInitCodeSize {
			return haltResult(vm.InstructionResultCreateInitCodeSizeLimit)
		}
	}

	// Calculate intrinsic gas and EIP-7623 floor gas
	initAndFloorGas := evm.calcIntrinsicGas(tx)
	intrinsicGas := initAndFloorGas.InitialGas
	if intrinsicGas > tx.GasLimit {
		return haltResult(vm.InstructionResultOutOfGas)
	}

	// EIP-7623: floor gas must not exceed gas limit (Prague+)
	if evm.ForkID.IsEnabledIn(spec.Prague) && initAndFloorGas.FloorGas > tx.GasLimit {
		return haltResult(vm.InstructionResultOutOfGas)
	}

	// --- Phase 2: Pre-execution ---

	// Load and validate caller
	callerResult, err := evm.Journal.LoadAccount(tx.Caller)
	if err != nil {
		return haltResult(vm.InstructionResultFatalExternalError)
	}
	callerAcc := callerResult.Data

	// EIP-3607: Reject transactions from senders with deployed code
	// EIP-7702: allow senders with delegation code (0xef0100 || address)
	if evm.ForkID.IsEnabledIn(spec.Shanghai) && len(callerAcc.Info.Code) > 0 && !IsEIP7702Bytecode(callerAcc.Info.Code) {
		return haltResult(vm.InstructionResultSenderNotEOA)
	}

	// Nonce check
	if callerAcc.Info.Nonce != tx.Nonce {
		return haltResult(vm.InstructionResultNonceMismatch)
	}

	// Balance check: caller must have balance >= max_gas_cost + value
	// For EIP-1559: use maxFeePerGas for balance reservation (not effective price)
	var maxGasPrice types.Uint256
	switch tx.TxType {
	case TxTypeEIP1559, TxTypeEIP4844, TxTypeEIP7702:
		maxGasPrice = tx.MaxFeePerGas
	default:
		maxGasPrice = tx.GasPrice
	}
	gasLimitU := types.U256From(tx.GasLimit)
	maxFee := maxGasPrice.Mul(gasLimitU)
	// Check for overflow: if gasPrice * gasLimit < gasPrice (and gasLimit > 0), overflow occurred
	if tx.GasLimit > 0 && maxFee.Lt(maxGasPrice) {
		return haltResult(vm.InstructionResultOutOfFunds)
	}
	// EIP-4844: include max blob gas cost in balance check
	if tx.TxType == TxTypeEIP4844 && len(tx.BlobHashes) > 0 {
		totalBlobGas := types.U256From(uint64(len(tx.BlobHashes)) * 131072)
		maxBlobCost := tx.MaxFeePerBlobGas.Mul(totalBlobGas)
		maxFee = maxFee.Add(maxBlobCost)
		// Overflow check
		if maxFee.Lt(maxBlobCost) {
			return haltResult(vm.InstructionResultOutOfFunds)
		}
	}
	totalCost := maxFee.Add(tx.Value)
	// Check for overflow in addition: if totalCost < maxFee, overflow occurred
	if totalCost.Lt(maxFee) {
		return haltResult(vm.InstructionResultOutOfFunds)
	}
	if callerAcc.Info.Balance.Lt(totalCost) {
		return haltResult(vm.InstructionResultOutOfFunds)
	}

	// Deduct gas costs from caller balance (using effective gas price).
	// Only gas costs are deducted here; value transfer happens during CALL/CREATE execution.
	gasDeduction := effectiveGasPrice.Mul(types.U256From(tx.GasLimit))

	// EIP-4844: deduct blob gas cost
	var blobGasCost types.Uint256
	if tx.TxType == TxTypeEIP4844 && len(tx.BlobHashes) > 0 {
		totalBlobGas := types.U256From(uint64(len(tx.BlobHashes)) * 131072) // GAS_PER_BLOB = 2^17
		blobGasCost = evm.Block.BlobGasPrice.Mul(totalBlobGas)
		gasDeduction = gasDeduction.Add(blobGasCost)
	}

	callerAcc.Info.Balance = callerAcc.Info.Balance.Sub(gasDeduction)

	// Increment nonce (only for CALL; CREATE bumps nonce in executeCreate)
	if tx.Kind == TxKindCall {
		callerAcc.Info.Nonce++
	}

	// Reuse host embedded in pooled Evm struct (no heap escape).
	// Block and Cfg stored by pointer to avoid ~316 bytes of duffcopy.
	evm.host.Block = &evm.Block
	evm.host.Tx = TxEnv{Caller: tx.Caller, EffectiveGasPrice: effectiveGasPrice, BlobHashes: tx.BlobHashes}
	evm.host.Cfg = &evm.Cfg
	evm.host.Journal = evm.Journal
	evm.host.ForkID = evm.ForkID

	// Use embedded root memory (avoids memoryPool Get/Put).
	vm.InitEmbeddedMemory(&evm.rootMemory)
	rootMemory := &evm.rootMemory

	// Use embedded root interpreter/bytecode (avoids pool Get/Put for depth-0).
	vm.InitEmbeddedInterpreter(&evm.rootInterp, &evm.rootStack)

	if evm.JumpTableCache == nil {
		evm.JumpTableCache = make(map[types.B256][]byte)
	}
	runner := evm.Runner
	if runner == nil {
		runner = vm.DefaultRunner{}
	}
	handler := Handler{
		Host:           &evm.host,
		Precompiles:    precompiles.ForSpec(evm.ForkID),
		RootMemory:     rootMemory,
		ReturnAlloc:    &evm.ReturnAlloc,
		JumpTableCache: evm.JumpTableCache,
		RootInterp:     &evm.rootInterp,
		RootBytecode:   &evm.rootBytecode,
		Runner:         runner,
	}
	// Extract lifecycle hooks from TracingRunner
	if tr, ok := runner.(*vm.TracingRunner); ok {
		handler.hooks = tr.Hooks
	}

	// Warm precompile addresses via shared map pointer (no Account objects created).
	// Precompile accounts are lazily loaded on first CALL; the warm map ensures no cold gas.
	evm.Journal.WarmAddresses.SetPrecompileAddresses(handler.Precompiles.WarmAddressMap())

	// Warm coinbase (EIP-3651, Shanghai+) — just set in WarmAddresses.
	// The account is lazily loaded when accessed (rewardBeneficiary or contract BALANCE).
	if evm.ForkID.IsEnabledIn(spec.Shanghai) {
		evm.Journal.WarmAddresses.SetCoinbase(evm.Block.Beneficiary)
	}

	// Warm access list addresses and storage keys (EIP-2930+)
	for _, item := range tx.AccessList {
		evm.Journal.LoadAccount(item.Address)
		for _, key := range item.StorageKeys {
			evm.Journal.SLoad(item.Address, key)
		}
	}

	// EIP-7702: apply authorization list (Prague+)
	var eip7702Refund int64
	if tx.TxType == TxTypeEIP7702 && len(tx.AuthorizationList) > 0 {
		eip7702Refund = evm.applyEIP7702AuthList(tx)
	}

	// --- Phase 3: Execution ---

	// OnTxStart hook
	if handler.hooks != nil && handler.hooks.OnTxStart != nil {
		handler.hooks.OnTxStart(tx.GasLimit, tx.Caller, tx.To, tx.Value, tx.Input, tx.Kind == TxKindCreate)
	}

	gasAvailable := tx.GasLimit - intrinsicGas

	// Call executeCall/executeCreate directly to avoid FrameInput heap escape.
	var frameResult vm.FrameResult
	switch tx.Kind {
	case TxKindCall:
		callInputs := vm.CallInputs{
			Input:              tx.Input,
			ReturnMemoryOffset: vm.MemoryRange{},
			GasLimit:           gasAvailable,
			BytecodeAddress:    tx.To,
			TargetAddress:      tx.To,
			Caller:             tx.Caller,
			Value:              vm.NewCallValueTransfer(tx.Value),
			Scheme:             vm.CallSchemeCall,
			IsStatic:           false,
		}
		frameResult = vm.NewFrameResultCall(handler.executeCall(&callInputs, 0, rootMemory))

	case TxKindCreate:
		createInputs := vm.CreateInputs{
			Caller:   tx.Caller,
			Scheme:   vm.NewCreateSchemeCreate(),
			Value:    tx.Value,
			InitCode: tx.Input,
			GasLimit: gasAvailable,
		}
		frameResult = vm.NewFrameResultCreate(handler.executeCreate(&createInputs, 0, rootMemory))
	}

	// --- Phase 4: Post-execution ---

	// Extract result from frame
	var interpResult vm.InterpreterResult
	var createdAddr *types.Address

	switch frameResult.Kind {
	case vm.FrameResultCall:
		interpResult = frameResult.Call.Result
	case vm.FrameResultCreate:
		interpResult = frameResult.Create.Result
		createdAddr = frameResult.Create.Address
	}

	// Replace the gas struct with one whose limit = tx.GasLimit (not gasAvailable).
	// This ensures gas.Spent()/gas.Used() include intrinsic gas for all post-execution steps.
	execRemaining := interpResult.Gas.Remaining()
	execRefunded := interpResult.Gas.Refunded()

	gas := vm.NewGasSpent(tx.GasLimit) // limit = tx.GasLimit, remaining = 0
	if interpResult.Result.IsOkOrRevert() {
		gas.EraseCost(execRemaining) // remaining += execRemaining
	}
	if interpResult.Result.IsOk() {
		gas.RecordRefund(execRefunded)
	}
	// EIP-7702: authorization refund is applied unconditionally (even on revert/failure).
	if eip7702Refund > 0 {
		gas.RecordRefund(eip7702Refund)
	}

	// Step 1: Refund cap per EIP-3529 — applied unconditionally.
	// For reverted/failed executions, only eip7702 refund may be non-zero.
	evm.applyRefundLimit(&gas)

	// Step 2: EIP-7623 gas floor check (Prague+).
	if initAndFloorGas.FloorGas > 0 && gas.SpentSubRefunded() < initAndFloorGas.FloorGas {
		gas.SetSpent(initAndFloorGas.FloorGas)
		gas.SetRefund(0)
	}

	// Step 3: Reimburse caller for unused gas.
	reimburseGas := gas.Remaining()
	if gas.Refunded() > 0 {
		reimburseGas += uint64(gas.Refunded())
	}
	refundAmount := effectiveGasPrice.Mul(types.U256From(reimburseGas))
	callerReload, _ := evm.Journal.LoadAccount(tx.Caller)
	callerReload.Data.Info.Balance = callerReload.Data.Info.Balance.Add(refundAmount)

	// Step 4: Reward beneficiary (coinbase).
	// Uses gas.Used() = gas.Spent() - refunded (includes intrinsic gas).
	evm.rewardBeneficiary(effectiveGasPrice, gas.Used())

	// Build execution result. Copy Output and Logs so the result is safe
	// to hold after ReleaseEvm (both alias pooled memory).
	var output types.Bytes
	if len(interpResult.Output) > 0 {
		output = make(types.Bytes, len(interpResult.Output))
		copy(output, interpResult.Output)
	}
	var logs []state.Log
	if len(evm.Journal.Logs) > 0 {
		logs = make([]state.Log, len(evm.Journal.Logs))
		copy(logs, evm.Journal.Logs)
		for i := range logs {
			if len(logs[i].Data) > 0 {
				data := make(types.Bytes, len(logs[i].Data))
				copy(data, logs[i].Data)
				logs[i].Data = data
			}
		}
	}
	result := ExecutionResult{
		GasUsed:     gas.Used(),
		GasRefund:   gas.Refunded(),
		Output:      output,
		Logs:        logs,
		CreatedAddr: createdAddr,
	}

	switch {
	case interpResult.Result.IsOk():
		result.Kind = ResultSuccess
		result.Reason = interpResult.Result
	case interpResult.Result.IsRevert():
		result.Kind = ResultRevert
		result.Reason = interpResult.Result
	default:
		result.Kind = ResultHalt
		result.Reason = interpResult.Result
	}

	// OnTxEnd hook
	if handler.hooks != nil && handler.hooks.OnTxEnd != nil {
		var txErr error
		if !interpResult.Result.IsOk() {
			txErr = interpResult.Result
		}
		handler.hooks.OnTxEnd(result.GasUsed, result.Output, txErr)
	}

	return result
}

// Gas constants for access list intrinsic gas (EIP-2930).
const (
	txAccessListAddressGas uint64 = 2400 // Per address in access list
	txAccessListStorageGas uint64 = 1900 // Per storage key in access list
)

// calcIntrinsicGas calculates the intrinsic gas and EIP-7623 floor gas for a transaction.
func (evm *Evm) calcIntrinsicGas(tx *Transaction) InitialAndFloorGas {
	var result InitialAndFloorGas

	// Count calldata tokens for both initial gas and floor gas.
	// tokens_in_calldata = zero_bytes + non_zero_bytes * multiplier
	var zeroBytes, nonZeroBytes uint64
	for _, b := range tx.Input {
		if b == 0 {
			zeroBytes++
		} else {
			nonZeroBytes++
		}
	}

	// Token multiplier for non-zero bytes
	nonZeroMultiplier := nonZeroByteTokenMultiplier // Istanbul+: 16/4 = 4
	if !evm.ForkID.IsEnabledIn(spec.Istanbul) {
		nonZeroMultiplier = nonZeroByteTokenMultiplierOld // Pre-Istanbul: 68/4 = 17
	}
	tokensInCalldata := zeroBytes + nonZeroBytes*nonZeroMultiplier

	// Initial gas: tokens * 4 (token cost) + base + access list + create
	result.InitialGas = txBaseGas
	result.InitialGas += tokensInCalldata * txDataZeroGas // token_cost = 4

	// CREATE additional cost (Homestead+, EIP-2)
	if tx.Kind == TxKindCreate && evm.ForkID.IsEnabledIn(spec.Homestead) {
		result.InitialGas += txCreateGas

		// EIP-3860: initcode word cost
		if evm.ForkID.IsEnabledIn(spec.Shanghai) {
			words := (uint64(len(tx.Input)) + 31) / 32
			result.InitialGas += words * initcodeWordGas
		}
	}

	// EIP-2930: access list gas
	for _, item := range tx.AccessList {
		_ = item.Address
		result.InitialGas += txAccessListAddressGas
		result.InitialGas += uint64(len(item.StorageKeys)) * txAccessListStorageGas
	}

	// EIP-7702: authorization list gas (PER_EMPTY_ACCOUNT_COST * len)
	if len(tx.AuthorizationList) > 0 {
		result.InitialGas += uint64(len(tx.AuthorizationList)) * eip7702PerEmptyAccountCost
	}

	// EIP-7623: floor gas = TOTAL_COST_FLOOR_PER_TOKEN * tokens + base_gas
	if evm.ForkID.IsEnabledIn(spec.Prague) {
		result.FloorGas = totalCostFloorPerToken*tokensInCalldata + txBaseGas
	}

	return result
}

// applyRefundLimit caps the gas refund per EIP-3529.
func (evm *Evm) applyRefundLimit(gas *vm.Gas) {
	gas.SetFinalRefund(evm.ForkID.IsEnabledIn(spec.London))
}

// EIP-7702 delegation code prefix: 0xef0100
var eip7702Prefix = []byte{0xef, 0x01, 0x00}

// secp256k1HalfN = SECP256K1N / 2, used for EIP-2 s-value check.
// SECP256K1N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
// SECP256K1N/2 = 0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0
// Stored as little-endian limbs: [0]=least significant, [3]=most significant.
var secp256k1HalfN = types.Uint256{
	0xDFE92F46681B20A0, // [0] = least significant
	0x5D576E7357A4501D, // [1]
	0xFFFFFFFFFFFFFFFF, // [2]
	0x7FFFFFFFFFFFFFFF, // [3] = most significant
}

// IsEIP7702Bytecode returns true if code is an EIP-7702 delegation designator (23 bytes: 0xef0100 || address).
func IsEIP7702Bytecode(code []byte) bool {
	return len(code) == 23 && code[0] == 0xef && code[1] == 0x01 && code[2] == 0x00
}

// EIP7702Address extracts the delegated address from EIP-7702 bytecode.
// Returns the address and true if valid, zero and false otherwise.
func EIP7702Address(code []byte) (types.Address, bool) {
	if !IsEIP7702Bytecode(code) {
		return types.Address{}, false
	}
	var addr types.Address
	copy(addr[:], code[3:23])
	return addr, true
}

// rewardBeneficiary pays the coinbase for transaction processing.
func (evm *Evm) rewardBeneficiary(gasPrice types.Uint256, gasUsed uint64) {
	// Calculate effective tip before loading the account.
	var effectivePrice types.Uint256
	if evm.ForkID.IsEnabledIn(spec.London) {
		// London+: base fee is burned, only tip goes to coinbase
		baseFee := evm.Block.BaseFee
		if gasPrice.Gt(baseFee) {
			effectivePrice = gasPrice.Sub(baseFee)
		}
		// else effectivePrice stays zero
	} else {
		effectivePrice = gasPrice
	}

	reward := effectivePrice.Mul(types.U256From(gasUsed))

	// Skip loading the beneficiary account if reward is zero.
	// LoadAccount without a subsequent touch has no observable state effect.
	if reward.IsZero() {
		return
	}

	beneficiary := evm.Block.Beneficiary
	beneficiaryResult, err := evm.Journal.LoadAccount(beneficiary)
	if err != nil {
		return
	}
	beneficiaryResult.Data.Info.Balance = beneficiaryResult.Data.Info.Balance.Add(reward)
}

// applyEIP7702AuthList processes the authorization list for EIP-7702 transactions.
// Returns the total refund to apply (existing authorities get PER_EMPTY_ACCOUNT_COST - PER_AUTH_BASE_COST refund).
func (evm *Evm) applyEIP7702AuthList(tx *Transaction) int64 {
	var refund int64

	for i := range tx.AuthorizationList {
		auth := &tx.AuthorizationList[i]

		// 1. Chain ID check: must be 0 (any chain) or match current chain ID
		if !auth.ChainId.IsZero() && auth.ChainId != evm.Cfg.ChainId {
			continue
		}

		// 2. Nonce max check
		if auth.Nonce == ^uint64(0) {
			continue
		}

		// 3. Recover authority from signature
		authority, ok := recoverEIP7702Authority(auth)
		if !ok {
			continue
		}

		// 4. Load authority account (warms it)
		loadResult, err := evm.Journal.LoadAccount(authority)
		if err != nil {
			continue
		}
		acc := loadResult.Data

		// 5. Authority code must be empty or already an EIP-7702 delegation.
		// acc.Info.Code is nil (LoadAccount only fetches balance/nonce/code_hash),
		// so check CodeHash first and load code only when needed.
		if acc.Info.CodeHash != types.KeccakEmpty {
			if acc.Info.Code == nil {
				if evm.Journal.DB != nil {
					code, _ := evm.Journal.DB.CodeByHash(acc.Info.CodeHash)
					if code != nil {
						acc.Info.Code = code
					}
				}
			}
			if !IsEIP7702Bytecode(acc.Info.Code) {
				continue
			}
		}

		// 6. Nonce must match
		if acc.Info.Nonce != auth.Nonce {
			continue
		}

		// 7. Refund: if authority already existed (not empty), refund partial cost
		if !acc.IsEmpty() {
			refund += int64(eip7702PerEmptyAccountCost - eip7702PerAuthBaseCost)
		}

		// 8. Set delegation code
		if auth.Address == (types.Address{}) {
			// Clear delegation: reset to empty code
			evm.Journal.SetCodeWithHash(authority, nil, types.KeccakEmpty)
		} else {
			// Set delegation code: 0xef0100 || address (23 bytes)
			code := make([]byte, 23)
			copy(code, eip7702Prefix)
			copy(code[3:], auth.Address[:])
			codeHash := types.Keccak256(code)
			evm.Journal.SetCodeWithHash(authority, code, codeHash)
		}

		// 9. Bump nonce
		acc.Info.Nonce++
	}

	return refund
}

// recoverEIP7702Authority recovers the authority address from an EIP-7702 authorization.
// Signing hash: keccak256(0x05 || rlp([chain_id, address, nonce]))
func recoverEIP7702Authority(auth *Authorization) (types.Address, bool) {
	// RLP encode [chain_id, address, nonce]
	chainIdBytes := rlpEncodeU256Compact(auth.ChainId)
	addrBytes := rlpEncodeFixedBytes(auth.Address[:])
	nonceBytes := rlpEncodeUint64Compact(auth.Nonce)

	// RLP list
	listPayload := make([]byte, 0, len(chainIdBytes)+len(addrBytes)+len(nonceBytes))
	listPayload = append(listPayload, chainIdBytes...)
	listPayload = append(listPayload, addrBytes...)
	listPayload = append(listPayload, nonceBytes...)

	var rlpList []byte
	if len(listPayload) < 56 {
		rlpList = make([]byte, 0, 1+len(listPayload))
		rlpList = append(rlpList, 0xc0+byte(len(listPayload)))
		rlpList = append(rlpList, listPayload...)
	} else {
		lenBytes := encodeLength(uint64(len(listPayload)))
		rlpList = make([]byte, 0, 1+len(lenBytes)+len(listPayload))
		rlpList = append(rlpList, 0xf7+byte(len(lenBytes)))
		rlpList = append(rlpList, lenBytes...)
		rlpList = append(rlpList, listPayload...)
	}

	// Prepend EIP-7702 magic: 0x05
	msg := make([]byte, 1+len(rlpList))
	msg[0] = 0x05
	copy(msg[1:], rlpList)

	msgHash := types.Keccak256(msg)

	// EIP-2: reject signatures with s > SECP256K1N/2 (prevents signature malleability).
	// EIP-7702 inherits this constraint from EIP-2 / the transaction validity rules.
	if auth.S.ToU256().Gt(secp256k1HalfN) {
		return types.Address{}, false
	}
	// Also reject y_parity values other than 0 or 1.
	if auth.YParity > 1 {
		return types.Address{}, false
	}

	// Build signature for ecrecover
	var sig [64]byte
	copy(sig[0:32], auth.R[:])
	copy(sig[32:64], auth.S[:])

	result, ok := precompiles.Ecrecover(sig, auth.YParity, [32]byte(msgHash))
	if !ok {
		return types.Address{}, false
	}

	var addr types.Address
	copy(addr[:], result[12:])
	return addr, true
}

// rlpEncodeU256Compact RLP-encodes a U256 as a big-endian integer (minimal encoding).
func rlpEncodeU256Compact(v types.Uint256) []byte {
	if v.IsZero() {
		return []byte{0x80}
	}
	b := v.ToBytes32()
	// Strip leading zeros
	i := 0
	for i < 32 && b[i] == 0 {
		i++
	}
	data := b[i:]
	if len(data) == 1 && data[0] < 0x80 {
		return data
	}
	return append([]byte{0x80 + byte(len(data))}, data...)
}

// rlpEncodeFixedBytes RLP-encodes a fixed-length byte string (like address).
func rlpEncodeFixedBytes(b []byte) []byte {
	if len(b) == 1 && b[0] < 0x80 {
		return b
	}
	if len(b) < 56 {
		return append([]byte{0x80 + byte(len(b))}, b...)
	}
	lenBytes := encodeLength(uint64(len(b)))
	out := make([]byte, 0, 1+len(lenBytes)+len(b))
	out = append(out, 0xb7+byte(len(lenBytes)))
	out = append(out, lenBytes...)
	out = append(out, b...)
	return out
}

// rlpEncodeUint64Compact RLP-encodes a uint64.
func rlpEncodeUint64Compact(v uint64) []byte {
	if v == 0 {
		return []byte{0x80}
	}
	if v < 0x80 {
		return []byte{byte(v)}
	}
	// Big-endian encode
	var buf [8]byte
	n := 0
	for tmp := v; tmp > 0; tmp >>= 8 {
		n++
	}
	for i := n - 1; i >= 0; i-- {
		buf[i] = byte(v)
		v >>= 8
	}
	return append([]byte{0x80 + byte(n)}, buf[:n]...)
}

// encodeLength encodes a length as big-endian bytes (for RLP long strings/lists).
func encodeLength(v uint64) []byte {
	var buf [8]byte
	n := 0
	for tmp := v; tmp > 0; tmp >>= 8 {
		n++
	}
	for i := n - 1; i >= 0; i-- {
		buf[i] = byte(v)
		v >>= 8
	}
	return buf[:n]
}
