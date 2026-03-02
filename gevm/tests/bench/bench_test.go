// End-to-end EVM execution benchmarks for GEVM.
// These exercise the full transaction pipeline: validation, frame handling,
// journal/state, memory pooling, call depth, and precompile routing.
package bench

import (
	_ "embed"
	"encoding/binary"
	"encoding/hex"
	"strings"
	"testing"

	"github.com/Giulio2002/gevm/host"
	keccak "github.com/Giulio2002/fastkeccak"
	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
	"github.com/Giulio2002/gevm/state"
	"github.com/Giulio2002/gevm/vm"
	"github.com/Giulio2002/gevm/precompiles"
	spectest "github.com/Giulio2002/gevm/tests/spec"

	bls12381 "github.com/consensys/gnark-crypto/ecc/bls12-381"
)

//go:embed testdata/analysis.hex
var analysisHex string

//go:embed testdata/snailtracer.hex
var snailtracerHex string

//go:embed testdata/erc20_runtime.hex
var erc20RuntimeHex string

// Addresses used across benchmarks.
var (
	benchCaller   = types.Address{0x01}
	benchContract = types.Address{0x10}
	benchCoinbase = types.Address{0xcc}
	benchEOA      = types.Address{0xe0}
)

// hugeBalance is a large balance to fund the caller.
var hugeBalance = types.U256FromLimbs(0, 0, 1, 0) // ~2^128

func blockEnvForSpec(forkID spec.ForkID) host.BlockEnv {
	env := host.BlockEnv{
		Beneficiary: benchCoinbase,
		GasLimit:    types.U256From(1_000_000_000),
		BaseFee:     types.U256From(1),
		Number:      types.U256From(1),
		Timestamp:   types.U256From(1000),
	}
	// Prevrandao required for Merge+
	if forkID.IsEnabledIn(spec.Merge) {
		prevrandao := types.U256Zero
		env.Prevrandao = &prevrandao
	}
	// Pre-London specs don't have BaseFee; set to zero to avoid validation failures
	if !forkID.IsEnabledIn(spec.London) {
		env.BaseFee = types.U256Zero
	}
	return env
}

func defaultCfgEnv() host.CfgEnv {
	return host.CfgEnv{ChainId: types.U256From(1)}
}

// newBenchDB creates a fresh MemDB with a funded caller account.
func newBenchDB() *spectest.MemDB {
	db := spectest.NewMemDB()
	db.InsertAccount(benchCaller, state.AccountInfo{
		Balance:  hugeBalance,
		CodeHash: types.KeccakEmpty,
	}, nil)
	return db
}

// benchmarkCode runs a full Transact() benchmark against contractCode.
// DB is created once and reused (Transact only modifies Journal state, not the DB).
func benchmarkCode(b *testing.B, forkID spec.ForkID, gasLimit uint64, contractCode []byte) {
	benchmarkCodeRunner(b, forkID, gasLimit, contractCode, nil)
}

func benchmarkCodeRunner(b *testing.B, forkID spec.ForkID, gasLimit uint64, contractCode []byte, runner vm.Runner) {
	codeHash := types.Keccak256(contractCode)
	block := blockEnvForSpec(forkID)
	cfg := defaultCfgEnv()

	db := newBenchDB()
	db.InsertAccount(benchContract, state.AccountInfo{
		Code:     contractCode,
		CodeHash: codeHash,
	}, nil)

	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		evm := host.NewEvm(db, forkID, block, cfg)
		if runner != nil {
			evm.Set(runner)
		}
		evm.Transact(&host.Transaction{
			Kind:     host.TxKindCall,
			TxType:   host.TxTypeLegacy,
			Caller:   benchCaller,
			To:       benchContract,
			GasLimit: gasLimit,
			GasPrice: types.U256From(1),
		})
		evm.ReleaseEvm()
	}
}

// benchmarkNonModifyingCode sets up extra accounts (EOA, reverting) like go-ethereum
// and runs the code with the given gas limit.
func benchmarkNonModifyingCode(b *testing.B, gasLimit uint64, contractCode []byte) {
	codeHash := types.Keccak256(contractCode)

	// Reverting contract: PUSH1(0) PUSH1(0) REVERT
	revertCode := []byte{0x60, 0x00, 0x60, 0x00, 0xFD}
	revertAddr := types.Address{0xee}
	revertCodeHash := types.Keccak256(revertCode)

	block := blockEnvForSpec(spec.Cancun)
	cfg := defaultCfgEnv()

	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		db := newBenchDB()
		db.InsertAccount(benchContract, state.AccountInfo{
			Code:     contractCode,
			CodeHash: codeHash,
		}, nil)
		// EOA account with nonce
		db.InsertAccount(benchEOA, state.AccountInfo{
			Nonce:    100,
			CodeHash: types.KeccakEmpty,
		}, nil)
		// Reverting contract
		db.InsertAccount(revertAddr, state.AccountInfo{
			Code:     revertCode,
			CodeHash: revertCodeHash,
		}, nil)

		evm := host.NewEvm(db, spec.Cancun, block, cfg)
		evm.Transact(&host.Transaction{
			Kind:     host.TxKindCall,
			TxType:   host.TxTypeLegacy,
			Caller:   benchCaller,
			To:       benchContract,
			GasLimit: gasLimit,
			GasPrice: types.U256From(1),
		})
		evm.ReleaseEvm()
	}
}

