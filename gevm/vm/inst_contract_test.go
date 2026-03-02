// Tests for CREATE, CREATE2, CALL, CALLCODE, DELEGATECALL, STATICCALL instructions.
package vm

import (
	"testing"

	"github.com/Giulio2002/gevm/opcode"
	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
)

// mockHost implements vm.Host for testing contract instructions.
type mockHost struct {
	accounts map[types.Address]mockAccount
}

type mockAccount struct {
	balance  types.Uint256
	code     types.Bytes
	codeHash types.B256
	nonce    uint64
	isEmpty  bool
	isCold   bool
}

func newMockHost() *mockHost {
	return &mockHost{
		accounts: make(map[types.Address]mockAccount),
	}
}

func (h *mockHost) Beneficiary() types.Address                                              { return types.Address{} }
func (h *mockHost) Timestamp() types.Uint256                                                   { return types.U256Zero }
func (h *mockHost) BlockNumber() types.Uint256                                                 { return types.U256Zero }
func (h *mockHost) Difficulty() types.Uint256                                                  { return types.U256Zero }
func (h *mockHost) Prevrandao() *types.Uint256                                                 { return nil }
func (h *mockHost) GasLimit() types.Uint256                                                    { return types.U256Zero }
func (h *mockHost) ChainId() types.Uint256                                                     { return types.U256Zero }
func (h *mockHost) BaseFee() types.Uint256                                                     { return types.U256Zero }
func (h *mockHost) BlobGasPrice() types.Uint256                                                { return types.U256Zero }
func (h *mockHost) SlotNum() types.Uint256                                                     { return types.U256Zero }
func (h *mockHost) Caller() types.Address                                                   { return types.Address{} }
func (h *mockHost) EffectiveGasPrice() types.Uint256                                           { return types.U256Zero }
func (h *mockHost) BlobHash(index int) *types.Uint256                                          { return nil }
func (h *mockHost) SelfBalance(types.Address) types.Uint256                               { return types.U256Zero }
func (h *mockHost) BlockHash(number types.Uint256) types.B256                             { return types.B256Zero }
func (h *mockHost) Log(addr types.Address, topics *[4]types.B256, numTopics int, data types.Bytes) {
}
func (h *mockHost) SelfDestruct(addr, target types.Address) SelfDestructResult {
	return SelfDestructResult{}
}
func (h *mockHost) TLoad(addr types.Address, key types.Uint256) types.Uint256 {
	return types.U256Zero
}
func (h *mockHost) TStore(addr types.Address, key, value types.Uint256) {}
func (h *mockHost) SLoadInto(_ types.Address, _ *types.Uint256, out *types.Uint256) bool {
	*out = types.U256Zero
	return false
}
func (h *mockHost) SStore(_ types.Address, _, _ *types.Uint256, out *SStoreResult) {
	*out = SStoreResult{}
}

func (h *mockHost) Balance(addr types.Address) (types.Uint256, bool) {
	acc, ok := h.accounts[addr]
	if !ok {
		return types.U256Zero, true
	}
	return acc.balance, acc.isCold
}

func (h *mockHost) CodeSize(addr types.Address) (int, bool) {
	acc, ok := h.accounts[addr]
	if !ok {
		return 0, true
	}
	return len(acc.code), acc.isCold
}

func (h *mockHost) CodeHash(addr types.Address) (types.B256, bool) {
	acc, ok := h.accounts[addr]
	if !ok {
		return types.B256Zero, true
	}
	return acc.codeHash, acc.isCold
}

func (h *mockHost) Code(addr types.Address) (types.Bytes, bool) {
	acc, ok := h.accounts[addr]
	if !ok {
		return nil, true
	}
	return acc.code, acc.isCold
}

func (h *mockHost) LoadAccountCode(addr types.Address) AccountCodeLoad {
	acc, ok := h.accounts[addr]
	if !ok {
		return AccountCodeLoad{IsCold: true, IsEmpty: true}
	}
	return AccountCodeLoad{
		Code:     acc.code,
		CodeHash: acc.codeHash,
		IsCold:   acc.isCold,
		IsEmpty:  acc.isEmpty,
	}
}

