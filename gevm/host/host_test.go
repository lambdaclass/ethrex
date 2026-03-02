package host

import (
	"testing"

	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
	"github.com/Giulio2002/gevm/state"
	"github.com/Giulio2002/gevm/vm"
)

// mockDB for testing.
type mockDB struct {
	accounts map[types.Address]*state.AccountInfo
	storage  map[types.Address]map[types.Uint256]types.Uint256
}

func newMockDB() *mockDB {
	return &mockDB{
		accounts: make(map[types.Address]*state.AccountInfo),
		storage:  make(map[types.Address]map[types.Uint256]types.Uint256),
	}
}

func (db *mockDB) Basic(address types.Address) (state.AccountInfo, bool, error) {
	info, ok := db.accounts[address]
	if !ok {
		return state.AccountInfo{}, false, nil
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
	return types.B256{byte(number)}, nil
}

func u(v uint64) types.Uint256 { return types.U256From(v) }

func addr(b byte) types.Address {
	var a types.Address
	a[19] = b
	return a
}

func TestEvmHostBlockInfo(t *testing.T) {
	journal := state.NewJournal(nil)
	host := NewEvmHost(journal, &BlockEnv{
		Beneficiary:  addr(0xCB),
		Timestamp:    u(1000),
		Number:       u(42),
		Difficulty:   u(0),
		GasLimit:     u(30000000),
		BaseFee:      u(100),
		BlobGasPrice: u(1),
		SlotNum:      u(7),
	}, TxEnv{
		Caller:            addr(0xCA),
		EffectiveGasPrice: u(200),
		BlobHashes:        []types.Uint256{u(0xAA), u(0xBB)},
	}, &CfgEnv{
		ChainId: u(1),
	})

	if host.Beneficiary() != addr(0xCB) {
		t.Fatal("wrong beneficiary")
	}
	if host.Timestamp() != u(1000) {
		t.Fatal("wrong timestamp")
	}
	if host.BlockNumber() != u(42) {
		t.Fatal("wrong block number")
	}
	if host.GasLimit() != u(30000000) {
		t.Fatal("wrong gas limit")
	}
	if host.BaseFee() != u(100) {
		t.Fatal("wrong base fee")
	}
	if host.ChainId() != u(1) {
		t.Fatal("wrong chain id")
	}
	if host.Caller() != addr(0xCA) {
		t.Fatal("wrong caller")
	}
	if host.EffectiveGasPrice() != u(200) {
		t.Fatal("wrong effective gas price")
	}

	bh := host.BlobHash(0)
	if bh == nil || *bh != u(0xAA) {
		t.Fatal("wrong blob hash 0")
	}
	bh = host.BlobHash(1)
	if bh == nil || *bh != u(0xBB) {
		t.Fatal("wrong blob hash 1")
	}
	bh = host.BlobHash(2)
	if bh != nil {
		t.Fatal("out of bounds blob hash should be nil")
	}
}

func TestEvmHostBalanceSLoad(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(1)] = &state.AccountInfo{
		Balance:  u(1000),
		CodeHash: types.KeccakEmpty,
	}
	db.storage[addr(1)] = map[types.Uint256]types.Uint256{
		u(10): u(42),
	}

	journal := state.NewJournal(db)
	journal.SetForkID(spec.Shanghai)
	host := NewEvmHost(journal, &BlockEnv{}, TxEnv{}, &CfgEnv{})

	// Balance
	bal, cold := host.Balance(addr(1))
	if bal != u(1000) {
		t.Fatalf("expected balance 1000, got %v", bal)
	}
	if !cold {
		t.Fatal("first access should be cold")
	}

	// Second access should be warm
	bal, cold = host.Balance(addr(1))
	if bal != u(1000) {
		t.Fatal("balance should still be 1000")
	}
	if cold {
		t.Fatal("second access should be warm")
	}

	// SLoad via SLoadInto
	sloadKey := u(10)
	var val types.Uint256
	cold = host.SLoadInto(addr(1), &sloadKey, &val)
	if val != u(42) {
		t.Fatalf("expected storage 42, got %v", val)
	}
	if !cold {
		t.Fatal("first sload should be cold")
	}

	sloadKey = u(10)
	cold = host.SLoadInto(addr(1), &sloadKey, &val)
	if val != u(42) {
		t.Fatal("sload should still be 42")
	}
	if cold {
		t.Fatal("second sload should be warm")
	}
}

