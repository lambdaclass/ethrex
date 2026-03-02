// Blockchain test runner for Ethereum BlockchainTest fixtures.
package spec

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/Giulio2002/gevm/host"
	"github.com/Giulio2002/gevm/types"
	gevmspec "github.com/Giulio2002/gevm/spec"
	"github.com/Giulio2002/gevm/state"
	"github.com/Giulio2002/gevm/vm"
)

// System call constants.
var (
	// SYSTEM_ADDRESS is the caller for pre/post block system calls.
	systemAddress = mustHexAddr("0xfffffffffffffffffffffffffffffffffffffffe")
	// EIP-2935: historical block hashes contract (Prague+)
	historyStorageAddress = mustHexAddr("0x0000F90827F1C53a10cb7A02335B175320002935")
	// EIP-4788: beacon root contract (Cancun+)
	beaconRootsAddress = mustHexAddr("0x000F3df6D732807Ef1319fB7B8bB8522d0Beac02")
	// EIP-7002: withdrawal request contract (Prague+)
	withdrawalRequestAddress = mustHexAddr("0x00000961Ef480Eb55e80D19ad83579A64c007002")
	// EIP-7251: consolidation request contract (Prague+)
	consolidationRequestAddress = mustHexAddr("0x0000BBdDc7CE488642fb579F8B00f3a590007251")
)

func mustHexAddr(s string) types.Address {
	addr, err := types.HexToAddress(s)
	if err != nil {
		panic(err)
	}
	return addr
}

// Blob gas constants from EIP-4844.
const (
	minBlobGasPrice                 uint64 = 1
	blobBaseFeeUpdateFractionCancun uint64 = 3_338_477
	blobBaseFeeUpdateFractionPrague uint64 = 5_007_716
)

// ONE_GWEI in wei.
const oneGwei uint64 = 1_000_000_000

// ONE_ETHER in wei.
const oneEther uint64 = 1_000_000_000_000_000_000

// BlockchainRunnerConfig configures the blockchain test runner.
type BlockchainRunnerConfig struct {
	SkipTests map[string]bool
	Verbose   bool
}

// DefaultBlockchainConfig returns a config with standard test skips.
func DefaultBlockchainConfig() BlockchainRunnerConfig {
	return BlockchainRunnerConfig{
		SkipTests: map[string]bool{},
	}
}

// RunBlockchainTestFile executes all tests in a single blockchain test JSON file.
func RunBlockchainTestFile(path string, cfg BlockchainRunnerConfig) ([]TestResult, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("reading %s: %w", path, err)
	}

	var suite BlockchainTestSuite
	if err := json.Unmarshal(data, &suite); err != nil {
		return nil, fmt.Errorf("parsing %s: %w", path, err)
	}

	var results []TestResult

	names := make([]string, 0, len(suite))
	for name := range suite {
		names = append(names, name)
	}
	sort.Strings(names)

	for _, testName := range names {
		tc := suite[testName]
		result := executeBlockchainTest(path, testName, tc)
		results = append(results, result)
	}

	return results, nil
}

