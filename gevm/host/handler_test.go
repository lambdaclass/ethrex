package host

import (
	"testing"

	"github.com/Giulio2002/gevm/opcode"
	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
	"github.com/Giulio2002/gevm/state"
	"github.com/Giulio2002/gevm/vm"
)

// --- Test helpers ---

// execFrame is a test helper: takes FrameInput by value, passes pointer to ExecuteFrame.
func execFrame(h *Handler, input vm.FrameInput, depth int, mem *vm.Memory) vm.FrameResult {
	return h.ExecuteFrame(&input, depth, mem)
}

func makeHandler(db state.Database, forkID spec.ForkID) (*Handler, *EvmHost) {
	journal := state.NewJournal(db)
	journal.SetForkID(forkID)
	host := NewEvmHost(journal, &BlockEnv{}, TxEnv{}, &CfgEnv{})
	rootMem := vm.AcquireMemory()
	handler := NewHandler(host, rootMem)
	handler.Runner = vm.DefaultRunner{}
	return handler, host
}

// preloadAccount loads an account into the journal so it's warm and
// available for TransferLoaded and other operations that assume pre-loading.
func preloadAccount(host *EvmHost, address types.Address) {
	host.Journal.LoadAccount(address)
}

// bytecodeStop returns bytecode that just STOPs immediately.
func bytecodeStop() []byte {
	return []byte{opcode.STOP}
}

// bytecodeReturn returns bytecode: PUSH1 <len> PUSH1 <offset> RETURN
// This returns `len` bytes starting at memory offset `offset`.
func bytecodeReturn(offset, length byte) []byte {
	return []byte{
		opcode.PUSH1, length,
		opcode.PUSH1, offset,
		opcode.RETURN,
	}
}

// bytecodeReturnValue returns bytecode that stores a single byte value in memory
// and returns it: PUSH1 <val> PUSH1 0 MSTORE8 PUSH1 1 PUSH1 0 RETURN
func bytecodeReturnValue(val byte) []byte {
	return []byte{
		opcode.PUSH1, val,
		opcode.PUSH1, 0,
		opcode.MSTORE8,
		opcode.PUSH1, 1,
		opcode.PUSH1, 0,
		opcode.RETURN,
	}
}

// bytecodeRevert returns bytecode: PUSH1 0 PUSH1 0 REVERT
func bytecodeRevert() []byte {
	return []byte{
		opcode.PUSH1, 0,
		opcode.PUSH1, 0,
		opcode.REVERT,
	}
}

// --- Tests ---

func TestCallEmptyAccount(t *testing.T) {
	// CALL to an account with no code should succeed immediately
	db := newMockDB()
	handler, _ := makeHandler(db, spec.Shanghai)

	result := execFrame(handler, vm.NewFrameInputCall(vm.CallInputs{
		Input:              nil,
		ReturnMemoryOffset: vm.MemoryRange{},
		GasLimit:           100000,
		BytecodeAddress:    addr(0x42),
		TargetAddress:      addr(0x42),
		Caller:             addr(0x01),
		Value:              vm.NewCallValueTransfer(types.U256Zero),
		Scheme:             vm.CallSchemeCall,
		IsStatic:           false,
	}), 0, handler.RootMemory)

	if result.Kind != vm.FrameResultCall {
		t.Fatal("expected FrameResultCall")
	}
	if !result.Call.Result.Result.IsOk() {
		t.Fatalf("expected success, got %v", result.Call.Result.Result)
	}
	if result.Call.Result.Gas.Remaining() != 100000 {
		t.Fatalf("gas should be fully returned, got %d", result.Call.Result.Gas.Remaining())
	}
}

func TestCallWithBytecode(t *testing.T) {
	// CALL to an account with STOP bytecode
	db := newMockDB()
	db.accounts[addr(0x42)] = &state.AccountInfo{
		Balance:  u(0),
		CodeHash: types.KeccakEmpty,
		Code:     bytecodeStop(),
	}
	handler, _ := makeHandler(db, spec.Shanghai)

	result := execFrame(handler, vm.NewFrameInputCall(vm.CallInputs{
		GasLimit:        100000,
		BytecodeAddress: addr(0x42),
		TargetAddress:   addr(0x42),
		Caller:          addr(0x01),
		Value:           vm.NewCallValueTransfer(types.U256Zero),
		Scheme:          vm.CallSchemeCall,
	}), 0, handler.RootMemory)

	if !result.Call.Result.Result.IsOk() {
		t.Fatalf("expected success, got %v", result.Call.Result.Result)
	}
}

