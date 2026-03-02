// Go-ethereum equivalent of the GEVM benchmarks.
// Uses identical benchmark names for benchstat comparison.
package gethbench

import (
	_ "embed"
	"encoding/binary"
	"encoding/hex"
	"math/big"
	"strings"
	"testing"

	"github.com/ethereum/go-ethereum/common"
	"github.com/ethereum/go-ethereum/core"
	"github.com/ethereum/go-ethereum/core/state"
	"github.com/ethereum/go-ethereum/core/tracing"
	"github.com/ethereum/go-ethereum/core/types"
	"github.com/ethereum/go-ethereum/core/vm"
	"github.com/ethereum/go-ethereum/core/vm/runtime"
	"github.com/ethereum/go-ethereum/crypto"
	"github.com/ethereum/go-ethereum/params"
	"github.com/holiman/uint256"
)

//go:embed testdata/analysis.hex
var analysisHex string

//go:embed testdata/snailtracer.hex
var snailtracerHex string

//go:embed testdata/erc20_runtime.hex
var erc20RuntimeHex string

func hexToBytes(s string) []byte {
	b, err := hex.DecodeString(s)
	if err != nil {
		panic(err)
	}
	return b
}

// benchmarkCode runs a go-ethereum Call benchmark.
func benchmarkCode(b *testing.B, gasLimit uint64, code []byte) {
	statedb, _ := state.New(types.EmptyRootHash, state.NewDatabaseForTesting())
	contractAddr := common.BytesToAddress([]byte("contract"))
	statedb.SetCode(contractAddr, code, tracing.CodeChangeUnspecified)

	cfg := &runtime.Config{
		State:    statedb,
		GasLimit: gasLimit,
	}

	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		runtime.Call(contractAddr, nil, cfg)
	}
}

// benchmarkNonModifyingCode mirrors go-ethereum's benchmarkNonModifyingCode.
func benchmarkNonModifyingCode(b *testing.B, gasLimit uint64, code []byte) {
	cfg := new(runtime.Config)
	cfg.State, _ = state.New(types.EmptyRootHash, state.NewDatabaseForTesting())
	cfg.GasLimit = gasLimit

	destination := common.BytesToAddress([]byte("contract"))
	cfg.State.CreateAccount(destination)

	eoa := common.HexToAddress("E0")
	cfg.State.CreateAccount(eoa)
	cfg.State.SetNonce(eoa, 100, tracing.NonceChangeUnspecified)

	reverting := common.HexToAddress("EE")
	cfg.State.CreateAccount(reverting)
	cfg.State.SetCode(reverting, []byte{
		byte(vm.PUSH1), 0x00,
		byte(vm.PUSH1), 0x00,
		byte(vm.REVERT),
	}, tracing.CodeChangeUnspecified)

	cfg.State.SetCode(destination, code, tracing.CodeChangeUnspecified)

	// Warm up
	runtime.Call(destination, nil, cfg)

	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		runtime.Call(destination, nil, cfg)
	}
}

// benchmarkCreate mirrors go-ethereum's benchmarkEVM_Create.
func benchmarkCreate(b *testing.B, codeHex string) {
	statedb, _ := state.New(types.EmptyRootHash, state.NewDatabaseForTesting())
	sender := common.BytesToAddress([]byte("sender"))
	receiver := common.BytesToAddress([]byte("receiver"))

	statedb.CreateAccount(sender)
	statedb.SetCode(receiver, common.FromHex(codeHex), tracing.CodeChangeUnspecified)

	cfg := runtime.Config{
		Origin:      sender,
		State:       statedb,
		GasLimit:    10_000_000,
		Difficulty:  big.NewInt(0x200000),
		Time:        0,
		Coinbase:    common.Address{},
		BlockNumber: new(big.Int).SetUint64(1),
		ChainConfig: &params.ChainConfig{
			ChainID:             big.NewInt(1),
			HomesteadBlock:      new(big.Int),
			ByzantiumBlock:      new(big.Int),
			ConstantinopleBlock: new(big.Int),
			DAOForkBlock:        new(big.Int),
			DAOForkSupport:      false,
			EIP150Block:         new(big.Int),
			EIP155Block:         new(big.Int),
			EIP158Block:         new(big.Int),
		},
		EVMConfig: vm.Config{},
	}

	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		runtime.Call(receiver, []byte{}, &cfg)
	}
}