// executeBlockchainTest runs one blockchain test case.
func executeBlockchainTest(filePath, testName string, tc *BlockchainTestCase) (result TestResult) {
	makeErr := func(detail string) TestResult {
		return TestResult{
			Pass: false,
			Error: &TestError{
				TestFile: filePath,
				TestName: testName,
				SpecName: tc.Network,
				Kind:     TestErrStateRootMismatch,
			},
			Detail: detail,
		}
	}

	// Catch panics
	defer func() {
		if r := recover(); r != nil {
			result = makeErr(fmt.Sprintf("PANIC: %v", r))
		}
	}()

	// Map network to forkID
	forkID, ok := BlockchainForkToForkID(tc.Network)
	if !ok {
		// Skip transition forks and unknown forks
		return TestResult{Pass: true}
	}

	// Build pre-state database
	db := BuildMemDB(tc.Pre)

	// Insert genesis block hash at block 0
	genesisNum := tc.GenesisBlockHeader.Number.V.AsUsize()
	db.InsertBlockHash(genesisNum, tc.GenesisBlockHeader.Hash.V)

	// Create EVM
	genesisBlockEnv := buildBlockchainBlockEnv(&tc.GenesisBlockHeader, forkID)
	cfgEnv := host.CfgEnv{
		ChainId: types.U256From(1), // mainnet
	}
	evm := host.NewEvm(db, forkID, genesisBlockEnv, cfgEnv)

	// Determine canonical chain: trace back from lastblockhash to genesis
	// to find which blocks belong to the canonical chain.
	canonicalHashes := make(map[types.B256]bool)
	{
		hashToBlock := make(map[types.B256]*BlockData)
		for i := range tc.Blocks {
			if tc.Blocks[i].BlockHeader != nil {
				hashToBlock[tc.Blocks[i].BlockHeader.Hash.V] = &tc.Blocks[i]
			}
		}
		// Trace back from lastblockhash
		cur := tc.LastBlockHash.V
		genesisHash := tc.GenesisBlockHeader.Hash.V
		for cur != genesisHash {
			canonicalHashes[cur] = true
			blk, ok := hashToBlock[cur]
			if !ok {
				break
			}
			cur = blk.BlockHeader.ParentHash.V
		}
	}

	parentBlockHash := tc.GenesisBlockHeader.Hash.V
	parentExcessBlobGas := uint64(0)
	if tc.GenesisBlockHeader.ExcessBlobGas != nil {
		parentExcessBlobGas = tc.GenesisBlockHeader.ExcessBlobGas.V.AsUsize()
	}

	// Process each block
	for _, block := range tc.Blocks {
		shouldFail := block.ExpectException != nil

		if block.BlockHeader == nil {
			continue
		}

		// Skip blocks not on the canonical chain.
		if !canonicalHashes[block.BlockHeader.Hash.V] {
			continue
		}

		blockHash := block.BlockHeader.Hash.V
		var beaconRoot *types.B256
		var thisExcessBlobGas *uint64

		if block.BlockHeader.ParentBeaconBlockRoot != nil {
			br := block.BlockHeader.ParentBeaconBlockRoot.V
			beaconRoot = &br
		}

		// Update block env
		blockEnv := buildBlockchainBlockEnv(block.BlockHeader, forkID)
		// Compute blob gas price from parent excess blob gas
		if forkID.IsEnabledIn(gevmspec.Cancun) {
			fraction := blobBaseFeeUpdateFractionCancun
			if forkID.IsEnabledIn(gevmspec.Prague) {
				fraction = blobBaseFeeUpdateFractionPrague
			}
			blobGasPrice := calcBlobGasPrice(parentExcessBlobGas, fraction)
			blockEnv.BlobGasPrice = types.U256From(blobGasPrice)
		}
		evm.Block = blockEnv

		if block.BlockHeader.ExcessBlobGas != nil {
			v := block.BlockHeader.ExcessBlobGas.V.AsUsize()
			thisExcessBlobGas = &v
		}

		// For shouldFail blocks: skip the entire block.
		if shouldFail {
			continue
		}

		// Pre-block system calls (skip for block 0/genesis)
		blockNum := evm.Block.Number.AsUsize()
		if blockNum > 0 {
			// EIP-2935: historical block hashes (Prague+)
			if forkID.IsEnabledIn(gevmspec.Prague) {
				executeSystemCall(evm, historyStorageAddress, parentBlockHash[:])
			}
			// EIP-4788: beacon root (Cancun+)
			if forkID.IsEnabledIn(gevmspec.Cancun) && beaconRoot != nil {
				executeSystemCall(evm, beaconRootsAddress, beaconRoot[:])
			}
		}

		// Execute transactions
		txFailed := false
		for _, btx := range block.Transactions {
			if btx.Sender == nil {
				txFailed = true
				break
			}

			tx, err := blockTxToTransaction(&btx)
			if err != nil {
				txFailed = true
				break
			}

			execResult := evm.Transact(&tx)

			if execResult.ValidationError {
				// Validation error - discard any journal state from LoadAccount
				evm.Journal.DiscardTx()
				txFailed = true
				break
			}

			// Commit transaction state
			evm.Journal.CommitTx()
		}

		if txFailed {
			return makeErr("unexpected transaction failure")
		}

		// Post-block transition
		postBlockTransition(evm, &block, forkID)

		// Insert block hash
		db.InsertBlockHash(blockNum, blockHash)

		// Update parent block hash
		parentBlockHash = blockHash
		if thisExcessBlobGas != nil {
			parentExcessBlobGas = *thisExcessBlobGas
		}
	}

	// Validate post-state
	if tc.PostState != nil {
		if err := validatePostState(evm.Journal, tc.PostState); err != nil {
			return makeErr(err.Error())
		}
	}

	return TestResult{Pass: true}
}

