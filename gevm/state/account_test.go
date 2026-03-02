package state

import (
	"testing"

	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
)

func u256(v uint64) types.Uint256 {
	return types.U256From(v)
}

func TestAccountIsEmptyBalance(t *testing.T) {
	acc := DefaultAccount()
	if !acc.IsEmpty() {
		t.Fatal("default account should be empty")
	}
	acc.Info.Balance = u256(1)
	if acc.IsEmpty() {
		t.Fatal("account with balance should not be empty")
	}
	acc.Info.Balance = types.U256Zero
	if !acc.IsEmpty() {
		t.Fatal("account with zero balance should be empty")
	}
}

func TestAccountIsEmptyNonce(t *testing.T) {
	acc := DefaultAccount()
	if !acc.IsEmpty() {
		t.Fatal("default should be empty")
	}
	acc.Info.Nonce = 1
	if acc.IsEmpty() {
		t.Fatal("account with nonce should not be empty")
	}
	acc.Info.Nonce = 0
	if !acc.IsEmpty() {
		t.Fatal("account with zero nonce should be empty")
	}
}

func TestAccountIsEmptyCodeHash(t *testing.T) {
	acc := DefaultAccount()
	if !acc.IsEmpty() {
		t.Fatal("default should be empty")
	}
	acc.Info.CodeHash = types.B256{1}
	if acc.IsEmpty() {
		t.Fatal("account with code hash should not be empty")
	}
	acc.Info.CodeHash = types.B256Zero
	if !acc.IsEmpty() {
		t.Fatal("account with zero code hash should be empty")
	}
	acc.Info.CodeHash = types.KeccakEmpty
	if !acc.IsEmpty() {
		t.Fatal("account with KECCAK_EMPTY code hash should be empty")
	}
}

func TestAccountStatusFlags(t *testing.T) {
	acc := DefaultAccount()
	if acc.IsTouched() {
		t.Fatal("should not be touched")
	}
	if acc.IsSelfdestructed() {
		t.Fatal("should not be selfdestructed")
	}

	acc.MarkTouch()
	if !acc.IsTouched() {
		t.Fatal("should be touched")
	}
	if acc.IsSelfdestructed() {
		t.Fatal("should not be selfdestructed")
	}

	acc.MarkSelfdestruct()
	if !acc.IsTouched() {
		t.Fatal("should be touched")
	}
	if !acc.IsSelfdestructed() {
		t.Fatal("should be selfdestructed")
	}

	acc.UnmarkSelfdestruct()
	if !acc.IsTouched() {
		t.Fatal("should be touched")
	}
	if acc.IsSelfdestructed() {
		t.Fatal("should not be selfdestructed after unmark")
	}
}

func TestAccountIsColdTransactionID(t *testing.T) {
	acc := DefaultAccount()
	// default transaction_id=0, not cold when same id and no Cold flag
	if acc.IsColdTransactionID(0) {
		t.Fatal("should not be cold for tx 0")
	}
	if !acc.IsColdTransactionID(1) {
		t.Fatal("should be cold for tx 1")
	}

	acc.MarkCold()
	if !acc.IsColdTransactionID(0) {
		t.Fatal("should be cold when Cold flag set")
	}
	if !acc.IsColdTransactionID(1) {
		t.Fatal("should be cold for tx 1 with Cold flag")
	}
}

func TestAccountMarkWarm(t *testing.T) {
	acc := DefaultAccount()
	// Warm by default for tx 0
	if acc.MarkWarmWithTransactionID(0) {
		t.Fatal("should not be cold for same tx id")
	}

	acc.MarkCold()
	if !acc.MarkWarmWithTransactionID(0) {
		t.Fatal("should be cold after MarkCold")
	}
	// Now warm
	if acc.MarkWarmWithTransactionID(0) {
		t.Fatal("should be warm now")
	}
}

func TestAccountCreatedLocally(t *testing.T) {
	acc := DefaultAccount()
	if acc.IsCreated() {
		t.Fatal("should not be created")
	}
	if acc.IsCreatedLocally() {
		t.Fatal("should not be created locally")
	}

	wasGlobalFirst := acc.MarkCreatedLocally()
	if !wasGlobalFirst {
		t.Fatal("should be globally created first time")
	}
	if !acc.IsCreated() {
		t.Fatal("should be marked created")
	}
	if !acc.IsCreatedLocally() {
		t.Fatal("should be marked created locally")
	}

	// Second call: global was already set
	wasGlobalFirst = acc.MarkCreatedLocally()
	if wasGlobalFirst {
		t.Fatal("should NOT be globally created first time")
	}
}

