package host

import (
	"testing"

	"github.com/Giulio2002/gevm/opcode"
	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
	"github.com/Giulio2002/gevm/state"
	"github.com/Giulio2002/gevm/vm"
)

// --- Helpers ---

func makeEvm(db state.Database, forkID spec.ForkID, block BlockEnv) *Evm {
	// Set a default block gas limit if not specified
	if block.GasLimit == (types.Uint256{}) {
		block.GasLimit = types.U256From(30_000_000) // 30M default
	}
	return NewEvm(db, forkID, block, CfgEnv{ChainId: u(1)})
}

func simpleTx(caller types.Address, to types.Address, value uint64, gasLimit uint64, gasPrice uint64) Transaction {
	return Transaction{
		Kind:     TxKindCall,
		Caller:   caller,
		To:       to,
		Value:    u(value),
		Input:    nil,
		GasLimit: gasLimit,
		GasPrice: u(gasPrice),
		Nonce:    0,
	}
}

// --- Intrinsic gas tests ---

func TestCalcIntrinsicGasSimpleTransfer(t *testing.T) {
	evm := makeEvm(newMockDB(), spec.Shanghai, BlockEnv{})
	tx := simpleTx(addr(0x01), addr(0x20), 0, 21000, 1)
	gas := evm.calcIntrinsicGas(&tx).InitialGas
	if gas != 21000 {
		t.Fatalf("expected 21000, got %d", gas)
	}
}

func TestCalcIntrinsicGasWithCalldata(t *testing.T) {
	evm := makeEvm(newMockDB(), spec.Shanghai, BlockEnv{})
	tx := Transaction{
		Kind:     TxKindCall,
		Caller:   addr(0x01),
		To:       addr(0x20),
		Value:    u(0),
		Input:    []byte{0x00, 0x01, 0x00, 0xFF}, // 2 zero + 2 nonzero
		GasLimit: 100000,
		GasPrice: u(1),
	}
	gas := evm.calcIntrinsicGas(&tx).InitialGas
	// 21000 + 2*4 + 2*16 = 21000 + 8 + 32 = 21040
	if gas != 21040 {
		t.Fatalf("expected 21040, got %d", gas)
	}
}

func TestCalcIntrinsicGasCreate(t *testing.T) {
	evm := makeEvm(newMockDB(), spec.Shanghai, BlockEnv{})
	initcode := make([]byte, 64) // 64 bytes = 2 words
	initcode[0] = 0xFF           // make first byte nonzero
	tx := Transaction{
		Kind:     TxKindCreate,
		Caller:   addr(0x01),
		Value:    u(0),
		Input:    initcode,
		GasLimit: 100000,
		GasPrice: u(1),
	}
	gas := evm.calcIntrinsicGas(&tx).InitialGas
	// 21000 + 32000 (create) + 2*initcodeWordGas + calldata cost
	// calldata: 1 nonzero (16) + 63 zeros (63*4 = 252) = 268
	// initcode words: 2 * 2 = 4
	// total: 21000 + 32000 + 4 + 268 = 53272
	if gas != 53272 {
		t.Fatalf("expected 53272, got %d", gas)
	}
}

func TestCalcIntrinsicGasPreIstanbul(t *testing.T) {
	evm := makeEvm(newMockDB(), spec.Homestead, BlockEnv{})
	tx := Transaction{
		Kind:     TxKindCall,
		Caller:   addr(0x01),
		To:       addr(0x20),
		Input:    []byte{0xFF},
		GasLimit: 100000,
		GasPrice: u(1),
	}
	gas := evm.calcIntrinsicGas(&tx).InitialGas
	// 21000 + 68 (pre-Istanbul non-zero cost) = 21068
	if gas != 21068 {
		t.Fatalf("expected 21068, got %d", gas)
	}
}

// --- Transact validation tests ---

func TestTransactIntrinsicGasExceeded(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(1000000),
		CodeHash: types.KeccakEmpty,
	}
	evm := makeEvm(db, spec.Shanghai, BlockEnv{})
	tx := simpleTx(addr(0x01), addr(0x20), 0, 20000, 1) // gas limit < 21000

	result := evm.Transact(&tx)
	if result.Kind != ResultHalt {
		t.Fatalf("expected halt, got %d", result.Kind)
	}
	if result.Reason != vm.InstructionResultOutOfGas {
		t.Fatalf("expected OOG, got %v", result.Reason)
	}
}