func addr(b byte) types.Address {
	var a types.Address
	a[19] = b
	return a
}

// runWithHost creates an interpreter with input and host, runs it, and returns the interpreter.
func runWithHost(code []byte, gasLimit uint64, forkID spec.ForkID, input Inputs, host Host) *Interpreter {
	interp := NewInterpreter(
		NewMemory(),
		NewBytecode(code),
		input,
		false,
		forkID,
		gasLimit,
	)
	DefaultRunner{}.Run(interp, host)
	return interp
}

func runStaticWithHost(code []byte, gasLimit uint64, forkID spec.ForkID, input Inputs, host Host) *Interpreter {
	interp := NewInterpreter(
		NewMemory(),
		NewBytecode(code),
		input,
		true,
		forkID,
		gasLimit,
	)
	DefaultRunner{}.Run(interp, host)
	return interp
}

// --- Action Types Tests ---

func TestCallValueTransfer(t *testing.T) {
	cv := NewCallValueTransfer(u(100))
	if !cv.IsTransfer() {
		t.Fatal("should be transfer")
	}
	if cv.IsApparent() {
		t.Fatal("should not be apparent")
	}
	if cv.TransferValue() == nil || *cv.TransferValue() != u(100) {
		t.Fatal("transfer value should be 100")
	}
	if !cv.TransfersValue() {
		t.Fatal("should transfer value (non-zero)")
	}
}

func TestCallValueTransferZero(t *testing.T) {
	cv := NewCallValueTransfer(types.U256Zero)
	if !cv.IsTransfer() {
		t.Fatal("should be transfer kind")
	}
	if cv.TransfersValue() {
		t.Fatal("should not transfer value (zero)")
	}
}

func TestCallValueApparent(t *testing.T) {
	cv := NewCallValueApparent(u(42))
	if cv.IsTransfer() {
		t.Fatal("should not be transfer")
	}
	if !cv.IsApparent() {
		t.Fatal("should be apparent")
	}
	if cv.TransferValue() != nil {
		t.Fatal("apparent should return nil for TransferValue")
	}
	if cv.TransfersValue() {
		t.Fatal("apparent should not transfer value")
	}
}

func TestCreateSchemes(t *testing.T) {
	c := NewCreateSchemeCreate()
	if c.Kind != CreateSchemeCreate {
		t.Fatal("wrong kind")
	}

	c2 := NewCreateSchemeCreate2(u(42))
	if c2.Kind != CreateSchemeCreate2 {
		t.Fatal("wrong kind")
	}
	if c2.Salt != u(42) {
		t.Fatal("wrong salt")
	}
}

func TestFrameInputCall(t *testing.T) {
	fi := NewFrameInputCall(CallInputs{
		GasLimit:        1000,
		TargetAddress:   addr(1),
		BytecodeAddress: addr(1),
		Caller:          addr(2),
		Scheme:          CallSchemeCall,
	})
	if fi.Kind != FrameInputCall {
		t.Fatal("wrong kind")
	}
	if fi.Call.GasLimit != 1000 {
		t.Fatal("wrong gas limit")
	}
}

func TestFrameInputCreate(t *testing.T) {
	fi := NewFrameInputCreate(CreateInputs{
		Caller:   addr(1),
		Scheme:   NewCreateSchemeCreate(),
		Value:    u(100),
		InitCode: []byte{0x60, 0x00},
		GasLimit: 5000,
	})
	if fi.Kind != FrameInputCreate {
		t.Fatal("wrong kind")
	}
	if fi.Create.GasLimit != 5000 {
		t.Fatal("wrong gas limit")
	}
}

// --- CREATE Instruction Tests ---