// --- Bytecode construction ---

func swapContract(n int) []byte {
	code := make([]byte, 0, 2+n)
	code = append(code, byte(vm.PUSH0), byte(vm.PUSH0))
	for i := 0; i < n; i++ {
		code = append(code, byte(vm.SWAP1))
	}
	return code
}

func returnContract(size uint64) []byte {
	code := []byte{
		byte(vm.PUSH8), 0, 0, 0, 0, 0, 0, 0, 0,
		byte(vm.PUSH0),
		byte(vm.RETURN),
	}
	binary.BigEndian.PutUint64(code[1:], size)
	return code
}

// SimpleLoop bytecodes (matching go-ethereum's program builder output).
var (
	staticCallIdentity = []byte{
		byte(vm.JUMPDEST),
		byte(vm.PUSH1), 0x00,
		byte(vm.PUSH1), 0x00,
		byte(vm.PUSH1), 0x00,
		byte(vm.PUSH1), 0x00,
		byte(vm.PUSH1), 0x04,
		byte(vm.GAS),
		byte(vm.STATICCALL),
		byte(vm.POP),
		byte(vm.PUSH1), 0x00,
		byte(vm.JUMP),
	}

	callIdentity = []byte{
		byte(vm.JUMPDEST),
		byte(vm.PUSH1), 0x00,
		byte(vm.PUSH1), 0x00,
		byte(vm.PUSH1), 0x00,
		byte(vm.PUSH1), 0x00,
		byte(vm.PUSH1), 0x00,
		byte(vm.PUSH1), 0x04,
		byte(vm.GAS),
		byte(vm.CALL),
		byte(vm.POP),
		byte(vm.PUSH1), 0x00,
		byte(vm.JUMP),
	}

	loopCode = []byte{
		byte(vm.JUMPDEST),
		byte(vm.PUSH1), 0x00,
		byte(vm.DUP1),
		byte(vm.DUP1),
		byte(vm.DUP1),
		byte(vm.PUSH1), 0x04,
		byte(vm.GAS),
		byte(vm.POP),
		byte(vm.POP),
		byte(vm.POP),
		byte(vm.POP),
		byte(vm.POP),
		byte(vm.POP),
		byte(vm.PUSH1), 0x00,
		byte(vm.JUMP),
	}

	callNonExist = []byte{
		byte(vm.JUMPDEST),
		byte(vm.PUSH1), 0x00,
		byte(vm.PUSH1), 0x00,
		byte(vm.PUSH1), 0x00,
		byte(vm.PUSH1), 0x00,
		byte(vm.PUSH1), 0x00,
		byte(vm.PUSH1), 0xFF,
		byte(vm.GAS),
		byte(vm.CALL),
		byte(vm.POP),
		byte(vm.PUSH1), 0x00,
		byte(vm.JUMP),
	}
)

// --- Benchmarks (identical names to GEVM benchmarks) ---

