package spec

import (
	"os"
	"path/filepath"
	"testing"
)

// TestBlockchainFixtureDir runs against the ethereum/tests blockchain fixtures.
// Set GEVM_BLOCKCHAIN_TESTS_DIR to the path containing blockchain test JSON files.
func TestBlockchainFixtureDir(t *testing.T) {
	testsDir := os.Getenv("GEVM_BLOCKCHAIN_TESTS_DIR")
	if testsDir == "" {
		t.Skip("GEVM_BLOCKCHAIN_TESTS_DIR not set; skipping blockchain fixture tests")
	}

	if _, err := os.Stat(testsDir); os.IsNotExist(err) {
		t.Skipf("BlockchainTests directory not found at %s", testsDir)
	}

	cfg := DefaultBlockchainConfig()
	passed, failed, failures := RunBlockchainTestDir(testsDir, cfg)
	t.Logf("BlockchainTests: %d passed, %d failed (%.1f%% pass rate)",
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

	// Print first 30 failure details
	for i, f := range failures {
		if i >= 30 {
			break
		}
		detail := f.Detail
		if len(detail) > 150 {
			detail = detail[:150]
		}
		t.Logf("FAIL: %s | %s", f.Error.Error(), detail)
	}

	if failed > 0 {
		t.Logf("WARNING: %d blockchain tests failed", failed)
	}
}

// TestBlockchainInvalidBlocks runs against the ethereum/tests InvalidBlocks fixtures.
// These are blocks with expectException that should fail during execution.
// Our runner skips blocks with expectException, so the post-state should still be valid.
// Set GEVM_BLOCKCHAIN_TESTS_DIR to the parent BlockchainTests directory
// (InvalidBlocks should be a subdirectory).
func TestBlockchainInvalidBlocks(t *testing.T) {
	testsDir := os.Getenv("GEVM_BLOCKCHAIN_TESTS_DIR")
	if testsDir == "" {
		t.Skip("GEVM_BLOCKCHAIN_TESTS_DIR not set; skipping InvalidBlocks tests")
	}

	invalidDir := filepath.Join(testsDir, "InvalidBlocks")
	if _, err := os.Stat(invalidDir); os.IsNotExist(err) {
		// Try without subdirectory (maybe they pointed directly at InvalidBlocks)
		invalidDir = testsDir
	}

	cfg := DefaultBlockchainConfig()
	passed, failed, failures := RunBlockchainTestDir(invalidDir, cfg)
	t.Logf("InvalidBlocks: %d passed, %d failed (%.1f%% pass rate)",
		passed, failed, float64(passed)*100/float64(passed+failed))

	// Print first 30 failure details
	for i, f := range failures {
		if i >= 30 {
			break
		}
		detail := f.Detail
		if len(detail) > 150 {
			detail = detail[:150]
		}
		t.Logf("FAIL: %s | %s", f.Error.Error(), detail)
	}

	if failed > 0 {
		t.Fatalf("InvalidBlocks: %d tests failed", failed)
	}
}