func TestCreateBasic(t *testing.T) {
	host := newMockHost()
	// Code: PUSH1 0 (code len), PUSH1 0 (code offset), PUSH1 0 (value), CREATE
	// Actually: PUSH1 len, PUSH1 offset, PUSH1 value  -> stack [value, offset, len]
	// But stack ordering for CREATE is: value (deepest), offset, len (top)
	// The instruction pops: value, codeOffset, length in order
	// So we need to push: length (first pushed = deepest), codeOffset, value (last pushed = top)
	// Wait, looking at createInner: it calls Pop3() which returns the first 3 pops.
	// pop3 returns (a, b, c) where a is topmost.
	// In createInner: value, codeOffset, length = Pop3() -> value=top, codeOffset=2nd, length=3rd
	// Wait no, let me re-read Pop3.

	// Looking at CREATE:
	// stack: [value, code_offset, len] with len on top
	// pop3: first pop = len, second pop = code_offset, third pop = value
	// createInner does: value, codeOffset, length, ok := interp.Stack.Pop3()
	// Pop3 returns (first, second, third) where first = top of stack
	// So: value = top (first pop), codeOffset = second pop, length = third pop
	// That means the stack should be: [length (deepest), codeOffset, value (top)]
	// Push order: PUSH length, PUSH codeOffset, PUSH value

	// For zero init code: push len=0, offset=0, value=0
	code := []byte{
		opcode.PUSH1, 0x00, // length = 0
		opcode.PUSH1, 0x00, // offset = 0
		opcode.PUSH1, 0x00, // value = 0
		opcode.CREATE,
	}
	interp := runWithHost(code, 1_000_000, spec.Shanghai, Inputs{
		TargetAddress: addr(1),
	}, host)

	// CREATE should set an action (frame request)
	if !interp.HasAction {
		t.Fatal("CREATE should set an action")
	}
	if interp.ActionData.Kind != FrameInputCreate {
		t.Fatal("action should be a create")
	}
	if interp.ActionData.Create.Caller != addr(1) {
		t.Fatal("caller should be target address")
	}
	if interp.ActionData.Create.Scheme.Kind != CreateSchemeCreate {
		t.Fatal("scheme should be Create")
	}
	if len(interp.ActionData.Create.InitCode) != 0 {
		t.Fatal("init code should be empty")
	}
}

func TestCreate2Basic(t *testing.T) {
	host := newMockHost()
	// CREATE2 pops: value(top), codeOffset, length, salt
	// Stack bottom-to-top: salt, length, codeOffset, value
	// Push order: salt first (goes to bottom), then length, offset, value
	code := []byte{
		opcode.PUSH1, 0x42, // salt (deepest)
		opcode.PUSH1, 0x00, // length
		opcode.PUSH1, 0x00, // offset
		opcode.PUSH1, 0x00, // value (top)
		opcode.CREATE2,
	}
	interp := runWithHost(code, 1_000_000, spec.Shanghai, Inputs{
		TargetAddress: addr(1),
	}, host)

	if !interp.HasAction {
		t.Fatal("CREATE2 should set an action")
	}
	if interp.ActionData.Kind != FrameInputCreate {
		t.Fatal("action should be a create")
	}
	if interp.ActionData.Create.Scheme.Kind != CreateSchemeCreate2 {
		t.Fatal("scheme should be Create2")
	}
	if interp.ActionData.Create.Scheme.Salt != u(0x42) {
		t.Fatalf("salt should be 0x42, got %v", interp.ActionData.Create.Scheme.Salt)
	}
}

