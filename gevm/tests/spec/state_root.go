// State root computation: merges pre-state (MemDB) with journal changes
// and computes the Merkle Patricia Trie root hash.
package spec

import (
	"github.com/Giulio2002/gevm/types"
	gevmspec "github.com/Giulio2002/gevm/spec"
	"github.com/Giulio2002/gevm/state"
)

// accountForRoot holds the final state of an account for root computation.
type accountForRoot struct {
	nonce    uint64
	balance  types.Uint256
	codeHash types.B256
	storage  map[types.Uint256]types.Uint256 // only non-zero slots
}

// StateRoot computes the post-execution state root hash.
func StateRoot(db *MemDB, journal *state.Journal, forkID gevmspec.ForkID) types.B256 {
	accounts := collectAccounts(db, journal, forkID)
	if len(accounts) == 0 {
		return emptyTrieRoot
	}

	// Build state trie entries.
	entries := make([]mptEntry, 0, len(accounts))
	for addr, acc := range accounts {
		sRoot := storageRoot(acc.storage)
		value := rlpEncodeAccount(acc.nonce, acc.balance, sRoot, acc.codeHash)
		addrHash := types.Keccak256(addr[:])
		entries = append(entries, mptEntry{
			keyNibbles: keyToNibbles(addrHash),
			value:      value,
		})
	}

	return mptRoot(entries)
}

// collectAccounts merges pre-state (MemDB) with journal changes.
func collectAccounts(db *MemDB, journal *state.Journal, forkID gevmspec.ForkID) map[types.Address]*accountForRoot {
	result := make(map[types.Address]*accountForRoot)

	// Step 1: Load all pre-state accounts.
	db.ForEachAccount(func(addr types.Address, info state.AccountInfo, storage map[types.Uint256]types.Uint256) {
		st := make(map[types.Uint256]types.Uint256, len(storage))
		for k, v := range storage {
			if !v.IsZero() {
				st[k] = v
			}
		}
		result[addr] = &accountForRoot{
			nonce:    info.Nonce,
			balance:  info.Balance,
			codeHash: info.CodeHash,
			storage:  st,
		}
	})

	// Step 2: Overlay journal state.
	for addr, acc := range journal.State {
		// Check EIP-161 emptiness: if the account is empty per fork rules, remove it.
		// This handles both pre-state accounts that became empty and
		// accounts loaded-as-not-existing that were never meaningfully touched.
		if acc.StateClearAwareIsEmpty(forkID) {
			delete(result, addr)
			continue
		}

		// Build final account state from journal.
		afr := &accountForRoot{
			nonce:    acc.Info.Nonce,
			balance:  acc.Info.Balance,
			codeHash: acc.Info.CodeHash,
			storage:  make(map[types.Uint256]types.Uint256),
		}

		// Start with pre-state storage (if any).
		if pre, ok := result[addr]; ok {
			for k, v := range pre.storage {
				afr.storage[k] = v
			}
		}

		// Overlay journal storage changes.
		if acc.Storage != nil {
			for slot, slotVal := range acc.Storage {
				if slotVal.PresentValue.IsZero() {
					delete(afr.storage, slot)
				} else {
					afr.storage[slot] = slotVal.PresentValue
				}
			}
		}

		result[addr] = afr
	}

	return result
}

// storageRoot computes the MPT root of an account's storage trie.
func storageRoot(storage map[types.Uint256]types.Uint256) types.B256 {
	if len(storage) == 0 {
		return emptyTrieRoot
	}

	entries := make([]mptEntry, 0, len(storage))
	for slot, value := range storage {
		slotBytes := slot.ToBytes32()
		slotHash := types.Keccak256(slotBytes[:])
		entries = append(entries, mptEntry{
			keyNibbles: keyToNibbles(slotHash),
			value:      RlpEncodeU256(value),
		})
	}

	return mptRoot(entries)
}

// rlpEncodeAccount encodes [nonce, balance, storageRoot, codeHash] as RLP.
func rlpEncodeAccount(nonce uint64, balance types.Uint256, sRoot, codeHash types.B256) []byte {
	return RlpEncodeList([][]byte{
		RlpEncodeUint64(nonce),
		RlpEncodeU256(balance),
		RlpEncodeBytes(sRoot[:]),
		RlpEncodeBytes(codeHash[:]),
	})
}