// --- Bytecode construction helpers ---

func hexToBytes(s string) []byte {
	b, err := hex.DecodeString(s)
	if err != nil {
		panic(err)
	}
	return b
}

// swapContract builds PUSH0 PUSH0 [SWAP1 × n].
func swapContract(n int) []byte {
	code := make([]byte, 0, 2+n)
	code = append(code, 0x5F, 0x5F) // PUSH0, PUSH0
	for i := 0; i < n; i++ {
		code = append(code, 0x90) // SWAP1
	}
	return code
}

// returnContract builds PUSH8(size) PUSH0 RETURN.
func returnContract(size uint64) []byte {
	code := []byte{
		0x67, 0, 0, 0, 0, 0, 0, 0, 0, // PUSH8 <size>
		0x5F, // PUSH0
		0xF3, // RETURN
	}
	binary.BigEndian.PutUint64(code[1:], size)
	return code
}

// SimpleLoop bytecodes (manually reconstructed from go-ethereum's program builder).
// Each is a tight loop that jumps back to JUMPDEST at PC=0.
var (
	// JUMPDEST PUSH1(0) PUSH1(0) PUSH1(0) PUSH1(0) PUSH1(4) GAS STATICCALL POP PUSH1(0) JUMP
	staticCallIdentity = []byte{
		0x5B,       // JUMPDEST
		0x60, 0x00, // PUSH1 0 (retSize)
		0x60, 0x00, // PUSH1 0 (retOffset)
		0x60, 0x00, // PUSH1 0 (argsSize)
		0x60, 0x00, // PUSH1 0 (argsOffset)
		0x60, 0x04, // PUSH1 4 (address: identity precompile)
		0x5A,       // GAS
		0xFA,       // STATICCALL
		0x50,       // POP
		0x60, 0x00, // PUSH1 0
		0x56, // JUMP
	}

	// JUMPDEST PUSH1(0) PUSH1(0) PUSH1(0) PUSH1(0) PUSH1(0) PUSH1(4) GAS CALL POP PUSH1(0) JUMP
	callIdentity = []byte{
		0x5B,       // JUMPDEST
		0x60, 0x00, // PUSH1 0 (retSize)
		0x60, 0x00, // PUSH1 0 (retOffset)
		0x60, 0x00, // PUSH1 0 (argsSize)
		0x60, 0x00, // PUSH1 0 (argsOffset)
		0x60, 0x00, // PUSH1 0 (value)
		0x60, 0x04, // PUSH1 4 (address: identity precompile)
		0x5A,       // GAS
		0xF1,       // CALL
		0x50,       // POP
		0x60, 0x00, // PUSH1 0
		0x56, // JUMP
	}

	// JUMPDEST PUSH1(0) DUP1 DUP1 DUP1 PUSH1(4) GAS POP POP POP POP POP POP PUSH1(0) JUMP
	loopCode = []byte{
		0x5B,       // JUMPDEST
		0x60, 0x00, // PUSH1 0
		0x80,       // DUP1
		0x80,       // DUP1
		0x80,       // DUP1
		0x60, 0x04, // PUSH1 4
		0x5A,       // GAS
		0x50,       // POP
		0x50,       // POP
		0x50,       // POP
		0x50,       // POP
		0x50,       // POP
		0x50,       // POP
		0x60, 0x00, // PUSH1 0
		0x56, // JUMP
	}

	// JUMPDEST PUSH1(0) PUSH1(0) PUSH1(0) PUSH1(0) PUSH1(0) PUSH1(0xFF) GAS CALL POP PUSH1(0) JUMP
	callNonExist = []byte{
		0x5B,       // JUMPDEST
		0x60, 0x00, // PUSH1 0 (retSize)
		0x60, 0x00, // PUSH1 0 (retOffset)
		0x60, 0x00, // PUSH1 0 (argsSize)
		0x60, 0x00, // PUSH1 0 (argsOffset)
		0x60, 0x00, // PUSH1 0 (value)
		0x60, 0xFF, // PUSH1 0xFF (non-existent address)
		0x5A,       // GAS
		0xF1,       // CALL
		0x50,       // POP
		0x60, 0x00, // PUSH1 0
		0x56, // JUMP
	}
)

