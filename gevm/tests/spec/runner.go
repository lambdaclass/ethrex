// Test runner for Ethereum GeneralStateTest fixtures.
package spec

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/Giulio2002/gevm/host"
	keccak "github.com/Giulio2002/fastkeccak"
	"github.com/Giulio2002/gevm/types"
	gevmspec "github.com/Giulio2002/gevm/spec"
	"github.com/decred/dcrd/dcrec/secp256k1/v4"
)

// TestError describes a single test failure.
type TestError struct {
	TestFile string
	TestName string
	SpecName string
	Index    int
	Kind     TestErrorKind
}

func (e *TestError) Error() string {
	return fmt.Sprintf("%s/%s [%s #%d]: %s",
		filepath.Base(e.TestFile), e.TestName, e.SpecName, e.Index, e.Kind.Error())
}

// TestErrorKind enumerates failure types.
type TestErrorKind int

const (
	TestErrLogsRootMismatch TestErrorKind = iota
	TestErrStateRootMismatch
	TestErrUnknownPrivateKey
	TestErrUnexpectedException
	TestErrUnexpectedOutput
	TestErrDeserialize
	TestErrInvalidTxType
)

func (k TestErrorKind) Error() string {
	switch k {
	case TestErrLogsRootMismatch:
		return "logs root mismatch"
	case TestErrStateRootMismatch:
		return "state root mismatch"
	case TestErrUnknownPrivateKey:
		return "unknown private key"
	case TestErrUnexpectedException:
		return "unexpected exception"
	case TestErrUnexpectedOutput:
		return "unexpected output"
	case TestErrDeserialize:
		return "deserialization error"
	case TestErrInvalidTxType:
		return "invalid transaction type"
	default:
		return "unknown error"
	}
}

// TestResult holds the outcome of executing one test case.
type TestResult struct {
	Pass  bool
	Error *TestError
	// Detail provides additional information about the failure.
	Detail string
}

// RunnerConfig configures the test runner.
type RunnerConfig struct {
	// SkipTests is a set of test file names to skip.
	SkipTests map[string]bool
	// OnlySpecs filters to only run these spec names. Empty means all.
	OnlySpecs map[string]bool
	// Verbose enables detailed output.
	Verbose bool
}

// DefaultConfig returns a RunnerConfig with standard test skips.
func DefaultConfig() RunnerConfig {
	return RunnerConfig{
		SkipTests: map[string]bool{
			// Skipped tests
			"CreateTransactionHighNonce.json": true,
			"ValueOverflow.json":              true,
		},
	}
}

// RunTestFile executes all tests in a single JSON test file.
// Returns a list of test results (one per test case).
func RunTestFile(path string, cfg RunnerConfig) ([]TestResult, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("reading %s: %w", path, err)
	}

	var suite TestSuite
	if err := json.Unmarshal(data, &suite); err != nil {
		return nil, fmt.Errorf("parsing %s: %w", path, err)
	}

	var results []TestResult

	// Sort test names for deterministic ordering
	names := make([]string, 0, len(suite))
	for name := range suite {
		names = append(names, name)
	}
	sort.Strings(names)

	for _, testName := range names {
		unit := suite[testName]

		// Sort spec names for deterministic ordering
		specNames := make([]string, 0, len(unit.Post))
		for specName := range unit.Post {
			specNames = append(specNames, specName)
		}
		sort.Strings(specNames)

		for _, specName := range specNames {
			// Skip Constantinople (uses Petersburg)
			if specName == "Constantinople" || specName == "ByzantiumToConstantinopleAt5" {
				continue
			}

			forkID, ok := SpecNameToForkID(specName)
			if !ok {
				continue // Unknown spec, skip
			}

			// Filter by spec if configured
			if len(cfg.OnlySpecs) > 0 && !cfg.OnlySpecs[specName] {
				continue
			}

			cases := unit.Post[specName]
			for i, tc := range cases {
				result := executeSingleTest(path, testName, specName, forkID, unit, &tc, i)
				results = append(results, result)
			}
		}
	}

	return results, nil
}