func BenchmarkSWAP1(b *testing.B) {
	b.Run("10k", func(b *testing.B) {
		benchmarkCode(b, 10_000_000, swapContract(10_000))
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
			benchmarkCode(b, 10_000_000, returnContract(size))
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

func BenchmarkCREATE_500(b *testing.B) {
	benchmarkCreate(b, "5b6207a120600080f0600152600056")
}

func BenchmarkCREATE2_500(b *testing.B) {
	benchmarkCreate(b, "5b586207a120600080f5600152600056")
}

func BenchmarkCREATE_1200(b *testing.B) {
	benchmarkCreate(b, "5b62124f80600080f0600152600056")
}

func BenchmarkCREATE2_1200(b *testing.B) {
	benchmarkCreate(b, "5b5862124f80600080f5600152600056")
}


// BenchmarkTransfer benchmarks a simple ETH value transfer.
func BenchmarkTransfer(b *testing.B) {
	statedb, _ := state.New(types.EmptyRootHash, state.NewDatabaseForTesting())
	sender := common.BytesToAddress([]byte{0x01})
	target := common.BytesToAddress([]byte{0x10})

	statedb.CreateAccount(sender)
	statedb.AddBalance(sender, uint256.NewInt(1_000_000_000_000), tracing.BalanceChangeUnspecified)
	statedb.CreateAccount(target)
	statedb.AddBalance(target, uint256.NewInt(1_000_000), tracing.BalanceChangeUnspecified)

	cfg := &runtime.Config{
		Origin:   sender,
		State:    statedb,
		GasLimit: 21_000,
		Value:    big.NewInt(1),
	}

	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		runtime.Call(target, nil, cfg)
	}
}

// BenchmarkAnalysis benchmarks execution of complex contract bytecode (ERC-20 deployment code).
func BenchmarkAnalysis(b *testing.B) {
	code := hexToBytes(strings.TrimSpace(analysisHex))

	statedb, _ := state.New(types.EmptyRootHash, state.NewDatabaseForTesting())
	contractAddr := common.BytesToAddress([]byte{0x10})
	statedb.SetCode(contractAddr, code, tracing.CodeChangeUnspecified)

	cfg := &runtime.Config{
		State:    statedb,
		GasLimit: 1_000_000,
	}

	input := hexToBytes("8035F0CE")

	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		runtime.Call(contractAddr, input, cfg)
	}
}

// BenchmarkSnailtracer benchmarks a compute-heavy ray tracer contract.
func BenchmarkSnailtracer(b *testing.B) {
	code := hexToBytes(strings.TrimSpace(snailtracerHex))

	statedb, _ := state.New(types.EmptyRootHash, state.NewDatabaseForTesting())
	contractAddr := common.BytesToAddress([]byte{0x10})
	statedb.SetCode(contractAddr, code, tracing.CodeChangeUnspecified)

	cfg := &runtime.Config{
		State:    statedb,
		GasLimit: 1_000_000_000,
	}

	input := hexToBytes("30627b7c")

	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		runtime.Call(contractAddr, input, cfg)
	}
}

// BenchmarkERC20Transfer benchmarks an ERC-20 token transfer.
func BenchmarkERC20Transfer(b *testing.B) {
	code := hexToBytes(strings.TrimSpace(erc20RuntimeHex))

	statedb, _ := state.New(types.EmptyRootHash, state.NewDatabaseForTesting())
	contractAddr := common.BytesToAddress([]byte{0x10})
	caller := common.BytesToAddress([]byte{0x01})
	recipient := common.BytesToAddress([]byte{0xe0})

	statedb.CreateAccount(caller)
	statedb.AddBalance(caller, uint256.NewInt(1_000_000_000), tracing.BalanceChangeUnspecified)
	statedb.SetCode(contractAddr, code, tracing.CodeChangeUnspecified)

	// Pre-populate ERC-20 storage: balances[caller] = huge amount
	// Solidity mapping: balances is at slot 1
	// balances[addr] = keccak256(abi.encode(addr, 1))
	callerSlot := solidityMappingSlot(caller, 1)
	largeBalance := new(uint256.Int).Lsh(uint256.NewInt(1), 128)
	b32 := largeBalance.Bytes32()
	statedb.SetState(contractAddr, callerSlot, common.BytesToHash(b32[:]))
	// totalSupply at slot 0
	statedb.SetState(contractAddr, common.Hash{}, common.BytesToHash(b32[:]))

	// ABI: transfer(address to, uint256 amount) selector = 0xa9059cbb
	calldata := make([]byte, 4+32+32)
	copy(calldata[0:4], hexToBytes("a9059cbb"))
	copy(calldata[4+12:4+32], recipient.Bytes())
	calldata[4+32+31] = 1

	cfg := &runtime.Config{
		Origin:   caller,
		State:    statedb,
		GasLimit: 100_000,
	}

	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		runtime.Call(contractAddr, calldata, cfg)
	}
}

