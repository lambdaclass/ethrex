// Transaction test runner for Ethereum TransactionTest fixtures.
// These tests validate transaction decoding, sender recovery, and intrinsic gas calculation.
package spec

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/Giulio2002/gevm/types"
	gevmspec "github.com/Giulio2002/gevm/spec"
)

// TxTestForkToForkID maps transaction test fork names to ForkID values.
func TxTestForkToForkID(name string) (gevmspec.ForkID, bool) {
	// TransactionTests use the same fork names as GeneralStateTests
	return SpecNameToForkID(name)
}

// RunTransactionTestFile executes all tests in a single transaction test JSON file.
func RunTransactionTestFile(path string, cfg RunnerConfig) ([]TestResult, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("reading %s: %w", path, err)
	}

	var suite TransactionTestSuite
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
		forkResults := executeTransactionTest(path, testName, tc, cfg)
		results = append(results, forkResults...)
	}

	return results, nil
}

// executeTransactionTest runs one transaction test case across all forks.
func executeTransactionTest(filePath, testName string, tc *TransactionTestCase, cfg RunnerConfig) []TestResult {
	var results []TestResult

	// Sort fork names for determinism
	forkNames := make([]string, 0, len(tc.Result))
	for name := range tc.Result {
		forkNames = append(forkNames, name)
	}
	sort.Strings(forkNames)

	for _, forkName := range forkNames {
		expected := tc.Result[forkName]

		forkID, ok := TxTestForkToForkID(forkName)
		if !ok {
			continue // Unknown fork, skip
		}

		// Filter by spec if configured
		if len(cfg.OnlySpecs) > 0 && !cfg.OnlySpecs[forkName] {
			continue
		}

		result := executeOneTransactionFork(filePath, testName, forkName, forkID, tc.TxBytes.V, expected)
		results = append(results, result)
	}

	return results
}

// executeOneTransactionFork runs one transaction test case for one fork.
func executeOneTransactionFork(
	filePath, testName, forkName string,
	forkID gevmspec.ForkID,
	txbytes []byte,
	expected *TxTestResult,
) (result TestResult) {
	makeErr := func(kind TestErrorKind, detail string) TestResult {
		return TestResult{
			Pass: false,
			Error: &TestError{
				TestFile: filePath,
				TestName: testName,
				SpecName: forkName,
				Kind:     kind,
			},
			Detail: detail,
		}
	}

	// Catch panics
	defer func() {
		if r := recover(); r != nil {
			result = makeErr(TestErrUnexpectedException, fmt.Sprintf("PANIC: %v", r))
		}
	}()

	expectsException := expected.Exception != nil

	// Step 1: Decode the transaction
	decodedTx, decodeErr := DecodeTx(txbytes)

	if decodeErr != nil {
		if expectsException {
			return TestResult{Pass: true}
		}
		return makeErr(TestErrDeserialize, fmt.Sprintf("decode failed (no exception expected): %v", decodeErr))
	}

	// Step 2: Fork-specific validation
	forkErr := validateTxForFork(decodedTx, forkID)
	if forkErr != nil {
		if expectsException {
			return TestResult{Pass: true}
		}
		return makeErr(TestErrUnexpectedException, fmt.Sprintf("fork validation failed: %v", forkErr))
	}

	// Step 3: Recover sender
	sender, recoverErr := RecoverSender(decodedTx)
	if recoverErr != nil {
		if expectsException {
			return TestResult{Pass: true}
		}
		return makeErr(TestErrUnexpectedException, fmt.Sprintf("sender recovery failed: %v", recoverErr))
	}

	// If exception was expected but we succeeded, that's a failure
	if expectsException {
		return makeErr(TestErrUnexpectedException,
			fmt.Sprintf("expected exception %q but decode+recovery succeeded", *expected.Exception))
	}

	// Step 4: Validate hash = Keccak256(txbytes)
	if expected.Hash != nil {
		expectedHash := strings.ToLower(*expected.Hash)
		actualHash := types.Keccak256(txbytes)
		actualHashHex := actualHash.Hex()
		if actualHashHex != expectedHash {
			return makeErr(TestErrStateRootMismatch,
				fmt.Sprintf("hash mismatch: got %s, want %s", actualHashHex, expectedHash))
		}
	}

	// Step 5: Validate sender
	if expected.Sender != nil {
		expectedSender := strings.ToLower(*expected.Sender)
		actualSender := sender.Hex()
		if actualSender != expectedSender {
			return makeErr(TestErrStateRootMismatch,
				fmt.Sprintf("sender mismatch: got %s, want %s", actualSender, expectedSender))
		}
	}

	// Step 6: Validate intrinsic gas
	if expected.IntrinsicGas != nil {
		expectedGasStr := *expected.IntrinsicGas
		expectedGasBytes, err := hexToBytes(expectedGasStr)
		if err != nil {
			return makeErr(TestErrDeserialize, fmt.Sprintf("invalid intrinsicGas hex: %v", err))
		}
		var expectedGas uint64
		for _, b := range expectedGasBytes {
			expectedGas = (expectedGas << 8) | uint64(b)
		}

		actualGas := calcTxIntrinsicGas(decodedTx, forkID)
		if actualGas != expectedGas {
			return makeErr(TestErrStateRootMismatch,
				fmt.Sprintf("intrinsicGas mismatch: got %d (0x%x), want %d (0x%x)",
					actualGas, actualGas, expectedGas, expectedGas))
		}
	}

	return TestResult{Pass: true}
}