// executeSingleTest runs one test case and returns the result.
func executeSingleTest(
	filePath string,
	testName string,
	specName string,
	forkID gevmspec.ForkID,
	unit *TestUnit,
	tc *TestCase,
	index int,
) (result TestResult) {
	makeErr := func(kind TestErrorKind, detail string) TestResult {
		return TestResult{
			Pass: false,
			Error: &TestError{
				TestFile: filePath,
				TestName: testName,
				SpecName: specName,
				Index:    index,
				Kind:     kind,
			},
			Detail: detail,
		}
	}

	// Catch panics from EVM execution
	defer func() {
		if r := recover(); r != nil {
			result = makeErr(TestErrUnexpectedException, fmt.Sprintf("PANIC: %v", r))
		}
	}()

	// Recover sender address from private key
	caller, err := RecoverAddress(unit.Transaction.SecretKey.V[:])
	if err != nil {
		if unit.Transaction.Sender != nil {
			caller = unit.Transaction.Sender.V
		} else {
			if tc.ExpectException != nil {
				// Expected exception and can't recover address: pass
				return TestResult{Pass: true}
			}
			return makeErr(TestErrUnknownPrivateKey, err.Error())
		}
	}

	// Build transaction from template + indices
	tx, txErr := BuildTransaction(unit, tc, caller)
	if txErr != nil {
		if tc.ExpectException != nil {
			// Expected exception and invalid tx: pass
			return TestResult{Pass: true}
		}
		return makeErr(TestErrInvalidTxType, txErr.Error())
	}

	// Build pre-state database
	db := BuildMemDB(unit.Pre)

	// Build block environment
	blockEnv := BuildBlockEnv(unit, forkID)

	// Build config
	cfgEnv := host.CfgEnv{
		ChainId: unit.ChainId(),
	}

	// Create EVM and execute
	evm := host.NewEvm(db, forkID, blockEnv, cfgEnv)
	execResult := evm.Transact(&tx)

	// --- Validation ---

	// Check exception expectations
	if tc.ExpectException != nil {
		// Expected a transaction-level exception (validation failure).
		// The transaction should NOT succeed or execute normally.
		if !execResult.ValidationError {
			// We either succeeded or got an execution error (not validation).
			// For expectException, we want a validation failure.
			if execResult.IsSuccess() || !execResult.ValidationError {
				return makeErr(TestErrUnexpectedException,
					fmt.Sprintf("expected exception %q but got result kind=%d reason=%v validationErr=%v",
						*tc.ExpectException, execResult.Kind, execResult.Reason, execResult.ValidationError))
			}
		}
		// Got a validation error as expected: pass
		return TestResult{Pass: true}
	}

	// No exception expected.
	// A validation error means our validation is wrong (the tx should be valid).
	if execResult.ValidationError {
		return makeErr(TestErrUnexpectedException,
			fmt.Sprintf("unexpected validation error: %v", execResult.Reason))
	}

	// --- Differential validation ---

	// Validate logs root hash (skip if expected is zero, which is a placeholder)
	if tc.Logs.V != types.B256Zero {
		logsRoot := LogsRoot(execResult.Logs)
		if logsRoot != tc.Logs.V {
			return makeErr(TestErrLogsRootMismatch,
				fmt.Sprintf("logs root: got %s, want %s", logsRoot.Hex(), tc.Logs.V.Hex()))
		}
	}

	// Validate post-state accounts if available in fixture.
	expectedState := tc.State
	if expectedState == nil {
		expectedState = tc.PostState
	}
	if expectedState != nil {
		if err := validatePostState(evm.Journal, expectedState); err != nil {
			return makeErr(TestErrStateRootMismatch, err.Error())
		}
	}

	return TestResult{Pass: true}
}