// BenchmarkTxType benchmarks ERC-20 transfer with each transaction type using core.ApplyMessage.
func BenchmarkTxType(b *testing.B) {
	code := hexToBytes(strings.TrimSpace(erc20RuntimeHex))
	caller := common.BytesToAddress([]byte{0x01})
	contractAddr := common.BytesToAddress([]byte{0x10})
	recipient := common.BytesToAddress([]byte{0xe0})
	coinbase := common.BytesToAddress([]byte{0xcc})

	callerSlot := solidityMappingSlot(caller, 1)
	recipientSlot := solidityMappingSlot(recipient, 1)
	largeBalance := new(uint256.Int).Lsh(uint256.NewInt(1), 128)
	b32 := largeBalance.Bytes32()

	// ABI: transfer(address to, uint256 amount) selector = 0xa9059cbb
	calldata := make([]byte, 4+32+32)
	copy(calldata[0:4], hexToBytes("a9059cbb"))
	copy(calldata[4+12:4+32], recipient.Bytes())
	calldata[4+32+31] = 1

	setupState := func() *state.StateDB {
		statedb, _ := state.New(types.EmptyRootHash, state.NewDatabaseForTesting())
		statedb.CreateAccount(caller)
		statedb.AddBalance(caller, new(uint256.Int).Lsh(uint256.NewInt(1), 128), tracing.BalanceChangeUnspecified)
		statedb.SetCode(contractAddr, code, tracing.CodeChangeUnspecified)
		statedb.SetState(contractAddr, callerSlot, common.BytesToHash(b32[:]))
		statedb.SetState(contractAddr, common.Hash{}, common.BytesToHash(b32[:]))
		return statedb
	}

	// Chain configs with all forks enabled up to target
	zero := uint64(0)
	cancunConfig := &params.ChainConfig{
		ChainID:                 big.NewInt(1),
		HomesteadBlock:          new(big.Int),
		DAOForkBlock:            new(big.Int),
		EIP150Block:             new(big.Int),
		EIP155Block:             new(big.Int),
		EIP158Block:             new(big.Int),
		ByzantiumBlock:          new(big.Int),
		ConstantinopleBlock:     new(big.Int),
		PetersburgBlock:         new(big.Int),
		IstanbulBlock:           new(big.Int),
		MuirGlacierBlock:        new(big.Int),
		BerlinBlock:             new(big.Int),
		LondonBlock:             new(big.Int),
		ArrowGlacierBlock:       new(big.Int),
		GrayGlacierBlock:        new(big.Int),
		MergeNetsplitBlock:      new(big.Int),
		TerminalTotalDifficulty: big.NewInt(0),
		ShanghaiTime:            &zero,
		CancunTime:              &zero,
	}
	pragueConfig := &params.ChainConfig{
		ChainID:                 big.NewInt(1),
		HomesteadBlock:          new(big.Int),
		DAOForkBlock:            new(big.Int),
		EIP150Block:             new(big.Int),
		EIP155Block:             new(big.Int),
		EIP158Block:             new(big.Int),
		ByzantiumBlock:          new(big.Int),
		ConstantinopleBlock:     new(big.Int),
		PetersburgBlock:         new(big.Int),
		IstanbulBlock:           new(big.Int),
		MuirGlacierBlock:        new(big.Int),
		BerlinBlock:             new(big.Int),
		LondonBlock:             new(big.Int),
		ArrowGlacierBlock:       new(big.Int),
		GrayGlacierBlock:        new(big.Int),
		MergeNetsplitBlock:      new(big.Int),
		TerminalTotalDifficulty: big.NewInt(0),
		ShanghaiTime:            &zero,
		CancunTime:              &zero,
		PragueTime:              &zero,
	}

	random := common.Hash{0x01}
	blockCtx := vm.BlockContext{
		CanTransfer: core.CanTransfer,
		Transfer:    core.Transfer,
		GetHash:     func(n uint64) common.Hash { return common.Hash{} },
		Coinbase:    coinbase,
		BlockNumber: big.NewInt(1),
		Time:        1000,
		Difficulty:  big.NewInt(0),
		BaseFee:     big.NewInt(1),
		BlobBaseFee: big.NewInt(1),
		GasLimit:    1_000_000_000,
		Random:      &random,
	}
	vmCfg := vm.Config{}

	to := contractAddr // local copy for pointer

	b.Run("Legacy", func(b *testing.B) {
		statedb := setupState()
		b.ReportAllocs()
		b.ResetTimer()
		for b.Loop() {
			snap := statedb.Snapshot()
			evm := vm.NewEVM(blockCtx, statedb, cancunConfig, vmCfg)
			gp := new(core.GasPool).AddGas(1_000_000_000)
			core.ApplyMessage(evm, &core.Message{
				From:      caller,
				To:        &to,
				GasLimit:  100_000,
				GasPrice:  big.NewInt(1),
				GasFeeCap: big.NewInt(1),
				GasTipCap: big.NewInt(1),
				Value:     new(big.Int),
				Data:      calldata,
			}, gp)
			statedb.RevertToSnapshot(snap)
		}
	})

	b.Run("EIP2930", func(b *testing.B) {
		statedb := setupState()
		b.ReportAllocs()
		b.ResetTimer()
		for b.Loop() {
			snap := statedb.Snapshot()
			evm := vm.NewEVM(blockCtx, statedb, cancunConfig, vmCfg)
			gp := new(core.GasPool).AddGas(1_000_000_000)
			core.ApplyMessage(evm, &core.Message{
				From:      caller,
				To:        &to,
				GasLimit:  100_000,
				GasPrice:  big.NewInt(1),
				GasFeeCap: big.NewInt(1),
				GasTipCap: big.NewInt(1),
				Value:     new(big.Int),
				Data:      calldata,
				AccessList: types.AccessList{
					{Address: contractAddr, StorageKeys: []common.Hash{callerSlot, recipientSlot}},
				},
			}, gp)
			statedb.RevertToSnapshot(snap)
		}
	})

	b.Run("EIP1559", func(b *testing.B) {
		statedb := setupState()
		b.ReportAllocs()
		b.ResetTimer()
		for b.Loop() {
			snap := statedb.Snapshot()
			evm := vm.NewEVM(blockCtx, statedb, cancunConfig, vmCfg)
			gp := new(core.GasPool).AddGas(1_000_000_000)
			core.ApplyMessage(evm, &core.Message{
				From:      caller,
				To:        &to,
				GasLimit:  100_000,
				GasPrice:  big.NewInt(2), // effective: min(tip+base, cap) = min(1+1,2) = 2
				GasFeeCap: big.NewInt(2),
				GasTipCap: big.NewInt(1),
				Value:     new(big.Int),
				Data:      calldata,
			}, gp)
			statedb.RevertToSnapshot(snap)
		}
	})

	b.Run("EIP4844", func(b *testing.B) {
		statedb := setupState()
		blobHash := common.Hash{0x01} // VERSIONED_HASH_VERSION_KZG prefix
		b.ReportAllocs()
		b.ResetTimer()
		for b.Loop() {
			snap := statedb.Snapshot()
			evm := vm.NewEVM(blockCtx, statedb, cancunConfig, vmCfg)
			gp := new(core.GasPool).AddGas(1_000_000_000)
			core.ApplyMessage(evm, &core.Message{
				From:          caller,
				To:            &to,
				GasLimit:      100_000,
				GasPrice:      big.NewInt(2),
				GasFeeCap:     big.NewInt(2),
				GasTipCap:     big.NewInt(1),
				Value:         new(big.Int),
				Data:          calldata,
				BlobHashes:    []common.Hash{blobHash},
				BlobGasFeeCap: big.NewInt(10),
			}, gp)
			statedb.RevertToSnapshot(snap)
		}
	})

	b.Run("EIP7702", func(b *testing.B) {
		statedb := setupState()
		auth := types.SetCodeAuthorization{
			ChainID: *uint256.NewInt(1),
			Address: contractAddr,
		}
		b.ReportAllocs()
		b.ResetTimer()
		for b.Loop() {
			snap := statedb.Snapshot()
			evm := vm.NewEVM(blockCtx, statedb, pragueConfig, vmCfg)
			gp := new(core.GasPool).AddGas(1_000_000_000)
			core.ApplyMessage(evm, &core.Message{
				From:                  caller,
				To:                    &to,
				GasLimit:              100_000,
				GasPrice:              big.NewInt(2),
				GasFeeCap:             big.NewInt(2),
				GasTipCap:             big.NewInt(1),
				Value:                 new(big.Int),
				Data:                  calldata,
				SetCodeAuthorizations: []types.SetCodeAuthorization{auth},
			}, gp)
			statedb.RevertToSnapshot(snap)
		}
	})
}

