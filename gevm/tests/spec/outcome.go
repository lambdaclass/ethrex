// Structured test outcome for differential testing and JSON output.
// Provides detailed execution results in JSON format.
package spec

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"sort"

	"github.com/Giulio2002/gevm/host"
	"github.com/Giulio2002/gevm/types"
	gevmspec "github.com/Giulio2002/gevm/spec"
	"github.com/Giulio2002/gevm/vm"
)

// TestOutcome holds detailed execution results for one test case.
// JSON output format for differential comparison.
type TestOutcome struct {
	StateRoot string `json:"stateRoot"`
	LogsRoot  string `json:"logsRoot"`
	Output    string `json:"output"`
	GasUsed   uint64 `json:"gasUsed"`
	Pass      bool   `json:"pass"`
	ErrorMsg  string `json:"errorMsg,omitempty"`
	EvmResult string `json:"evmResult"`
	Fork      string `json:"fork"`
	Test      string `json:"test"`
	D         int    `json:"d"`
	G         int    `json:"g"`
	V         int    `json:"v"`
}

// RunTestFileOutcomes executes all tests in a JSON file and returns detailed outcomes.
func RunTestFileOutcomes(path string, cfg RunnerConfig) ([]TestOutcome, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("reading %s: %w", path, err)
	}

	var suite TestSuite
	if err := json.Unmarshal(data, &suite); err != nil {
		return nil, fmt.Errorf("parsing %s: %w", path, err)
	}

	var outcomes []TestOutcome

	// Sort test names for deterministic ordering
	names := make([]string, 0, len(suite))
	for name := range suite {
		names = append(names, name)
	}
	sort.Strings(names)

	for _, testName := range names {
		unit := suite[testName]

		specNames := make([]string, 0, len(unit.Post))
		for specName := range unit.Post {
			specNames = append(specNames, specName)
		}
		sort.Strings(specNames)

		for _, specName := range specNames {
			if specName == "Constantinople" || specName == "ByzantiumToConstantinopleAt5" {
				continue
			}

			forkID, ok := SpecNameToForkID(specName)
			if !ok {
				continue
			}

			if len(cfg.OnlySpecs) > 0 && !cfg.OnlySpecs[specName] {
				continue
			}

			cases := unit.Post[specName]
			for i, tc := range cases {
				outcome := executeTestOutcome(testName, specName, forkID, unit, &tc, i)
				outcomes = append(outcomes, outcome)
			}
		}
	}

	return outcomes, nil
}

// ExecuteTestOutcome runs a single test case and returns the detailed outcome.
// This is exported for use by the differential test harness.
func ExecuteTestOutcome(
	testName string,
	specName string,
	forkID gevmspec.ForkID,
	unit *TestUnit,
	tc *TestCase,
	index int,
) TestOutcome {
	return executeTestOutcome(testName, specName, forkID, unit, tc, index)
}

func executeTestOutcome(
	testName string,
	specName string,
	forkID gevmspec.ForkID,
	unit *TestUnit,
	tc *TestCase,
	index int,
) (outcome TestOutcome) {
	outcome.Fork = specName
	outcome.Test = testName
	outcome.D = tc.Indexes.Data
	outcome.G = tc.Indexes.Gas
	outcome.V = tc.Indexes.Value

	defer func() {
		if r := recover(); r != nil {
			outcome.Pass = false
			outcome.ErrorMsg = fmt.Sprintf("PANIC: %v", r)
			outcome.EvmResult = "Panic"
		}
	}()

	// Recover sender
	caller, err := RecoverAddress(unit.Transaction.SecretKey.V[:])
	if err != nil {
		if unit.Transaction.Sender != nil {
			caller = unit.Transaction.Sender.V
		} else {
			outcome.Pass = tc.ExpectException != nil
			outcome.ErrorMsg = "unknown private key"
			return
		}
	}

	// Build transaction
	tx, txErr := BuildTransaction(unit, tc, caller)
	if txErr != nil {
		outcome.Pass = tc.ExpectException != nil
		outcome.ErrorMsg = fmt.Sprintf("invalid tx: %v", txErr)
		return
	}

	// Build pre-state
	db := BuildMemDB(unit.Pre)
	blockEnv := BuildBlockEnv(unit, forkID)
	cfgEnv := host.CfgEnv{ChainId: unit.ChainId()}

	// Execute
	evm := host.NewEvm(db, forkID, blockEnv, cfgEnv)
	execResult := evm.Transact(&tx)

	// Capture results
	outcome.GasUsed = execResult.GasUsed
	outcome.Output = bytesToHex(execResult.Output)
	outcome.EvmResult = FormatEvmResult(&execResult)

	// Compute logs root
	logsRoot := LogsRoot(execResult.Logs)
	outcome.LogsRoot = logsRoot.Hex()

	// Validate
	if tc.ExpectException != nil {
		outcome.Pass = execResult.ValidationError
		if !outcome.Pass {
			outcome.ErrorMsg = fmt.Sprintf("expected exception %q but got %s",
				*tc.ExpectException, outcome.EvmResult)
		}
		return
	}

	if execResult.ValidationError {
		outcome.Pass = false
		outcome.ErrorMsg = fmt.Sprintf("unexpected validation error: %v", execResult.Reason)
		return
	}

	// Validate logs root
	if tc.Logs.V != types.B256Zero {
		if logsRoot != tc.Logs.V {
			outcome.Pass = false
			outcome.ErrorMsg = fmt.Sprintf("logs root mismatch: got %s, want %s",
				logsRoot.Hex(), tc.Logs.V.Hex())
			return
		}
	}

	// Validate post-state accounts
	expectedState := tc.State
	if expectedState == nil {
		expectedState = tc.PostState
	}
	if expectedState != nil {
		if err := validatePostState(evm.Journal, expectedState); err != nil {
			outcome.Pass = false
			outcome.ErrorMsg = err.Error()
			return
		}
	}

	outcome.Pass = true
	return
}