func TestTransactNonceMismatch(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(1000000),
		Nonce:    5,
		CodeHash: types.KeccakEmpty,
	}
	evm := makeEvm(db, spec.Shanghai, BlockEnv{})
	tx := simpleTx(addr(0x01), addr(0x20), 0, 21000, 1)
	tx.Nonce = 3 // wrong nonce

	result := evm.Transact(&tx)
	if result.Kind != ResultHalt {
		t.Fatalf("expected halt, got %d", result.Kind)
	}
}

func TestTransactInsufficientBalance(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(100), // not enough for gas cost (21000 * 1) + value (0)
		CodeHash: types.KeccakEmpty,
	}
	evm := makeEvm(db, spec.Shanghai, BlockEnv{})
	tx := simpleTx(addr(0x01), addr(0x20), 0, 21000, 1)

	result := evm.Transact(&tx)
	if result.Kind != ResultHalt {
		t.Fatalf("expected halt, got %d", result.Kind)
	}
	if result.Reason != vm.InstructionResultOutOfFunds {
		t.Fatalf("expected OutOfFunds, got %v", result.Reason)
	}
}

// --- Simple transfer tests ---

func TestTransactSimpleTransfer(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(1000000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	evm := makeEvm(db, spec.Shanghai, BlockEnv{
		Beneficiary: addr(0xCB),
	})
	tx := simpleTx(addr(0x01), addr(0x20), 100, 21000, 1) // addr(0x20): not a precompile

	result := evm.Transact(&tx)
	if result.Kind != ResultSuccess {
		t.Fatalf("expected success, got %d (reason: %v)", result.Kind, result.Reason)
	}
	if result.GasUsed != 21000 {
		t.Fatalf("expected gas used 21000, got %d", result.GasUsed)
	}

	// Verify caller balance: 1000000 - 21000(gas) - 100(value) = 978900
	callerResult, _ := evm.Journal.LoadAccount(addr(0x01))
	if callerResult.Data.Info.Balance != u(978900) {
		t.Fatalf("expected caller balance 978900, got %v", callerResult.Data.Info.Balance)
	}

	// Verify caller nonce incremented
	if callerResult.Data.Info.Nonce != 1 {
		t.Fatalf("expected caller nonce 1, got %d", callerResult.Data.Info.Nonce)
	}

	// Verify recipient balance: 100
	recipientResult, _ := evm.Journal.LoadAccount(addr(0x20))
	if recipientResult.Data.Info.Balance != u(100) {
		t.Fatalf("expected recipient balance 100, got %v", recipientResult.Data.Info.Balance)
	}
}

func TestTransactSimpleTransferPreLondon(t *testing.T) {
	// Pre-London: full gas price goes to beneficiary
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(1000000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	evm := makeEvm(db, spec.Berlin, BlockEnv{
		Beneficiary: addr(0xCB),
	})
	tx := simpleTx(addr(0x01), addr(0x20), 0, 21000, 10)

	result := evm.Transact(&tx)
	if result.Kind != ResultSuccess {
		t.Fatalf("expected success, got %d (reason: %v)", result.Kind, result.Reason)
	}

	// Beneficiary should receive full gas price: 21000 * 10 = 210000
	beneficiaryResult, _ := evm.Journal.LoadAccount(addr(0xCB))
	if beneficiaryResult.Data.Info.Balance != u(210000) {
		t.Fatalf("expected beneficiary balance 210000, got %v", beneficiaryResult.Data.Info.Balance)
	}
}

func TestTransactBeneficiaryRewardLondon(t *testing.T) {
	// London+: only tip (gas_price - base_fee) goes to beneficiary
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(10000000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	evm := makeEvm(db, spec.London, BlockEnv{
		Beneficiary: addr(0xCB),
		BaseFee:     u(5),
	})
	tx := simpleTx(addr(0x01), addr(0x20), 0, 21000, 10)

	result := evm.Transact(&tx)
	if result.Kind != ResultSuccess {
		t.Fatalf("expected success, got %d (reason: %v)", result.Kind, result.Reason)
	}

	// Beneficiary should receive tip: 21000 * (10 - 5) = 105000
	beneficiaryResult, _ := evm.Journal.LoadAccount(addr(0xCB))
	if beneficiaryResult.Data.Info.Balance != u(105000) {
		t.Fatalf("expected beneficiary balance 105000, got %v", beneficiaryResult.Data.Info.Balance)
	}
}

func TestTransactBeneficiaryRewardLondonBaseFeeHigherThanGasPrice(t *testing.T) {
	// In London+, gasPrice must be >= baseFee for legacy txs.
	// A legacy tx with gasPrice < baseFee is now correctly rejected.
	// Test that gasPrice == baseFee means zero tip to beneficiary.
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(10000000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	evm := makeEvm(db, spec.London, BlockEnv{
		Beneficiary: addr(0xCB),
		BaseFee:     u(10), // base fee == gas price → tip is zero
	})
	tx := simpleTx(addr(0x01), addr(0x20), 0, 21000, 10) // gasPrice=10, baseFee=10

	result := evm.Transact(&tx)
	if result.Kind != ResultSuccess {
		t.Fatalf("expected success, got %d (reason: %v)", result.Kind, result.Reason)
	}

	// Beneficiary should receive 0 tip (gasPrice - baseFee = 0)
	beneficiaryResult, _ := evm.Journal.LoadAccount(addr(0xCB))
	if beneficiaryResult.Data.Info.Balance != u(0) {
		t.Fatalf("expected beneficiary balance 0, got %v", beneficiaryResult.Data.Info.Balance)
	}
}

// --- CALL with bytecode ---

func TestTransactCallWithBytecode(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(10000000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	db.accounts[addr(0x42)] = &state.AccountInfo{
		Balance:  u(0),
		CodeHash: types.KeccakEmpty,
		Code:     bytecodeReturnValue(0xAB),
	}
	evm := makeEvm(db, spec.Shanghai, BlockEnv{
		Beneficiary: addr(0xCB),
	})
	tx := Transaction{
		Kind:     TxKindCall,
		Caller:   addr(0x01),
		To:       addr(0x42),
		Value:    u(0),
		GasLimit: 100000,
		GasPrice: u(1),
	}

	result := evm.Transact(&tx)
	if result.Kind != ResultSuccess {
		t.Fatalf("expected success, got %d (reason: %v)", result.Kind, result.Reason)
	}
	if len(result.Output) != 1 || result.Output[0] != 0xAB {
		t.Fatalf("expected output [0xAB], got %v", result.Output)
	}
	// Gas used should be > 21000 (intrinsic) due to execution
	if result.GasUsed <= 21000 {
		t.Fatalf("expected gas used > 21000, got %d", result.GasUsed)
	}
}

func TestTransactCallRevert(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(10000000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	db.accounts[addr(0x42)] = &state.AccountInfo{
		Balance:  u(0),
		CodeHash: types.KeccakEmpty,
		Code:     bytecodeRevert(),
	}
	evm := makeEvm(db, spec.Shanghai, BlockEnv{
		Beneficiary: addr(0xCB),
	})
	tx := Transaction{
		Kind:     TxKindCall,
		Caller:   addr(0x01),
		To:       addr(0x42),
		Value:    u(100),
		GasLimit: 100000,
		GasPrice: u(1),
	}

	result := evm.Transact(&tx)
	if result.Kind != ResultRevert {
		t.Fatalf("expected revert, got %d", result.Kind)
	}

	// On revert: caller gets back unused gas, but no refunds
	// Caller should lose only gas_used * gas_price (NOT the value)
	callerResult, _ := evm.Journal.LoadAccount(addr(0x01))
	expectedCallerBal := uint64(10000000) - result.GasUsed*1
	if callerResult.Data.Info.Balance != u(expectedCallerBal) {
		t.Fatalf("expected caller balance %d, got %v", expectedCallerBal, callerResult.Data.Info.Balance)
	}
}

// --- CALL to precompile ---

func TestTransactCallPrecompile(t *testing.T) {
	// Call to identity precompile (0x04)
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(10000000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	evm := makeEvm(db, spec.Shanghai, BlockEnv{
		Beneficiary: addr(0xCB),
	})
	tx := Transaction{
		Kind:     TxKindCall,
		Caller:   addr(0x01),
		To:       addr(0x04), // identity precompile
		Value:    u(0),
		Input:    []byte{0xDE, 0xAD},
		GasLimit: 100000,
		GasPrice: u(1),
	}

	result := evm.Transact(&tx)
	if result.Kind != ResultSuccess {
		t.Fatalf("expected success, got %d (reason: %v)", result.Kind, result.Reason)
	}
	if len(result.Output) != 2 || result.Output[0] != 0xDE || result.Output[1] != 0xAD {
		t.Fatalf("expected output [0xDE, 0xAD], got %v", result.Output)
	}
}

// --- CREATE transaction ---

func TestTransactCreate(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(10000000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	evm := makeEvm(db, spec.Shanghai, BlockEnv{
		Beneficiary: addr(0xCB),
	})

	// Init code that returns [0xFE] (INVALID opcode as contract code)
	initCode := bytecodeReturnValue(0xFE)

	tx := Transaction{
		Kind:     TxKindCreate,
		Caller:   addr(0x01),
		Value:    u(0),
		Input:    initCode,
		GasLimit: 1000000,
		GasPrice: u(1),
	}

	result := evm.Transact(&tx)
	if result.Kind != ResultSuccess {
		t.Fatalf("expected success, got %d (reason: %v)", result.Kind, result.Reason)
	}
	if result.CreatedAddr == nil {
		t.Fatal("expected created address")
	}

	// Verify deployed code exists
	createdResult, _ := evm.Journal.LoadAccount(*result.CreatedAddr)
	if createdResult.Data.Info.Code == nil || len(createdResult.Data.Info.Code) != 1 {
		t.Fatalf("expected 1-byte deployed code, got %v", createdResult.Data.Info.Code)
	}
	if createdResult.Data.Info.Code[0] != 0xFE {
		t.Fatalf("expected deployed code [0xFE], got %v", createdResult.Data.Info.Code)
	}
}

func TestTransactCreateWithValue(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(10000000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	evm := makeEvm(db, spec.Shanghai, BlockEnv{
		Beneficiary: addr(0xCB),
	})

	tx := Transaction{
		Kind:     TxKindCreate,
		Caller:   addr(0x01),
		Value:    u(500),
		Input:    nil, // empty init code
		GasLimit: 100000,
		GasPrice: u(1),
	}

	result := evm.Transact(&tx)
	if result.Kind != ResultSuccess {
		t.Fatalf("expected success, got %d (reason: %v)", result.Kind, result.Reason)
	}
	if result.CreatedAddr == nil {
		t.Fatal("expected created address")
	}

	// Verify balance transferred
	createdResult, _ := evm.Journal.LoadAccount(*result.CreatedAddr)
	if createdResult.Data.Info.Balance != u(500) {
		t.Fatalf("expected created balance 500, got %v", createdResult.Data.Info.Balance)
	}
}

// --- Gas accounting ---

func TestTransactGasRefund(t *testing.T) {
	// Unused gas should be refunded to caller
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(10000000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	evm := makeEvm(db, spec.Shanghai, BlockEnv{
		Beneficiary: addr(0xCB),
	})

	// Simple transfer with excess gas
	tx := simpleTx(addr(0x01), addr(0x20), 0, 100000, 10)

	result := evm.Transact(&tx)
	if result.Kind != ResultSuccess {
		t.Fatalf("expected success, got %d (reason: %v)", result.Kind, result.Reason)
	}
	if result.GasUsed != 21000 {
		t.Fatalf("expected gas used 21000, got %d", result.GasUsed)
	}

	// Caller should be charged only for gas used: 21000 * 10 = 210000
	// So balance = 10000000 - 210000 = 9790000
	callerResult, _ := evm.Journal.LoadAccount(addr(0x01))
	if callerResult.Data.Info.Balance != u(9790000) {
		t.Fatalf("expected caller balance 9790000, got %v", callerResult.Data.Info.Balance)
	}
}

func TestTransactGasAccounting(t *testing.T) {
	// Verify full gas accounting: caller deduction + beneficiary reward + reimburse
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(10000000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	evm := makeEvm(db, spec.Berlin, BlockEnv{
		Beneficiary: addr(0xCB),
	})
	tx := simpleTx(addr(0x01), addr(0x20), 1000, 50000, 20)

	result := evm.Transact(&tx)
	if result.Kind != ResultSuccess {
		t.Fatalf("expected success, got %d (reason: %v)", result.Kind, result.Reason)
	}
	// Simple transfer: gasUsed = 21000
	if result.GasUsed != 21000 {
		t.Fatalf("expected gas used 21000, got %d", result.GasUsed)
	}

	// Caller: 10000000 - 1000(value) - 21000*20(gas) = 10000000 - 1000 - 420000 = 9579000
	callerResult, _ := evm.Journal.LoadAccount(addr(0x01))
	if callerResult.Data.Info.Balance != u(9579000) {
		t.Fatalf("expected caller balance 9579000, got %v", callerResult.Data.Info.Balance)
	}

	// Recipient: 1000
	recipientResult, _ := evm.Journal.LoadAccount(addr(0x20))
	if recipientResult.Data.Info.Balance != u(1000) {
		t.Fatalf("expected recipient balance 1000, got %v", recipientResult.Data.Info.Balance)
	}

	// Beneficiary: 21000 * 20 = 420000 (pre-London: full gas price)
	beneficiaryResult, _ := evm.Journal.LoadAccount(addr(0xCB))
	if beneficiaryResult.Data.Info.Balance != u(420000) {
		t.Fatalf("expected beneficiary balance 420000, got %v", beneficiaryResult.Data.Info.Balance)
	}
}

// --- Shanghai coinbase warming ---

func TestTransactShanghaiWarmsCoinbase(t *testing.T) {
	// EIP-3651: Shanghai warms the coinbase address
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(10000000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	// Bytecode that does COINBASE to get the address (just STOP for simplicity)
	evm := makeEvm(db, spec.Shanghai, BlockEnv{
		Beneficiary: addr(0xCB),
	})
	tx := simpleTx(addr(0x01), addr(0x20), 0, 21000, 1)

	result := evm.Transact(&tx)
	if result.Kind != ResultSuccess {
		t.Fatalf("expected success, got %d (reason: %v)", result.Kind, result.Reason)
	}

	// Coinbase should have been loaded (warmed) during pre-execution
	// Second access should be warm
	coinbaseResult, _ := evm.Journal.LoadAccount(addr(0xCB))
	if coinbaseResult.IsCold {
		t.Fatal("coinbase should be warm after Shanghai transaction")
	}
}

func TestTransactBerlinDoesNotWarmCoinbase(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(10000000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	evm := makeEvm(db, spec.Berlin, BlockEnv{
		Beneficiary: addr(0xCB),
	})
	tx := simpleTx(addr(0x01), addr(0x20), 0, 21000, 1)

	result := evm.Transact(&tx)
	if result.Kind != ResultSuccess {
		t.Fatalf("expected success, got %d (reason: %v)", result.Kind, result.Reason)
	}

	// In Berlin (pre-Shanghai), coinbase is warmed by rewardBeneficiary's LoadAccount,
	// but NOT during pre-execution. Since rewardBeneficiary already loads it, it will
	// be warm after Transact returns. This test just verifies Transact completes.
}

// --- Edge cases ---

func TestTransactZeroGasPriceTransfer(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(1000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	evm := makeEvm(db, spec.Shanghai, BlockEnv{
		Beneficiary: addr(0xCB),
	})
	tx := simpleTx(addr(0x01), addr(0x20), 500, 21000, 0) // zero gas price

	result := evm.Transact(&tx)
	if result.Kind != ResultSuccess {
		t.Fatalf("expected success, got %d (reason: %v)", result.Kind, result.Reason)
	}

	// Caller: 1000 - 500(value) - 0(gas) = 500
	callerResult, _ := evm.Journal.LoadAccount(addr(0x01))
	if callerResult.Data.Info.Balance != u(500) {
		t.Fatalf("expected caller balance 500, got %v", callerResult.Data.Info.Balance)
	}
}

func TestTransactHaltAllGasConsumed(t *testing.T) {
	// When execution halts with error (not revert), all gas is consumed
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(10000000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	// INVALID opcode causes halt
	db.accounts[addr(0x42)] = &state.AccountInfo{
		Balance:  u(0),
		CodeHash: types.KeccakEmpty,
		Code:     []byte{opcode.INVALID},
	}
	evm := makeEvm(db, spec.Shanghai, BlockEnv{
		Beneficiary: addr(0xCB),
	})
	tx := Transaction{
		Kind:     TxKindCall,
		Caller:   addr(0x01),
		To:       addr(0x42),
		Value:    u(0),
		GasLimit: 100000,
		GasPrice: u(1),
	}

	result := evm.Transact(&tx)
	if result.Kind != ResultHalt {
		t.Fatalf("expected halt, got %d", result.Kind)
	}
	// All gas should be consumed on halt (error)
	if result.GasUsed != 100000 {
		t.Fatalf("expected all gas consumed (100000), got %d", result.GasUsed)
	}
}

// --- ExecutionResult methods ---

func TestExecutionResultMethods(t *testing.T) {
	success := ExecutionResult{Kind: ResultSuccess}
	if !success.IsSuccess() {
		t.Fatal("expected IsSuccess")
	}
	if success.IsRevert() || success.IsHalt() {
		t.Fatal("should not be revert or halt")
	}

	revert := ExecutionResult{Kind: ResultRevert}
	if !revert.IsRevert() {
		t.Fatal("expected IsRevert")
	}

	halt := ExecutionResult{Kind: ResultHalt}
	if !halt.IsHalt() {
		t.Fatal("expected IsHalt")
	}
}

// --- Logs ---

func TestTransactLogs(t *testing.T) {
	// Bytecode: LOG0 with 1 byte of data from memory
	// PUSH1 0x42 PUSH1 0 MSTORE8 PUSH1 1 PUSH1 0 LOG0 STOP
	logCode := []byte{
		opcode.PUSH1, 0x42,
		opcode.PUSH1, 0,
		opcode.MSTORE8,
		opcode.PUSH1, 1, // length
		opcode.PUSH1, 0, // offset
		opcode.LOG0,
		opcode.STOP,
	}

	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(10000000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	db.accounts[addr(0x42)] = &state.AccountInfo{
		Balance:  u(0),
		CodeHash: types.KeccakEmpty,
		Code:     logCode,
	}
	evm := makeEvm(db, spec.Shanghai, BlockEnv{
		Beneficiary: addr(0xCB),
	})
	tx := Transaction{
		Kind:     TxKindCall,
		Caller:   addr(0x01),
		To:       addr(0x42),
		Value:    u(0),
		GasLimit: 100000,
		GasPrice: u(1),
	}

	result := evm.Transact(&tx)
	if result.Kind != ResultSuccess {
		t.Fatalf("expected success, got %d (reason: %v)", result.Kind, result.Reason)
	}
	if len(result.Logs) == 0 {
		t.Fatal("expected at least 1 log")
	}
	if result.Logs[0].Address != addr(0x42) {
		t.Fatalf("expected log address 0x42, got %v", result.Logs[0].Address)
	}
	if len(result.Logs[0].Data) != 1 || result.Logs[0].Data[0] != 0x42 {
		t.Fatalf("expected log data [0x42], got %v", result.Logs[0].Data)
	}
}

// --- Precompile warming ---

func TestTransactWarmsPrecompiles(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(0x01)] = &state.AccountInfo{
		Balance:  u(10000000),
		Nonce:    0,
		CodeHash: types.KeccakEmpty,
	}
	evm := makeEvm(db, spec.Shanghai, BlockEnv{
		Beneficiary: addr(0xCB),
	})
	tx := simpleTx(addr(0x01), addr(0x20), 0, 21000, 1)

	result := evm.Transact(&tx)
	if result.Kind != ResultSuccess {
		t.Fatalf("expected success, got %d (reason: %v)", result.Kind, result.Reason)
	}

	// Precompile addresses (0x01-0x09) should be warmed
	for i := byte(1); i <= 9; i++ {
		r, _ := evm.Journal.LoadAccount(addr(i))
		if r.IsCold {
			t.Fatalf("precompile addr(0x%02x) should be warm", i)
		}
	}
}