// CREATE loop bytecodes (exact go-ethereum hex).
var (
	create500    = hexToBytes("5b6207a120600080f0600152600056")
	create2_500  = hexToBytes("5b586207a120600080f5600152600056")
	create1200   = hexToBytes("5b62124f80600080f0600152600056")
	create2_1200 = hexToBytes("5b5862124f80600080f5600152600056")
)

// --- Benchmarks ---

func BenchmarkSWAP1(b *testing.B) {
	b.Run("10k", func(b *testing.B) {
		benchmarkCode(b, spec.Cancun, 10_000_000, swapContract(10_000))
	})
}

func BenchmarkRETURN(b *testing.B) {
	for _, size := range []uint64{1_000, 10_000, 100_000, 1_000_000} {
		name := ""
		switch size {
		case 1_000:
			name = "1K"
		case 10_000:
			name = "10K"
		case 100_000:
			name = "100K"
		case 1_000_000:
			name = "1M"
		}
		b.Run(name, func(b *testing.B) {
			benchmarkCode(b, spec.Cancun, 10_000_000, returnContract(size))
		})
	}
}

func BenchmarkSimpleLoop(b *testing.B) {
	b.Run("staticcall-identity-100M", func(b *testing.B) {
		benchmarkNonModifyingCode(b, 100_000_000, staticCallIdentity)
	})
	b.Run("call-identity-100M", func(b *testing.B) {
		benchmarkNonModifyingCode(b, 100_000_000, callIdentity)
	})
	b.Run("loop-100M", func(b *testing.B) {
		benchmarkNonModifyingCode(b, 100_000_000, loopCode)
	})
	b.Run("call-nonexist-100M", func(b *testing.B) {
		benchmarkNonModifyingCode(b, 100_000_000, callNonExist)
	})
}

// CREATE benchmarks use Petersburg to match go-ethereum's pre-Shanghai chain config
// (no EIP-3860 initcode size limit, no EIP-2929 warm/cold tracking).
func BenchmarkCREATE_500(b *testing.B) {
	benchmarkCode(b, spec.Petersburg, 10_000_000, create500)
}

func BenchmarkCREATE2_500(b *testing.B) {
	benchmarkCode(b, spec.Petersburg, 10_000_000, create2_500)
}

func BenchmarkCREATE_1200(b *testing.B) {
	benchmarkCode(b, spec.Petersburg, 10_000_000, create1200)
}

func BenchmarkCREATE2_1200(b *testing.B) {
	benchmarkCode(b, spec.Petersburg, 10_000_000, create2_1200)
}

// BenchmarkTransfer benchmarks a simple ETH value transfer (no contract code).
func BenchmarkTransfer(b *testing.B) {
	block := blockEnvForSpec(spec.Cancun)
	cfg := defaultCfgEnv()

	db := newBenchDB()
	// Target account exists with some balance.
	db.InsertAccount(benchContract, state.AccountInfo{
		Balance:  types.U256From(1_000_000),
		CodeHash: types.KeccakEmpty,
	}, nil)

	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		evm := host.NewEvm(db, spec.Cancun, block, cfg)
		evm.Transact(&host.Transaction{
			Kind:     host.TxKindCall,
			TxType:   host.TxTypeLegacy,
			Caller:   benchCaller,
			To:       benchContract,
			GasLimit: 21_000,
			GasPrice: types.U256From(1),
			Value:    types.U256From(1),
		})
		evm.ReleaseEvm()
	}
}

// BenchmarkAnalysis benchmarks execution of complex contract bytecode (ERC-20 deployment code).
func BenchmarkAnalysis(b *testing.B) {
	code := hexToBytes(strings.TrimSpace(analysisHex))
	codeHash := types.Keccak256(code)
	block := blockEnvForSpec(spec.Cancun)
	cfg := defaultCfgEnv()

	db := newBenchDB()
	db.InsertAccount(benchContract, state.AccountInfo{
		Code:     code,
		CodeHash: codeHash,
	}, nil)

	// Calldata: function selector 0x8035F0CE
	calldata := hexToBytes("8035F0CE")

	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		evm := host.NewEvm(db, spec.Cancun, block, cfg)
		evm.Transact(&host.Transaction{
			Kind:     host.TxKindCall,
			TxType:   host.TxTypeLegacy,
			Caller:   benchCaller,
			To:       benchContract,
			GasLimit: 1_000_000,
			GasPrice: types.U256From(1),
			Input:    calldata,
		})
		evm.ReleaseEvm()
	}
}

// BenchmarkSnailtracer benchmarks a compute-heavy ray tracer contract.
func BenchmarkSnailtracer(b *testing.B) {
	code := hexToBytes(strings.TrimSpace(snailtracerHex))
	codeHash := types.Keccak256(code)
	block := blockEnvForSpec(spec.Cancun)
	cfg := defaultCfgEnv()

	db := newBenchDB()
	db.InsertAccount(benchContract, state.AccountInfo{
		Code:     code,
		CodeHash: codeHash,
	}, nil)

	// Calldata: function selector 0x30627b7c
	calldata := hexToBytes("30627b7c")

	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		evm := host.NewEvm(db, spec.Cancun, block, cfg)
		evm.Transact(&host.Transaction{
			Kind:     host.TxKindCall,
			TxType:   host.TxTypeLegacy,
			Caller:   benchCaller,
			To:       benchContract,
			GasLimit: 1_000_000_000,
			GasPrice: types.U256From(1),
			Input:    calldata,
		})
		evm.ReleaseEvm()
	}
}

