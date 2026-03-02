// In-memory database for Ethereum spec tests.
// Loads pre-state from test fixtures and serves it to the GEVM journal.
package spec

import (
	"fmt"

	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/state"
)

// MemDB is a simple in-memory database for spec tests.
// It stores the pre-state accounts/storage from test fixtures.
type MemDB struct {
	accounts map[types.Address]*memAccount
	blocks   map[uint64]types.B256
}

type memAccount struct {
	info    state.AccountInfo
	storage map[types.Uint256]types.Uint256
}

// NewMemDB creates an empty MemDB.
func NewMemDB() *MemDB {
	return &MemDB{
		accounts: make(map[types.Address]*memAccount),
		blocks:   make(map[uint64]types.B256),
	}
}

// InsertAccount adds an account with its storage to the database.
func (db *MemDB) InsertAccount(addr types.Address, info state.AccountInfo, storage map[types.Uint256]types.Uint256) {
	db.accounts[addr] = &memAccount{
		info:    info,
		storage: storage,
	}
}

// InsertBlockHash adds a block hash mapping.
func (db *MemDB) InsertBlockHash(number uint64, hash types.B256) {
	db.blocks[number] = hash
}

// --- state.Database interface ---

func (db *MemDB) Basic(address types.Address) (state.AccountInfo, bool, error) {
	acc, ok := db.accounts[address]
	if !ok {
		return state.AccountInfo{}, false, nil // account does not exist
	}
	// Return info directly; Code slice is shared (never mutated in-place).
	return acc.info, true, nil
}

func (db *MemDB) CodeByHash(codeHash types.B256) (types.Bytes, error) {
	// Search all accounts for matching code hash
	for _, acc := range db.accounts {
		if acc.info.CodeHash == codeHash && acc.info.Code != nil {
			return acc.info.Code, nil
		}
	}
	return nil, fmt.Errorf("code not found for hash %s", codeHash.Hex())
}

func (db *MemDB) Storage(address types.Address, index types.Uint256) (types.Uint256, error) {
	acc, ok := db.accounts[address]
	if !ok {
		return types.U256Zero, nil
	}
	v, ok := acc.storage[index]
	if !ok {
		return types.U256Zero, nil
	}
	return v, nil
}

func (db *MemDB) HasStorage(address types.Address) (bool, error) {
	acc, ok := db.accounts[address]
	if !ok {
		return false, nil
	}
	for _, v := range acc.storage {
		if !v.IsZero() {
			return true, nil
		}
	}
	return false, nil
}

func (db *MemDB) BlockHash(number uint64) (types.B256, error) {
	hash, ok := db.blocks[number]
	if !ok {
		return types.B256Zero, nil
	}
	return hash, nil
}

// ForEachAccount iterates over all pre-state accounts.
func (db *MemDB) ForEachAccount(fn func(addr types.Address, info state.AccountInfo, storage map[types.Uint256]types.Uint256)) {
	for addr, acc := range db.accounts {
		fn(addr, acc.info, acc.storage)
	}
}

// Compile-time check.
var _ state.Database = (*MemDB)(nil)

// BuildMemDB creates a MemDB from the test unit's pre-state.
func BuildMemDB(pre map[HexAddr]*TestAccountInfo) *MemDB {
	db := NewMemDB()
	for hexAddr, acct := range pre {
		code := make(types.Bytes, len(acct.Code.V))
		copy(code, acct.Code.V)

		codeHash := types.KeccakEmpty
		if len(code) > 0 {
			codeHash = types.Keccak256(code)
		}

		info := state.AccountInfo{
			Balance:  acct.Balance.V,
			Nonce:    acct.Nonce.V,
			CodeHash: codeHash,
			Code:     code,
		}

		storage := make(map[types.Uint256]types.Uint256)
		for hexKey, hexVal := range acct.Storage {
			storage[hexKey.V.ToU256()] = hexVal.V
		}

		db.InsertAccount(hexAddr.V, info, storage)
	}
	return db
}
