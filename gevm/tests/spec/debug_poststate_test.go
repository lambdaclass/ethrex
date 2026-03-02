package spec

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/Giulio2002/gevm/host"
	"github.com/Giulio2002/gevm/types"
	gevmspec "github.com/Giulio2002/gevm/spec"
)

// TestDebugPostState runs post-state validation on specific directories.
func TestDebugPostState(t *testing.T) {
	baseDir := "/Users/Giulio2002/work/gevm/tests/fixtures/ethereum-tests/GeneralStateTests/GeneralStateTests"
	if _, err := os.Stat(baseDir); os.IsNotExist(err) {
		t.Skip("fixtures not found")
	}

	dirs := []string{
		"stBadOpcode",
		"stReturnDataTest",
		"stCreate2",
		"stCreateTest",
		"stRevertTest",
	}

	cfg := DefaultConfig()

	for _, dir := range dirs {
		fullDir := filepath.Join(baseDir, dir)
		if _, err := os.Stat(fullDir); os.IsNotExist(err) {
			continue
		}

		passed, failed, failures := RunTestDir(fullDir, cfg)
		t.Logf("%s: %d passed, %d failed", dir, passed, failed)

		// Categorize failure types
		types := map[string]int{}
		for _, f := range failures {
			// Extract the type of mismatch
			detail := f.Detail
			if len(detail) > 200 {
				detail = detail[:200]
			}
			// Categorize: nonce, balance, storage, code
			switch {
			case contains(detail, "nonce mismatch"):
				types["nonce"]++
			case contains(detail, "balance mismatch"):
				types["balance"]++
			case contains(detail, "storage") && contains(detail, "mismatch"):
				types["storage"]++
			case contains(detail, "code mismatch"):
				types["code"]++
			case contains(detail, "unexpected storage"):
				types["extra-storage"]++
			case contains(detail, "not found"):
				types["missing-account"]++
			default:
				types["other"]++
			}
		}
		for typ, count := range types {
			t.Logf("  %s: %d", typ, count)
		}

		// Print first 5 failures with full detail (unlimited)
		for i, f := range failures {
			if i >= 5 {
				break
			}
			t.Logf("  FAIL: %s | %s", f.Error.Error(), f.Detail)
		}
	}
}