// BenchmarkTenThousandHashes benchmarks 10,000 sequential keccak256 hashes.
func BenchmarkTenThousandHashes(b *testing.B) {
	code := hexToBytes("5b6020600020600052602051600101806020526127101060005700")
	benchmarkCode(b, 10_000_000, code)
}

// BenchmarkOpcode isolates individual opcode throughput in a tight loop contract.
func BenchmarkOpcode(b *testing.B) {
	const gas = 10_000_000

	tests := []struct {
		name string
		code []byte
	}{
		{"ADD", opcodeLoop(byte(vm.PUSH1), 0x01, byte(vm.PUSH1), 0x02, byte(vm.ADD), byte(vm.POP))},
		{"MUL", opcodeLoop(byte(vm.PUSH1), 0x03, byte(vm.PUSH1), 0x07, byte(vm.MUL), byte(vm.POP))},
		{"SUB", opcodeLoop(byte(vm.PUSH1), 0x02, byte(vm.PUSH1), 0x05, byte(vm.SUB), byte(vm.POP))},
		{"DIV", opcodeLoop(byte(vm.PUSH1), 0x02, byte(vm.PUSH1), 0x0A, byte(vm.DIV), byte(vm.POP))},
		{"MOD", opcodeLoop(byte(vm.PUSH1), 0x03, byte(vm.PUSH1), 0x0A, byte(vm.MOD), byte(vm.POP))},
		{"EXP", opcodeLoop(byte(vm.PUSH1), 0x0A, byte(vm.PUSH1), 0x02, byte(vm.EXP), byte(vm.POP))},
		{"LT", opcodeLoop(byte(vm.PUSH1), 0x02, byte(vm.PUSH1), 0x01, byte(vm.LT), byte(vm.POP))},
		{"EQ", opcodeLoop(byte(vm.PUSH1), 0x01, byte(vm.PUSH1), 0x01, byte(vm.EQ), byte(vm.POP))},
		{"ISZERO", opcodeLoop(byte(vm.PUSH1), 0x00, byte(vm.ISZERO), byte(vm.POP))},
		{"AND", opcodeLoop(byte(vm.PUSH1), 0xFF, byte(vm.PUSH1), 0x0F, byte(vm.AND), byte(vm.POP))},
		{"SHL", opcodeLoop(byte(vm.PUSH1), 0xFF, byte(vm.PUSH1), 0x04, byte(vm.SHL), byte(vm.POP))},
		{"SHR", opcodeLoop(byte(vm.PUSH1), 0xFF, byte(vm.PUSH1), 0x04, byte(vm.SHR), byte(vm.POP))},
		{"KECCAK256", opcodeLoop(byte(vm.PUSH1), 0x20, byte(vm.PUSH1), 0x00, byte(vm.KECCAK256), byte(vm.POP))},
		{"MLOAD", opcodeLoop(byte(vm.PUSH1), 0x00, byte(vm.MLOAD), byte(vm.POP))},
		{"MSTORE", opcodeLoop(byte(vm.PUSH1), 0x2A, byte(vm.PUSH1), 0x00, byte(vm.MSTORE))},
		{"CALLDATALOAD", opcodeLoop(byte(vm.PUSH1), 0x00, byte(vm.CALLDATALOAD), byte(vm.POP))},
		{"PUSH1_POP", opcodeLoop(byte(vm.PUSH1), 0x01, byte(vm.POP))},
		{"DUP1_POP", opcodeLoopWithSetup([]byte{byte(vm.PUSH1), 0x00}, byte(vm.DUP1), byte(vm.POP))},
		{"SWAP1", opcodeLoopWithSetup([]byte{byte(vm.PUSH1), 0x00, byte(vm.PUSH1), 0x00}, byte(vm.SWAP1))},
	}

	for _, tc := range tests {
		b.Run(tc.name, func(b *testing.B) {
			benchmarkCode(b, gas, tc.code)
		})
	}
}