// secp256k1 order N and N/2 for signature validation.
var (
	secp256k1N = types.U256FromBytes([]byte{
		0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
		0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe,
		0xba, 0xae, 0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b,
		0xbf, 0xd2, 0x5e, 0x8c, 0xd0, 0x36, 0x41, 0x41,
	})
	secp256k1HalfN = types.U256FromBytes([]byte{
		0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
		0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
		0x5d, 0x57, 0x6e, 0x73, 0x57, 0xa4, 0x50, 0x1d,
		0xdf, 0xe9, 0x2f, 0x46, 0x68, 0x1b, 0x20, 0xa0,
	})
)

// validateTxForFork checks fork-specific validity of a decoded transaction.
// This performs all validation checks that TransactionTests expect.
func validateTxForFork(tx *DecodedTx, forkID gevmspec.ForkID) error {
	// --- Tx type support per fork ---
	switch tx.TxType {
	case 1:
		if !forkID.IsEnabledIn(gevmspec.Berlin) {
			return fmt.Errorf("EIP-2930 tx not supported before Berlin")
		}
	case 2:
		if !forkID.IsEnabledIn(gevmspec.London) {
			return fmt.Errorf("EIP-1559 tx not supported before London")
		}
	case 3:
		if !forkID.IsEnabledIn(gevmspec.Cancun) {
			return fmt.Errorf("EIP-4844 tx not supported before Cancun")
		}
	}

	// --- Chain ID validation ---
	if tx.TxType == 0 && tx.ChainId != nil {
		// EIP-155 legacy tx: chain ID must match expected (1 for mainnet test fixtures)
		if *tx.ChainId != 1 {
			return fmt.Errorf("invalid chain ID: %d (expected 1)", *tx.ChainId)
		}
	}
	if tx.TxType > 0 && tx.ChainId != nil {
		// Typed txs always have chain ID, must match expected chain
		if *tx.ChainId != 1 {
			return fmt.Errorf("invalid chain ID: %d (expected 1)", *tx.ChainId)
		}
	}

	// --- Signature validation ---
	// R and S must be non-zero and < secp256k1 order N
	if tx.R.IsZero() {
		return fmt.Errorf("R is zero")
	}
	if tx.S.IsZero() {
		return fmt.Errorf("S is zero")
	}
	if !tx.R.Lt(secp256k1N) {
		return fmt.Errorf("R >= secp256k1 order")
	}
	if !tx.S.Lt(secp256k1N) {
		return fmt.Errorf("S >= secp256k1 order")
	}

	// EIP-2: s must be <= secp256k1n/2 (Homestead+)
	if forkID.IsEnabledIn(gevmspec.Homestead) {
		if tx.S.Gt(secp256k1HalfN) {
			return fmt.Errorf("EIP-2: s > secp256k1n/2")
		}
	}

	// V validation for typed transactions (must be 0 or 1)
	if tx.TxType > 0 {
		vU64 := tx.V.AsUsize()
		if vU64 > 1 {
			return fmt.Errorf("invalid yParity: %d", vU64)
		}
	}

	// V validation for legacy transactions
	if tx.TxType == 0 {
		vU64 := tx.V.AsUsize()
		if vU64 != 27 && vU64 != 28 {
			// EIP-155: V = 2*chainId + 35 + {0,1}
			if vU64 < 35 {
				return fmt.Errorf("invalid legacy V: %d", vU64)
			}
			// EIP-155 replay protection is only valid from Spurious Dragon (EIP-158) onwards
			if !forkID.IsEnabledIn(gevmspec.SpuriousDragon) {
				return fmt.Errorf("EIP-155 tx not valid before Spurious Dragon (V=%d)", vU64)
			}
			// Verify chain ID derived from V matches expected chain ID (1 for mainnet tests)
			chainIdFromV := (vU64 - 35) / 2
			if chainIdFromV != 1 {
				return fmt.Errorf("EIP-155 chain ID mismatch: V implies %d, expected 1", chainIdFromV)
			}
		}
	}

	// --- EIP-1559: priority fee must not exceed max fee ---
	if tx.TxType == 2 || tx.TxType == 3 {
		if tx.MaxPriorityFeePerGas.Gt(tx.MaxFeePerGas) {
			return fmt.Errorf("maxPriorityFeePerGas > maxFeePerGas")
		}
	}

	// --- Gas limit * gas price overflow check ---
	if tx.GasLimit > 0 {
		var gasPrice types.Uint256
		switch tx.TxType {
		case 0, 1:
			gasPrice = tx.GasPrice
		case 2, 3:
			gasPrice = tx.MaxFeePerGas
		}
		if !gasPrice.IsZero() {
			gasLimitU := types.U256From(tx.GasLimit)
			product := gasPrice.Mul(gasLimitU)
			// Check for overflow using division: product / gasLimit should equal gasPrice
			quotient := product.Div(gasLimitU)
			if quotient != gasPrice {
				return fmt.Errorf("gasLimit * gasPrice overflow")
			}
		}
	}

	// --- Intrinsic gas check ---
	intrinsicGas := calcTxIntrinsicGas(tx, forkID)
	if tx.GasLimit < intrinsicGas {
		return fmt.Errorf("gasLimit %d < intrinsicGas %d", tx.GasLimit, intrinsicGas)
	}

	// --- EIP-7623 floor gas check (Prague+) ---
	if forkID.IsEnabledIn(gevmspec.Prague) {
		floorGas := calcTxFloorGas(tx, forkID)
		if floorGas > 0 && tx.GasLimit < floorGas {
			return fmt.Errorf("gasLimit %d < floorGas %d (EIP-7623)", tx.GasLimit, floorGas)
		}
	}

	// --- Nonce too big: nonce must be < 2^64-1 ---
	// nonce = uint64_max is rejected (NONCE_TOO_BIG)
	// nonce = uint64_max-1 is valid
	if tx.Nonce == ^uint64(0) {
		return fmt.Errorf("nonce too big: %d", tx.Nonce)
	}

	// --- EIP-3860: initcode size limit (Shanghai+) ---
	if forkID.IsEnabledIn(gevmspec.Shanghai) && tx.To == nil {
		if uint64(len(tx.Data)) > 2*24576 { // MaxInitCodeSize = 2 * MaxCodeSize(24576)
			return fmt.Errorf("initcode size %d exceeds max %d", len(tx.Data), 2*24576)
		}
	}

	// --- EIP-4844 specific validation ---
	if tx.TxType == 3 {
		// Must have at least one blob hash
		if len(tx.BlobHashes) == 0 {
			return fmt.Errorf("EIP-4844: no blob hashes")
		}
		// Blob tx cannot be CREATE
		if tx.To == nil {
			return fmt.Errorf("EIP-4844: cannot create")
		}
	}

	return nil
}