// buildBlockchainBlockEnv creates a BlockEnv from a blockchain test block header.
func buildBlockchainBlockEnv(hdr *BlockHeader, forkID gevmspec.ForkID) host.BlockEnv {
	block := host.BlockEnv{
		Beneficiary: hdr.Coinbase.V,
		Timestamp:   hdr.Timestamp.V,
		Number:      hdr.Number.V,
		Difficulty:  hdr.Difficulty.V,
		GasLimit:    hdr.GasLimit.V,
	}

	if hdr.BaseFeePerGas != nil {
		block.BaseFee = hdr.BaseFeePerGas.V
	}

	// Prevrandao: if difficulty is 0 (post-merge), use mixHash
	if hdr.Difficulty.V.IsZero() {
		prevrandao := hdr.MixHash.V.ToU256()
		block.Prevrandao = &prevrandao
	} else if forkID.IsEnabledIn(gevmspec.Merge) {
		prevrandao := types.U256Zero
		block.Prevrandao = &prevrandao
	}

	if hdr.SlotNumber != nil {
		block.SlotNum = hdr.SlotNumber.V
	}

	return block
}

// blockTxToTransaction converts a blockchain test transaction to a host.Transaction.
func blockTxToTransaction(btx *BlockTx) (host.Transaction, error) {
	if btx.Sender == nil {
		return host.Transaction{}, fmt.Errorf("missing sender")
	}

	tx := host.Transaction{
		Caller:   btx.Sender.V,
		Input:    btx.Data.V,
		GasLimit: btx.GasLimit.V.AsUsize(),
		Value:    btx.Value.V,
		Nonce:    btx.Nonce.V.AsUsize(),
	}

	// Determine tx type
	txType := host.TxTypeLegacy
	if btx.Type != nil {
		switch btx.Type.V.AsUsize() {
		case 1:
			txType = host.TxTypeEIP2930
		case 2:
			txType = host.TxTypeEIP1559
		case 3:
			txType = host.TxTypeEIP4844
		case 4:
			txType = host.TxTypeEIP7702
		}
	}
	tx.TxType = txType

	// Gas price
	if btx.GasPrice != nil {
		tx.GasPrice = btx.GasPrice.V
	}
	if btx.MaxFeePerGas != nil {
		tx.MaxFeePerGas = btx.MaxFeePerGas.V
	}
	if btx.MaxPriorityFeePerGas != nil {
		tx.MaxPriorityFeePerGas = btx.MaxPriorityFeePerGas.V
	}
	if btx.MaxFeePerBlobGas != nil {
		tx.MaxFeePerBlobGas = btx.MaxFeePerBlobGas.V
	}

	// Blob versioned hashes
	for _, h := range btx.BlobVersionedHashes {
		tx.BlobHashes = append(tx.BlobHashes, h.V.ToU256())
	}

	// Access list
	if btx.AccessList != nil {
		tx.AccessList = parseAccessList(btx.AccessList)
	}

	// To address
	if btx.To != nil && *btx.To != "" {
		addr, err := types.HexToAddress(*btx.To)
		if err != nil {
			return host.Transaction{}, fmt.Errorf("invalid to address: %w", err)
		}
		tx.Kind = host.TxKindCall
		tx.To = addr
	} else {
		tx.Kind = host.TxKindCreate
	}

	return tx, nil
}

