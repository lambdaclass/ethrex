// Tests for EVM tracing hooks and StructLogger.
package trace

import (
	"testing"

	"github.com/Giulio2002/gevm/host"
	"github.com/Giulio2002/gevm/opcode"
	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
	"github.com/Giulio2002/gevm/state"
	"github.com/Giulio2002/gevm/vm"
	spectest "github.com/Giulio2002/gevm/tests/spec"
)

var (
	caller   = types.Address{0x01}
	contract = types.Address{0x10}
	coinbase = types.Address{0xcc}
)

var hugeBalance = types.U256FromLimbs(0, 0, 1, 0)

func blockEnv() host.BlockEnv {
	prevrandao := types.U256Zero
	return host.BlockEnv{
		Beneficiary: coinbase,
		GasLimit:    types.U256From(30_000_000),
		BaseFee:     types.U256From(1),
		Number:      types.U256From(1),
		Timestamp:   types.U256From(1000),
		Prevrandao:  &prevrandao,
	}
}

func runTraced(t *testing.T, code []byte, cfg vm.LogConfig) (*host.ExecutionResult, *vm.StructLogger) {
	t.Helper()
	codeHash := types.Keccak256(code)
	db := spectest.NewMemDB()
	db.InsertAccount(caller, state.AccountInfo{
		Balance:  hugeBalance,
		CodeHash: types.KeccakEmpty,
	}, nil)
	db.InsertAccount(contract, state.AccountInfo{
		Balance:  types.U256Zero,
		Code:     code,
		CodeHash: codeHash,
	}, nil)

	logger := vm.NewStructLogger(cfg)
	evm := host.NewEvm(db, spec.Prague, blockEnv(), host.CfgEnv{ChainId: types.U256From(1)})
	evm.Set(vm.NewTracingRunner(logger.Hooks(), spec.Prague))
	defer evm.ReleaseEvm()

	tx := host.Transaction{
		Kind:     host.TxKindCall,
		Caller:   caller,
		To:       contract,
		GasLimit: 1_000_000,
		GasPrice: types.U256From(1),
	}
	result := evm.Transact(&tx)
	return &result, logger
}

// TestTraceSimpleReturn: PUSH1 0x42, PUSH1 0x00, MSTORE, PUSH1 0x20, PUSH1 0x00, RETURN
func TestTraceSimpleReturn(t *testing.T) {
	code := []byte{
		opcode.PUSH1, 0x42, // PUSH1 0x42
		opcode.PUSH1, 0x00, // PUSH1 0x00
		opcode.MSTORE,      // MSTORE
		opcode.PUSH1, 0x20, // PUSH1 0x20
		opcode.PUSH1, 0x00, // PUSH1 0x00
		opcode.RETURN,      // RETURN
	}

	result, logger := runTraced(t, code, vm.LogConfig{})
	if !result.IsSuccess() {
		t.Fatalf("expected success, got %v", result.Reason)
	}

	logs := logger.Logs()
	if len(logs) != 6 {
		t.Fatalf("expected 6 trace steps, got %d", len(logs))
	}

	// Check opcodes match
	expectedOps := []byte{opcode.PUSH1, opcode.PUSH1, opcode.MSTORE, opcode.PUSH1, opcode.PUSH1, opcode.RETURN}
	for i, want := range expectedOps {
		if logs[i].Op != want {
			t.Errorf("step %d: expected op 0x%02x, got 0x%02x", i, want, logs[i].Op)
		}
	}

	// Check PCs
	expectedPCs := []uint64{0, 2, 4, 5, 7, 9}
	for i, want := range expectedPCs {
		if logs[i].Pc != want {
			t.Errorf("step %d: expected pc %d, got %d", i, want, logs[i].Pc)
		}
	}

	// Check depth is 1 (1-indexed)
	for i, l := range logs {
		if l.Depth != 1 {
			t.Errorf("step %d: expected depth 1, got %d", i, l.Depth)
		}
	}

	// First step should have empty stack
	if len(logs[0].Stack) != 0 {
		t.Errorf("step 0 stack should be empty, got %d items", len(logs[0].Stack))
	}

	// After first PUSH1, stack should have 1 element
	if len(logs[1].Stack) != 1 {
		t.Errorf("step 1 should have 1 stack item, got %d", len(logs[1].Stack))
	}
	if logs[1].Stack[0] != types.U256From(0x42) {
		t.Errorf("step 1 stack[0] = %v, want 0x42", logs[1].Stack[0])
	}

	// After MSTORE, memory should have 32 bytes with 0x42 at the end
	if len(logs[3].Memory) != 32 {
		t.Errorf("step 3 memory should be 32 bytes, got %d", len(logs[3].Memory))
	}
}

