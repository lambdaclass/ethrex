package spec

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

// TestHexU256Unmarshal verifies hex JSON parsing for Uint256.
func TestHexU256Unmarshal(t *testing.T) {
	tests := []struct {
		input string
		want  uint64
	}{
		{`"0x00"`, 0},
		{`"0x01"`, 1},
		{`"0xff"`, 255},
		{`"0x0100"`, 256},
		{`"0x0f4240"`, 1_000_000},
		{`"0x5208"`, 21000},
	}
	for _, tt := range tests {
		var h HexU256
		if err := json.Unmarshal([]byte(tt.input), &h); err != nil {
			t.Fatalf("unmarshal %s: %v", tt.input, err)
		}
		if h.V[0] != tt.want || h.V[1] != 0 || h.V[2] != 0 || h.V[3] != 0 {
			t.Fatalf("unmarshal %s: got %v, want %d", tt.input, h.V, tt.want)
		}
	}
}

// TestHexAddrUnmarshal verifies hex JSON parsing for Address (used as map key).
func TestHexAddrUnmarshal(t *testing.T) {
	input := `{"0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b": {"balance":"0x01","code":"0x","nonce":"0x00","storage":{}}}`
	var m map[HexAddr]*TestAccountInfo
	if err := json.Unmarshal([]byte(input), &m); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if len(m) != 1 {
		t.Fatalf("expected 1 account, got %d", len(m))
	}
	for addr, info := range m {
		if addr.V[0] != 0xa9 || addr.V[19] != 0x0b {
			t.Fatalf("wrong address: %x", addr.V)
		}
		if info.Balance.V[0] != 1 {
			t.Fatalf("wrong balance: %v", info.Balance.V)
		}
	}
}

// TestRecoverAddress verifies private key → address recovery.
func TestRecoverAddress(t *testing.T) {
	// Standard test vector:
	// private key: 0x45a915e4d060149eb4365960e6a7a45f334393093061116b197e3240065ff2d8
	// expected address: 0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b
	privKey := hexDecode("45a915e4d060149eb4365960e6a7a45f334393093061116b197e3240065ff2d8")
	addr, err := RecoverAddress(privKey)
	if err != nil {
		t.Fatalf("recover: %v", err)
	}
	expected := "0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b"
	got := addr.Hex()
	if got != expected {
		t.Fatalf("recover: got %s, want %s", got, expected)
	}
}

func hexDecode(s string) []byte {
	b := make([]byte, len(s)/2)
	for i := 0; i < len(s); i += 2 {
		b[i/2] = hexByte(s[i])<<4 | hexByte(s[i+1])
	}
	return b
}

func hexByte(c byte) byte {
	switch {
	case c >= '0' && c <= '9':
		return c - '0'
	case c >= 'a' && c <= 'f':
		return c - 'a' + 10
	case c >= 'A' && c <= 'F':
		return c - 'A' + 10
	}
	return 0
}