func TestCreateWithInitCode(t *testing.T) {
	host := newMockHost()
	// Store init code in memory first, then CREATE with it
	initCode := []byte{0x60, 0x42, 0x60, 0x00, 0x52} // PUSH1 0x42, PUSH1 0x00, MSTORE
	code := make([]byte, 0, 200)

	// Store init code in memory at offset 0
	for i, b := range initCode {
		code = append(code, opcode.PUSH1, b)       // push the byte value
		code = append(code, opcode.PUSH1, byte(i)) // push the offset
		code = append(code, opcode.MSTORE8)        // store byte
	}

	// Push CREATE args: length, offset, value
	code = append(code, opcode.PUSH1, byte(len(initCode))) // length
	code = append(code, opcode.PUSH1, 0x00)                // offset
	code = append(code, opcode.PUSH1, 0x00)                // value
	code = append(code, opcode.CREATE)

	interp := runWithHost(code, 1_000_000, spec.Shanghai, Inputs{
		TargetAddress: addr(1),
	}, host)

	if !interp.HasAction {
		t.Fatal("CREATE should set an action")
	}
	if len(interp.ActionData.Create.InitCode) != len(initCode) {
		t.Fatalf("init code length: got %d, want %d", len(interp.ActionData.Create.InitCode), len(initCode))
	}
	for i, b := range initCode {
		if interp.ActionData.Create.InitCode[i] != b {
			t.Fatalf("init code byte %d: got %x, want %x", i, interp.ActionData.Create.InitCode[i], b)
		}
	}
}

func TestCreateStaticContext(t *testing.T) {
	host := newMockHost()
	code := []byte{
		opcode.PUSH1, 0x00, // length
		opcode.PUSH1, 0x00, // offset
		opcode.PUSH1, 0x00, // value
		opcode.CREATE,
	}
	interp := runStaticWithHost(code, 1_000_000, spec.Shanghai, Inputs{}, host)
	if interp.HaltResult != InstructionResultStateChangeDuringStaticCall {
		t.Fatalf("expected StateChangeDuringStaticCall, got %v", interp.HaltResult)
	}
}

func TestCreateInitCodeSizeLimit(t *testing.T) {
	host := newMockHost()
	// Push init code size > maxInitcodeSize (49152)
	tooBig := types.U256From(maxInitcodeSize + 1)
	code := make([]byte, 0, 100)
	// Push length (too big)
	code = append(code, opcode.PUSH32)
	bb := tooBig.ToBytes32()
	code = append(code, bb[:]...)
	// Push offset = 0
	code = append(code, opcode.PUSH1, 0x00)
	// Push value = 0
	code = append(code, opcode.PUSH1, 0x00)
	code = append(code, opcode.CREATE)

	interp := runWithHost(code, 10_000_000, spec.Shanghai, Inputs{
		TargetAddress: addr(1),
	}, host)

	if interp.HaltResult != InstructionResultCreateInitCodeSizeLimit {
		t.Fatalf("expected CreateInitCodeSizeLimit, got %v", interp.HaltResult)
	}
}

// --- CALL Instruction Tests ---

func TestCallBasic(t *testing.T) {
	host := newMockHost()
	host.accounts[addr(0x42)] = mockAccount{
		code:     []byte{0x00}, // STOP
		codeHash: types.B256{0x01},
		isCold:   false,
		isEmpty:  false,
	}

	// CALL: stack [retLen, retOffset, argsLen, argsOffset, value, to, gas] with gas on top
	// Pop order: gas(top), to, value -> Pop3
	// Then: argsOffset, argsLen, retOffset, retLen -> getMemoryInputAndOutRanges pops 4
	// Push order (deepest first): retLen, retOffset, argsLen, argsOffset, value, to, gas
	code := []byte{
		opcode.PUSH1, 0x20, // retLen = 32
		opcode.PUSH1, 0x00, // retOffset = 0
		opcode.PUSH1, 0x00, // argsLen = 0
		opcode.PUSH1, 0x00, // argsOffset = 0
		opcode.PUSH1, 0x00, // value = 0
		opcode.PUSH1, 0x42, // to
		opcode.PUSH2, 0x27, 0x10, // gas = 10000
		opcode.CALL,
	}
	interp := runWithHost(code, 1_000_000, spec.Shanghai, Inputs{
		TargetAddress: addr(1),
	}, host)

	if !interp.HasAction {
		t.Fatal("CALL should set an action")
	}
	if interp.ActionData.Kind != FrameInputCall {
		t.Fatal("action should be a call")
	}
	ci := &interp.ActionData.Call
	if ci.Scheme != CallSchemeCall {
		t.Fatal("scheme should be Call")
	}
	if ci.TargetAddress != addr(0x42) {
		t.Fatalf("target should be addr(0x42), got %v", ci.TargetAddress)
	}
	if ci.BytecodeAddress != addr(0x42) {
		t.Fatal("bytecode address should be addr(0x42)")
	}
	if ci.Caller != addr(1) {
		t.Fatal("caller should be addr(1)")
	}
	if !ci.Value.IsTransfer() {
		t.Fatal("value should be transfer")
	}
	if ci.ReturnMemoryOffset.Length != 32 {
		t.Fatal("return memory length should be 32")
	}
}

