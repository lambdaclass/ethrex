package state

import (
	"testing"

	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
)

// mockDB is a simple in-memory database for testing.
type mockDB struct {
	accounts map[types.Address]*AccountInfo
	storage  map[types.Address]map[types.Uint256]types.Uint256
}

func newMockDB() *mockDB {
	return &mockDB{
		accounts: make(map[types.Address]*AccountInfo),
		storage:  make(map[types.Address]map[types.Uint256]types.Uint256),
	}
}

func (db *mockDB) Basic(address types.Address) (AccountInfo, bool, error) {
	info, ok := db.accounts[address]
	if !ok {
		return AccountInfo{}, false, nil
	}
	return *info, true, nil
}

func (db *mockDB) CodeByHash(codeHash types.B256) (types.Bytes, error) {
	return nil, nil
}

func (db *mockDB) Storage(address types.Address, index types.Uint256) (types.Uint256, error) {
	if slots, ok := db.storage[address]; ok {
		if val, found := slots[index]; found {
			return val, nil
		}
	}
	return types.U256Zero, nil
}

func (db *mockDB) HasStorage(address types.Address) (bool, error) {
	if slots, ok := db.storage[address]; ok {
		for _, v := range slots {
			if !v.IsZero() {
				return true, nil
			}
		}
	}
	return false, nil
}

func (db *mockDB) BlockHash(number uint64) (types.B256, error) {
	return types.B256Zero, nil
}

func addr(b byte) types.Address {
	var a types.Address
	a[19] = b
	return a
}

func TestJournalCheckpointRevert(t *testing.T) {
	db := newMockDB()
	j := NewJournal(db)
	j.SetForkID(spec.Shanghai)

	// Pre-load two accounts
	a1 := addr(1)
	a2 := addr(2)
	j.State[a1] = NewAccountFromInfo(AccountInfo{
		Balance:  u256(1000),
		CodeHash: types.KeccakEmpty,
	})
	j.State[a1].MarkWarmWithTransactionID(0)
	j.State[a2] = NewAccountFromInfo(AccountInfo{
		Balance:  u256(500),
		CodeHash: types.KeccakEmpty,
	})
	j.State[a2].MarkWarmWithTransactionID(0)

	// Create checkpoint
	cp := j.Checkpoint()

	// Transfer 100 from a1 to a2
	err := j.TransferLoaded(a1, a2, u256(100))
	if err != nil {
		t.Fatalf("transfer should succeed, got %v", err)
	}

	if j.State[a1].Info.Balance != u256(900) {
		t.Fatalf("a1 balance should be 900, got %v", j.State[a1].Info.Balance)
	}
	if j.State[a2].Info.Balance != u256(600) {
		t.Fatalf("a2 balance should be 600, got %v", j.State[a2].Info.Balance)
	}

	// Revert
	j.CheckpointRevert(cp)

	if j.State[a1].Info.Balance != u256(1000) {
		t.Fatalf("a1 balance should be reverted to 1000, got %v", j.State[a1].Info.Balance)
	}
	if j.State[a2].Info.Balance != u256(500) {
		t.Fatalf("a2 balance should be reverted to 500, got %v", j.State[a2].Info.Balance)
	}
}

func TestJournalTransferOutOfFunds(t *testing.T) {
	db := newMockDB()
	j := NewJournal(db)
	j.SetForkID(spec.Shanghai)

	a1 := addr(1)
	a2 := addr(2)
	j.State[a1] = NewAccountFromInfo(AccountInfo{
		Balance:  u256(50),
		CodeHash: types.KeccakEmpty,
	})
	j.State[a1].MarkWarmWithTransactionID(0)
	j.State[a2] = NewAccountFromInfo(AccountInfo{
		CodeHash: types.KeccakEmpty,
	})
	j.State[a2].MarkWarmWithTransactionID(0)

	err := j.TransferLoaded(a1, a2, u256(100))
	if err == nil || *err != TransferErrorOutOfFunds {
		t.Fatal("should get OutOfFunds error")
	}
}

func TestJournalTransferSelfTransfer(t *testing.T) {
	db := newMockDB()
	j := NewJournal(db)
	j.SetForkID(spec.Shanghai)

	a1 := addr(1)
	j.State[a1] = NewAccountFromInfo(AccountInfo{
		Balance:  u256(1000),
		CodeHash: types.KeccakEmpty,
	})
	j.State[a1].MarkWarmWithTransactionID(0)

	err := j.TransferLoaded(a1, a1, u256(100))
	if err != nil {
		t.Fatal("self-transfer should succeed when balance is sufficient")
	}
	// Balance should be unchanged
	if j.State[a1].Info.Balance != u256(1000) {
		t.Fatal("self-transfer should not change balance")
	}
}