// BenchmarkERC20Transfer benchmarks an ERC-20 token transfer.
// Uses the runtime bytecode extracted from analysis.hex with pre-populated storage.
func BenchmarkERC20Transfer(b *testing.B) {
	code := hexToBytes(strings.TrimSpace(erc20RuntimeHex))
	codeHash := types.Keccak256(code)
	block := blockEnvForSpec(spec.Cancun)
	cfg := defaultCfgEnv()

	// ERC-20 storage layout (Solidity):
	//   slot 0: totalSupply
	//   slot 1: mapping(address => uint256) balances
	//   slot 2: mapping(address => mapping(address => uint256)) allowed
	//
	// balances[addr] is at keccak256(abi.encode(addr, 1))
	callerSlot := solidityMappingSlot(benchCaller, 1)
	largeBalance := types.U256FromLimbs(0, 0, 1, 0) // ~2^128

	storage := map[types.Uint256]types.Uint256{
		// totalSupply
		types.U256From(0): largeBalance,
		// balances[benchCaller]
		callerSlot: largeBalance,
	}

	db := newBenchDB()
	db.InsertAccount(benchContract, state.AccountInfo{
		Code:     code,
		CodeHash: codeHash,
	}, storage)

	// ABI: transfer(address to, uint256 amount)
	// Selector: 0xa9059cbb
	calldata := make([]byte, 4+32+32)
	copy(calldata[0:4], hexToBytes("a9059cbb"))
	// to = benchEOA (address padded to 32 bytes)
	copy(calldata[4+12:4+32], benchEOA[:])
	// amount = 1
	calldata[4+32+31] = 1

	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		evm := host.NewEvm(db, spec.Cancun, block, cfg)
		evm.Transact(&host.Transaction{
			Kind:     host.TxKindCall,
			TxType:   host.TxTypeLegacy,
			Caller:   benchCaller,
			To:       benchContract,
			GasLimit: 100_000,
			GasPrice: types.U256From(1),
			Input:    calldata,
		})
		evm.ReleaseEvm()
	}
}