// Gas constants for intrinsic gas calculation (mirroring host/evm.go).
const (
	txTestBaseGas           uint64 = 21000
	txTestCreateGas         uint64 = 32000
	txTestDataZeroGas       uint64 = 4
	txTestDataNonZeroGas    uint64 = 16
	txTestDataNonZeroGasOld uint64 = 68
	txTestInitcodeWordGas   uint64 = 2
	txTestAccessListAddr    uint64 = 2400
	txTestAccessListStorage uint64 = 1900
)

// calcTxIntrinsicGas calculates the intrinsic gas for a decoded transaction.
// Mirrors host.Evm.calcIntrinsicGas but works on DecodedTx.
func calcTxIntrinsicGas(tx *DecodedTx, forkID gevmspec.ForkID) uint64 {
	gas := txTestBaseGas

	// Calldata cost
	for _, b := range tx.Data {
		if b == 0 {
			gas += txTestDataZeroGas
		} else {
			if forkID.IsEnabledIn(gevmspec.Istanbul) {
				gas += txTestDataNonZeroGas
			} else {
				gas += txTestDataNonZeroGasOld
			}
		}
	}

	// CREATE cost (Homestead+ only; Frontier had no CREATE bonus)
	isCreate := tx.To == nil
	if isCreate && forkID.IsEnabledIn(gevmspec.Homestead) {
		gas += txTestCreateGas

		// EIP-3860: initcode word cost (Shanghai+)
		if forkID.IsEnabledIn(gevmspec.Shanghai) {
			words := (uint64(len(tx.Data)) + 31) / 32
			gas += words * txTestInitcodeWordGas
		}
	}

	// Access list cost (EIP-2930+)
	for _, entry := range tx.AccessList {
		_ = entry.Address
		gas += txTestAccessListAddr
		gas += uint64(len(entry.StorageKeys)) * txTestAccessListStorage
	}

	return gas
}

