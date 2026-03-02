package spec

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/Giulio2002/gevm/types"
)

// TestRlpDecode tests basic RLP decoding.
func TestRlpDecode(t *testing.T) {
	// Single byte
	item, err := RlpDecodeComplete([]byte{0x42})
	if err != nil {
		t.Fatal(err)
	}
	if item.Kind != RlpString || len(item.Data) != 1 || item.Data[0] != 0x42 {
		t.Fatalf("single byte: %+v", item)
	}

	// Empty string
	item, err = RlpDecodeComplete([]byte{0x80})
	if err != nil {
		t.Fatal(err)
	}
	if item.Kind != RlpString || len(item.Data) != 0 {
		t.Fatalf("empty string: %+v", item)
	}

	// Short string "dog" = 0x83, 'd', 'o', 'g'
	item, err = RlpDecodeComplete([]byte{0x83, 'd', 'o', 'g'})
	if err != nil {
		t.Fatal(err)
	}
	if item.Kind != RlpString || string(item.Data) != "dog" {
		t.Fatalf("short string: %+v", item)
	}

	// Empty list
	item, err = RlpDecodeComplete([]byte{0xc0})
	if err != nil {
		t.Fatal(err)
	}
	if item.Kind != RlpList || len(item.Items) != 0 {
		t.Fatalf("empty list: %+v", item)
	}

	// List ["cat", "dog"]
	item, err = RlpDecodeComplete([]byte{0xc8, 0x83, 'c', 'a', 't', 0x83, 'd', 'o', 'g'})
	if err != nil {
		t.Fatal(err)
	}
	if item.Kind != RlpList || len(item.Items) != 2 {
		t.Fatalf("list: %+v", item)
	}
	if string(item.Items[0].Data) != "cat" || string(item.Items[1].Data) != "dog" {
		t.Fatalf("list items: %+v %+v", item.Items[0], item.Items[1])
	}
}

// TestRlpDecodeStrict tests strict validation rules.
func TestRlpDecodeStrict(t *testing.T) {
	// Non-canonical: single byte 0x00 encoded with 0x81 prefix
	_, err := RlpDecodeComplete([]byte{0x81, 0x00})
	if err == nil {
		t.Fatal("expected error for non-canonical single byte encoding")
	}

	// Non-canonical: single byte 0x7f encoded with 0x81 prefix
	_, err = RlpDecodeComplete([]byte{0x81, 0x7f})
	if err == nil {
		t.Fatal("expected error for non-canonical single byte encoding")
	}

	// Trailing bytes
	_, err = RlpDecodeComplete([]byte{0x80, 0x00})
	if err == nil {
		t.Fatal("expected error for trailing bytes")
	}
}

// TestRlpAsUint64 tests uint64 parsing from RLP items.
func TestRlpAsUint64(t *testing.T) {
	// Zero
	item := RlpItem{Kind: RlpString, Data: nil}
	v, err := item.AsUint64()
	if err != nil || v != 0 {
		t.Fatalf("zero: %d %v", v, err)
	}

	// 1
	item = RlpItem{Kind: RlpString, Data: []byte{0x01}}
	v, err = item.AsUint64()
	if err != nil || v != 1 {
		t.Fatalf("1: %d %v", v, err)
	}

	// Leading zeros rejected
	item = RlpItem{Kind: RlpString, Data: []byte{0x00, 0x01}}
	_, err = item.AsUint64()
	if err == nil {
		t.Fatal("expected error for leading zeros")
	}

	// Max uint64
	item = RlpItem{Kind: RlpString, Data: []byte{0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff}}
	v, err = item.AsUint64()
	if err != nil || v != ^uint64(0) {
		t.Fatalf("max uint64: %d %v", v, err)
	}
}