// BenchmarkTxType benchmarks ERC-20 transfer with each transaction type.
// All call the same contract with identical calldata; only the tx envelope differs.
func BenchmarkTxType(b *testing.B) {
	code := hexToBytes(strings.TrimSpace(erc20RuntimeHex))
	codeHash := types.Keccak256(code)
	cfg := defaultCfgEnv()

	callerSlot := solidityMappingSlot(benchCaller, 1)
	recipientSlot := solidityMappingSlot(benchEOA, 1)
	largeBalance := types.U256FromLimbs(0, 0, 1, 0)

	storage := map[types.Uint256]types.Uint256{
		types.U256From(0): largeBalance,
		callerSlot:             largeBalance,
	}

	// ABI: transfer(address to, uint256 amount) selector = 0xa9059cbb
	calldata := make([]byte, 4+32+32)
	copy(calldata[0:4], hexToBytes("a9059cbb"))
	copy(calldata[4+12:4+32], benchEOA[:])
	calldata[4+32+31] = 1

	makeDB := func() *spectest.MemDB {
		db := newBenchDB()
		db.InsertAccount(benchContract, state.AccountInfo{
			Code:     code,
			CodeHash: codeHash,
		}, storage)
		return db
	}

	b.Run("Legacy", func(b *testing.B) {
		block := blockEnvForSpec(spec.Cancun)
		db := makeDB()
		b.ReportAllocs()
		b.ResetTimer()
		for b.Loop() {
			evm := host.NewEvm(db, spec.Cancun, block, cfg)
			evm.Transact(&host.Transaction{
				Kind:     host.TxKindCall,
				TxType:   host.TxTypeLegacy,
				Caller:   benchCaller,
				To:       benchContract,
				GasLimit: 100_000,
				GasPrice: types.U256From(1),
				Input:    calldata,
			})
			evm.ReleaseEvm()
		}
	})

	b.Run("EIP2930", func(b *testing.B) {
		block := blockEnvForSpec(spec.Cancun)
		db := makeDB()
		b.ReportAllocs()
		b.ResetTimer()
		for b.Loop() {
			evm := host.NewEvm(db, spec.Cancun, block, cfg)
			evm.Transact(&host.Transaction{
				Kind:     host.TxKindCall,
				TxType:   host.TxTypeEIP2930,
				Caller:   benchCaller,
				To:       benchContract,
				GasLimit: 100_000,
				GasPrice: types.U256From(1),
				Input:    calldata,
				AccessList: []host.AccessListItem{
					{Address: benchContract, StorageKeys: []types.Uint256{callerSlot, recipientSlot}},
				},
			})
			evm.ReleaseEvm()
		}
	})

	b.Run("EIP1559", func(b *testing.B) {
		block := blockEnvForSpec(spec.Cancun)
		db := makeDB()
		b.ReportAllocs()
		b.ResetTimer()
		for b.Loop() {
			evm := host.NewEvm(db, spec.Cancun, block, cfg)
			evm.Transact(&host.Transaction{
				Kind:                 host.TxKindCall,
				TxType:               host.TxTypeEIP1559,
				Caller:               benchCaller,
				To:                   benchContract,
				GasLimit:             100_000,
				MaxFeePerGas:         types.U256From(2),
				MaxPriorityFeePerGas: types.U256From(1),
				Input:                calldata,
			})
			evm.ReleaseEvm()
		}
	})

	b.Run("EIP4844", func(b *testing.B) {
		block := blockEnvForSpec(spec.Cancun)
		block.BlobGasPrice = types.U256From(1)
		db := makeDB()
		// Versioned hash: 0x01 prefix (VERSIONED_HASH_VERSION_KZG)
		var hashBytes types.B256
		hashBytes[0] = 0x01 // version byte in big-endian position
		blobHash := hashBytes.ToU256()
		b.ReportAllocs()
		b.ResetTimer()
		for b.Loop() {
			evm := host.NewEvm(db, spec.Cancun, block, cfg)
			evm.Transact(&host.Transaction{
				Kind:                 host.TxKindCall,
				TxType:               host.TxTypeEIP4844,
				Caller:               benchCaller,
				To:                   benchContract,
				GasLimit:             100_000,
				MaxFeePerGas:         types.U256From(2),
				MaxPriorityFeePerGas: types.U256From(1),
				MaxFeePerBlobGas:     types.U256From(10),
				Input:                calldata,
				BlobHashes:           []types.Uint256{blobHash},
			})
			evm.ReleaseEvm()
		}
	})

	b.Run("EIP7702", func(b *testing.B) {
		block := blockEnvForSpec(spec.Prague)
		db := makeDB()
		// Dummy authorization: invalid signature will be skipped but list is non-empty
		auth := host.Authorization{
			ChainId: types.U256From(1),
			Address: benchContract,
		}
		b.ReportAllocs()
		b.ResetTimer()
		for b.Loop() {
			evm := host.NewEvm(db, spec.Prague, block, cfg)
			evm.Transact(&host.Transaction{
				Kind:                 host.TxKindCall,
				TxType:               host.TxTypeEIP7702,
				Caller:               benchCaller,
				To:                   benchContract,
				GasLimit:             100_000,
				MaxFeePerGas:         types.U256From(2),
				MaxPriorityFeePerGas: types.U256From(1),
				Input:                calldata,
				AuthorizationList:    []host.Authorization{auth},
			})
			evm.ReleaseEvm()
		}
	})
}

// BenchmarkTenThousandHashes benchmarks a contract that computes 10,000 sequential
// keccak256 hashes (each hash feeds into the next). Tests keccak throughput.
func BenchmarkTenThousandHashes(b *testing.B) {
	// Contract bytecode:
	// JUMPDEST            ; 0x00: loop top
	// PUSH1 0x20          ; size = 32
	// PUSH1 0x00          ; offset = 0
	// SHA3                ; keccak256(memory[0:32])
	// PUSH1 0x00          ; offset = 0
	// MSTORE              ; store hash back
	// PUSH1 0x20          ; offset = 32
	// MLOAD               ; load counter
	// PUSH1 0x01          ; 1
	// ADD                 ; counter++
	// DUP1                ; dup counter
	// PUSH1 0x20          ; offset = 32
	// MSTORE              ; store counter
	// PUSH2 0x2710        ; 10000
	// LT                  ; counter < 10000?
	// PUSH1 0x00          ; jump target
	// JUMPI               ; loop if true
	// STOP
	code := hexToBytes("5b6020600020600052602051600101806020526127101060005700")
	benchmarkCode(b, spec.Cancun, 10_000_000, code)
}