func TestCallReturnsData(t *testing.T) {
	// CALL to an account that returns a byte
	db := newMockDB()
	db.accounts[addr(0x42)] = &state.AccountInfo{
		Balance:  u(0),
		CodeHash: types.KeccakEmpty,
		Code:     bytecodeReturnValue(0xAB),
	}
	handler, _ := makeHandler(db, spec.Shanghai)

	result := execFrame(handler, vm.NewFrameInputCall(vm.CallInputs{
		GasLimit:        100000,
		BytecodeAddress: addr(0x42),
		TargetAddress:   addr(0x42),
		Caller:          addr(0x01),
		Value:           vm.NewCallValueTransfer(types.U256Zero),
		Scheme:          vm.CallSchemeCall,
	}), 0, handler.RootMemory)

	if result.Call.Result.Result != vm.InstructionResultReturn {
		t.Fatalf("expected Return, got %v", result.Call.Result.Result)
	}
	if len(result.Call.Result.Output) != 1 || result.Call.Result.Output[0] != 0xAB {
		t.Fatalf("expected output [0xAB], got %v", result.Call.Result.Output)
	}
}

func TestCallDepthLimit(t *testing.T) {
	db := newMockDB()
	handler, _ := makeHandler(db, spec.Shanghai)

	result := execFrame(handler, vm.NewFrameInputCall(vm.CallInputs{
		GasLimit:        100000,
		BytecodeAddress: addr(0x42),
		TargetAddress:   addr(0x42),
		Caller:          addr(0x01),
		Value:           vm.NewCallValueTransfer(types.U256Zero),
		Scheme:          vm.CallSchemeCall,
	}), CallStackLimit+1, handler.RootMemory)

	if result.Call.Result.Result != vm.InstructionResultCallTooDeep {
		t.Fatalf("expected CallTooDeep, got %v", result.Call.Result.Result)
	}
	// Gas should be fully returned on depth limit
	if result.Call.Result.Gas.Remaining() != 100000 {
		t.Fatalf("gas should be returned, got %d", result.Call.Result.Gas.Remaining())
	}
}

func TestCallValueTransferInsufficientFunds(t *testing.T) {
	// Caller has 50 wei, tries to send 100
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(50),
		CodeHash: types.KeccakEmpty,
	}
	db.accounts[addr(0x42)] = &state.AccountInfo{
		Balance:  u(0),
		CodeHash: types.KeccakEmpty,
	}
	handler, host := makeHandler(db, spec.Shanghai)
	preloadAccount(host, addr(0x01))
	preloadAccount(host, addr(0x42))

	result := execFrame(handler, vm.NewFrameInputCall(vm.CallInputs{
		GasLimit:        100000,
		BytecodeAddress: addr(0x42),
		TargetAddress:   addr(0x42),
		Caller:          addr(0x01),
		Value:           vm.NewCallValueTransfer(u(100)),
		Scheme:          vm.CallSchemeCall,
	}), 0, handler.RootMemory)

	if result.Call.Result.Result != vm.InstructionResultOutOfFunds {
		t.Fatalf("expected OutOfFunds, got %v", result.Call.Result.Result)
	}
}