// TestDecodeLegacyTx tests decoding a simple legacy transaction.
func TestDecodeLegacyTx(t *testing.T) {
	// Build a minimal legacy tx: [nonce=0, gasPrice=1, gasLimit=21000, to=addr, value=0, data=empty, v=27, r=1, s=1]
	// This is a minimal structure test, not a real signed tx.
	var items [][]byte
	items = append(items, RlpEncodeUint64(0))     // nonce
	items = append(items, RlpEncodeUint64(1))     // gasPrice
	items = append(items, RlpEncodeUint64(21000)) // gasLimit
	addr := make([]byte, 20)                      // to
	addr[19] = 0x01
	items = append(items, RlpEncodeBytes(addr))
	items = append(items, RlpEncodeUint64(0))  // value
	items = append(items, RlpEncodeBytes(nil)) // data
	items = append(items, RlpEncodeUint64(27)) // v
	items = append(items, RlpEncodeUint64(1))  // r
	items = append(items, RlpEncodeUint64(1))  // s

	txBytes := RlpEncodeList(items)

	tx, err := DecodeTx(txBytes)
	if err != nil {
		t.Fatalf("decode: %v", err)
	}

	if tx.TxType != 0 {
		t.Fatalf("type: got %d", tx.TxType)
	}
	if tx.Nonce != 0 {
		t.Fatalf("nonce: got %d", tx.Nonce)
	}
	if tx.GasLimit != 21000 {
		t.Fatalf("gasLimit: got %d", tx.GasLimit)
	}
	if tx.To == nil {
		t.Fatal("to: nil")
	}
	if tx.To[19] != 0x01 {
		t.Fatalf("to: %x", tx.To)
	}
}

// TestSigningHashLegacy tests that signing hash is computed correctly.
func TestSigningHashLegacy(t *testing.T) {
	// A pre-EIP-155 legacy tx with V=27 signs: RLP([nonce, gasPrice, gasLimit, to, value, data])
	tx := &DecodedTx{
		TxType:   0,
		Nonce:    0,
		GasPrice: types.U256From(1),
		GasLimit: 21000,
		Value:    types.U256Zero,
		V:        types.U256From(27),
	}
	// Not checking the exact hash value, just that it doesn't panic
	hash := SigningHash(tx)
	if hash == types.B256Zero {
		t.Fatal("signing hash should not be zero")
	}
}

// TestCalcTxIntrinsicGas tests intrinsic gas calculation.
func TestCalcTxIntrinsicGas(t *testing.T) {
	// Simple CALL with no data
	tx := &DecodedTx{
		TxType: 0,
		To:     &types.AddressZero,
	}
	gas := calcTxIntrinsicGas(tx, 9) // Istanbul
	if gas != 21000 {
		t.Fatalf("empty CALL: got %d, want 21000", gas)
	}

	// CREATE (no data)
	tx = &DecodedTx{TxType: 0, To: nil}
	gas = calcTxIntrinsicGas(tx, 9) // Istanbul
	if gas != 53000 {
		t.Fatalf("empty CREATE: got %d, want 53000", gas)
	}
}

// TestTransactionFixtureDir runs against the ethereum/tests TransactionTests fixtures.
// Set GEVM_TRANSACTION_TESTS_DIR to the path containing TransactionTest JSON files.
func TestTransactionFixtureDir(t *testing.T) {
	testsDir := os.Getenv("GEVM_TRANSACTION_TESTS_DIR")
	if testsDir == "" {
		t.Skip("GEVM_TRANSACTION_TESTS_DIR not set; skipping transaction fixture tests")
	}

	if _, err := os.Stat(testsDir); os.IsNotExist(err) {
		t.Skipf("TransactionTests directory not found at %s", testsDir)
	}

	cfg := RunnerConfig{
		SkipTests: map[string]bool{},
	}
	passed, failed, failures := RunTransactionTestDir(testsDir, cfg)
	t.Logf("TransactionTests: %d passed, %d failed (%.1f%% pass rate)",
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

	// Print first 50 failure details
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
		t.Fatalf("TransactionTests: %d tests failed", failed)
	}
}