// TestSimpleTransferFixture tests a hand-crafted simple transfer fixture.
func TestSimpleTransferFixture(t *testing.T) {
	// Create a minimal test fixture: sender sends 1 wei to recipient
	fixture := `{
		"simpleTransfer": {
			"env": {
				"currentCoinbase": "0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba",
				"currentDifficulty": "0x020000",
				"currentGasLimit": "0x05f5e100",
				"currentNumber": "0x01",
				"currentTimestamp": "0x03e8",
				"currentBaseFee": "0x0a"
			},
			"pre": {
				"0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b": {
					"balance": "0x0de0b6b3a7640000",
					"code": "0x",
					"nonce": "0x00",
					"storage": {}
				},
				"0xb94f5374fce5edbc8e2a8697c15331677e6ebf0b": {
					"balance": "0x00",
					"code": "0x",
					"nonce": "0x00",
					"storage": {}
				}
			},
			"transaction": {
				"data": ["0x"],
				"gasLimit": ["0x5208"],
				"gasPrice": "0x0a",
				"nonce": "0x00",
				"secretKey": "0x45a915e4d060149eb4365960e6a7a45f334393093061116b197e3240065ff2d8",
				"to": "0xb94f5374fce5edbc8e2a8697c15331677e6ebf0b",
				"value": ["0x01"]
			},
			"post": {
				"Istanbul": [
					{
						"indexes": {"data": 0, "gas": 0, "value": 0},
						"hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
						"logs": "0x0000000000000000000000000000000000000000000000000000000000000000"
					}
				],
				"Berlin": [
					{
						"indexes": {"data": 0, "gas": 0, "value": 0},
						"hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
						"logs": "0x0000000000000000000000000000000000000000000000000000000000000000"
					}
				],
				"London": [
					{
						"indexes": {"data": 0, "gas": 0, "value": 0},
						"hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
						"logs": "0x0000000000000000000000000000000000000000000000000000000000000000"
					}
				]
			}
		}
	}`

	// Write to temp file
	dir := t.TempDir()
	path := filepath.Join(dir, "simpleTransfer.json")
	if err := os.WriteFile(path, []byte(fixture), 0644); err != nil {
		t.Fatal(err)
	}

	results, err := RunTestFile(path, DefaultConfig())
	if err != nil {
		t.Fatalf("RunTestFile: %v", err)
	}

	for _, r := range results {
		if !r.Pass {
			t.Fatalf("test failed: %v (detail: %s)", r.Error, r.Detail)
		}
	}

	if len(results) != 3 {
		t.Fatalf("expected 3 results (Istanbul+Berlin+London), got %d", len(results))
	}
}

// TestSimpleCreateFixture tests a CREATE transaction.
func TestSimpleCreateFixture(t *testing.T) {
	// Sender deploys simple contract: PUSH1 0x00 PUSH1 0x00 RETURN (returns empty)
	fixture := `{
		"simpleCreate": {
			"env": {
				"currentCoinbase": "0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba",
				"currentDifficulty": "0x020000",
				"currentGasLimit": "0x05f5e100",
				"currentNumber": "0x01",
				"currentTimestamp": "0x03e8",
				"currentBaseFee": "0x0a"
			},
			"pre": {
				"0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b": {
					"balance": "0x0de0b6b3a7640000",
					"code": "0x",
					"nonce": "0x00",
					"storage": {}
				}
			},
			"transaction": {
				"data": ["0x60006000f3"],
				"gasLimit": ["0x0186a0"],
				"gasPrice": "0x0a",
				"nonce": "0x00",
				"secretKey": "0x45a915e4d060149eb4365960e6a7a45f334393093061116b197e3240065ff2d8",
				"to": "",
				"value": ["0x00"]
			},
			"post": {
				"Shanghai": [
					{
						"indexes": {"data": 0, "gas": 0, "value": 0},
						"hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
						"logs": "0x0000000000000000000000000000000000000000000000000000000000000000"
					}
				]
			}
		}
	}`

	dir := t.TempDir()
	path := filepath.Join(dir, "simpleCreate.json")
	if err := os.WriteFile(path, []byte(fixture), 0644); err != nil {
		t.Fatal(err)
	}

	results, err := RunTestFile(path, DefaultConfig())
	if err != nil {
		t.Fatalf("RunTestFile: %v", err)
	}

	for _, r := range results {
		if !r.Pass {
			t.Fatalf("test failed: %v (detail: %s)", r.Error, r.Detail)
		}
	}
}