func TestCallWithValue(t *testing.T) {
	host := newMockHost()
	host.accounts[addr(0x42)] = mockAccount{
		code:     []byte{0x00},
		codeHash: types.B256{0x01},
		isCold:   false,
		isEmpty:  false,
	}

	code := []byte{
		opcode.PUSH1, 0x00, // retLen
		opcode.PUSH1, 0x00, // retOffset
		opcode.PUSH1, 0x00, // argsLen
		opcode.PUSH1, 0x00, // argsOffset
		opcode.PUSH1, 0x01, // value = 1
		opcode.PUSH1, 0x42, // to
		opcode.PUSH2, 0x27, 0x10, // gas = 10000
		opcode.CALL,
	}
	interp := runWithHost(code, 1_000_000, spec.Shanghai, Inputs{
		TargetAddress: addr(1),
	}, host)

	if !interp.HasAction {
		t.Fatal("CALL with value should set an action")
	}
	ci := &interp.ActionData.Call
	if !ci.Value.TransfersValue() {
		t.Fatal("should transfer value")
	}
	if ci.Value.Value != u(1) {
		t.Fatalf("value should be 1, got %v", ci.Value.Value)
	}
	// Gas limit should include call stipend (2300)
	if ci.GasLimit < 2300 {
		t.Fatalf("gas limit should include call stipend, got %d", ci.GasLimit)
	}
}

func TestCallStaticWithValue(t *testing.T) {
	host := newMockHost()
	code := []byte{
		opcode.PUSH1, 0x00, // retLen
		opcode.PUSH1, 0x00, // retOffset
		opcode.PUSH1, 0x00, // argsLen
		opcode.PUSH1, 0x00, // argsOffset
		opcode.PUSH1, 0x01, // value = 1 (non-zero!)
		opcode.PUSH1, 0x42, // to
		opcode.PUSH2, 0x27, 0x10, // gas = 10000
		opcode.CALL,
	}
	interp := runStaticWithHost(code, 1_000_000, spec.Shanghai, Inputs{}, host)

	if interp.HaltResult != InstructionResultCallNotAllowedInsideStatic {
		t.Fatalf("expected CallNotAllowedInsideStatic, got %v", interp.HaltResult)
	}
}

// --- CALLCODE Tests ---

func TestCallCodeBasic(t *testing.T) {
	host := newMockHost()
	host.accounts[addr(0x42)] = mockAccount{
		code:     []byte{0x00},
		codeHash: types.B256{0x01},
		isCold:   false,
		isEmpty:  false,
	}

	code := []byte{
		opcode.PUSH1, 0x00, // retLen
		opcode.PUSH1, 0x00, // retOffset
		opcode.PUSH1, 0x00, // argsLen
		opcode.PUSH1, 0x00, // argsOffset
		opcode.PUSH1, 0x00, // value = 0
		opcode.PUSH1, 0x42, // to
		opcode.PUSH2, 0x27, 0x10, // gas = 10000
		opcode.CALLCODE,
	}
	interp := runWithHost(code, 1_000_000, spec.Shanghai, Inputs{
		TargetAddress: addr(1),
	}, host)

	if !interp.HasAction {
		t.Fatal("CALLCODE should set an action")
	}
	ci := &interp.ActionData.Call
	if ci.Scheme != CallSchemeCallCode {
		t.Fatal("scheme should be CallCode")
	}
	// CALLCODE: bytecode_address = to, target_address = self
	if ci.BytecodeAddress != addr(0x42) {
		t.Fatal("bytecode address should be to")
	}
	if ci.TargetAddress != addr(1) {
		t.Fatal("target should be self (addr(1))")
	}
}