// TestTraceStop: simple STOP
func TestTraceStop(t *testing.T) {
	code := []byte{opcode.STOP}
	result, logger := runTraced(t, code, vm.LogConfig{})
	if !result.IsSuccess() {
		t.Fatalf("expected success, got %v", result.Reason)
	}

	logs := logger.Logs()
	if len(logs) != 1 {
		t.Fatalf("expected 1 trace step, got %d", len(logs))
	}
	if logs[0].Op != opcode.STOP {
		t.Errorf("expected STOP, got 0x%02x", logs[0].Op)
	}
	if logs[0].Pc != 0 {
		t.Errorf("expected pc 0, got %d", logs[0].Pc)
	}
}

// TestTraceArithmetic: PUSH1 3, PUSH1 4, ADD → stack=[7]
func TestTraceArithmetic(t *testing.T) {
	code := []byte{
		opcode.PUSH1, 0x03,
		opcode.PUSH1, 0x04,
		opcode.ADD,
		opcode.STOP,
	}
	result, logger := runTraced(t, code, vm.LogConfig{})
	if !result.IsSuccess() {
		t.Fatalf("expected success, got %v", result.Reason)
	}

	logs := logger.Logs()
	if len(logs) != 4 {
		t.Fatalf("expected 4 steps, got %d", len(logs))
	}

	// After ADD (step 3 = STOP, which sees the result), stack should be [7]
	// step 2 = ADD's state before execution: stack = [3, 4]
	if len(logs[2].Stack) != 2 {
		t.Fatalf("step 2 (ADD) should see 2 stack items, got %d", len(logs[2].Stack))
	}

	// step 3 = STOP, stack should have the ADD result [7]
	if len(logs[3].Stack) != 1 {
		t.Fatalf("step 3 (STOP) should see 1 stack item, got %d", len(logs[3].Stack))
	}
	if logs[3].Stack[0] != types.U256From(7) {
		t.Errorf("step 3 stack[0] = %v, want 7", logs[3].Stack[0])
	}
}

// TestTraceDisableStack: stack should be nil when disabled
func TestTraceDisableStack(t *testing.T) {
	code := []byte{opcode.PUSH1, 0x01, opcode.STOP}
	_, logger := runTraced(t, code, vm.LogConfig{DisableStack: true})

	for i, l := range logger.Logs() {
		if l.Stack != nil {
			t.Errorf("step %d: stack should be nil when disabled", i)
		}
	}
}

// TestTraceDisableMemory: memory should be nil when disabled
func TestTraceDisableMemory(t *testing.T) {
	code := []byte{
		opcode.PUSH1, 0x42,
		opcode.PUSH1, 0x00,
		opcode.MSTORE,
		opcode.STOP,
	}
	_, logger := runTraced(t, code, vm.LogConfig{DisableMemory: true})

	for i, l := range logger.Logs() {
		if l.Memory != nil {
			t.Errorf("step %d: memory should be nil when disabled", i)
		}
	}
}

// TestTraceGasDecreases: gas should decrease over the trace
func TestTraceGasDecreases(t *testing.T) {
	code := []byte{
		opcode.PUSH1, 0x01,
		opcode.PUSH1, 0x02,
		opcode.ADD,
		opcode.POP,
		opcode.STOP,
	}
	_, logger := runTraced(t, code, vm.LogConfig{DisableStack: true, DisableMemory: true})

	logs := logger.Logs()
	if len(logs) == 0 {
		t.Fatal("no trace logs")
	}

	// Gas should generally be non-increasing (it can only decrease or stay the same)
	for i := 1; i < len(logs); i++ {
		if logs[i].Gas > logs[i-1].Gas {
			t.Errorf("step %d: gas increased from %d to %d", i, logs[i-1].Gas, logs[i].Gas)
		}
	}
}