func TestAccountSelfdestructedLocally(t *testing.T) {
	acc := DefaultAccount()
	wasGlobalFirst := acc.MarkSelfdestructedLocally()
	if !wasGlobalFirst {
		t.Fatal("should be first global selfdestruct")
	}
	if !acc.IsSelfdestructed() {
		t.Fatal("should be selfdestructed")
	}
	if !acc.IsSelfdestructedLocally() {
		t.Fatal("should be locally selfdestructed")
	}

	wasGlobalFirst = acc.MarkSelfdestructedLocally()
	if wasGlobalFirst {
		t.Fatal("should NOT be first global")
	}
}

func TestStateClearAwareIsEmpty(t *testing.T) {
	acc := DefaultAccount()
	// Post-spurious dragon: uses IsEmpty
	if !acc.StateClearAwareIsEmpty(spec.Shanghai) {
		t.Fatal("empty account should be empty post-spurious-dragon")
	}
	acc.Info.Balance = u256(1)
	if acc.StateClearAwareIsEmpty(spec.Shanghai) {
		t.Fatal("account with balance should not be empty")
	}

	// Pre-spurious dragon: uses not-existing-not-touched
	acc2 := NewAccountNotExisting(0)
	if !acc2.StateClearAwareIsEmpty(spec.Homestead) {
		t.Fatal("not-existing account should be empty pre-spurious-dragon")
	}
	acc2.MarkTouch()
	if acc2.StateClearAwareIsEmpty(spec.Homestead) {
		t.Fatal("touched not-existing account should not be empty pre-spurious-dragon")
	}
}

func TestEvmStorageSlotBasic(t *testing.T) {
	slot := NewEvmStorageSlot(u256(42), 0)
	if slot.OriginalValue != u256(42) {
		t.Fatal("original should be 42")
	}
	if slot.PresentValue != u256(42) {
		t.Fatal("present should be 42")
	}
	if slot.IsChanged() {
		t.Fatal("unchanged slot should not be changed")
	}

	changed := NewEvmStorageSlotChanged(u256(10), u256(20), 0)
	if !changed.IsChanged() {
		t.Fatal("changed slot should be changed")
	}
}

func TestEvmStorageSlotWarmCold(t *testing.T) {
	slot := NewEvmStorageSlot(types.U256Zero, 0)
	slot.IsCold = true
	slot.TransactionID = 0
	if !slot.MarkWarmWithTransactionID(1) {
		t.Fatal("should be cold (different tx id)")
	}

	slot.IsCold = false
	slot.TransactionID = 0
	if !slot.MarkWarmWithTransactionID(1) {
		t.Fatal("should be cold (different tx id)")
	}

	slot.IsCold = true
	slot.TransactionID = 1
	if !slot.MarkWarmWithTransactionID(1) {
		t.Fatal("should be cold (is_cold flag)")
	}

	slot.IsCold = false
	slot.TransactionID = 1
	if slot.MarkWarmWithTransactionID(1) {
		t.Fatal("should be warm (same tx id, not cold)")
	}
}

func TestEvmStorageSlotOriginalResetOnWarm(t *testing.T) {
	slot := NewEvmStorageSlotChanged(u256(100), u256(200), 0)
	// When warming from cold, original should be reset to present
	slot.MarkCold()
	slot.MarkWarmWithTransactionID(1)
	if slot.OriginalValue != u256(200) {
		t.Fatalf("original should be reset to present (200), got %v", slot.OriginalValue)
	}
}

func TestNewAccountFromInfo(t *testing.T) {
	info := AccountInfo{
		Balance:  u256(1000),
		Nonce:    5,
		CodeHash: types.KeccakEmpty,
	}
	acc := NewAccountFromInfo(info)
	if acc.Info.Balance != u256(1000) {
		t.Fatal("balance mismatch")
	}
	if acc.Info.Nonce != 5 {
		t.Fatal("nonce mismatch")
	}
	if acc.OriginalInfo.Balance != u256(1000) {
		t.Fatal("original info balance mismatch")
	}
	if acc.Status != 0 {
		t.Fatal("status should be empty")
	}
}

func TestNewAccountNotExisting(t *testing.T) {
	acc := NewAccountNotExisting(3)
	if !acc.IsLoadedAsNotExisting() {
		t.Fatal("should be marked as not existing")
	}
	if acc.TransactionID != 3 {
		t.Fatal("transaction ID mismatch")
	}
	if !acc.IsEmpty() {
		t.Fatal("not-existing account should be empty")
	}
}