// --- DELEGATECALL Tests ---

func TestDelegateCallBasic(t *testing.T) {
	host := newMockHost()
	host.accounts[addr(0x42)] = mockAccount{
		code:     []byte{0x00},
		codeHash: types.B256{0x01},
		isCold:   false,
		isEmpty:  false,
	}

	// DELEGATECALL: no value parameter
	// Stack [retLen, retOffset, argsLen, argsOffset, to, gas] with gas on top
	code := []byte{
		opcode.PUSH1, 0x00, // retLen
		opcode.PUSH1, 0x00, // retOffset
		opcode.PUSH1, 0x00, // argsLen
		opcode.PUSH1, 0x00, // argsOffset
		opcode.PUSH1, 0x42, // to
		opcode.PUSH2, 0x27, 0x10, // gas = 10000
		opcode.DELEGATECALL,
	}
	interp := runWithHost(code, 1_000_000, spec.Shanghai, Inputs{
		TargetAddress: addr(1),
		CallerAddress: addr(0xCA),
		CallValue:     u(999),
	}, host)

	if !interp.HasAction {
		t.Fatal("DELEGATECALL should set an action")
	}
	ci := &interp.ActionData.Call
	if ci.Scheme != CallSchemeDelegateCall {
		t.Fatal("scheme should be DelegateCall")
	}
	// DELEGATECALL: target = self, caller = parent's caller
	if ci.TargetAddress != addr(1) {
		t.Fatal("target should be self")
	}
	if ci.Caller != addr(0xCA) {
		t.Fatalf("caller should be preserved from parent, got %v", ci.Caller)
	}
	// Value should be apparent, not transfer
	if !ci.Value.IsApparent() {
		t.Fatal("value should be apparent")
	}
	if ci.Value.Value != u(999) {
		t.Fatal("apparent value should be parent's call value")
	}
}

func TestDelegateCallPreHomestead(t *testing.T) {
	host := newMockHost()
	code := []byte{
		opcode.PUSH1, 0x00,
		opcode.PUSH1, 0x00,
		opcode.PUSH1, 0x00,
		opcode.PUSH1, 0x00,
		opcode.PUSH1, 0x42,
		opcode.PUSH2, 0x27, 0x10,
		opcode.DELEGATECALL,
	}
	interp := runWithHost(code, 1_000_000, spec.Frontier, Inputs{}, host)
	if interp.HaltResult != InstructionResultNotActivated {
		t.Fatalf("expected NotActivated for pre-Homestead, got %v", interp.HaltResult)
	}
}

// --- STATICCALL Tests ---

func TestStaticCallBasic(t *testing.T) {
	host := newMockHost()
	host.accounts[addr(0x42)] = mockAccount{
		code:     []byte{0x00},
		codeHash: types.B256{0x01},
		isCold:   false,
		isEmpty:  false,
	}

	// STATICCALL: no value parameter
	code := []byte{
		opcode.PUSH1, 0x00, // retLen
		opcode.PUSH1, 0x00, // retOffset
		opcode.PUSH1, 0x00, // argsLen
		opcode.PUSH1, 0x00, // argsOffset
		opcode.PUSH1, 0x42, // to
		opcode.PUSH2, 0x27, 0x10, // gas = 10000
		opcode.STATICCALL,
	}
	interp := runWithHost(code, 1_000_000, spec.Shanghai, Inputs{
		TargetAddress: addr(1),
	}, host)

	if !interp.HasAction {
		t.Fatal("STATICCALL should set an action")
	}
	ci := &interp.ActionData.Call
	if ci.Scheme != CallSchemeStaticCall {
		t.Fatal("scheme should be StaticCall")
	}
	if !ci.IsStatic {
		t.Fatal("should be static")
	}
	if ci.Value.TransfersValue() {
		t.Fatal("static call should not transfer value")
	}
}