func TestCallValueTransferSuccess(t *testing.T) {
	// Caller has 1000 wei, sends 100 to target
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(1000),
		CodeHash: types.KeccakEmpty,
	}
	db.accounts[addr(0x42)] = &state.AccountInfo{
		Balance:  u(0),
		CodeHash: types.KeccakEmpty,
		Code:     bytecodeStop(),
	}
	handler, host := makeHandler(db, spec.Shanghai)
	preloadAccount(host, addr(0x01))
	preloadAccount(host, addr(0x42))

	result := execFrame(handler, vm.NewFrameInputCall(vm.CallInputs{
		GasLimit:        100000,
		BytecodeAddress: addr(0x42),
		TargetAddress:   addr(0x42),
		Caller:          addr(0x01),
		Value:           vm.NewCallValueTransfer(u(100)),
		Scheme:          vm.CallSchemeCall,
	}), 0, handler.RootMemory)

	if !result.Call.Result.Result.IsOk() {
		t.Fatalf("expected success, got %v", result.Call.Result.Result)
	}

	// Verify balances changed
	callerBal, _ := host.Balance(addr(0x01))
	targetBal, _ := host.Balance(addr(0x42))
	if callerBal != u(900) {
		t.Fatalf("caller balance should be 900, got %v", callerBal)
	}
	if targetBal != u(100) {
		t.Fatalf("target balance should be 100, got %v", targetBal)
	}
}

func TestCallRevert(t *testing.T) {
	// CALL to an account that REVERTs - state changes should be rolled back
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(1000),
		CodeHash: types.KeccakEmpty,
	}
	db.accounts[addr(0x42)] = &state.AccountInfo{
		Balance:  u(0),
		CodeHash: types.KeccakEmpty,
		Code:     bytecodeRevert(),
	}
	handler, host := makeHandler(db, spec.Shanghai)
	preloadAccount(host, addr(0x01))
	preloadAccount(host, addr(0x42))

	result := execFrame(handler, vm.NewFrameInputCall(vm.CallInputs{
		GasLimit:        100000,
		BytecodeAddress: addr(0x42),
		TargetAddress:   addr(0x42),
		Caller:          addr(0x01),
		Value:           vm.NewCallValueTransfer(u(100)),
		Scheme:          vm.CallSchemeCall,
	}), 0, handler.RootMemory)

	if result.Call.Result.Result != vm.InstructionResultRevert {
		t.Fatalf("expected Revert, got %v", result.Call.Result.Result)
	}

	// Balances should be unchanged (value transfer reverted)
	callerBal, _ := host.Balance(addr(0x01))
	targetBal, _ := host.Balance(addr(0x42))
	if callerBal != u(1000) {
		t.Fatalf("caller balance should be 1000 (reverted), got %v", callerBal)
	}
	if targetBal != u(0) {
		t.Fatalf("target balance should be 0 (reverted), got %v", targetBal)
	}
}