func opcodeLoop(body ...byte) []byte {
	code := make([]byte, 0, 1+len(body)+3)
	code = append(code, byte(vm.JUMPDEST))
	code = append(code, body...)
	code = append(code, byte(vm.PUSH1), 0x00, byte(vm.JUMP))
	return code
}

func opcodeLoopWithSetup(setup []byte, body ...byte) []byte {
	jdOffset := len(setup)
	code := make([]byte, 0, len(setup)+1+len(body)+3)
	code = append(code, setup...)
	code = append(code, byte(vm.JUMPDEST))
	code = append(code, body...)
	code = append(code, byte(vm.PUSH1), byte(jdOffset), byte(vm.JUMP))
	return code
}

// solidityMappingSlot computes keccak256(abi.encode(key, slot)) for a Solidity mapping.
func solidityMappingSlot(key common.Address, slot uint64) common.Hash {
	var buf [64]byte
	copy(buf[12:32], key.Bytes())
	buf[63] = byte(slot)
	return common.Hash(crypto.Keccak256(buf[:]))
}

// --- Precompile benchmarks ---
// These call the precompile contracts directly (no EVM overhead).
// Benchmark names match GEVM's tests/bench/ for benchstat comparison.

func benchmarkPrecompile(b *testing.B, addr common.Address, input []byte) {
	p := vm.PrecompiledContractsPrague[addr]
	if p == nil {
		b.Fatalf("precompile not found at %s", addr.Hex())
	}
	reqGas := p.RequiredGas(input)
	data := make([]byte, len(input))
	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		copy(data, input)
		p.Run(data)
	}
	_ = reqGas
}

