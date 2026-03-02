package state

import (
	"github.com/Giulio2002/gevm/types"
)

// SelfdestructionRevertStatus tracks the selfdestruct state for revert purposes.
type SelfdestructionRevertStatus uint8

const (
	// SelfdestructGlobal means selfdestruct was called for the first time globally.
	SelfdestructGlobal SelfdestructionRevertStatus = iota
	// SelfdestructLocal means selfdestruct was called for the first time in this transaction scope.
	SelfdestructLocal
	// SelfdestructRepeated means selfdestruct was called again in this transaction scope.
	SelfdestructRepeated
)

// JournalEntryKind identifies the type of journal entry.
type JournalEntryKind uint8

const (
	JournalAccountWarmed JournalEntryKind = iota
	JournalAccountDestroyed
	JournalAccountTouched
	JournalBalanceChange
	JournalBalanceTransfer
	JournalNonceChange
	JournalNonceBump
	JournalAccountCreated
	JournalStorageChanged
	JournalStorageWarmed
	JournalTransientStorageChange
	JournalCodeChange
)

// JournalEntry records a single state change that can be reverted.
// Uses a tagged-struct approach (Go equivalent of Rust enum).
type JournalEntry struct {
	Kind JournalEntryKind

	// Address is the primary address for this entry.
	// Used by all entry kinds.
	Address types.Address

	// Target is the secondary address.
	// Used by: AccountDestroyed (target), BalanceTransfer (to).
	Target types.Address

	// Balance stores balance-related data.
	// Used by: AccountDestroyed (had_balance), BalanceChange (old_balance),
	//          BalanceTransfer (balance).
	Balance types.Uint256

	// Key stores the storage key.
	// Used by: StorageChanged, StorageWarmed, TransientStorageChange.
	Key types.Uint256

	// HadValue stores the previous storage value.
	// Used by: StorageChanged, TransientStorageChange.
	HadValue types.Uint256

	// PrevNonce stores the previous nonce value.
	// Used by: NonceChange.
	PrevNonce uint64

	// DestroyedStatus tracks selfdestruct revert state.
	// Used by: AccountDestroyed.
	DestroyedStatus SelfdestructionRevertStatus

	// IsCreatedGlobally tracks if account was created globally for the first time.
	// Used by: AccountCreated.
	IsCreatedGlobally bool
}

// --- Constructor functions matching JournalEntryTr trait ---

func JournalEntryAccountWarmed(address types.Address) JournalEntry {
	return JournalEntry{Kind: JournalAccountWarmed, Address: address}
}

func JournalEntryAccountDestroyed(address, target types.Address, status SelfdestructionRevertStatus, hadBalance types.Uint256) JournalEntry {
	return JournalEntry{
		Kind:            JournalAccountDestroyed,
		Address:         address,
		Target:          target,
		DestroyedStatus: status,
		Balance:         hadBalance,
	}
}

func JournalEntryAccountTouched(address types.Address) JournalEntry {
	return JournalEntry{Kind: JournalAccountTouched, Address: address}
}

func JournalEntryBalanceChange(address types.Address, oldBalance types.Uint256) JournalEntry {
	return JournalEntry{Kind: JournalBalanceChange, Address: address, Balance: oldBalance}
}

func JournalEntryBalanceTransfer(from, to types.Address, balance types.Uint256) JournalEntry {
	return JournalEntry{Kind: JournalBalanceTransfer, Address: from, Target: to, Balance: balance}
}

func JournalEntryNonceChange(address types.Address, previousNonce uint64) JournalEntry {
	return JournalEntry{Kind: JournalNonceChange, Address: address, PrevNonce: previousNonce}
}

func JournalEntryNonceBump(address types.Address) JournalEntry {
	return JournalEntry{Kind: JournalNonceBump, Address: address}
}

func JournalEntryAccountCreated(address types.Address, isCreatedGlobally bool) JournalEntry {
	return JournalEntry{Kind: JournalAccountCreated, Address: address, IsCreatedGlobally: isCreatedGlobally}
}

func JournalEntryStorageChanged(address types.Address, key, hadValue types.Uint256) JournalEntry {
	return JournalEntry{Kind: JournalStorageChanged, Address: address, Key: key, HadValue: hadValue}
}

func JournalEntryStorageWarmed(address types.Address, key types.Uint256) JournalEntry {
	return JournalEntry{Kind: JournalStorageWarmed, Address: address, Key: key}
}

func JournalEntryTransientStorageChange(address types.Address, key, hadValue types.Uint256) JournalEntry {
	return JournalEntry{Kind: JournalTransientStorageChange, Address: address, Key: key, HadValue: hadValue}
}

func JournalEntryCodeChange(address types.Address) JournalEntry {
	return JournalEntry{Kind: JournalCodeChange, Address: address}
}

// Revert undoes the state change recorded by this journal entry.
//
// transientStorage may be nil when reverting a whole transaction (transient storage is already cleared).
// isSpuriousDragonEnabled is used to skip reverting the Precompile3 touch (historical EIP-161 bug).
func (e *JournalEntry) Revert(state EvmState, transientStorage TransientStorage, isSpuriousDragonEnabled bool) {
	switch e.Kind {
	case JournalAccountWarmed:
		state[e.Address].MarkCold()

	case JournalAccountTouched:
		if isSpuriousDragonEnabled && e.Address == types.Precompile3 {
			return
		}
		state[e.Address].UnmarkTouch()

	case JournalAccountDestroyed:
		acc := state[e.Address]
		switch e.DestroyedStatus {
		case SelfdestructGlobal:
			acc.UnmarkSelfdestruct()
			acc.UnmarkSelfdestructedLocally()
		case SelfdestructLocal:
			acc.UnmarkSelfdestructedLocally()
		case SelfdestructRepeated:
			// do nothing
		}
		acc.Info.Balance = acc.Info.Balance.Add(e.Balance)
		if e.Address != e.Target {
			target := state[e.Target]
			target.Info.Balance = target.Info.Balance.Sub(e.Balance)
		}

	case JournalBalanceChange:
		state[e.Address].Info.Balance = e.Balance

	case JournalBalanceTransfer:
		from := state[e.Address]
		from.Info.Balance = from.Info.Balance.Add(e.Balance)
		to := state[e.Target]
		to.Info.Balance = to.Info.Balance.Sub(e.Balance)

	case JournalNonceChange:
		state[e.Address].Info.Nonce = e.PrevNonce

	case JournalNonceBump:
		acc := state[e.Address]
		if acc.Info.Nonce > 0 {
			acc.Info.Nonce--
		}

	case JournalAccountCreated:
		acc := state[e.Address]
		acc.UnmarkCreatedLocally()
		if e.IsCreatedGlobally {
			acc.UnmarkCreated()
		}
		acc.Info.Nonce = 0

	case JournalStorageWarmed:
		slot := state[e.Address].Storage[e.Key]
		slot.MarkCold()

	case JournalStorageChanged:
		slot := state[e.Address].Storage[e.Key]
		slot.PresentValue = e.HadValue

	case JournalTransientStorageChange:
		if transientStorage == nil {
			return
		}
		tkey := TransientKey{Address: e.Address, Key: e.Key}
		if e.HadValue == types.U256Zero {
			delete(transientStorage, tkey)
		} else {
			transientStorage[tkey] = e.HadValue
		}

	case JournalCodeChange:
		acc := state[e.Address]
		acc.Info.CodeHash = types.KeccakEmpty
		acc.Info.Code = nil
	}
}