func TestJournalSloadSstore(t *testing.T) {
	db := newMockDB()
	a1 := addr(1)
	db.storage[a1] = map[types.Uint256]types.Uint256{
		u256(1): u256(42),
	}

	j := NewJournal(db)
	j.SetForkID(spec.Shanghai)

	// Load account first
	j.State[a1] = NewAccountFromInfo(AccountInfo{
		Balance:  u256(1000),
		CodeHash: types.KeccakEmpty,
	})
	j.State[a1].MarkWarmWithTransactionID(0)

	// SLoad - should get value from DB
	result, err := j.SLoad(a1, u256(1))
	if err != nil {
		t.Fatalf("sload failed: %v", err)
	}
	if result.Data != u256(42) {
		t.Fatalf("expected 42, got %v", result.Data)
	}
	if !result.IsCold {
		t.Fatal("first sload should be cold")
	}

	// Second SLoad - should be warm
	result2, err := j.SLoad(a1, u256(1))
	if err != nil {
		t.Fatalf("sload failed: %v", err)
	}
	if result2.Data != u256(42) {
		t.Fatalf("expected 42, got %v", result2.Data)
	}
	if result2.IsCold {
		t.Fatal("second sload should be warm")
	}

	// SStore
	cp := j.Checkpoint()
	storeResult, err := j.SStore(a1, u256(1), u256(99))
	if err != nil {
		t.Fatalf("sstore failed: %v", err)
	}
	if storeResult.Data.OriginalValue != u256(42) {
		t.Fatalf("original should be 42, got %v", storeResult.Data.OriginalValue)
	}
	if storeResult.Data.PresentValue != u256(42) {
		t.Fatalf("present should be 42, got %v", storeResult.Data.PresentValue)
	}
	if storeResult.Data.NewValue != u256(99) {
		t.Fatalf("new should be 99, got %v", storeResult.Data.NewValue)
	}

	// Verify the slot was updated
	slot := j.State[a1].Storage[u256(1)]
	if slot.PresentValue != u256(99) {
		t.Fatalf("slot present value should be 99, got %v", slot.PresentValue)
	}

	// Revert
	j.CheckpointRevert(cp)
	slot = j.State[a1].Storage[u256(1)]
	if slot.PresentValue != u256(42) {
		t.Fatalf("slot should be reverted to 42, got %v", slot.PresentValue)
	}
}

func TestJournalTransientStorage(t *testing.T) {
	j := NewJournal(nil)
	j.SetForkID(spec.Cancun)

	a1 := addr(1)

	// TLoad of empty returns zero
	val := j.TLoad(a1, u256(1))
	if val != types.U256Zero {
		t.Fatal("empty tload should return zero")
	}

	// TStore
	j.TStore(a1, u256(1), u256(42))
	val = j.TLoad(a1, u256(1))
	if val != u256(42) {
		t.Fatalf("expected 42, got %v", val)
	}

	// TStore same value - no journal entry
	entriesBefore := len(j.Entries)
	j.TStore(a1, u256(1), u256(42))
	if len(j.Entries) != entriesBefore {
		t.Fatal("storing same value should not add journal entry")
	}

	// TStore different value - journal entry
	j.TStore(a1, u256(1), u256(99))
	if len(j.Entries) != entriesBefore+1 {
		t.Fatal("storing different value should add journal entry")
	}
	val = j.TLoad(a1, u256(1))
	if val != u256(99) {
		t.Fatalf("expected 99, got %v", val)
	}

	// TStore zero - removes entry
	j.TStore(a1, u256(1), types.U256Zero)
	val = j.TLoad(a1, u256(1))
	if val != types.U256Zero {
		t.Fatal("should be zero after storing zero")
	}
}

func TestJournalTransientStorageRevert(t *testing.T) {
	j := NewJournal(nil)
	j.SetForkID(spec.Cancun)

	a1 := addr(1)

	j.TStore(a1, u256(1), u256(42))
	cp := j.Checkpoint()
	j.TStore(a1, u256(1), u256(99))

	if j.TLoad(a1, u256(1)) != u256(99) {
		t.Fatal("should be 99 before revert")
	}

	j.CheckpointRevert(cp)
	if j.TLoad(a1, u256(1)) != u256(42) {
		t.Fatal("should be reverted to 42")
	}
}