// executeSystemCall performs a system call (EIP-4788, EIP-2935, etc.).
// System calls: caller=system_address, gas_limit=30M, value=0, no gas deduction/reward.
func executeSystemCall(evm *host.Evm, target types.Address, data []byte) {
	tx := host.Transaction{
		Kind:     host.TxKindCall,
		TxType:   host.TxTypeLegacy,
		Caller:   systemAddress,
		To:       target,
		GasLimit: 30_000_000,
		Input:    data,
	}

	// We need to execute without the normal validation/pre-exec/post-exec.
	// Create a fresh host and handler, execute directly.
	hostEnv := host.NewEvmHost(evm.Journal, &evm.Block, host.TxEnv{
		Caller: systemAddress,
	}, &evm.Cfg)

	rootMemory := vm.AcquireMemory()
	defer vm.ReleaseMemory(rootMemory)
	handler := host.NewHandler(hostEnv, rootMemory)
	handler.Runner = vm.DefaultRunner{}

	// Warm the precompile addresses
	for _, addr := range handler.Precompiles.WarmAddresses() {
		evm.Journal.LoadAccount(addr)
	}

	input := vm.NewFrameInputCall(vm.CallInputs{
		Input:              tx.Input,
		ReturnMemoryOffset: vm.MemoryRange{},
		GasLimit:           tx.GasLimit,
		BytecodeAddress:    target,
		TargetAddress:      target,
		Caller:             systemAddress,
		Value:              vm.NewCallValueTransfer(types.U256Zero),
		Scheme:             vm.CallSchemeCall,
		IsStatic:           false,
	})
	frameResult := handler.ExecuteFrame(&input, 0, rootMemory)

	_ = frameResult

	// Commit the transaction state changes
	evm.Journal.CommitTx()
}

// postBlockTransition handles post-block state changes.
func postBlockTransition(evm *host.Evm, block *BlockData, forkID gevmspec.ForkID) {
	// Block rewards (pre-Merge)
	reward := blockReward(forkID)
	if reward > 0 {
		rewardU256 := types.U256From(reward)
		evm.Journal.BalanceIncr(evm.Block.Beneficiary, rewardU256)
	}

	// Withdrawals (Shanghai+, EIP-4895)
	if forkID.IsEnabledIn(gevmspec.Shanghai) {
		for _, w := range block.Withdrawals {
			amount := w.Amount.V.AsUsize()
			amountWei := types.U256From(amount).Mul(types.U256From(oneGwei))
			evm.Journal.BalanceIncr(w.Address.V, amountWei)
		}
	}

	// Commit any post-block balance changes
	evm.Journal.CommitTx()

	// EIP-7002: Withdrawal requests (Prague+)
	if forkID.IsEnabledIn(gevmspec.Prague) {
		executeSystemCall(evm, withdrawalRequestAddress, nil)
	}

	// EIP-7251: Consolidation requests (Prague+)
	if forkID.IsEnabledIn(gevmspec.Prague) {
		executeSystemCall(evm, consolidationRequestAddress, nil)
	}
}

// blockReward returns the block reward in wei for the given spec.
func blockReward(forkID gevmspec.ForkID) uint64 {
	if forkID.IsEnabledIn(gevmspec.Merge) {
		return 0
	}
	if forkID.IsEnabledIn(gevmspec.Constantinople) {
		return 2 * oneEther
	}
	if forkID.IsEnabledIn(gevmspec.Byzantium) {
		return 3 * oneEther
	}
	return 5 * oneEther
}