// BuildTransaction constructs a Transaction from the test template and indices.
func BuildTransaction(unit *TestUnit, tc *TestCase, caller types.Address) (host.Transaction, error) {
	idx := tc.Indexes

	if idx.Data >= len(unit.Transaction.Data) {
		return host.Transaction{}, fmt.Errorf("data index %d out of range (len=%d)", idx.Data, len(unit.Transaction.Data))
	}
	if idx.Gas >= len(unit.Transaction.GasLimit) {
		return host.Transaction{}, fmt.Errorf("gas index %d out of range (len=%d)", idx.Gas, len(unit.Transaction.GasLimit))
	}
	if idx.Value >= len(unit.Transaction.Value) {
		return host.Transaction{}, fmt.Errorf("value index %d out of range (len=%d)", idx.Value, len(unit.Transaction.Value))
	}

	tx := host.Transaction{
		Caller:   caller,
		Input:    unit.Transaction.Data[idx.Data].V,
		GasLimit: unit.Transaction.GasLimit[idx.Gas].V.AsUsize(),
		Value:    unit.Transaction.Value[idx.Value].V,
		Nonce:    unit.Transaction.Nonce.V.AsUsize(),
	}

	// Determine transaction type
	txType := host.TxTypeLegacy
	if unit.Transaction.TxType != nil {
		switch unit.Transaction.TxType.V {
		case 1:
			txType = host.TxTypeEIP2930
		case 2:
			txType = host.TxTypeEIP1559
		case 3:
			txType = host.TxTypeEIP4844
		case 4:
			txType = host.TxTypeEIP7702
		}
	} else if len(unit.Transaction.AuthorizationList) > 0 {
		// Presence of authorizationList means EIP-7702
		txType = host.TxTypeEIP7702
	} else if unit.Transaction.MaxFeePerBlobGas != nil {
		// Presence of maxFeePerBlobGas means EIP-4844 blob transaction
		txType = host.TxTypeEIP4844
	} else if unit.Transaction.MaxFeePerGas != nil {
		// Infer EIP-1559 from presence of maxFeePerGas
		txType = host.TxTypeEIP1559
	} else if len(unit.Transaction.AccessLists) > 0 {
		txType = host.TxTypeEIP2930
	}
	tx.TxType = txType

	// Gas price fields
	switch txType {
	case TxTypeLegacy, TxTypeEIP2930:
		if unit.Transaction.GasPrice != nil {
			tx.GasPrice = unit.Transaction.GasPrice.V
		}
	case TxTypeEIP1559, TxTypeEIP4844, TxTypeEIP7702:
		if unit.Transaction.MaxFeePerGas != nil {
			tx.MaxFeePerGas = unit.Transaction.MaxFeePerGas.V
		}
		if unit.Transaction.MaxPriorityFeePerGas != nil {
			tx.MaxPriorityFeePerGas = unit.Transaction.MaxPriorityFeePerGas.V
		}
		// For EIP-1559+, also set GasPrice as a fallback for pre-London forks
		if unit.Transaction.GasPrice != nil {
			tx.GasPrice = unit.Transaction.GasPrice.V
		}
	}

	// EIP-4844: blob fields
	if txType == TxTypeEIP4844 {
		if unit.Transaction.MaxFeePerBlobGas != nil {
			tx.MaxFeePerBlobGas = unit.Transaction.MaxFeePerBlobGas.V
		}
		for _, h := range unit.Transaction.BlobVersionedHashes {
			tx.BlobHashes = append(tx.BlobHashes, h.V.ToU256())
		}
	}

	// Access list (parse for the specific data index)
	if len(unit.Transaction.AccessLists) > 0 {
		alIdx := idx.Data
		if alIdx >= len(unit.Transaction.AccessLists) {
			alIdx = len(unit.Transaction.AccessLists) - 1
		}
		if alIdx >= 0 && unit.Transaction.AccessLists[alIdx] != nil {
			tx.AccessList = parseAccessList(unit.Transaction.AccessLists[alIdx])
		}
	}

	// EIP-7702: authorization list
	if txType == TxTypeEIP7702 && len(unit.Transaction.AuthorizationList) > 0 {
		tx.AuthorizationList = parseAuthorizationList(unit.Transaction.AuthorizationList)
	}

	// To address
	toAddr, isCall := unit.Transaction.ParseTo()
	if isCall {
		tx.Kind = host.TxKindCall
		tx.To = toAddr
	} else {
		tx.Kind = host.TxKindCreate
	}

	return tx, nil
}

// TxType aliases for use in this package.
const (
	TxTypeLegacy  = host.TxTypeLegacy
	TxTypeEIP2930 = host.TxTypeEIP2930
	TxTypeEIP1559 = host.TxTypeEIP1559
	TxTypeEIP4844 = host.TxTypeEIP4844
	TxTypeEIP7702 = host.TxTypeEIP7702
)

// parseAccessList parses a JSON access list into AccessListItems.
func parseAccessList(raw json.RawMessage) []host.AccessListItem {
	if raw == nil {
		return nil
	}
	type alEntry struct {
		Address     HexAddr   `json:"address"`
		StorageKeys []HexB256 `json:"storageKeys"`
	}
	var entries []alEntry
	if err := json.Unmarshal(raw, &entries); err != nil {
		return nil
	}
	items := make([]host.AccessListItem, len(entries))
	for i, e := range entries {
		items[i].Address = e.Address.V
		for _, k := range e.StorageKeys {
			items[i].StorageKeys = append(items[i].StorageKeys, k.V.ToU256())
		}
	}
	return items
}