// EIP-7623 floor gas constants.
const txTestFloorCostPerToken uint64 = 10

// calcTxFloorGas calculates the EIP-7623 floor gas for a decoded transaction.
func calcTxFloorGas(tx *DecodedTx, forkID gevmspec.ForkID) uint64 {
	if !forkID.IsEnabledIn(gevmspec.Prague) {
		return 0
	}
	// tokens = zero_bytes + non_zero_bytes * multiplier
	var zeroBytes, nonZeroBytes uint64
	for _, b := range tx.Data {
		if b == 0 {
			zeroBytes++
		} else {
			nonZeroBytes++
		}
	}
	nonZeroMultiplier := uint64(4) // Istanbul+: 16/4
	tokens := zeroBytes + nonZeroBytes*nonZeroMultiplier
	return txTestFloorCostPerToken*tokens + txTestBaseGas
}

// skipTransactionTest returns true if the test file should be skipped.
func skipTransactionTest(path string, cfg RunnerConfig) bool {
	base := filepath.Base(path)
	if cfg.SkipTests[base] {
		return true
	}
	// Skip .meta files
	if strings.Contains(path, ".meta") {
		return true
	}
	return false
}

// RunTransactionTestDir runs all transaction test JSON files in a directory.
func RunTransactionTestDir(dir string, cfg RunnerConfig) (int, int, []*FailureDetail) {
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
		if skipTransactionTest(f, cfg) {
			continue
		}
		results, err := RunTransactionTestFile(f, cfg)
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