func TestJournalLogs(t *testing.T) {
	j := NewJournal(nil)

	j.AppendLog(Log{Address: addr(1)})
	j.AppendLog(Log{Address: addr(2)})

	cp := j.Checkpoint()
	j.AppendLog(Log{Address: addr(3)})

	if len(j.Logs) != 3 {
		t.Fatal("should have 3 logs")
	}

	j.CheckpointRevert(cp)
	if len(j.Logs) != 2 {
		t.Fatal("should have 2 logs after revert")
	}
}

func TestJournalAccountWarmCold(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(1)] = &AccountInfo{
		Balance:  u256(1000),
		CodeHash: types.KeccakEmpty,
	}

	j := NewJournal(db)
	j.SetForkID(spec.Shanghai)

	// First load should be cold
	result, err := j.LoadAccount(addr(1))
	if err != nil {
		t.Fatalf("load failed: %v", err)
	}
	if !result.IsCold {
		t.Fatal("first load should be cold")
	}
	if result.Data.Info.Balance != u256(1000) {
		t.Fatalf("expected balance 1000, got %v", result.Data.Info.Balance)
	}

	// Second load should be warm
	result2, err := j.LoadAccount(addr(1))
	if err != nil {
		t.Fatalf("load failed: %v", err)
	}
	if result2.IsCold {
		t.Fatal("second load should be warm")
	}
}

func TestJournalAccountNotExisting(t *testing.T) {
	db := newMockDB()
	j := NewJournal(db)
	j.SetForkID(spec.Shanghai)

	result, err := j.LoadAccount(addr(99))
	if err != nil {
		t.Fatalf("load failed: %v", err)
	}
	if !result.Data.IsLoadedAsNotExisting() {
		t.Fatal("account should be marked as not existing")
	}
}

func TestJournalNonceRevert(t *testing.T) {
	j := NewJournal(nil)
	j.SetForkID(spec.Shanghai)

	a1 := addr(1)
	j.State[a1] = NewAccountFromInfo(AccountInfo{
		Balance:  u256(1000),
		Nonce:    5,
		CodeHash: types.KeccakEmpty,
	})
	j.State[a1].MarkWarmWithTransactionID(0)

	cp := j.Checkpoint()

	// Manually record a nonce bump
	j.State[a1].Info.Nonce = 6
	j.Entries = append(j.Entries, JournalEntryNonceBump(a1))

	if j.State[a1].Info.Nonce != 6 {
		t.Fatal("nonce should be 6")
	}

	j.CheckpointRevert(cp)
	if j.State[a1].Info.Nonce != 5 {
		t.Fatalf("nonce should be reverted to 5, got %d", j.State[a1].Info.Nonce)
	}
}

func TestJournalNonceChangeRevert(t *testing.T) {
	j := NewJournal(nil)
	j.SetForkID(spec.Shanghai)

	a1 := addr(1)
	j.State[a1] = NewAccountFromInfo(AccountInfo{
		Balance:  u256(1000),
		Nonce:    5,
		CodeHash: types.KeccakEmpty,
	})
	j.State[a1].MarkWarmWithTransactionID(0)

	cp := j.Checkpoint()

	j.State[a1].Info.Nonce = 10
	j.Entries = append(j.Entries, JournalEntryNonceChange(a1, 5))

	j.CheckpointRevert(cp)
	if j.State[a1].Info.Nonce != 5 {
		t.Fatalf("nonce should be reverted to 5, got %d", j.State[a1].Info.Nonce)
	}
}

func TestJournalBalanceChangeRevert(t *testing.T) {
	j := NewJournal(nil)
	j.SetForkID(spec.Shanghai)

	a1 := addr(1)
	j.State[a1] = NewAccountFromInfo(AccountInfo{
		Balance:  u256(1000),
		CodeHash: types.KeccakEmpty,
	})
	j.State[a1].MarkWarmWithTransactionID(0)

	cp := j.Checkpoint()

	oldBalance := j.State[a1].Info.Balance
	j.State[a1].Info.Balance = u256(2000)
	j.Entries = append(j.Entries, JournalEntryBalanceChange(a1, oldBalance))

	j.CheckpointRevert(cp)
	if j.State[a1].Info.Balance != u256(1000) {
		t.Fatalf("balance should be reverted to 1000, got %v", j.State[a1].Info.Balance)
	}
}

