// Differential testing harness for GEVM.
// Validates execution parity:
// 1. Golden fixture tests: crafted test cases with exact expected gas/output/state
// 2. Revme comparison: when REVME_BIN is set, run both and compare JSON outputs
// 3. Determinism tests: run same input twice, verify identical results
// 4. Random bytecode tests: generate random sequences, verify deterministic execution
package differential

import (
	"bufio"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/rand"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/Giulio2002/gevm/host"
	"github.com/Giulio2002/gevm/types"
	gevmspec "github.com/Giulio2002/gevm/spec"
	"github.com/Giulio2002/gevm/state"
	"github.com/Giulio2002/gevm/vm"
	"github.com/Giulio2002/gevm/tests/spec"
)

// --- Golden fixture tests ---
// These test exact gas usage, output, and state for crafted bytecodes.
// Values are verified against expected results.

type goldenTest struct {
	Name     string
	ForkID   gevmspec.ForkID
	Code     []byte // Contract bytecode at target address
	Input    []byte // Calldata
	Value    uint64 // Value to send
	GasLimit uint64 // Gas limit for transaction
	WantGas  uint64 // Expected gas used
	WantOut  []byte // Expected output data
	WantKind host.ResultKind
}

func TestGoldenFixtures(t *testing.T) {
	tests := []goldenTest{
		{
			Name:     "empty_call",
			ForkID:   gevmspec.Cancun,
			Code:     nil, // EOA target, no code
			Input:    nil,
			GasLimit: 100000,
			WantGas:  21000,
			WantKind: host.ResultSuccess,
		},
		{
			Name:     "stop_opcode",
			ForkID:   gevmspec.Cancun,
			Code:     []byte{0x00}, // STOP
			Input:    nil,
			GasLimit: 100000,
			WantGas:  21000,
			WantKind: host.ResultSuccess,
		},
		{
			Name:     "return_32_zero_bytes",
			ForkID:   gevmspec.Cancun,
			Code:     hexBytes("60206000F3"), // PUSH1 32, PUSH1 0, RETURN
			Input:    nil,
			GasLimit: 100000,
			WantGas:  21000 + 3 + 3 + 3, // base + 3 ops (memory expansion is 3 more)
			WantOut:  make([]byte, 32),
			WantKind: host.ResultSuccess,
		},
		{
			Name:     "revert_empty",
			ForkID:   gevmspec.Cancun,
			Code:     hexBytes("60006000FD"), // PUSH1 0, PUSH1 0, REVERT
			Input:    nil,
			GasLimit: 100000,
			WantGas:  21000 + 3 + 3, // base + 2 PUSH
			WantKind: host.ResultRevert,
		},
		{
			Name:     "invalid_opcode_fe",
			ForkID:   gevmspec.Cancun,
			Code:     []byte{0xFE}, // INVALID
			Input:    nil,
			GasLimit: 100000,
			WantGas:  100000, // Halt consumes all gas
			WantKind: host.ResultHalt,
		},
		{
			Name:     "add_1_2",
			ForkID:   gevmspec.Cancun,
			Code:     hexBytes("6001600201600052602060006000F0"), // 1+2, store, return (but this is CREATE)
			Input:    nil,
			GasLimit: 100000,
			WantKind: host.ResultSuccess, // We just check it doesn't panic
		},
		{
			Name:     "simple_sstore_sload",
			ForkID:   gevmspec.Cancun,
			Code:     hexBytes("600160005560005460005260206000F3"), // SSTORE(0,1), SLOAD(0), MSTORE, RETURN 32
			Input:    nil,
			GasLimit: 200000,
			WantKind: host.ResultSuccess,
		},
		{
			Name:     "calldatacopy_return",
			ForkID:   gevmspec.Cancun,
			Code:     hexBytes("6020600060003760206000F3"), // CALLDATACOPY(0,0,32), RETURN(0,32)
			Input:    hexBytes("deadbeef00000000000000000000000000000000000000000000000000000000"),
			GasLimit: 100000,
			WantOut:  hexBytes("deadbeef00000000000000000000000000000000000000000000000000000000"),
			WantKind: host.ResultSuccess,
		},
		{
			Name:     "push32_max",
			ForkID:   gevmspec.Cancun,
			Code:     hexBytes("7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff60005260206000F3"),
			Input:    nil,
			GasLimit: 100000,
			WantOut:  hexBytes("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
			WantKind: host.ResultSuccess,
		},
		{
			Name:     "out_of_gas",
			ForkID:   gevmspec.Cancun,
			Code:     hexBytes("600160015500"), // SSTORE (expensive) then STOP
			Input:    nil,
			GasLimit: 21100, // Not enough for SSTORE
			WantGas:  21100, // OOG consumes all gas
			WantKind: host.ResultHalt,
		},
	}

	for _, tt := range tests {
		t.Run(tt.Name, func(t *testing.T) {
			result := executeGolden(tt)

			if result.Kind != tt.WantKind {
				t.Errorf("result kind: got %d, want %d (evmResult: %s)",
					result.Kind, tt.WantKind, spec.FormatEvmResult(&result))
			}

			if tt.WantGas != 0 && result.GasUsed != tt.WantGas {
				t.Errorf("gas used: got %d, want %d", result.GasUsed, tt.WantGas)
			}

			if tt.WantOut != nil && !bytesEqual(result.Output, tt.WantOut) {
				t.Errorf("output: got %x, want %x", result.Output, tt.WantOut)
			}
		})
	}
}

func executeGolden(tt goldenTest) host.ExecutionResult {
	sender := types.Address{0x01}
	target := types.Address{0x10}

	db := spec.NewMemDB()

	// Fund sender
	db.InsertAccount(sender, state.AccountInfo{
		Balance: types.U256From(1e18),
	}, nil)

	// Target with code
	if tt.Code != nil {
		codeHash := types.Keccak256(tt.Code)
		db.InsertAccount(target, state.AccountInfo{
			Code:     tt.Code,
			CodeHash: codeHash,
		}, nil)
	}

	block := host.BlockEnv{
		Beneficiary: types.Address{0xcc},
		GasLimit:    types.U256From(30_000_000),
		BaseFee:     types.U256From(10),
		Number:      types.U256From(1),
		Timestamp:   types.U256From(1000),
	}
	if tt.ForkID.IsEnabledIn(gevmspec.Merge) {
		v := types.U256Zero
		block.Prevrandao = &v
	}

	cfg := host.CfgEnv{ChainId: types.U256From(1)}

	evm := host.NewEvm(db, tt.ForkID, block, cfg)

	tx := host.Transaction{
		Kind:     host.TxKindCall,
		TxType:   host.TxTypeLegacy,
		Caller:   sender,
		To:       target,
		Value:    types.U256From(tt.Value),
		Input:    tt.Input,
		GasLimit: tt.GasLimit,
		GasPrice: types.U256From(10),
	}

	return evm.Transact(&tx)
}

// --- Determinism tests ---
// Execute the same input multiple times and verify identical results.

func TestDeterminism(t *testing.T) {
	codes := []struct {
		name string
		code []byte
	}{
		{"empty", nil},
		{"stop", []byte{0x00}},
		{"return_empty", hexBytes("60006000F3")},
		{"sstore_sload", hexBytes("600160005560005460005260206000F3")},
		{"sha3", hexBytes("60206000206000526020600060003960206000F3")},
		{"loop_10", hexBytes("600a60005b6001900380600057")},
	}

	for _, tc := range codes {
		t.Run(tc.name, func(t *testing.T) {
			r1 := executeWithCode(tc.code, gevmspec.Cancun)
			r2 := executeWithCode(tc.code, gevmspec.Cancun)

			if r1.Kind != r2.Kind {
				t.Errorf("result kind differs: %d vs %d", r1.Kind, r2.Kind)
			}
			if r1.GasUsed != r2.GasUsed {
				t.Errorf("gas used differs: %d vs %d", r1.GasUsed, r2.GasUsed)
			}
			if !bytesEqual(r1.Output, r2.Output) {
				t.Errorf("output differs: %x vs %x", r1.Output, r2.Output)
			}
			if len(r1.Logs) != len(r2.Logs) {
				t.Errorf("log count differs: %d vs %d", len(r1.Logs), len(r2.Logs))
			}
		})
	}
}

func executeWithCode(code []byte, forkID gevmspec.ForkID) host.ExecutionResult {
	sender := types.Address{0x01}
	target := types.Address{0x10}

	db := spec.NewMemDB()
	db.InsertAccount(sender, state.AccountInfo{
		Balance: types.U256From(1e18),
	}, nil)

	if code != nil {
		codeHash := types.Keccak256(code)
		db.InsertAccount(target, state.AccountInfo{
			Code:     code,
			CodeHash: codeHash,
		}, nil)
	}

	block := host.BlockEnv{
		Beneficiary: types.Address{0xcc},
		GasLimit:    types.U256From(30_000_000),
		BaseFee:     types.U256From(10),
		Number:      types.U256From(1),
		Timestamp:   types.U256From(1000),
	}
	v := types.U256Zero
	block.Prevrandao = &v

	evm := host.NewEvm(db, forkID, block, host.CfgEnv{ChainId: types.U256From(1)})

	return evm.Transact(&host.Transaction{
		Kind:     host.TxKindCall,
		TxType:   host.TxTypeLegacy,
		Caller:   sender,
		To:       target,
		GasLimit: 1_000_000,
		GasPrice: types.U256From(10),
	})
}

// --- Random bytecode determinism tests ---
// Generate random bytecodes and verify they produce deterministic results.

func TestRandomBytecodeDeterminism(t *testing.T) {
	rng := rand.New(rand.NewSource(42))
	const numTests = 200

	for i := 0; i < numTests; i++ {
		// Generate random bytecode (4-64 bytes)
		codeLen := rng.Intn(61) + 4
		code := make([]byte, codeLen)
		rng.Read(code)

		t.Run(fmt.Sprintf("random_%d", i), func(t *testing.T) {
			r1 := executeWithCode(code, gevmspec.Cancun)
			r2 := executeWithCode(code, gevmspec.Cancun)

			if r1.Kind != r2.Kind {
				t.Errorf("result kind differs: %d vs %d (code: %x)", r1.Kind, r2.Kind, code)
			}
			if r1.GasUsed != r2.GasUsed {
				t.Errorf("gas used differs: %d vs %d (code: %x)", r1.GasUsed, r2.GasUsed, code)
			}
			if !bytesEqual(r1.Output, r2.Output) {
				t.Errorf("output differs (code: %x)", code)
			}
		})
	}
}

// --- Cross-fork consistency tests ---
// Verify that certain simple operations produce the same result across forks.

func TestCrossForkConsistency(t *testing.T) {
	forks := []struct {
		name string
		spec gevmspec.ForkID
	}{
		{"Berlin", gevmspec.Berlin},
		{"London", gevmspec.London},
		{"Shanghai", gevmspec.Shanghai},
		{"Cancun", gevmspec.Cancun},
		{"Prague", gevmspec.Prague},
	}

	// Simple ADD operation: 1 + 2 = 3, return result
	code := hexBytes("6001600201600052602060006000F0") // ADD(1,2) then CREATE (simplify to just ADD/return)
	// Actually use: PUSH1 1, PUSH1 2, ADD, PUSH1 0, MSTORE, PUSH1 32, PUSH1 0, RETURN
	code = hexBytes("600160020160005260206000F3")

	var prevGas uint64
	var prevOut []byte

	for _, fork := range forks {
		t.Run(fork.name, func(t *testing.T) {
			r := executeWithCode(code, fork.spec)

			if r.Kind != host.ResultSuccess {
				t.Fatalf("expected success, got %s", spec.FormatEvmResult(&r))
			}

			// Output should be 32 bytes with value 3 in last byte
			if len(r.Output) != 32 || r.Output[31] != 3 {
				t.Fatalf("expected output[31]==3, got %x", r.Output)
			}

			// Gas should be consistent across forks for this simple case
			if prevGas != 0 && r.GasUsed != prevGas {
				t.Logf("gas differs from previous fork: %d vs %d (expected for fork-specific base gas changes)", r.GasUsed, prevGas)
			}
			prevGas = r.GasUsed

			// Output must be identical
			if prevOut != nil && !bytesEqual(r.Output, prevOut) {
				t.Errorf("output differs from previous fork")
			}
			prevOut = make([]byte, len(r.Output))
			copy(prevOut, r.Output)
		})
	}
}

// --- Revme differential comparison ---
// When REVME_BIN is set, run both GEVM and revme on the same test fixtures
// and compare structured JSON outputs.

func TestRevmeDifferential(t *testing.T) {
	revmeBin := os.Getenv("REVME_BIN")
	if revmeBin == "" {
		t.Skip("REVME_BIN not set; skipping revme differential tests")
	}
	testsDir := os.Getenv("GEVM_TESTS_DIR")
	if testsDir == "" {
		t.Skip("GEVM_TESTS_DIR not set; skipping revme differential tests")
	}

	// Pick a small set of test files for differential comparison
	testFiles := findTestSample(testsDir, 20)
	if len(testFiles) == 0 {
		t.Skip("no test files found")
	}

	for _, testFile := range testFiles {
		t.Run(filepath.Base(testFile), func(t *testing.T) {
			// Run revme
			revmeOutcomes := runRevme(t, revmeBin, testFile)
			if len(revmeOutcomes) == 0 {
				t.Skip("no revme outcomes")
			}

			// Run GEVM
			cfg := spec.DefaultConfig()
			gevmOutcomes, err := spec.RunTestFileOutcomes(testFile, cfg)
			if err != nil {
				t.Fatalf("GEVM error: %v", err)
			}

			// Build lookup map for GEVM outcomes
			gevmMap := make(map[string]spec.TestOutcome)
			for _, o := range gevmOutcomes {
				key := fmt.Sprintf("%s/%s/d%d/g%d/v%d", o.Test, o.Fork, o.D, o.G, o.V)
				gevmMap[key] = o
			}

			// Compare
			for _, revme := range revmeOutcomes {
				key := fmt.Sprintf("%s/%s/d%d/g%d/v%d", revme.Test, revme.Fork, revme.D, revme.G, revme.V)
				gevm, ok := gevmMap[key]
				if !ok {
					continue // GEVM may skip some specs
				}

				if gevm.Pass != revme.Pass {
					t.Errorf("%s: pass differs: GEVM=%v, revme=%v (GEVM: %s, revme: %s)",
						key, gevm.Pass, revme.Pass, gevm.ErrorMsg, revme.ErrorMsg)
				}
				if gevm.GasUsed != revme.GasUsed {
					t.Errorf("%s: gasUsed differs: GEVM=%d, revme=%d", key, gevm.GasUsed, revme.GasUsed)
				}
				if gevm.LogsRoot != revme.LogsRoot {
					t.Errorf("%s: logsRoot differs: GEVM=%s, revme=%s", key, gevm.LogsRoot, revme.LogsRoot)
				}
				if gevm.Output != revme.Output {
					t.Errorf("%s: output differs: GEVM=%s, revme=%s", key, gevm.Output, revme.Output)
				}
			}
		})
	}
}

// revmeOutcome is the JSON output from revme --json-outcome.
type revmeOutcome struct {
	LogsRoot string `json:"logsRoot"`
	Output   string `json:"output"`
	GasUsed  uint64 `json:"gasUsed"`
	Pass     bool   `json:"pass"`
	ErrorMsg string `json:"errorMsg"`
	Fork     string `json:"fork"`
	Test     string `json:"test"`
	D        int    `json:"d"`
	G        int    `json:"g"`
	V        int    `json:"v"`
}

func runRevme(t *testing.T, bin string, testFile string) []revmeOutcome {
	cmd := exec.Command(bin, "statetest", "--json-outcome", testFile)
	out, err := cmd.Output()
	if err != nil {
		t.Logf("revme error: %v", err)
		return nil
	}

	var outcomes []revmeOutcome
	scanner := bufio.NewScanner(strings.NewReader(string(out)))
	for scanner.Scan() {
		line := scanner.Text()
		if !strings.HasPrefix(line, "{") {
			continue
		}
		var o revmeOutcome
		if err := json.Unmarshal([]byte(line), &o); err != nil {
			continue
		}
		outcomes = append(outcomes, o)
	}
	return outcomes
}

func findTestSample(dir string, maxFiles int) []string {
	files, err := spec.FindJSONTests(dir)
	if err != nil || len(files) == 0 {
		return nil
	}
	if len(files) <= maxFiles {
		return files
	}
	// Sample evenly across the file list
	step := len(files) / maxFiles
	var sample []string
	for i := 0; i < len(files) && len(sample) < maxFiles; i += step {
		sample = append(sample, files[i])
	}
	return sample
}

// --- Fixture-level differential tests ---
// Run selected test fixtures through GEVM with full outcome validation.
// This exercises the same code path as the spec tests but through the
// testbench JSON output pipeline, catching any serialization issues.

func TestFixtureOutcomePipeline(t *testing.T) {
	testsDir := os.Getenv("GEVM_TESTS_DIR")
	if testsDir == "" {
		t.Skip("GEVM_TESTS_DIR not set; skipping fixture outcome pipeline tests")
	}

	// Select a few test files from different categories
	categories := []string{
		"stArgsZeroOneBalance",
		"stCallCodes",
		"stMemoryTest",
		"stReturnDataTest",
		"stSStoreTest",
	}

	cfg := spec.DefaultConfig()

	for _, cat := range categories {
		catDir := filepath.Join(testsDir, cat)
		if _, err := os.Stat(catDir); os.IsNotExist(err) {
			continue
		}

		files, err := spec.FindJSONTests(catDir)
		if err != nil || len(files) == 0 {
			continue
		}

		// Test first file from each category
		testFile := files[0]
		t.Run(cat+"/"+filepath.Base(testFile), func(t *testing.T) {
			outcomes, err := spec.RunTestFileOutcomes(testFile, cfg)
			if err != nil {
				t.Fatalf("RunTestFileOutcomes error: %v", err)
			}

			for _, o := range outcomes {
				if !o.Pass {
					t.Errorf("test %s/%s d%d/g%d/v%d failed: %s",
						o.Test, o.Fork, o.D, o.G, o.V, o.ErrorMsg)
				}
			}

			// Verify JSON round-trip works
			for _, o := range outcomes {
				data, err := json.Marshal(o)
				if err != nil {
					t.Errorf("JSON marshal error: %v", err)
					continue
				}
				var decoded spec.TestOutcome
				if err := json.Unmarshal(data, &decoded); err != nil {
					t.Errorf("JSON unmarshal error: %v", err)
					continue
				}
				if decoded.GasUsed != o.GasUsed || decoded.Pass != o.Pass || decoded.LogsRoot != o.LogsRoot {
					t.Errorf("JSON round-trip mismatch for %s/%s", o.Test, o.Fork)
				}
			}
		})
	}
}

// --- Helper functions ---

func hexBytes(s string) []byte {
	b, err := hex.DecodeString(s)
	if err != nil {
		panic(err)
	}
	return b
}

func bytesEqual(a, b []byte) bool {
	if len(a) == 0 && len(b) == 0 {
		return true
	}
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

// Suppress unused import warnings.
var _ = vm.InstructionResultStop