// TestTraceTxStartEnd: verify OnTxStart and OnTxEnd are called
func TestTraceTxStartEnd(t *testing.T) {
	var txStartCalled, txEndCalled bool
	var txStartFrom, txStartTo types.Address
	var txEndGasUsed uint64

	code := []byte{opcode.STOP}
	codeHash := types.Keccak256(code)
	db := spectest.NewMemDB()
	db.InsertAccount(caller, state.AccountInfo{Balance: hugeBalance, CodeHash: types.KeccakEmpty}, nil)
	db.InsertAccount(contract, state.AccountInfo{Code: code, CodeHash: codeHash}, nil)

	hooks := &vm.Hooks{
		OnTxStart: func(gasLimit uint64, from, to types.Address, value types.Uint256, input []byte, isCreate bool) {
			txStartCalled = true
			txStartFrom = from
			txStartTo = to
		},
		OnTxEnd: func(gasUsed uint64, output []byte, err error) {
			txEndCalled = true
			txEndGasUsed = gasUsed
		},
	}

	evm := host.NewEvm(db, spec.Prague, blockEnv(), host.CfgEnv{ChainId: types.U256From(1)})
	evm.Set(vm.NewTracingRunner(hooks, spec.Prague))
	defer evm.ReleaseEvm()

	tx := host.Transaction{
		Kind:     host.TxKindCall,
		Caller:   caller,
		To:       contract,
		GasLimit: 100_000,
		GasPrice: types.U256From(1),
	}
	result := evm.Transact(&tx)

	if !txStartCalled {
		t.Error("OnTxStart not called")
	}
	if txStartFrom != caller {
		t.Errorf("OnTxStart from = %v, want %v", txStartFrom, caller)
	}
	if txStartTo != contract {
		t.Errorf("OnTxStart to = %v, want %v", txStartTo, contract)
	}
	if !txEndCalled {
		t.Error("OnTxEnd not called")
	}
	if !result.IsSuccess() {
		t.Errorf("expected success, got %v", result.Reason)
	}
	if txEndGasUsed != result.GasUsed {
		t.Errorf("OnTxEnd gasUsed = %d, want %d", txEndGasUsed, result.GasUsed)
	}
}

// TestTraceEnterExit: verify OnEnter/OnExit are called for frames
func TestTraceEnterExit(t *testing.T) {
	var enterCalls, exitCalls []int

	code := []byte{opcode.STOP}
	codeHash := types.Keccak256(code)
	db := spectest.NewMemDB()
	db.InsertAccount(caller, state.AccountInfo{Balance: hugeBalance, CodeHash: types.KeccakEmpty}, nil)
	db.InsertAccount(contract, state.AccountInfo{Code: code, CodeHash: codeHash}, nil)

	hooks := &vm.Hooks{
		OnEnter: func(depth int, opType byte, from, to types.Address, input []byte, gas uint64, value types.Uint256) {
			enterCalls = append(enterCalls, depth)
		},
		OnExit: func(depth int, output []byte, gasUsed uint64, err error, reverted bool) {
			exitCalls = append(exitCalls, depth)
		},
	}

	evm := host.NewEvm(db, spec.Prague, blockEnv(), host.CfgEnv{ChainId: types.U256From(1)})
	evm.Set(vm.NewTracingRunner(hooks, spec.Prague))
	defer evm.ReleaseEvm()

	tx := host.Transaction{
		Kind:     host.TxKindCall,
		Caller:   caller,
		To:       contract,
		GasLimit: 100_000,
		GasPrice: types.U256From(1),
	}
	evm.Transact(&tx)

	if len(enterCalls) != 1 {
		t.Fatalf("expected 1 OnEnter call, got %d", len(enterCalls))
	}
	if enterCalls[0] != 0 {
		t.Errorf("OnEnter depth = %d, want 0", enterCalls[0])
	}
	if len(exitCalls) != 1 {
		t.Fatalf("expected 1 OnExit call, got %d", len(exitCalls))
	}
	if exitCalls[0] != 0 {
		t.Errorf("OnExit depth = %d, want 0", exitCalls[0])
	}
}

// TestTraceNestedCall: CALL into another contract, verify depth
func TestTraceNestedCall(t *testing.T) {
	// Address 0xAA placed in last byte so PUSH1 0xAA matches
	inner := types.Address{0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xAA}
	innerCode := []byte{opcode.PUSH1, 0x01, opcode.POP, opcode.STOP}
	innerHash := types.Keccak256(innerCode)

	// Outer code: CALL(gas=50000, addr=0xAA, value=0, argOff=0, argLen=0, retOff=0, retLen=0)
	outerCode := []byte{
		opcode.PUSH1, 0x00, // retLen
		opcode.PUSH1, 0x00, // retOff
		opcode.PUSH1, 0x00, // argsLen
		opcode.PUSH1, 0x00, // argsOff
		opcode.PUSH1, 0x00, // value
		opcode.PUSH1, 0xAA, // to (inner)
		opcode.PUSH2, 0xC3, 0x50, // gas (50000)
		opcode.CALL,
		opcode.STOP,
	}
	outerHash := types.Keccak256(outerCode)

	db := spectest.NewMemDB()
	db.InsertAccount(caller, state.AccountInfo{Balance: hugeBalance, CodeHash: types.KeccakEmpty}, nil)
	db.InsertAccount(contract, state.AccountInfo{Code: outerCode, CodeHash: outerHash}, nil)
	db.InsertAccount(inner, state.AccountInfo{Code: innerCode, CodeHash: innerHash}, nil)

	var depths []int
	hooks := &vm.Hooks{
		OnEnter: func(depth int, opType byte, from, to types.Address, input []byte, gas uint64, value types.Uint256) {
			depths = append(depths, depth)
		},
		OnExit: func(depth int, output []byte, gasUsed uint64, err error, reverted bool) {},
	}

	evm := host.NewEvm(db, spec.Prague, blockEnv(), host.CfgEnv{ChainId: types.U256From(1)})
	evm.Set(vm.NewTracingRunner(hooks, spec.Prague))
	defer evm.ReleaseEvm()

	tx := host.Transaction{
		Kind:     host.TxKindCall,
		Caller:   caller,
		To:       contract,
		GasLimit: 200_000,
		GasPrice: types.U256From(1),
	}
	result := evm.Transact(&tx)

	if !result.IsSuccess() {
		t.Fatalf("expected success, got %v", result.Reason)
	}

	// Should have 2 OnEnter calls: depth 0 (outer) and depth 1 (inner)
	if len(depths) != 2 {
		t.Fatalf("expected 2 OnEnter calls, got %d: %v", len(depths), depths)
	}
	if depths[0] != 0 {
		t.Errorf("first OnEnter depth = %d, want 0", depths[0])
	}
	if depths[1] != 1 {
		t.Errorf("second OnEnter depth = %d, want 1", depths[1])
	}
}