func TestJournalCodeChangeRevert(t *testing.T) {
	j := NewJournal(nil)
	j.SetForkID(spec.Shanghai)

	a1 := addr(1)
	j.State[a1] = NewAccountFromInfo(AccountInfo{
		CodeHash: types.KeccakEmpty,
	})
	j.State[a1].MarkWarmWithTransactionID(0)

	cp := j.Checkpoint()

	j.SetCodeWithHash(a1, types.Bytes{0x60, 0x00}, types.B256{0xAB})

	if j.State[a1].Info.CodeHash != (types.B256{0xAB}) {
		t.Fatal("code hash should be set")
	}

	j.CheckpointRevert(cp)
	if j.State[a1].Info.CodeHash != types.KeccakEmpty {
		t.Fatal("code hash should be reverted to KECCAK_EMPTY")
	}
	if j.State[a1].Info.Code != nil {
		t.Fatal("code should be nil after revert")
	}
}

func TestJournalAccountCreatedRevert(t *testing.T) {
	j := NewJournal(nil)
	j.SetForkID(spec.Shanghai)

	a1 := addr(1)
	j.State[a1] = NewAccountFromInfo(AccountInfo{
		CodeHash: types.KeccakEmpty,
	})
	j.State[a1].MarkWarmWithTransactionID(0)

	cp := j.Checkpoint()

	isGlobal := j.State[a1].MarkCreatedLocally()
	j.Entries = append(j.Entries, JournalEntryAccountCreated(a1, isGlobal))
	j.State[a1].Info.Nonce = 1

	if !j.State[a1].IsCreated() {
		t.Fatal("should be created")
	}
	if !j.State[a1].IsCreatedLocally() {
		t.Fatal("should be created locally")
	}

	j.CheckpointRevert(cp)
	if j.State[a1].IsCreated() {
		t.Fatal("should not be created after revert")
	}
	if j.State[a1].IsCreatedLocally() {
		t.Fatal("should not be created locally after revert")
	}
	if j.State[a1].Info.Nonce != 0 {
		t.Fatal("nonce should be 0 after revert")
	}
}

func TestJournalCommitTx(t *testing.T) {
	j := NewJournal(nil)
	j.SetForkID(spec.Shanghai)

	a1 := addr(1)
	j.State[a1] = NewAccountFromInfo(AccountInfo{
		Balance:  u256(1000),
		CodeHash: types.KeccakEmpty,
	})

	j.AppendLog(Log{Address: a1})
	j.TStore(a1, u256(1), u256(42))
	j.Entries = append(j.Entries, JournalEntryAccountTouched(a1))

	j.CommitTx()

	if len(j.Logs) != 0 {
		t.Fatal("logs should be cleared")
	}
	if len(j.Entries) != 0 {
		t.Fatal("journal entries should be cleared")
	}
	if j.TLoad(a1, u256(1)) != types.U256Zero {
		t.Fatal("transient storage should be cleared")
	}
	if j.TransactionID != 1 {
		t.Fatal("transaction ID should be incremented")
	}
	// State should be preserved
	if j.State[a1].Info.Balance != u256(1000) {
		t.Fatal("state should be preserved across commit")
	}
}

func TestJournalDiscardTx(t *testing.T) {
	j := NewJournal(nil)
	j.SetForkID(spec.Shanghai)

	a1 := addr(1)
	a2 := addr(2)
	j.State[a1] = NewAccountFromInfo(AccountInfo{
		Balance:  u256(1000),
		CodeHash: types.KeccakEmpty,
	})
	j.State[a1].MarkWarmWithTransactionID(0)
	j.State[a2] = NewAccountFromInfo(AccountInfo{
		Balance:  u256(500),
		CodeHash: types.KeccakEmpty,
	})
	j.State[a2].MarkWarmWithTransactionID(0)

	// Do a transfer
	j.TransferLoaded(a1, a2, u256(100))

	if j.State[a1].Info.Balance != u256(900) {
		t.Fatal("a1 should be 900 after transfer")
	}

	// Discard should revert everything
	j.DiscardTx()

	if j.State[a1].Info.Balance != u256(1000) {
		t.Fatalf("a1 should be reverted to 1000, got %v", j.State[a1].Info.Balance)
	}
	if j.State[a2].Info.Balance != u256(500) {
		t.Fatalf("a2 should be reverted to 500, got %v", j.State[a2].Info.Balance)
	}
	if j.TransactionID != 1 {
		t.Fatal("transaction ID should be incremented on discard")
	}
}