// BenchmarkOpcode isolates individual opcode throughput in a tight loop contract.
// Each sub-benchmark creates a contract: JUMPDEST <setup+opcode> PUSH1(0) JUMP
// and runs with 10M gas until exhaustion.
func BenchmarkOpcode(b *testing.B) {
	const gas = 10_000_000

	tests := []struct {
		name string
		// Contract bytecode: JUMPDEST <body> PUSH1(0) JUMP
		code []byte
	}{
		{"ADD", opcodeLoop(0x60, 0x01, 0x60, 0x02, 0x01, 0x50)},           // PUSH1(1) PUSH1(2) ADD POP
		{"MUL", opcodeLoop(0x60, 0x03, 0x60, 0x07, 0x02, 0x50)},           // PUSH1(3) PUSH1(7) MUL POP
		{"SUB", opcodeLoop(0x60, 0x02, 0x60, 0x05, 0x03, 0x50)},           // PUSH1(2) PUSH1(5) SUB POP
		{"DIV", opcodeLoop(0x60, 0x02, 0x60, 0x0A, 0x04, 0x50)},           // PUSH1(2) PUSH1(10) DIV POP
		{"MOD", opcodeLoop(0x60, 0x03, 0x60, 0x0A, 0x06, 0x50)},           // PUSH1(3) PUSH1(10) MOD POP
		{"EXP", opcodeLoop(0x60, 0x0A, 0x60, 0x02, 0x0A, 0x50)},           // PUSH1(10) PUSH1(2) EXP POP
		{"LT", opcodeLoop(0x60, 0x02, 0x60, 0x01, 0x10, 0x50)},            // PUSH1(2) PUSH1(1) LT POP
		{"EQ", opcodeLoop(0x60, 0x01, 0x60, 0x01, 0x14, 0x50)},            // PUSH1(1) PUSH1(1) EQ POP
		{"ISZERO", opcodeLoop(0x60, 0x00, 0x15, 0x50)},                    // PUSH1(0) ISZERO POP
		{"AND", opcodeLoop(0x60, 0xFF, 0x60, 0x0F, 0x16, 0x50)},           // PUSH1(0xFF) PUSH1(0x0F) AND POP
		{"SHL", opcodeLoop(0x60, 0xFF, 0x60, 0x04, 0x1B, 0x50)},           // PUSH1(0xFF) PUSH1(4) SHL POP
		{"SHR", opcodeLoop(0x60, 0xFF, 0x60, 0x04, 0x1C, 0x50)},           // PUSH1(0xFF) PUSH1(4) SHR POP
		{"KECCAK256", opcodeLoop(0x60, 0x20, 0x60, 0x00, 0x20, 0x50)},     // PUSH1(32) PUSH1(0) KECCAK256 POP
		{"MLOAD", opcodeLoop(0x60, 0x00, 0x51, 0x50)},                     // PUSH1(0) MLOAD POP
		{"MSTORE", opcodeLoop(0x60, 0x2A, 0x60, 0x00, 0x52)},              // PUSH1(42) PUSH1(0) MSTORE
		{"CALLDATALOAD", opcodeLoop(0x60, 0x00, 0x35, 0x50)},              // PUSH1(0) CALLDATALOAD POP
		{"PUSH1_POP", opcodeLoop(0x60, 0x01, 0x50)},                       // PUSH1(1) POP (baseline dispatch)
		{"DUP1_POP", opcodeLoopWithSetup([]byte{0x60, 0x00}, 0x80, 0x50)}, // setup: PUSH1(0), loop: DUP1 POP
		{"SWAP1", opcodeLoopWithSetup([]byte{0x60, 0x00, 0x60, 0x00}, 0x90)}, // setup: PUSH1(0) PUSH1(0), loop: SWAP1
	}

	for _, tc := range tests {
		b.Run(tc.name, func(b *testing.B) {
			benchmarkCode(b, spec.Cancun, gas, tc.code)
		})
	}
}

// opcodeLoop builds: JUMPDEST <body bytes> PUSH1(0) JUMP
func opcodeLoop(body ...byte) []byte {
	code := make([]byte, 0, 1+len(body)+3)
	code = append(code, 0x5B) // JUMPDEST
	code = append(code, body...)
	code = append(code, 0x60, 0x00, 0x56) // PUSH1(0) JUMP
	return code
}

// opcodeLoopWithSetup builds: <setup> JUMPDEST <body bytes> PUSH1(<jumpdest>) JUMP
// The JUMPDEST is placed after setup so the setup runs once, then the loop body repeats.
func opcodeLoopWithSetup(setup []byte, body ...byte) []byte {
	jdOffset := len(setup)
	code := make([]byte, 0, len(setup)+1+len(body)+3)
	code = append(code, setup...)
	code = append(code, 0x5B) // JUMPDEST
	code = append(code, body...)
	code = append(code, 0x60, byte(jdOffset), 0x56) // PUSH1(jdOffset) JUMP
	return code
}

// solidityMappingSlot computes keccak256(abi.encode(key, slot)) for a Solidity mapping.
// key is an address, slot is the storage slot of the mapping.
func solidityMappingSlot(key types.Address, slot uint64) types.Uint256 {
	var buf [64]byte
	// key padded to 32 bytes (left-padded with zeros, address in last 20 bytes)
	copy(buf[12:32], key[:])
	// slot as 32-byte big-endian
	buf[56] = byte(slot >> 56)
	buf[57] = byte(slot >> 48)
	buf[58] = byte(slot >> 40)
	buf[59] = byte(slot >> 32)
	buf[60] = byte(slot >> 24)
	buf[61] = byte(slot >> 16)
	buf[62] = byte(slot >> 8)
	buf[63] = byte(slot)
	hash := types.B256(keccak.Sum256(buf[:]))
	return hash.ToU256()
}