// TestTraceRevert: REVERT should show up in the trace
func TestTraceRevert(t *testing.T) {
	code := []byte{
		opcode.PUSH1, 0x00,
		opcode.PUSH1, 0x00,
		opcode.REVERT,
	}
	result, logger := runTraced(t, code, vm.LogConfig{})
	if !result.IsRevert() {
		t.Fatalf("expected revert, got %v", result.Reason)
	}

	logs := logger.Logs()
	if len(logs) != 3 {
		t.Fatalf("expected 3 steps, got %d", len(logs))
	}
	if logs[2].Op != opcode.REVERT {
		t.Errorf("last op should be REVERT, got 0x%02x", logs[2].Op)
	}
}

// TestTraceReset: StructLogger.Reset() clears state
func TestTraceReset(t *testing.T) {
	code := []byte{opcode.PUSH1, 0x01, opcode.STOP}
	_, logger := runTraced(t, code, vm.LogConfig{})

	if len(logger.Logs()) == 0 {
		t.Fatal("should have logs before reset")
	}

	logger.Reset()

	if len(logger.Logs()) != 0 {
		t.Error("logs should be empty after reset")
	}
	if logger.Error() != nil {
		t.Error("error should be nil after reset")
	}
}

// TestTraceNoHooksOverhead: running without hooks should not panic or change results
func TestTraceNoHooksOverhead(t *testing.T) {
	code := []byte{
		opcode.PUSH1, 0x42,
		opcode.PUSH1, 0x00,
		opcode.MSTORE,
		opcode.PUSH1, 0x20,
		opcode.PUSH1, 0x00,
		opcode.RETURN,
	}
	codeHash := types.Keccak256(code)
	db := spectest.NewMemDB()
	db.InsertAccount(caller, state.AccountInfo{Balance: hugeBalance, CodeHash: types.KeccakEmpty}, nil)
	db.InsertAccount(contract, state.AccountInfo{Code: code, CodeHash: codeHash}, nil)

	// Run WITHOUT hooks (normal path)
	evm := host.NewEvm(db, spec.Prague, blockEnv(), host.CfgEnv{ChainId: types.U256From(1)})
	defer evm.ReleaseEvm()

	tx := host.Transaction{
		Kind:     host.TxKindCall,
		Caller:   caller,
		To:       contract,
		GasLimit: 1_000_000,
		GasPrice: types.U256From(1),
	}
	result := evm.Transact(&tx)
	if !result.IsSuccess() {
		t.Fatalf("expected success without hooks, got %v", result.Reason)
	}
	if len(result.Output) != 32 {
		t.Fatalf("expected 32-byte output, got %d", len(result.Output))
	}
}

// TestTraceInvalidOpcode: verify fault hook fires on invalid opcode
func TestTraceInvalidOpcode(t *testing.T) {
	code := []byte{opcode.INVALID}

	result, logger := runTraced(t, code, vm.LogConfig{})
	if result.IsSuccess() {
		t.Fatal("expected halt, got success")
	}

	logs := logger.Logs()
	if len(logs) != 1 {
		t.Fatalf("expected 1 step, got %d", len(logs))
	}
	if logs[0].Op != opcode.INVALID {
		t.Errorf("expected INVALID (0xFE), got 0x%02x", logs[0].Op)
	}
}