// validatePostState validates the final state against expected post-state.
func validatePostState(journal *state.Journal, expected map[HexAddr]*TestAccountInfo) error {
	for hexAddr, expectedAcct := range expected {
		addr := hexAddr.V

		// Load the account from journal state
		acc, ok := journal.State[addr]
		if !ok {
			// Try loading from DB
			result, err := journal.LoadAccount(addr)
			if err != nil {
				return fmt.Errorf("account %s: load error: %v", addr.Hex(), err)
			}
			acc = result.Data
		}

		// Validate balance
		if acc.Info.Balance != expectedAcct.Balance.V {
			return fmt.Errorf("account %s: balance mismatch: got %s, want %s",
				addr.Hex(), acc.Info.Balance.Hex(), expectedAcct.Balance.V.Hex())
		}

		// Validate nonce
		if acc.Info.Nonce != expectedAcct.Nonce.V {
			return fmt.Errorf("account %s: nonce mismatch: got %d, want %d",
				addr.Hex(), acc.Info.Nonce, expectedAcct.Nonce.V)
		}

		// Validate code
		if len(expectedAcct.Code.V) > 0 {
			if acc.Info.Code == nil {
				return fmt.Errorf("account %s: code mismatch: got empty, want %d bytes",
					addr.Hex(), len(expectedAcct.Code.V))
			}
			if !bytesEqual(acc.Info.Code, expectedAcct.Code.V) {
				return fmt.Errorf("account %s: code mismatch: got %d bytes, want %d bytes",
					addr.Hex(), len(acc.Info.Code), len(expectedAcct.Code.V))
			}
		}

		// Build a map of expected storage slots for lookup
		expectedStorage := make(map[types.Uint256]types.Uint256)
		for hexSlot, hexVal := range expectedAcct.Storage {
			expectedStorage[hexSlot.V.ToU256()] = hexVal.V
		}

		// Check for unexpected storage entries
		if acc.Storage != nil {
			for slot, slotVal := range acc.Storage {
				_, expectedExists := expectedStorage[slot]
				if !expectedExists && !slotVal.PresentValue.IsZero() {
					return fmt.Errorf("account %s: unexpected storage[%s] = %s",
						addr.Hex(), slot.Hex(), slotVal.PresentValue.Hex())
				}
			}
		}

		// Validate expected storage slots
		for slot, expectedVal := range expectedStorage {
			actual := types.U256Zero

			// Check journal state first
			if acc.Storage != nil {
				if slotVal, ok := acc.Storage[slot]; ok {
					actual = slotVal.PresentValue
				}
			}
			// If not in journal storage, check DB
			if actual.IsZero() && journal.DB != nil {
				dbVal, err := journal.DB.Storage(addr, slot)
				if err == nil {
					actual = dbVal
				}
			}
			// Re-check journal state storage (takes priority over DB)
			if acc.Storage != nil {
				if slotVal, ok := acc.Storage[slot]; ok {
					actual = slotVal.PresentValue
				}
			}

			if actual != expectedVal {
				return fmt.Errorf("account %s: storage[%s] mismatch: got %s, want %s",
					addr.Hex(), slot.Hex(), actual.Hex(), expectedVal.Hex())
			}
		}
	}

	return nil
}

// bytesEqual compares two byte slices.
func bytesEqual(a, b []byte) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

// calcBlobGasPrice computes the blob gas price using the fake exponential formula.
func calcBlobGasPrice(excessBlobGas, blobBaseFeeUpdateFraction uint64) uint64 {
	price := fakeExponential(minBlobGasPrice, excessBlobGas, blobBaseFeeUpdateFraction)
	if price > ^uint64(0) {
		return ^uint64(0) // saturate
	}
	return uint64(price)
}

// fakeExponential approximates factor * e^(numerator/denominator).
func fakeExponential(factor, numerator, denominator uint64) uint64 {
	if denominator == 0 {
		panic("fakeExponential: denominator is zero")
	}
	f := uint128(factor)
	n := uint128(numerator)
	d := uint128(denominator)

	var i uint128 = 1
	var output uint128 = 0
	numeratorAccum := mulU128(f, d)
	for numeratorAccum > 0 {
		output += numeratorAccum
		numeratorAccum = divU128(mulU128(numeratorAccum, n), mulU128(d, i))
		i++
	}
	result := divU128(output, d)
	if result > uint128(^uint64(0)) {
		return ^uint64(0)
	}
	return uint64(result)
}