func TestEvmHostSStore(t *testing.T) {
	db := newMockDB()
	db.accounts[addr(1)] = &state.AccountInfo{
		Balance:  u(1000),
		CodeHash: types.KeccakEmpty,
	}
	db.storage[addr(1)] = map[types.Uint256]types.Uint256{
		u(10): u(42),
	}

	journal := state.NewJournal(db)
	journal.SetForkID(spec.Shanghai)
	host := NewEvmHost(journal, &BlockEnv{}, TxEnv{}, &CfgEnv{})

	// First load the account to make it warm
	host.Balance(addr(1))

	// SStore
	var result vm.SStoreResult
	sstoreKey := u(10)
	sstoreVal := u(99)
	host.SStore(addr(1), &sstoreKey, &sstoreVal, &result)
	if result.OriginalValue != u(42) {
		t.Fatalf("original should be 42, got %v", result.OriginalValue)
	}
	if result.PresentValue != u(42) {
		t.Fatalf("present should be 42, got %v", result.PresentValue)
	}
	if result.NewValue != u(99) {
		t.Fatalf("new should be 99, got %v", result.NewValue)
	}

	// Verify stored value
	verifyKey := u(10)
	var storedVal types.Uint256
	host.SLoadInto(addr(1), &verifyKey, &storedVal)
	if storedVal != u(99) {
		t.Fatalf("stored value should be 99, got %v", storedVal)
	}
}

func TestEvmHostTransientStorage(t *testing.T) {
	journal := state.NewJournal(nil)
	journal.SetForkID(spec.Cancun)
	host := NewEvmHost(journal, &BlockEnv{}, TxEnv{}, &CfgEnv{})

	val := host.TLoad(addr(1), u(5))
	if val != types.U256Zero {
		t.Fatal("empty tload should be zero")
	}

	host.TStore(addr(1), u(5), u(42))
	val = host.TLoad(addr(1), u(5))
	if val != u(42) {
		t.Fatalf("expected 42, got %v", val)
	}
}

func TestEvmHostBlockHash(t *testing.T) {
	db := newMockDB()
	journal := state.NewJournal(db)
	// Block.Number must be > target and within 256-block window.
	block := &BlockEnv{Number: u(10)}
	host := NewEvmHost(journal, block, TxEnv{}, &CfgEnv{})

	hash := host.BlockHash(u(5))
	expected := types.B256{5}
	if hash != expected {
		t.Fatalf("expected block hash %v, got %v", expected, hash)
	}

	// Block hash of current block returns zero.
	if h := host.BlockHash(u(10)); h != types.B256Zero {
		t.Fatalf("expected zero for current block, got %v", h)
	}
	// Block hash beyond 256-block window returns zero.
	if h := host.BlockHash(u(0)); h != types.B256Zero {
		t.Fatalf("expected zero for out-of-window block, got %v", h)
	}
}

func TestEvmHostCodeHash(t *testing.T) {
	db := newMockDB()
	// Account with non-empty code hash
	codeHash := types.B256{0xAB, 0xCD}
	db.accounts[addr(1)] = &state.AccountInfo{
		Balance:  u(100),
		Nonce:    1,
		CodeHash: codeHash,
	}
	// Empty account (not in DB)

	journal := state.NewJournal(db)
	journal.SetForkID(spec.Shanghai)
	host := NewEvmHost(journal, &BlockEnv{}, TxEnv{}, &CfgEnv{})

	// Non-empty account: should return its code hash
	hash, cold := host.CodeHash(addr(1))
	if hash != codeHash {
		t.Fatalf("expected code hash %v, got %v", codeHash, hash)
	}
	if !cold {
		t.Fatal("first access should be cold")
	}

	// Empty account: should return zero hash
	hash, _ = host.CodeHash(addr(99))
	if hash != types.B256Zero {
		t.Fatalf("empty account should have zero code hash, got %v", hash)
	}
}

func TestEvmHostLog(t *testing.T) {
	journal := state.NewJournal(nil)
	host := NewEvmHost(journal, &BlockEnv{}, TxEnv{}, &CfgEnv{})

	host.Log(addr(1), &[4]types.B256{{0x01}}, 1, types.Bytes{0x42})

	if len(journal.Logs) != 1 {
		t.Fatal("should have 1 log")
	}
	if journal.Logs[0].Address != addr(1) {
		t.Fatal("wrong log address")
	}
	if journal.Logs[0].NumTopics != 1 || journal.Logs[0].Topics[0] != (types.B256{0x01}) {
		t.Fatal("wrong log topic")
	}
	if len(journal.Logs[0].Data) != 1 || journal.Logs[0].Data[0] != 0x42 {
		t.Fatal("wrong log data")
	}
}