func TestStaticCallPreByzantium(t *testing.T) {
	host := newMockHost()
	code := []byte{
		opcode.PUSH1, 0x00,
		opcode.PUSH1, 0x00,
		opcode.PUSH1, 0x00,
		opcode.PUSH1, 0x00,
		opcode.PUSH1, 0x42,
		opcode.PUSH2, 0x27, 0x10,
		opcode.STATICCALL,
	}
	interp := runWithHost(code, 1_000_000, spec.Homestead, Inputs{}, host)
	if interp.HaltResult != InstructionResultNotActivated {
		t.Fatalf("expected NotActivated for pre-Byzantium, got %v", interp.HaltResult)
	}
}

// --- Gas Calculation Tests ---

func TestCallGasStipendReduction(t *testing.T) {
	// Post-Tangerine: gas limit is min(63/64 * remaining, stack_gas_limit)
	// Use a small parent gas so 63/64 of remaining < stack gas limit
	host := newMockHost()
	host.accounts[addr(0x42)] = mockAccount{
		code:     []byte{0x00},
		codeHash: types.B256{0x01},
		isCold:   false,
		isEmpty:  false,
	}

	// Request very high gas from stack (0xFFFFFF = 16M)
	code := make([]byte, 0, 100)
	code = append(code, opcode.PUSH1, 0x00) // retLen
	code = append(code, opcode.PUSH1, 0x00) // retOffset
	code = append(code, opcode.PUSH1, 0x00) // argsLen
	code = append(code, opcode.PUSH1, 0x00) // argsOffset
	code = append(code, opcode.PUSH1, 0x00) // value = 0
	code = append(code, opcode.PUSH1, 0x42) // to
	// Push gas limit higher than parent gas: 16M
	code = append(code, opcode.PUSH3, 0xFF, 0xFF, 0xFF)
	code = append(code, opcode.CALL)

	// Give parent only 50000 gas
	interp := runWithHost(code, 50000, spec.Shanghai, Inputs{
		TargetAddress: addr(1),
	}, host)

	if !interp.HasAction {
		t.Fatal("CALL should set an action")
	}
	ci := &interp.ActionData.Call
	// Gas limit should be capped by 63/64 rule since 63/64 of remaining < 16M
	// After 7*3 = 21 gas for PUSH instructions + CALL static gas (100):
	// remaining ≈ 50000 - 121 = 49879, 63/64 * 49879 ≈ 49100
	if ci.GasLimit >= 16777215 {
		t.Fatalf("gas limit should be reduced by 63/64 rule, got %d", ci.GasLimit)
	}
	if ci.GasLimit == 0 {
		t.Fatal("gas limit should be non-zero")
	}
}

func TestCallColdAccountCost(t *testing.T) {
	// Berlin+: cold account access costs extra 2500 gas
	host := newMockHost()
	host.accounts[addr(0x42)] = mockAccount{
		code:     []byte{0x00},
		codeHash: types.B256{0x01},
		isCold:   true, // cold!
		isEmpty:  false,
	}

	code := []byte{
		opcode.PUSH1, 0x00, // retLen
		opcode.PUSH1, 0x00, // retOffset
		opcode.PUSH1, 0x00, // argsLen
		opcode.PUSH1, 0x00, // argsOffset
		opcode.PUSH1, 0x00, // value = 0
		opcode.PUSH1, 0x42, // to
		opcode.PUSH2, 0x27, 0x10, // gas = 10000
		opcode.CALL,
	}

	// Run with Berlin spec (cold access costs extra)
	interp := runWithHost(code, 1_000_000, spec.Berlin, Inputs{
		TargetAddress: addr(1),
	}, host)

	if !interp.HasAction {
		t.Fatal("CALL should set an action")
	}

	// Compare gas spent with cold vs warm
	coldSpent := interp.Gas.Spent()

	// Run again with warm account
	host.accounts[addr(0x42)] = mockAccount{
		code:     []byte{0x00},
		codeHash: types.B256{0x01},
		isCold:   false, // warm!
		isEmpty:  false,
	}
	interp2 := runWithHost(code, 1_000_000, spec.Berlin, Inputs{
		TargetAddress: addr(1),
	}, host)

	if !interp2.HasAction {
		t.Fatal("CALL should set an action")
	}
	warmSpent := interp2.Gas.Spent()

	// Cold should cost more than warm (2500 additional)
	if coldSpent <= warmSpent {
		t.Fatalf("cold access should cost more: cold=%d, warm=%d", coldSpent, warmSpent)
	}
	diff := coldSpent - warmSpent
	if diff != 2500 {
		t.Fatalf("cold-warm difference should be 2500, got %d", diff)
	}
}