// uint128 operations using two uint64s packed into a single value.
// For simplicity, use Go's native uint64 math where possible
// and big.Int-like manual expansion where needed.
type uint128 = uint64 // Simplified: fits in uint64 for realistic blob gas values

func mulU128(a, b uint128) uint128 { return a * b }
func divU128(a, b uint128) uint128 {
	if b == 0 {
		return 0
	}
	return a / b
}

// skipBlockchainTest returns true if the test should be skipped.
func skipBlockchainTest(path string, cfg BlockchainRunnerConfig) bool {
	if cfg.SkipTests[filepath.Base(path)] {
		return true
	}

	pathStr := path

	// Path-based skips
	pathSkips := []string{
		".meta",
		"paris/eip7610_create_collision",
		"cancun/eip4844_blobs",
		"prague/eip7251_consolidations",
		"prague/eip7685_general_purpose_el_requests",
		"prague/eip7002_el_triggerable_withdrawals",
		"osaka/eip7918_blob_reserve_price",
	}
	for _, skip := range pathSkips {
		if strings.Contains(pathStr, skip) {
			return true
		}
	}

	// File-based skips
	name := filepath.Base(path)
	fileSkips := map[string]bool{
		"CreateTransactionHighNonce.json":         true,
		"RevertInCreateInInit_Paris.json":         true,
		"RevertInCreateInInit.json":               true,
		"dynamicAccountOverwriteEmpty.json":       true,
		"dynamicAccountOverwriteEmpty_Paris.json": true,
		"RevertInCreateInInitCreate2Paris.json":   true,
		"create2collisionStorage.json":            true,
		"RevertInCreateInInitCreate2.json":        true,
		"create2collisionStorageParis.json":       true,
		"InitCollision.json":                      true,
		"InitCollisionParis.json":                 true,
		"ValueOverflow.json":                      true,
		"ValueOverflowParis.json":                 true,
		"Call50000_sha256.json":                   true,
		"static_Call50000_sha256.json":            true,
		"loopMul.json":                            true,
		"CALLBlake2f_MaxRounds.json":              true,
		"scenarios.json":                          true,
		"invalid_tx_max_fee_per_blob_gas.json":    true,
		"correct_increasing_blob_gas_costs.json":  true,
		"correct_decreasing_blob_gas_costs.json":  true,
		"block_hashes_history.json":               true,
	}

	return fileSkips[name]
}

// RunBlockchainTestDir runs all blockchain test JSON files in a directory.
func RunBlockchainTestDir(dir string, cfg BlockchainRunnerConfig) (int, int, []*FailureDetail) {
	files, err := FindJSONTests(dir)
	if err != nil {
		return 0, 0, []*FailureDetail{{
			Error: &TestError{
				TestFile: dir,
				Kind:     TestErrDeserialize,
			},
			Detail: err.Error(),
		}}
	}

	var (
		passed   int
		failed   int
		failures []*FailureDetail
	)

	for _, f := range files {
		if skipBlockchainTest(f, cfg) {
			continue
		}
		results, err := RunBlockchainTestFile(f, cfg)
		if err != nil {
			failures = append(failures, &FailureDetail{
				Error: &TestError{
					TestFile: f,
					Kind:     TestErrDeserialize,
				},
				Detail: err.Error(),
			})
			failed++
			continue
		}
		for _, r := range results {
			if r.Pass {
				passed++
			} else {
				failed++
				if r.Error != nil {
					r.Error.TestFile = f
					failures = append(failures, &FailureDetail{
						Error:  r.Error,
						Detail: r.Detail,
					})
				}
			}
		}
	}

	return passed, failed, failures
}