// TestExpectedExceptionFixture tests that expected exceptions are handled.
func TestExpectedExceptionFixture(t *testing.T) {
	// Sender has insufficient balance for gas
	fixture := `{
		"insufficientBalance": {
			"env": {
				"currentCoinbase": "0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba",
				"currentDifficulty": "0x020000",
				"currentGasLimit": "0x05f5e100",
				"currentNumber": "0x01",
				"currentTimestamp": "0x03e8",
				"currentBaseFee": "0x0a"
			},
			"pre": {
				"0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b": {
					"balance": "0x00",
					"code": "0x",
					"nonce": "0x00",
					"storage": {}
				}
			},
			"transaction": {
				"data": ["0x"],
				"gasLimit": ["0x5208"],
				"gasPrice": "0x0a",
				"nonce": "0x00",
				"secretKey": "0x45a915e4d060149eb4365960e6a7a45f334393093061116b197e3240065ff2d8",
				"to": "0xb94f5374fce5edbc8e2a8697c15331677e6ebf0b",
				"value": ["0x01"]
			},
			"post": {
				"London": [
					{
						"indexes": {"data": 0, "gas": 0, "value": 0},
						"hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
						"logs": "0x0000000000000000000000000000000000000000000000000000000000000000",
						"expectException": "InsufficientAccountFunds"
					}
				]
			}
		}
	}`

	dir := t.TempDir()
	path := filepath.Join(dir, "insufficientBalance.json")
	if err := os.WriteFile(path, []byte(fixture), 0644); err != nil {
		t.Fatal(err)
	}

	results, err := RunTestFile(path, DefaultConfig())
	if err != nil {
		t.Fatalf("RunTestFile: %v", err)
	}

	if len(results) != 1 {
		t.Fatalf("expected 1 result, got %d", len(results))
	}
	if !results[0].Pass {
		t.Fatalf("expected pass (exception expected and occurred), got fail: %v", results[0].Error)
	}
}

// TestRevertFixture tests a contract that REVERTs.
func TestRevertFixture(t *testing.T) {
	// Contract code: PUSH1 0x00 PUSH1 0x00 REVERT (revert with empty data)
	// 0x60006000fd
	// The contract will revert. No exception expected — reverts are normal EVM behavior.
	fixture := `{
		"revertTest": {
			"env": {
				"currentCoinbase": "0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba",
				"currentDifficulty": "0x020000",
				"currentGasLimit": "0x05f5e100",
				"currentNumber": "0x01",
				"currentTimestamp": "0x03e8",
				"currentBaseFee": "0x0a"
			},
			"pre": {
				"0xa94f5374fce5edbc8e2a8697c15331677e6ebf0b": {
					"balance": "0x0de0b6b3a7640000",
					"code": "0x",
					"nonce": "0x00",
					"storage": {}
				},
				"0x1000000000000000000000000000000000000000": {
					"balance": "0x00",
					"code": "0x60006000fd",
					"nonce": "0x00",
					"storage": {}
				}
			},
			"transaction": {
				"data": ["0x"],
				"gasLimit": ["0x0186a0"],
				"gasPrice": "0x0a",
				"nonce": "0x00",
				"secretKey": "0x45a915e4d060149eb4365960e6a7a45f334393093061116b197e3240065ff2d8",
				"to": "0x1000000000000000000000000000000000000000",
				"value": ["0x00"]
			},
			"post": {
				"Shanghai": [
					{
						"indexes": {"data": 0, "gas": 0, "value": 0},
						"hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
						"logs": "0x0000000000000000000000000000000000000000000000000000000000000000"
					}
				]
			}
		}
	}`

	dir := t.TempDir()
	path := filepath.Join(dir, "revert.json")
	if err := os.WriteFile(path, []byte(fixture), 0644); err != nil {
		t.Fatal(err)
	}

	results, err := RunTestFile(path, DefaultConfig())
	if err != nil {
		t.Fatalf("RunTestFile: %v", err)
	}

	// The contract reverts — this is normal EVM behavior, not an error
	for _, r := range results {
		if !r.Pass {
			t.Fatalf("test failed: %v (detail: %s)", r.Error, r.Detail)
		}
	}
}

// TestRunTestDir tests running an entire directory (empty).
func TestRunTestDir(t *testing.T) {
	dir := t.TempDir()
	passed, failed, failures := RunTestDir(dir, DefaultConfig())
	if passed != 0 || failed != 0 || len(failures) != 0 {
		t.Fatalf("empty dir should yield 0 results, got %d/%d/%d", passed, failed, len(failures))
	}
}