// parseAuthorizationList parses a JSON EIP-7702 authorization list.
func parseAuthorizationList(raw json.RawMessage) []host.Authorization {
	if raw == nil {
		return nil
	}
	var entries []TestAuthorization
	if err := json.Unmarshal(raw, &entries); err != nil {
		return nil
	}
	auths := make([]host.Authorization, len(entries))
	for i, e := range entries {
		auths[i].ChainId = e.ChainId.V
		auths[i].Address = e.Address.V
		auths[i].Nonce = e.Nonce.V.AsUsize()
		copy(auths[i].R[:], e.R.V[:])
		copy(auths[i].S[:], e.S.V[:])
		// Prefer yParity over v
		if e.YParity != nil {
			auths[i].YParity = uint8(e.YParity.V.AsUsize())
		} else {
			auths[i].YParity = uint8(e.V.V.AsUsize())
		}
	}
	return auths
}

// BuildBlockEnv constructs a BlockEnv from the test environment.
func BuildBlockEnv(unit *TestUnit, forkID gevmspec.ForkID) host.BlockEnv {
	env := unit.Env
	block := host.BlockEnv{
		Beneficiary: env.CurrentCoinbase.V,
		Timestamp:   env.CurrentTimestamp.V,
		Number:      env.CurrentNumber.V,
		Difficulty:  env.CurrentDifficulty.V,
		GasLimit:    env.CurrentGasLimit.V,
	}

	if env.CurrentBaseFee != nil {
		block.BaseFee = env.CurrentBaseFee.V
	}

	if env.CurrentRandom != nil {
		v := env.CurrentRandom.V.ToU256()
		block.Prevrandao = &v
	} else if forkID.IsEnabledIn(gevmspec.Merge) {
		v := types.U256Zero
		block.Prevrandao = &v
	}

	// EIP-4844: compute blob gas price from excess blob gas
	if env.CurrentExcessBlobGas != nil {
		excessBlobGas := env.CurrentExcessBlobGas.V.AsUsize()
		blobGasPrice := gevmspec.CalcBlobGasPrice(excessBlobGas, forkID)
		block.BlobGasPrice = types.U256From(blobGasPrice)
	}

	return block
}

// RecoverAddress derives an Ethereum address from a secp256k1 private key.
func RecoverAddress(privKeyBytes []byte) (types.Address, error) {
	privKey := secp256k1.PrivKeyFromBytes(privKeyBytes)
	pubKey := privKey.PubKey()

	// Serialize uncompressed: 0x04 || X(32) || Y(32)
	uncompressed := pubKey.SerializeUncompressed()

	// Keccak256 of the 64 bytes (without the 0x04 prefix)
	hash := keccak.Sum256(uncompressed[1:])

	// Address is the last 20 bytes
	var addr types.Address
	copy(addr[:], hash[12:])
	return addr, nil
}

// skipTest returns true if the test file should be skipped.
func skipTest(path string, cfg RunnerConfig) bool {
	base := filepath.Base(path)
	if cfg.SkipTests[base] {
		return true
	}
	return false
}

// FindJSONTests recursively finds all .json files in a directory.
func FindJSONTests(dir string) ([]string, error) {
	var files []string
	err := filepath.Walk(dir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		if !info.IsDir() && strings.HasSuffix(path, ".json") {
			files = append(files, path)
		}
		return nil
	})
	return files, err
}

// FailureDetail holds an error with its detail message.
type FailureDetail struct {
	Error  *TestError
	Detail string
}

// RunTestDir runs all test JSON files in a directory.
// Returns total pass count, total fail count, and list of failure details.
func RunTestDir(dir string, cfg RunnerConfig) (int, int, []*FailureDetail) {
	files, err := FindJSONTests(dir)
	if err != nil {
		return 0, 0, []*FailureDetail{{
			Error: &TestError{
				TestFile: dir,
				Kind:     TestErrDeserialize,
			},
		}}
	}

	var (
		passed   int
		failed   int
		failures []*FailureDetail
	)

	for _, f := range files {
		if skipTest(f, cfg) {
			continue
		}
		results, err := RunTestFile(f, cfg)
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
					r.Error.TestFile = f // ensure file is set
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