// --- Precompile benchmarks ---
// These call the precompile functions directly (no EVM overhead).
// Benchmark names match gethbench/ for benchstat comparison.

// ecrecoverInput is a valid ECRECOVER input (msg hash + v=28 + r + s).
var ecrecoverInput = hexToBytes(
	"456e9aea5e197a1f1af7a3e85a3212fa4049a3ba34c2289b4c860fc0b0c64ef3" +
		"000000000000000000000000000000000000000000000000000000000000001c" +
		"9242685bf161793cc25603c231bc2f568eb630ea16aa137d2664ac8038825608" +
		"4f8ae3bd7535248d0bd448298cc2e2071e56992d0774dc340c368ae950852ada")

func BenchmarkPrecompileEcrecover(b *testing.B) {
	b.ReportAllocs()
	for b.Loop() {
		precompiles.EcRecoverRun(ecrecoverInput, 10000)
	}
}

func BenchmarkPrecompileSha256(b *testing.B) {
	input := make([]byte, 128) // 4 words
	b.ReportAllocs()
	for b.Loop() {
		precompiles.Sha256Run(input, 10000)
	}
}

func BenchmarkPrecompileRipemd160(b *testing.B) {
	input := make([]byte, 128)
	b.ReportAllocs()
	for b.Loop() {
		precompiles.Ripemd160Run(input, 10000)
	}
}

func BenchmarkPrecompileIdentity(b *testing.B) {
	b.Run("128B", func(b *testing.B) {
		input := make([]byte, 128)
		b.ReportAllocs()
		for b.Loop() {
			precompiles.IdentityRun(input, 10000)
		}
	})
	b.Run("1KB", func(b *testing.B) {
		input := make([]byte, 1024)
		b.ReportAllocs()
		for b.Loop() {
			precompiles.IdentityRun(input, 10000)
		}
	})
}

// modexpInput: 2^65537 mod (2^256-1) — realistic RSA-style exponent.
var modexpInput = hexToBytes(
	"0000000000000000000000000000000000000000000000000000000000000001" + // base_len=1
		"0000000000000000000000000000000000000000000000000000000000000003" + // exp_len=3
		"0000000000000000000000000000000000000000000000000000000000000020" + // mod_len=32
		"02" +         // base=2
		"010001" +     // exp=65537
		"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff") // mod=2^256-1

func BenchmarkPrecompileModexp(b *testing.B) {
	b.ReportAllocs()
	for b.Loop() {
		precompiles.ModExpBerlinRun(modexpInput, 100000)
	}
}

// BN254 inputs: G1 generator (1,2).
var bn254G1Gen = hexToBytes(
	"0000000000000000000000000000000000000000000000000000000000000001" +
		"0000000000000000000000000000000000000000000000000000000000000002")

func BenchmarkPrecompileBn254Add(b *testing.B) {
	input := make([]byte, 128)
	copy(input[0:64], bn254G1Gen)
	copy(input[64:128], bn254G1Gen)
	b.ReportAllocs()
	for b.Loop() {
		precompiles.Bn254AddIstanbulRun(input, 10000)
	}
}

func BenchmarkPrecompileBn254Mul(b *testing.B) {
	// G1 * scalar(7)
	input := make([]byte, 96)
	copy(input[0:64], bn254G1Gen)
	input[95] = 7
	b.ReportAllocs()
	for b.Loop() {
		precompiles.Bn254MulIstanbulRun(input, 100000)
	}
}

func BenchmarkPrecompileBn254Pairing(b *testing.B) {
	// Single pair: G1 generator + G2 generator
	// BN254 G2 generator: EVM encoding: x.A1(imag) | x.A0(real) | y.A1(imag) | y.A0(real)
	g2Gen := hexToBytes(
		"198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c2" +
			"1800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed" +
			"090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b" +
			"12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa")

	// e(G1, G2): 192 bytes = 64 (G1) + 128 (G2)
	input := make([]byte, 192)
	copy(input[0:64], bn254G1Gen)
	copy(input[64:192], g2Gen)
	b.ReportAllocs()
	for b.Loop() {
		precompiles.Bn254PairingIstanbulRun(input, 200000)
	}
}

// blake2fInput: EIP-152 test vector (12 rounds).
var blake2fInput = hexToBytes(
	"0000000c" +
		"48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5" +
		"d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b" +
		"6162630000000000000000000000000000000000000000000000000000000000" +
		"0000000000000000000000000000000000000000000000000000000000000000" +
		"0000000000000000000000000000000000000000000000000000000000000000" +
		"0000000000000000000000000000000000000000000000000000000000000000" +
		"0300000000000000" +
		"0000000000000000" +
		"01")