func TestCreateEmptyInitCode(t *testing.T) {
	// CREATE with empty init code: should create account and return address
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(1000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	handler, _ := makeHandler(db, spec.Shanghai)

	result := execFrame(handler, vm.NewFrameInputCreate(vm.CreateInputs{
		Caller:   addr(0x01),
		Scheme:   vm.NewCreateSchemeCreate(),
		Value:    u(0),
		InitCode: nil,
		GasLimit: 100000,
	}), 0, handler.RootMemory)

	if result.Kind != vm.FrameResultCreate {
		t.Fatal("expected FrameResultCreate")
	}
	if !result.Create.Result.Result.IsOk() {
		t.Fatalf("expected success, got %v", result.Create.Result.Result)
	}
	if result.Create.Address == nil {
		t.Fatal("expected non-nil created address")
	}

	// Verify address matches CREATE(addr(0x01), nonce=0)
	expectedAddr := types.CreateAddress(addr(0x01), 0)
	if *result.Create.Address != expectedAddr {
		t.Fatalf("wrong created address: got %s, want %s",
			result.Create.Address.Hex(), expectedAddr.Hex())
	}
}

func TestCreateWithInitCode(t *testing.T) {
	// CREATE with init code that returns a single-byte contract
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(1000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	handler, host := makeHandler(db, spec.Shanghai)

	// Init code returns 0xFE (a single INVALID opcode as deployed bytecode)
	initCode := bytecodeReturnValue(0xFE)

	result := execFrame(handler, vm.NewFrameInputCreate(vm.CreateInputs{
		Caller:   addr(0x01),
		Scheme:   vm.NewCreateSchemeCreate(),
		Value:    u(0),
		InitCode: initCode,
		GasLimit: 1000000,
	}), 0, handler.RootMemory)

	if result.Create.Result.Result != vm.InstructionResultReturn {
		t.Fatalf("expected Return, got %v", result.Create.Result.Result)
	}
	if result.Create.Address == nil {
		t.Fatal("expected non-nil created address")
	}

	createdAddr := *result.Create.Address

	// CREATE should not return output data (code is stored in state, not returned)
	if len(result.Create.Result.Output) != 0 {
		t.Fatalf("CREATE should not return output data, got %d bytes", len(result.Create.Result.Output))
	}

	// Verify the deployed code is stored in the journal
	codeResult, _ := host.Journal.LoadAccount(createdAddr)
	if codeResult.Data.Info.Code == nil || len(codeResult.Data.Info.Code) != 1 || codeResult.Data.Info.Code[0] != 0xFE {
		t.Fatalf("expected deployed code [0xFE], got %v", codeResult.Data.Info.Code)
	}
}

func TestCreateWithValue(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(1000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	handler, host := makeHandler(db, spec.Shanghai)

	result := execFrame(handler, vm.NewFrameInputCreate(vm.CreateInputs{
		Caller:   addr(0x01),
		Scheme:   vm.NewCreateSchemeCreate(),
		Value:    u(500),
		InitCode: nil,
		GasLimit: 100000,
	}), 0, handler.RootMemory)

	if !result.Create.Result.Result.IsOk() {
		t.Fatalf("expected success, got %v", result.Create.Result.Result)
	}

	createdAddr := *result.Create.Address

	// Verify balances
	callerBal, _ := host.Balance(addr(0x01))
	createdBal, _ := host.Balance(createdAddr)
	if callerBal != u(500) {
		t.Fatalf("caller balance should be 500, got %v", callerBal)
	}
	if createdBal != u(500) {
		t.Fatalf("created balance should be 500, got %v", createdBal)
	}
}

func TestCreateInsufficientBalance(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(50),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	handler, _ := makeHandler(db, spec.Shanghai)

	result := execFrame(handler, vm.NewFrameInputCreate(vm.CreateInputs{
		Caller:   addr(0x01),
		Scheme:   vm.NewCreateSchemeCreate(),
		Value:    u(100),
		InitCode: nil,
		GasLimit: 100000,
	}), 0, handler.RootMemory)

	if result.Create.Result.Result != vm.InstructionResultOutOfFunds {
		t.Fatalf("expected OutOfFunds, got %v", result.Create.Result.Result)
	}
	if result.Create.Address != nil {
		t.Fatal("should not return address on failure")
	}
}

func TestCreateDepthLimit(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(1000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	handler, _ := makeHandler(db, spec.Shanghai)

	result := execFrame(handler, vm.NewFrameInputCreate(vm.CreateInputs{
		Caller:   addr(0x01),
		Scheme:   vm.NewCreateSchemeCreate(),
		Value:    u(0),
		InitCode: nil,
		GasLimit: 100000,
	}), CallStackLimit+1, handler.RootMemory)

	if result.Create.Result.Result != vm.InstructionResultCallTooDeep {
		t.Fatalf("expected CallTooDeep, got %v", result.Create.Result.Result)
	}
}

func TestCreateNonceOverflow(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(1000),
		Nonce:    ^uint64(0), // max nonce
		CodeHash: types.KeccakEmpty,
	}
	handler, _ := makeHandler(db, spec.Shanghai)

	result := execFrame(handler, vm.NewFrameInputCreate(vm.CreateInputs{
		Caller:   addr(0x01),
		Scheme:   vm.NewCreateSchemeCreate(),
		Value:    u(0),
		InitCode: nil,
		GasLimit: 100000,
	}), 0, handler.RootMemory)

	// Return (Ok) for nonce overflow so gas is returned to parent.
	// Address=nil signals CREATE failed.
	if result.Create.Result.Result != vm.InstructionResultReturn {
		t.Fatalf("expected Return, got %v", result.Create.Result.Result)
	}
	if result.Create.Address != nil {
		t.Fatalf("expected nil address for nonce overflow")
	}
}

func TestCreate2Address(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(1000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	handler, _ := makeHandler(db, spec.Shanghai)

	salt := types.U256From(0x42)
	initCode := bytecodeStop()

	result := execFrame(handler, vm.NewFrameInputCreate(vm.CreateInputs{
		Caller:   addr(0x01),
		Scheme:   vm.NewCreateSchemeCreate2(salt),
		Value:    u(0),
		InitCode: initCode,
		GasLimit: 1000000,
	}), 0, handler.RootMemory)

	if !result.Create.Result.Result.IsOk() {
		t.Fatalf("expected success, got %v", result.Create.Result.Result)
	}
	if result.Create.Address == nil {
		t.Fatal("expected address")
	}

	// Verify address matches CREATE2 formula
	codeHash := types.Keccak256(initCode)
	expectedAddr := types.Create2Address(addr(0x01), salt.ToBytes32(), codeHash)
	if *result.Create.Address != expectedAddr {
		t.Fatalf("wrong CREATE2 address: got %s, want %s",
			result.Create.Address.Hex(), expectedAddr.Hex())
	}
}

func TestCreateCodeSizeLimit(t *testing.T) {
	// EIP-170: code > 24576 bytes should fail on Spurious Dragon+
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(1000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}

	// Build init code that returns MaxCodeSize+1 bytes
	// We'll build bytecode that: PUSH2 <size> PUSH1 0 RETURN
	oversizeLen := MaxCodeSize + 1

	// We need init code that stores oversizeLen bytes in memory and returns them.
	// For simplicity, use PUSH2 <oversizeLen> PUSH1 0 RETURN.
	// Memory will be all zeros but that's fine - we just need the size.
	initCode := []byte{
		opcode.PUSH2, byte(oversizeLen >> 8), byte(oversizeLen),
		opcode.PUSH1, 0,
		opcode.RETURN,
	}

	handler, _ := makeHandler(db, spec.Shanghai)

	result := execFrame(handler, vm.NewFrameInputCreate(vm.CreateInputs{
		Caller:   addr(0x01),
		Scheme:   vm.NewCreateSchemeCreate(),
		Value:    u(0),
		InitCode: initCode,
		GasLimit: 100_000_000, // lots of gas for memory expansion
	}), 0, handler.RootMemory)

	if result.Create.Result.Result != vm.InstructionResultCreateContractSizeLimit {
		t.Fatalf("expected CreateContractSizeLimit, got %v", result.Create.Result.Result)
	}
	if result.Create.Address != nil {
		t.Fatal("should not return address on size limit failure")
	}
}

func TestCreateEIP3541RejectEF(t *testing.T) {
	// EIP-3541: code starting with 0xEF should be rejected on London+
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(1000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}

	// Init code that returns [0xEF]
	initCode := bytecodeReturnValue(0xEF)

	handler, _ := makeHandler(db, spec.Shanghai) // London+

	result := execFrame(handler, vm.NewFrameInputCreate(vm.CreateInputs{
		Caller:   addr(0x01),
		Scheme:   vm.NewCreateSchemeCreate(),
		Value:    u(0),
		InitCode: initCode,
		GasLimit: 1000000,
	}), 0, handler.RootMemory)

	if result.Create.Result.Result != vm.InstructionResultCreateContractStartingWithEF {
		t.Fatalf("expected CreateContractStartingWithEF, got %v", result.Create.Result.Result)
	}
}

func TestCreateEIP3541PreLondon(t *testing.T) {
	// Pre-London: code starting with 0xEF should be allowed
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(1000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}

	initCode := bytecodeReturnValue(0xEF)

	handler, _ := makeHandler(db, spec.Berlin) // pre-London

	result := execFrame(handler, vm.NewFrameInputCreate(vm.CreateInputs{
		Caller:   addr(0x01),
		Scheme:   vm.NewCreateSchemeCreate(),
		Value:    u(0),
		InitCode: initCode,
		GasLimit: 1000000,
	}), 0, handler.RootMemory)

	// Should succeed pre-London
	if result.Create.Result.Result != vm.InstructionResultReturn {
		t.Fatalf("expected Return (pre-London allows 0xEF), got %v", result.Create.Result.Result)
	}
	if result.Create.Address == nil {
		t.Fatal("expected address")
	}
}

func TestFrameResultTypes(t *testing.T) {
	// Verify FrameResult tagged union correctness
	callOutcome := vm.NewCallOutcome(
		vm.NewInterpreterResult(vm.InstructionResultStop, nil, vm.NewGas(1000)),
		vm.MemoryRange{Offset: 10, Length: 20},
	)
	fr := vm.NewFrameResultCall(callOutcome)
	if fr.Kind != vm.FrameResultCall {
		t.Fatal("expected FrameResultCall kind")
	}
	if fr.Call.MemoryOffset.Offset != 10 || fr.Call.MemoryOffset.Length != 20 {
		t.Fatal("wrong memory offset in call outcome")
	}

	a := addr(0x42)
	createOutcome := vm.NewCreateOutcome(
		vm.NewInterpreterResult(vm.InstructionResultReturn, nil, vm.NewGas(2000)),
		&a,
	)
	fr2 := vm.NewFrameResultCreate(createOutcome)
	if fr2.Kind != vm.FrameResultCreate {
		t.Fatal("expected FrameResultCreate kind")
	}
	if fr2.Create.Address == nil || *fr2.Create.Address != addr(0x42) {
		t.Fatal("wrong address in create outcome")
	}
}

func TestHandlerCallReturnGas(t *testing.T) {
	// Verify that gas is properly returned from sub-call
	// Use bytecode that returns a value (spends gas on PUSH/MSTORE/RETURN + memory)
	db := newMockDB()
	db.accounts[addr(0x42)] = &state.AccountInfo{
		Balance:  u(0),
		CodeHash: types.KeccakEmpty,
		Code:     bytecodeReturnValue(0x42),
	}
	handler, _ := makeHandler(db, spec.Shanghai)

	result := execFrame(handler, vm.NewFrameInputCall(vm.CallInputs{
		GasLimit:        100000,
		BytecodeAddress: addr(0x42),
		TargetAddress:   addr(0x42),
		Caller:          addr(0x01),
		Value:           vm.NewCallValueTransfer(types.U256Zero),
		Scheme:          vm.CallSchemeCall,
	}), 0, handler.RootMemory)

	remaining := result.Call.Result.Gas.Remaining()
	if remaining == 0 {
		t.Fatal("gas remaining should be > 0")
	}
	if remaining >= 100000 {
		t.Fatal("some gas should have been spent on PUSH/MSTORE/RETURN + memory")
	}
}

func TestHandlerCallApparentValue(t *testing.T) {
	// DELEGATECALL uses Apparent value - should not actually transfer
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(1000),
		CodeHash: types.KeccakEmpty,
	}
	db.accounts[addr(0x42)] = &state.AccountInfo{
		Balance:  u(0),
		CodeHash: types.KeccakEmpty,
		Code:     bytecodeStop(),
	}
	handler, host := makeHandler(db, spec.Shanghai)
	preloadAccount(host, addr(0x01))
	preloadAccount(host, addr(0x42))

	result := execFrame(handler, vm.NewFrameInputCall(vm.CallInputs{
		GasLimit:        100000,
		BytecodeAddress: addr(0x42),
		TargetAddress:   addr(0x01), // DELEGATECALL: target is self
		Caller:          addr(0x01),
		Value:           vm.NewCallValueApparent(u(100)), // Apparent, not transferred
		Scheme:          vm.CallSchemeDelegateCall,
	}), 0, handler.RootMemory)

	if !result.Call.Result.Result.IsOk() {
		t.Fatalf("expected success, got %v", result.Call.Result.Result)
	}

	// Balances should be unchanged (apparent value is not transferred)
	callerBal, _ := host.Balance(addr(0x01))
	if callerBal != u(1000) {
		t.Fatalf("caller balance should be unchanged at 1000, got %v", callerBal)
	}
}

// --- Precompile routing tests ---

func TestCallPrecompileIdentity(t *testing.T) {
	// CALL to address 0x04 (IDENTITY) should route to the precompile
	db := newMockDB()
	handler, _ := makeHandler(db, spec.Shanghai)

	callData := []byte{0x01, 0x02, 0x03, 0x04}
	result := execFrame(handler, vm.NewFrameInputCall(vm.CallInputs{
		Input:              callData,
		ReturnMemoryOffset: vm.MemoryRange{},
		GasLimit:           100000,
		BytecodeAddress:    addr(0x04), // IDENTITY precompile
		TargetAddress:      addr(0x04),
		Caller:             addr(0x01),
		Value:              vm.NewCallValueTransfer(types.U256Zero),
		Scheme:             vm.CallSchemeCall,
	}), 0, handler.RootMemory)

	if result.Call.Result.Result != vm.InstructionResultReturn {
		t.Fatalf("expected Return, got %v", result.Call.Result.Result)
	}
	// Identity should return input unchanged
	if len(result.Call.Result.Output) != 4 {
		t.Fatalf("expected 4-byte output, got %d", len(result.Call.Result.Output))
	}
	for i, b := range result.Call.Result.Output {
		if b != callData[i] {
			t.Fatalf("byte %d: got %02x, want %02x", i, b, callData[i])
		}
	}
	// Gas: 100000 - (15 + 3) = 99982
	if result.Call.Result.Gas.Remaining() != 99982 {
		t.Fatalf("expected remaining gas 99982, got %d", result.Call.Result.Gas.Remaining())
	}
}

func TestCallPrecompileSha256(t *testing.T) {
	db := newMockDB()
	handler, _ := makeHandler(db, spec.Shanghai)

	result := execFrame(handler, vm.NewFrameInputCall(vm.CallInputs{
		Input:           nil, // empty input
		GasLimit:        100000,
		BytecodeAddress: addr(0x02), // SHA256 precompile
		TargetAddress:   addr(0x02),
		Caller:          addr(0x01),
		Value:           vm.NewCallValueTransfer(types.U256Zero),
		Scheme:          vm.CallSchemeCall,
	}), 0, handler.RootMemory)

	if result.Call.Result.Result != vm.InstructionResultReturn {
		t.Fatalf("expected Return, got %v", result.Call.Result.Result)
	}
	// SHA256 of empty = e3b0c44298fc1c149afbf4c8996fb924...
	if len(result.Call.Result.Output) != 32 {
		t.Fatalf("expected 32-byte output, got %d", len(result.Call.Result.Output))
	}
	if result.Call.Result.Output[0] != 0xe3 {
		t.Fatalf("wrong first byte of SHA256(empty): got %02x", result.Call.Result.Output[0])
	}
}

func TestCallPrecompileOOG(t *testing.T) {
	db := newMockDB()
	handler, _ := makeHandler(db, spec.Shanghai)

	result := execFrame(handler, vm.NewFrameInputCall(vm.CallInputs{
		Input:           []byte{1, 2, 3},
		GasLimit:        10, // not enough for identity (needs 18)
		BytecodeAddress: addr(0x04),
		TargetAddress:   addr(0x04),
		Caller:          addr(0x01),
		Value:           vm.NewCallValueTransfer(types.U256Zero),
		Scheme:          vm.CallSchemeCall,
	}), 0, handler.RootMemory)

	if result.Call.Result.Result != vm.InstructionResultPrecompileOOG {
		t.Fatalf("expected PrecompileOOG, got %v", result.Call.Result.Result)
	}
}

func TestCallNonPrecompileAddress(t *testing.T) {
	// Address 0x42 is NOT a precompile - should go through normal execution
	db := newMockDB()
	db.accounts[addr(0x42)] = &state.AccountInfo{
		Balance:  u(0),
		CodeHash: types.KeccakEmpty,
		Code:     bytecodeReturnValue(0xAB),
	}
	handler, _ := makeHandler(db, spec.Shanghai)

	result := execFrame(handler, vm.NewFrameInputCall(vm.CallInputs{
		GasLimit:        100000,
		BytecodeAddress: addr(0x42),
		TargetAddress:   addr(0x42),
		Caller:          addr(0x01),
		Value:           vm.NewCallValueTransfer(types.U256Zero),
		Scheme:          vm.CallSchemeCall,
	}), 0, handler.RootMemory)

	if result.Call.Result.Result != vm.InstructionResultReturn {
		t.Fatalf("expected Return, got %v", result.Call.Result.Result)
	}
	if len(result.Call.Result.Output) != 1 || result.Call.Result.Output[0] != 0xAB {
		t.Fatalf("expected output [0xAB], got %v", result.Call.Result.Output)
	}
}