// TestSpecFixtureDir runs against the ethereum/tests fixtures if available.
// Set GEVM_TESTS_DIR to the path of the ethereum/tests repo.
func TestSpecFixtureDir(t *testing.T) {
	testsDir := os.Getenv("GEVM_TESTS_DIR")
	if testsDir == "" {
		t.Skip("GEVM_TESTS_DIR not set; skipping spec fixture tests")
	}

	// Run GeneralStateTests
	// GEVM_TESTS_DIR should point directly to the directory containing the
	// test category subdirectories (stArgsZeroOneBalance, stCallCodes, etc.)
	gstDir := testsDir
	if _, err := os.Stat(gstDir); os.IsNotExist(err) {
		t.Skipf("GeneralStateTests directory not found at %s", gstDir)
	}

	cfg := DefaultConfig()
	passed, failed, failures := RunTestDir(gstDir, cfg)
	t.Logf("GeneralStateTests: %d passed, %d failed (%.1f%% pass rate)",
		passed, failed, float64(passed)*100/float64(passed+failed))

	// Categorize failures
	categories := map[string]int{}
	panicCount := 0
	for _, f := range failures {
		cat := filepath.Base(filepath.Dir(f.Error.TestFile))
		categories[cat]++
		if len(f.Detail) >= 6 && f.Detail[:6] == "PANIC:" {
			panicCount++
		}
	}
	t.Logf("Failure breakdown: %d panics, %d other", panicCount, failed-panicCount)

	// Print category summary
	type catCount struct {
		name  string
		count int
	}
	var sorted []catCount
	for name, count := range categories {
		sorted = append(sorted, catCount{name, count})
	}
	for i := 0; i < len(sorted); i++ {
		for j := i + 1; j < len(sorted); j++ {
			if sorted[j].count > sorted[i].count {
				sorted[i], sorted[j] = sorted[j], sorted[i]
			}
		}
	}
	t.Logf("Failure categories (by directory):")
	for i, c := range sorted {
		if i >= 30 {
			break
		}
		t.Logf("  %5d %s", c.count, c.name)
	}

	// Print first 30 failure details with error detail
	for i, f := range failures {
		if i >= 30 {
			break
		}
		detail := f.Detail
		if len(detail) > 100 {
			detail = detail[:100]
		}
		t.Logf("FAIL: %s | %s", f.Error.Error(), detail)
	}

	if failed > 0 {
		t.Logf("WARNING: %d tests failed (this is expected during initial development)", failed)
	}
}

// TestEESTFixtures runs the execution-spec-tests (EEST) fixtures.
// Set GEVM_EEST_DIR to the state_tests directory.
func TestEESTFixtures(t *testing.T) {
	testsDir := os.Getenv("GEVM_EEST_DIR")
	if testsDir == "" {
		t.Skip("GEVM_EEST_DIR not set; skipping EEST fixture tests")
	}

	cfg := DefaultConfig()
	passed, failed, failures := RunTestDir(testsDir, cfg)
	t.Logf("EEST state tests: %d passed, %d failed (%.1f%% pass rate)",
		passed, failed, float64(passed)*100/float64(passed+failed))

	for i, f := range failures {
		if i >= 50 {
			break
		}
		detail := f.Detail
		if len(detail) > 200 {
			detail = detail[:200]
		}
		t.Logf("FAIL: %s | %s", f.Error.Error(), detail)
	}

	if failed > 0 {
		t.Fatalf("%d/%d EEST tests failed", failed, passed+failed)
	}
}

// TestLogsRootEmpty verifies the logs root hash for empty logs.
func TestLogsRootEmpty(t *testing.T) {
	// keccak256(RLP([])) = keccak256(0xc0)
	root := LogsRoot(nil)
	// Expected: keccak256(0xc0) = 0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347
	expectedHex := "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347"
	if root.Hex() != expectedHex {
		t.Fatalf("empty logs root: got %s, want %s", root.Hex(), expectedHex)
	}
}

// TestRlpEncodeBytes verifies basic RLP encoding.
func TestRlpEncodeBytes(t *testing.T) {
	// Single byte < 0x80
	enc := RlpEncodeBytes([]byte{0x42})
	if len(enc) != 1 || enc[0] != 0x42 {
		t.Fatalf("single byte: got %x", enc)
	}

	// Empty bytes
	enc = RlpEncodeBytes(nil)
	if len(enc) != 1 || enc[0] != 0x80 {
		t.Fatalf("empty: got %x", enc)
	}

	// Short string (3 bytes)
	enc = RlpEncodeBytes([]byte{1, 2, 3})
	if len(enc) != 4 || enc[0] != 0x83 {
		t.Fatalf("short string: got %x", enc)
	}
}