// TestDebugClearReturnBuffer categorizes clearReturnBuffer failures by target opcode.
func TestDebugClearReturnBuffer(t *testing.T) {
	testFile := "/Users/Giulio2002/work/gevm/tests/fixtures/ethereum-tests/GeneralStateTests/GeneralStateTests/stReturnDataTest/clearReturnBuffer.json"
	if _, err := os.Stat(testFile); os.IsNotExist(err) {
		t.Skip("fixture not found")
	}

	data, err := os.ReadFile(testFile)
	if err != nil {
		t.Fatal(err)
	}

	var suite TestSuite
	if err := json.Unmarshal(data, &suite); err != nil {
		t.Fatal(err)
	}

	for testName, unit := range suite {
		// Track failures by target opcode
		targetFailures := map[uint64]int{}
		targetPasses := map[uint64]int{}

		for specName, cases := range unit.Post {
			if specName == "Constantinople" || specName == "ByzantiumToConstantinopleAt5" {
				continue
			}
			forkID, ok := SpecNameToForkID(specName)
			if !ok {
				continue
			}

			for _, tc := range cases {
				caller, err := RecoverAddress(unit.Transaction.SecretKey.V[:])
				if err != nil {
					continue
				}

				tx, txErr := BuildTransaction(unit, &tc, caller)
				if txErr != nil {
					continue
				}

				// Extract target opcode from calldata (param1, starts at byte 4, 32 bytes)
				var targetOpcode uint64
				if len(tx.Input) >= 36 {
					// param1 is at bytes 4..36 (big-endian Uint256)
					param1 := types.U256FromBytes32([32]byte(tx.Input[4:36]))
					targetOpcode = param1.AsUsize()
				}

				// Extract size from calldata (param3, starts at byte 68, 32 bytes)
				var sizeParam uint64
				if len(tx.Input) >= 100 {
					param3 := types.U256FromBytes32([32]byte(tx.Input[68:100]))
					sizeParam = param3.AsUsize()
				}

				db := BuildMemDB(unit.Pre)
				blockEnv := BuildBlockEnv(unit, forkID)
				cfgEnv := host.CfgEnv{ChainId: unit.ChainId()}
				evm := host.NewEvm(db, forkID, blockEnv, cfgEnv)
				execResult := evm.Transact(&tx)

				if execResult.ValidationError {
					continue
				}

				expectedState := tc.State
				if expectedState == nil {
					expectedState = tc.PostState
				}
				if expectedState == nil {
					continue
				}

				hasFail := false
				for hexAddr, expectedAcct := range expectedState {
					addr := hexAddr.V
					acc, ok := evm.Journal.State[addr]
					if !ok {
						result, loadErr := evm.Journal.LoadAccount(addr)
						if loadErr != nil {
							continue
						}
						acc = result.Data
					}

					if acc.Info.Balance != expectedAcct.Balance.V {
						hasFail = true
						break
					}

					// Check storage too
					for hexSlot, hexVal := range expectedAcct.Storage {
						slot := hexSlot.V.ToU256()
						wantVal := hexVal.V
						gotVal := types.U256Zero
						if acc.Storage != nil {
							if sv, ok := acc.Storage[slot]; ok {
								gotVal = sv.PresentValue
							}
						}
						if gotVal != wantVal {
							hasFail = true
							break
						}
					}
					if hasFail {
						break
					}
				}

				if hasFail {
					targetFailures[targetOpcode]++

					// Print ALL failures for target=0xf1 (CALL) to see size/delta pattern
					if targetOpcode == 0xf1 || targetOpcode == 0xf0 {
						for hexAddr, expectedAcct := range expectedState {
							addr := hexAddr.V
							acc, ok := evm.Journal.State[addr]
							if !ok {
								result, loadErr := evm.Journal.LoadAccount(addr)
								if loadErr != nil {
									continue
								}
								acc = result.Data
							}
							gotBal := acc.Info.Balance
							wantBal := expectedAcct.Balance.V
							if gotBal != wantBal {
								gasPrice := tx.GasPrice.AsUsize()
								var deltaGas uint64
								var sign string
								if gotBal.Gt(wantBal) {
									delta := gotBal.Sub(wantBal)
									deltaGas = delta.AsUsize() / gasPrice
									sign = "over"
								} else {
									delta := wantBal.Sub(gotBal)
									deltaGas = delta.AsUsize() / gasPrice
									sign = "under"
								}
								perByte := uint64(0)
								if sizeParam > 0 {
									perByte = deltaGas / sizeParam
								}
								t.Logf("[%s] target=0x%x size=%d delta_gas=%d (%s) per_byte=%d gasUsed=%d gasRefund=%d",
									specName, targetOpcode, sizeParam, deltaGas, sign, perByte, execResult.GasUsed, execResult.GasRefund)
							}
						}
					}
				} else {
					targetPasses[targetOpcode]++
				}
			}
		}

		// Summary
		t.Logf("\n%s - Summary by target opcode:", testName)
		allTargets := map[uint64]bool{}
		for k := range targetFailures {
			allTargets[k] = true
		}
		for k := range targetPasses {
			allTargets[k] = true
		}
		for target := range allTargets {
			t.Logf("  target=0x%x: %d pass, %d fail", target, targetPasses[target], targetFailures[target])
		}
	}
}

func contains(s, sub string) bool {
	for i := 0; i <= len(s)-len(sub); i++ {
		if s[i:i+len(sub)] == sub {
			return true
		}
	}
	return false
}

func init() {
	_ = fmt.Sprint // avoid unused import
	_ = gevmspec.Cancun
}