func BenchmarkPrecompileBlake2f(b *testing.B) {
	b.ReportAllocs()
	for b.Loop() {
		precompiles.Blake2FRun(blake2fInput, 10000)
	}
}

// blsG1Gen/blsG2Gen: BLS12-381 generator points in EVM encoding.
var (
	blsG1GenEncoded [128]byte
	blsG2GenEncoded [256]byte
)

func init() {
	_, _, g1Aff, g2Aff := bls12381.Generators()
	copy(blsG1GenEncoded[:], encodeBlsG1(&g1Aff))
	copy(blsG2GenEncoded[:], encodeBlsG2(&g2Aff))
}

// encodeBlsG1 encodes a BLS12-381 G1 point to 128-byte EVM format (16-byte padding + 48-byte Fp).
func encodeBlsG1(p *bls12381.G1Affine) []byte {
	out := make([]byte, 128)
	xBytes := p.X.Bytes() // [48]byte big-endian
	yBytes := p.Y.Bytes()
	copy(out[16:64], xBytes[:])
	copy(out[80:128], yBytes[:])
	return out
}

// encodeBlsG2 encodes a BLS12-381 G2 point to 256-byte EVM format.
func encodeBlsG2(p *bls12381.G2Affine) []byte {
	out := make([]byte, 256)
	// G2 point has coordinates (x, y) where x, y are Fp2 elements (a0 + a1*u)
	// EVM encoding: x.A0 (64) | x.A1 (64) | y.A0 (64) | y.A1 (64)
	x0Bytes := p.X.A0.Bytes()
	x1Bytes := p.X.A1.Bytes()
	y0Bytes := p.Y.A0.Bytes()
	y1Bytes := p.Y.A1.Bytes()
	copy(out[16:64], x0Bytes[:])
	copy(out[80:128], x1Bytes[:])
	copy(out[144:192], y0Bytes[:])
	copy(out[208:256], y1Bytes[:])
	return out
}

func BenchmarkPrecompileBlsG1Add(b *testing.B) {
	input := make([]byte, 256)
	copy(input[0:128], blsG1GenEncoded[:])
	copy(input[128:256], blsG1GenEncoded[:])
	b.ReportAllocs()
	for b.Loop() {
		precompiles.Bls12G1AddRun(input, 10000)
	}
}

func BenchmarkPrecompileBlsG1Msm(b *testing.B) {
	// G1 * scalar(7)
	input := make([]byte, 160)
	copy(input[0:128], blsG1GenEncoded[:])
	input[159] = 7
	b.ReportAllocs()
	for b.Loop() {
		precompiles.Bls12G1MsmRun(input, 100000)
	}
}

func BenchmarkPrecompileBlsG2Add(b *testing.B) {
	input := make([]byte, 512)
	copy(input[0:256], blsG2GenEncoded[:])
	copy(input[256:512], blsG2GenEncoded[:])
	b.ReportAllocs()
	for b.Loop() {
		precompiles.Bls12G2AddRun(input, 10000)
	}
}

func BenchmarkPrecompileBlsG2Msm(b *testing.B) {
	// G2 * scalar(7)
	input := make([]byte, 288)
	copy(input[0:256], blsG2GenEncoded[:])
	input[287] = 7
	b.ReportAllocs()
	for b.Loop() {
		precompiles.Bls12G2MsmRun(input, 100000)
	}
}

func BenchmarkPrecompileBlsPairing(b *testing.B) {
	// Single pair: (G1, G2)
	input := make([]byte, 384)
	copy(input[0:128], blsG1GenEncoded[:])
	copy(input[128:384], blsG2GenEncoded[:])
	b.ReportAllocs()
	for b.Loop() {
		precompiles.Bls12PairingRun(input, 200000)
	}
}

func BenchmarkPrecompileBlsMapFpToG1(b *testing.B) {
	// Map field element 1 to G1
	input := make([]byte, 64)
	input[63] = 1
	b.ReportAllocs()
	for b.Loop() {
		precompiles.Bls12MapFpToG1Run(input, 100000)
	}
}

func BenchmarkPrecompileBlsMapFp2ToG2(b *testing.B) {
	// Map Fp2 element (1, 1) to G2
	input := make([]byte, 128)
	input[63] = 1
	input[127] = 1
	b.ReportAllocs()
	for b.Loop() {
		precompiles.Bls12MapFp2ToG2Run(input, 100000)
	}
}

// p256Input: valid P256VERIFY input.
var p256Input = hexToBytes(
	"4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4d" +
		"a73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac" +
		"36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d60" +
		"4aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff3" +
		"7618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e")

func BenchmarkPrecompileP256Verify(b *testing.B) {
	b.ReportAllocs()
	for b.Loop() {
		precompiles.P256VerifyRun(p256Input, 10000)
	}
}