var ecrecoverInput = common.FromHex(
	"456e9aea5e197a1f1af7a3e85a3212fa4049a3ba34c2289b4c860fc0b0c64ef3" +
		"000000000000000000000000000000000000000000000000000000000000001c" +
		"9242685bf161793cc25603c231bc2f568eb630ea16aa137d2664ac8038825608" +
		"4f8ae3bd7535248d0bd448298cc2e2071e56992d0774dc340c368ae950852ada")

func BenchmarkPrecompileEcrecover(b *testing.B) {
	benchmarkPrecompile(b, common.BytesToAddress([]byte{0x01}), ecrecoverInput)
}

func BenchmarkPrecompileSha256(b *testing.B) {
	benchmarkPrecompile(b, common.BytesToAddress([]byte{0x02}), make([]byte, 128))
}

func BenchmarkPrecompileRipemd160(b *testing.B) {
	benchmarkPrecompile(b, common.BytesToAddress([]byte{0x03}), make([]byte, 128))
}

func BenchmarkPrecompileIdentity(b *testing.B) {
	b.Run("128B", func(b *testing.B) {
		benchmarkPrecompile(b, common.BytesToAddress([]byte{0x04}), make([]byte, 128))
	})
	b.Run("1KB", func(b *testing.B) {
		benchmarkPrecompile(b, common.BytesToAddress([]byte{0x04}), make([]byte, 1024))
	})
}

var modexpInput = common.FromHex(
	"0000000000000000000000000000000000000000000000000000000000000001" +
		"0000000000000000000000000000000000000000000000000000000000000003" +
		"0000000000000000000000000000000000000000000000000000000000000020" +
		"02" +
		"010001" +
		"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")

func BenchmarkPrecompileModexp(b *testing.B) {
	benchmarkPrecompile(b, common.BytesToAddress([]byte{0x05}), modexpInput)
}

var bn254G1Gen = common.FromHex(
	"0000000000000000000000000000000000000000000000000000000000000001" +
		"0000000000000000000000000000000000000000000000000000000000000002")

func BenchmarkPrecompileBn254Add(b *testing.B) {
	input := make([]byte, 128)
	copy(input[0:64], bn254G1Gen)
	copy(input[64:128], bn254G1Gen)
	benchmarkPrecompile(b, common.BytesToAddress([]byte{0x06}), input)
}

func BenchmarkPrecompileBn254Mul(b *testing.B) {
	input := make([]byte, 96)
	copy(input[0:64], bn254G1Gen)
	input[95] = 7
	benchmarkPrecompile(b, common.BytesToAddress([]byte{0x07}), input)
}

func BenchmarkPrecompileBn254Pairing(b *testing.B) {
	g2Gen := common.FromHex(
		"198e9393920d483a7260bfb731fb5d25f1aa493335a9e71297e485b7aef312c2" +
			"1800deef121f1e76426a00665e5c4479674322d4f75edadd46debd5cd992f6ed" +
			"090689d0585ff075ec9e99ad690c3395bc4b313370b38ef355acdadcd122975b" +
			"12c85ea5db8c6deb4aab71808dcb408fe3d1e7690c43d37b4ce6cc0166fa7daa")
	input := make([]byte, 192)
	copy(input[0:64], bn254G1Gen)
	copy(input[64:192], g2Gen)
	benchmarkPrecompile(b, common.BytesToAddress([]byte{0x08}), input)
}