// --- Interpreter Action/Halt Tests ---

func TestInterpreterSetAction(t *testing.T) {
	interp := DefaultInterpreter()
	fi := NewFrameInputCall(CallInputs{GasLimit: 42})
	interp.SetAction(fi)

	if !interp.HasAction {
		t.Fatal("action should be set")
	}
	if interp.Bytecode.IsRunning() {
		t.Fatal("should not be running after SetAction")
	}
	if interp.ActionData.Call.GasLimit != 42 {
		t.Fatal("action gas limit mismatch")
	}
}

func TestInterpreterResetAction(t *testing.T) {
	interp := DefaultInterpreter()
	fi := NewFrameInputCall(CallInputs{GasLimit: 42})
	interp.SetAction(fi)
	interp.ResetAction()

	if interp.HasAction {
		t.Fatal("action should be nil after reset")
	}
	if !interp.Bytecode.IsRunning() {
		t.Fatal("should be running after ResetAction")
	}
}

func TestInterpreterHaltResult(t *testing.T) {
	interp := DefaultInterpreter()
	interp.Halt(InstructionResultOutOfGas)

	if interp.HaltResult != InstructionResultOutOfGas {
		t.Fatalf("halt result: got %v, want OutOfGas", interp.HaltResult)
	}
	result := interp.InterpreterResultFromHalt()
	if result.Result != InstructionResultOutOfGas {
		t.Fatal("result should be OutOfGas")
	}
}

func TestCreate2PrePetersburg(t *testing.T) {
	host := newMockHost()
	code := []byte{
		opcode.PUSH1, 0x00, // length
		opcode.PUSH1, 0x00, // offset
		opcode.PUSH1, 0x00, // value
		opcode.PUSH1, 0x00, // salt
		opcode.CREATE2,
	}
	interp := runWithHost(code, 1_000_000, spec.Byzantium, Inputs{
		TargetAddress: addr(1),
	}, host)
	if interp.HaltResult != InstructionResultNotActivated {
		t.Fatalf("expected NotActivated for pre-Petersburg CREATE2, got %v", interp.HaltResult)
	}
}

// --- CREATE gas accounting ---

func TestCreateGasAccounting(t *testing.T) {
	host := newMockHost()
	code := []byte{
		opcode.PUSH1, 0x00, // length = 0
		opcode.PUSH1, 0x00, // offset = 0
		opcode.PUSH1, 0x00, // value = 0
		opcode.CREATE,
	}
	gasLimit := uint64(1_000_000)
	interp := runWithHost(code, gasLimit, spec.Shanghai, Inputs{
		TargetAddress: addr(1),
	}, host)

	if !interp.HasAction {
		t.Fatal("CREATE should set an action")
	}

	// Gas should include: static gas for PUSH1*3 + CREATE + create_cost(32000) + 63/64 of remaining
	// Verify the create gas limit was set
	if interp.ActionData.Create.GasLimit == 0 {
		t.Fatal("create gas limit should be non-zero")
	}
}