// FormatEvmResult returns a human-readable result string.
func FormatEvmResult(r *host.ExecutionResult) string {
	switch r.Kind {
	case host.ResultSuccess:
		return fmt.Sprintf("Success: %s", InstructionResultName(r.Reason))
	case host.ResultRevert:
		return "Revert"
	case host.ResultHalt:
		return fmt.Sprintf("Halt: %s", InstructionResultName(r.Reason))
	default:
		return "Unknown"
	}
}

// InstructionResultName returns a name for an InstructionResult.
func InstructionResultName(r vm.InstructionResult) string {
	switch r {
	case vm.InstructionResultStop:
		return "Stop"
	case vm.InstructionResultReturn:
		return "Return"
	case vm.InstructionResultRevert:
		return "Revert"
	case vm.InstructionResultSelfDestruct:
		return "SelfDestruct"
	case vm.InstructionResultOutOfGas:
		return "OutOfGas"
	case vm.InstructionResultOutOfFunds:
		return "OutOfFunds"
	case vm.InstructionResultStackUnderflow:
		return "StackUnderflow"
	case vm.InstructionResultStackOverflow:
		return "StackOverflow"
	case vm.InstructionResultOpcodeNotFound:
		return "OpcodeNotFound"
	case vm.InstructionResultInvalidFEOpcode:
		return "InvalidFEOpcode"
	case vm.InstructionResultInvalidJump:
		return "InvalidJump"
	case vm.InstructionResultCreateInitCodeSizeLimit:
		return "CreateInitCodeSizeLimit"
	case vm.InstructionResultFatalExternalError:
		return "FatalExternalError"
	case vm.InstructionResultInvalidTxType:
		return "InvalidTxType"
	case vm.InstructionResultGasPriceBelowBaseFee:
		return "GasPriceBelowBaseFee"
	case vm.InstructionResultPriorityFeeTooHigh:
		return "PriorityFeeTooHigh"
	case vm.InstructionResultBlobGasPriceTooHigh:
		return "BlobGasPriceTooHigh"
	case vm.InstructionResultEmptyBlobs:
		return "EmptyBlobs"
	case vm.InstructionResultTooManyBlobs:
		return "TooManyBlobs"
	case vm.InstructionResultInvalidBlobVersion:
		return "InvalidBlobVersion"
	case vm.InstructionResultCreateNotAllowed:
		return "CreateNotAllowed"
	case vm.InstructionResultEmptyAuthorizationList:
		return "EmptyAuthorizationList"
	case vm.InstructionResultGasLimitTooHigh:
		return "GasLimitTooHigh"
	case vm.InstructionResultSenderNotEOA:
		return "SenderNotEOA"
	case vm.InstructionResultNonceMismatch:
		return "NonceMismatch"
	default:
		return fmt.Sprintf("Unknown(%d)", int(r))
	}
}

func bytesToHex(b []byte) string {
	if len(b) == 0 {
		return "0x"
	}
	return "0x" + hex.EncodeToString(b)
}