var blake2fInput = common.FromHex(
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
	benchmarkPrecompile(b, common.BytesToAddress([]byte{0x09}), blake2fInput)
}

// BLS12-381 precompile benchmarks (Prague+).
// Use gnark-crypto generators for valid test inputs.

// BLS12-381 generator points in EVM encoding (gnark-crypto generators).
var blsG1GenEncoded = common.FromHex(
	"0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb" +
		"0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1")
var blsG2GenEncoded = common.FromHex(
	"00000000000000000000000000000000024aa2b2f08f0a91260805272dc51051c6e47ad4fa403b02b4510b647ae3d1770bac0326a805bbefd48056c8c121bdb8" +
		"0000000000000000000000000000000013e02b6052719f607dacd3a088274f65596bd0d09920b61ab5da61bbdc7f5049334cf11213945d57e5ac7d055d042b7e" +
		"000000000000000000000000000000000ce5d527727d6e118cc9cdc6da2e351aadfd9baa8cbdd3a76d429a695160d12c923ac9cc3baca289e193548608b82801" +
		"000000000000000000000000000000000606c4a02ea734cc32acd2b02bc28b99cb3e287e85a763af267492ab572e99ab3f370d275cec1da1aaa9075ff05f79be")

func BenchmarkPrecompileBlsG1Add(b *testing.B) {
	input := make([]byte, 256)
	copy(input[0:128], blsG1GenEncoded[:])
	copy(input[128:256], blsG1GenEncoded[:])
	benchmarkPrecompile(b, common.BytesToAddress([]byte{0x0B}), input)
}

func BenchmarkPrecompileBlsG1Msm(b *testing.B) {
	input := make([]byte, 160)
	copy(input[0:128], blsG1GenEncoded[:])
	input[159] = 7
	benchmarkPrecompile(b, common.BytesToAddress([]byte{0x0C}), input)
}

func BenchmarkPrecompileBlsG2Add(b *testing.B) {
	input := make([]byte, 512)
	copy(input[0:256], blsG2GenEncoded[:])
	copy(input[256:512], blsG2GenEncoded[:])
	benchmarkPrecompile(b, common.BytesToAddress([]byte{0x0D}), input)
}

func BenchmarkPrecompileBlsG2Msm(b *testing.B) {
	input := make([]byte, 288)
	copy(input[0:256], blsG2GenEncoded[:])
	input[287] = 7
	benchmarkPrecompile(b, common.BytesToAddress([]byte{0x0E}), input)
}

func BenchmarkPrecompileBlsPairing(b *testing.B) {
	input := make([]byte, 384)
	copy(input[0:128], blsG1GenEncoded[:])
	copy(input[128:384], blsG2GenEncoded[:])
	benchmarkPrecompile(b, common.BytesToAddress([]byte{0x0F}), input)
}

func BenchmarkPrecompileBlsMapFpToG1(b *testing.B) {
	input := make([]byte, 64)
	input[63] = 1
	benchmarkPrecompile(b, common.BytesToAddress([]byte{0x10}), input)
}

func BenchmarkPrecompileBlsMapFp2ToG2(b *testing.B) {
	input := make([]byte, 128)
	input[63] = 1
	input[127] = 1
	benchmarkPrecompile(b, common.BytesToAddress([]byte{0x11}), input)
}

var p256Input = common.FromHex(
	"4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4d" +
		"a73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac" +
		"36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d60" +
		"4aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff3" +
		"7618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e")

func BenchmarkPrecompileP256Verify(b *testing.B) {
	// P256VERIFY is at address 0x0100 in Osaka
	p := vm.PrecompiledContractsOsaka[common.BytesToAddress([]byte{0x01, 0x00})]
	if p == nil {
		b.Fatal("P256VERIFY precompile not found at 0x0100")
	}
	data := make([]byte, len(p256Input))
	b.ReportAllocs()
	b.ResetTimer()
	for b.Loop() {
		copy(data, p256Input)
		p.Run(data)
	}
}