func TestJournalFinalize(t *testing.T) {
	j := NewJournal(nil)
	j.SetForkID(spec.Shanghai)

	a1 := addr(1)
	j.State[a1] = NewAccountFromInfo(AccountInfo{
		Balance:  u256(1000),
		CodeHash: types.KeccakEmpty,
	})

	state := j.Finalize()
	if state[a1].Info.Balance != u256(1000) {
		t.Fatal("finalized state should have account")
	}
	if len(j.State) != 0 {
		t.Fatal("journal state should be cleared after finalize")
	}
	if j.TransactionID != 0 {
		t.Fatal("transaction ID should be reset")
	}
}

func TestJournalDepth(t *testing.T) {
	j := NewJournal(nil)
	if j.Depth != 0 {
		t.Fatal("initial depth should be 0")
	}

	cp1 := j.Checkpoint()
	if j.Depth != 1 {
		t.Fatal("depth should be 1")
	}

	cp2 := j.Checkpoint()
	if j.Depth != 2 {
		t.Fatal("depth should be 2")
	}

	j.CheckpointCommit()
	if j.Depth != 1 {
		t.Fatal("depth should be 1 after commit")
	}

	j.CheckpointRevert(cp1)
	_ = cp2
	if j.Depth != 0 {
		t.Fatal("depth should be 0 after revert")
	}
}

func TestJournalSStoreNoChangeNoEntry(t *testing.T) {
	db := newMockDB()
	a1 := addr(1)
	db.storage[a1] = map[types.Uint256]types.Uint256{
		u256(1): u256(42),
	}

	j := NewJournal(db)
	j.SetForkID(spec.Shanghai)
	j.State[a1] = NewAccountFromInfo(AccountInfo{
		Balance:  u256(1000),
		CodeHash: types.KeccakEmpty,
	})
	j.State[a1].MarkWarmWithTransactionID(0)

	// First SLoad to populate
	j.SLoad(a1, u256(1))

	entriesBefore := len(j.Entries)

	// SStore same value
	j.SStore(a1, u256(1), u256(42))

	// Should only have the touch entry, no StorageChanged entry for same value
	storageChanges := 0
	for i := entriesBefore; i < len(j.Entries); i++ {
		if j.Entries[i].Kind == JournalStorageChanged {
			storageChanges++
		}
	}
	if storageChanges != 0 {
		t.Fatal("storing same value should not create StorageChanged entry")
	}
}

func TestJournalWarmAddresses(t *testing.T) {
	j := NewJournal(nil)
	j.SetForkID(spec.Shanghai)

	// Set coinbase warm
	coinbase := addr(0xCB)
	j.WarmAddresses.SetCoinbase(coinbase)

	if !j.WarmAddresses.IsWarm(coinbase) {
		t.Fatal("coinbase should be warm")
	}

	// Set access list
	accessList := map[types.Address]map[types.Uint256]struct{}{
		addr(0xAA): {u256(1): {}, u256(2): {}},
	}
	j.WarmAddresses.SetAccessList(accessList)

	if !j.WarmAddresses.IsWarm(addr(0xAA)) {
		t.Fatal("access list address should be warm")
	}
	if !j.WarmAddresses.IsStorageWarm(addr(0xAA), u256(1)) {
		t.Fatal("access list storage key should be warm")
	}
	if j.WarmAddresses.IsStorageWarm(addr(0xAA), u256(3)) {
		t.Fatal("non-access-list storage key should be cold")
	}

	// Clear on commit
	j.WarmAddresses.ClearCoinbaseAndAccessList()
	if j.WarmAddresses.IsWarm(coinbase) {
		t.Fatal("coinbase should be cold after clear")
	}
	if j.WarmAddresses.IsWarm(addr(0xAA)) {
		t.Fatal("access list should be cold after clear")
	}
}

func TestJournalPrecompileWarm(t *testing.T) {
	j := NewJournal(nil)

	precompiles := map[types.Address]struct{}{
		addr(1): {},
		addr(2): {},
		addr(3): {},
	}
	j.WarmAddresses.SetPrecompileAddresses(precompiles)

	if !j.WarmAddresses.IsWarm(addr(1)) {
		t.Fatal("precompile 1 should be warm")
	}
	if !j.WarmAddresses.IsWarm(addr(3)) {
		t.Fatal("precompile 3 should be warm")
	}
	if j.WarmAddresses.IsWarm(addr(4)) {
		t.Fatal("non-precompile should be cold")
	}
}
