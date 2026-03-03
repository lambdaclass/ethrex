package state

import (
	"sync"

	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
)

// WarmAddresses tracks which addresses and storage keys are warm-loaded.
// WarmAddresses tracks warmed addresses and storage keys.
type WarmAddresses struct {
	precompiles map[types.Address]struct{}
	coinbase    types.Address
	hasCoinbase bool
	accessList  map[types.Address]map[types.Uint256]struct{}
}

// NewWarmAddresses creates an empty WarmAddresses.
func NewWarmAddresses() WarmAddresses {
	return WarmAddresses{
		accessList: make(map[types.Address]map[types.Uint256]struct{}),
	}
}

// SetPrecompileAddresses sets the precompile address set.
func (w *WarmAddresses) SetPrecompileAddresses(addresses map[types.Address]struct{}) {
	w.precompiles = addresses
}

// Precompiles returns the precompile addresses.
func (w *WarmAddresses) Precompiles() map[types.Address]struct{} {
	return w.precompiles
}

// SetCoinbase sets the coinbase address.
func (w *WarmAddresses) SetCoinbase(address types.Address) {
	w.coinbase = address
	w.hasCoinbase = true
}

// SetAccessList sets the access list.
func (w *WarmAddresses) SetAccessList(accessList map[types.Address]map[types.Uint256]struct{}) {
	w.accessList = accessList
}

// AccessList returns the access list.
func (w *WarmAddresses) AccessList() map[types.Address]map[types.Uint256]struct{} {
	return w.accessList
}

// ClearCoinbaseAndAccessList clears coinbase and access list for next tx.
func (w *WarmAddresses) ClearCoinbaseAndAccessList() {
	w.hasCoinbase = false
	clear(w.accessList)
}

// IsWarm returns true if the address is warm-loaded.
func (w *WarmAddresses) IsWarm(address types.Address) bool {
	if w.hasCoinbase && w.coinbase == address {
		return true
	}
	if _, ok := w.accessList[address]; ok {
		return true
	}
	if w.precompiles != nil {
		if _, ok := w.precompiles[address]; ok {
			return true
		}
	}
	return false
}

// IsStorageWarm returns true if the storage slot is in the access list.
func (w *WarmAddresses) IsStorageWarm(address types.Address, key types.Uint256) bool {
	if slots, ok := w.accessList[address]; ok {
		_, found := slots[key]
		return found
	}
	return false
}

// JournalCfg holds configuration for the journal.
type JournalCfg struct {
	Spec spec.ForkID
}

// Journal tracks all state changes during EVM execution with revert capability.
type Journal struct {
	DB                      Database
	State                   EvmState
	TransientStorage        TransientStorage
	Logs                    []Log
	Depth                   int
	Entries                 []JournalEntry
	TransactionID           int
	Cfg                     JournalCfg
	WarmAddresses           WarmAddresses
	SelfdestructedAddresses []types.Address
	slotArena               slotArena

	// Single-entry account cache: avoids repeated map lookups for the
	// same address (common in ERC-20 where all storage ops target one contract).
	// Safe because j.State never removes accounts; pointers remain valid.
	cachedAddr types.Address
	cachedAcc  *Account

	// 2-entry storage slot cache: avoids map[Uint256]*EvmStorageSlot lookups
	// for SLOAD→SSTORE same-key patterns (common in ERC-20 balance updates).
	// Safe because slot pointers remain valid through reverts (values updated in-place).
	slotCacheAddr types.Address
	slotCacheKeys [2]types.Uint256
	slotCacheVals [2]*EvmStorageSlot
	slotCacheLen  uint8

	// Tracks addresses inserted into State, for fast iteration in ReleaseJournal
	// instead of ranging over the map.
	stateAddrs []types.Address

	// Account arena: slab allocator for Account objects.
	// Avoids sync.Pool Get/Put overhead per account (typically 3-6 per tx).
	accountArena accountArena

}

// accountArena is a slab allocator for Account objects.
// Avoids sync.Pool Get/Put overhead (typically 3-6 accounts per tx).
// Accounts are valid until reset() is called.
type accountArena struct {
	accounts []Account
	idx      int
}

// alloc returns a pointer to a zeroed Account from the arena.
// The returned Account has a pre-allocated Storage map if a previously-used slot is reused.
func (a *accountArena) alloc() *Account {
	if a.idx >= len(a.accounts) {
		// Grow the slab.
		newLen := max(8, len(a.accounts)*2)
		newSlice := make([]Account, newLen)
		copy(newSlice, a.accounts)
		a.accounts = newSlice
	}
	acc := &a.accounts[a.idx]
	a.idx++
	return acc
}

// reset clears account fields for reuse but retains Storage map allocations.
func (a *accountArena) reset() {
	for i := 0; i < a.idx; i++ {
		acc := &a.accounts[i]
		acc.Info.Code = nil
		acc.OriginalInfo.Code = nil
		if acc.Storage != nil {
			clear(acc.Storage)
		}
	}
	a.idx = 0
}

// slotArena is a slab allocator for EvmStorageSlot to avoid per-slot heap allocations.
type slotArena struct {
	slabs [][]EvmStorageSlot
	idx   int // next free index in the current slab
}

const slotArenaSlabSize = 1024

// allocSlot returns a pointer to a new EvmStorageSlot from the arena.
func (a *slotArena) allocSlot() *EvmStorageSlot {
	if len(a.slabs) == 0 || a.idx == len(a.slabs[len(a.slabs)-1]) {
		a.slabs = append(a.slabs, make([]EvmStorageSlot, slotArenaSlabSize))
		a.idx = 0
	}
	slab := a.slabs[len(a.slabs)-1]
	slot := &slab[a.idx]
	a.idx++
	return slot
}

// reset clears the arena for reuse. Retains capacity.
func (a *slotArena) reset() {
	// Keep slabs allocated, just reset index.
	// Note: we keep only the first slab to avoid holding too much memory.
	if len(a.slabs) > 1 {
		a.slabs = a.slabs[:1]
	}
	a.idx = 0
}

// NewJournal creates a new Journal with the given database.
func NewJournal(db Database) *Journal {
	return &Journal{
		DB:               db,
		State:            make(EvmState),
		TransientStorage: make(TransientStorage),
		Entries:          make([]JournalEntry, 0, 256),
		WarmAddresses:    NewWarmAddresses(),
	}
}

// journalPool reuses Journal objects across transactions.
var journalPool = sync.Pool{
	New: func() any {
		return &Journal{
			State:            make(EvmState),
			TransientStorage: make(TransientStorage),
			Entries:          make([]JournalEntry, 0, 256),
			WarmAddresses:    NewWarmAddresses(),
		}
	},
}

// AcquireJournal gets a Journal from the pool, reset and bound to db.
func AcquireJournal(db Database) *Journal {
	j := journalPool.Get().(*Journal)
	j.DB = db
	return j
}

// ReleaseJournal returns a Journal to the pool after clearing state.
func ReleaseJournal(j *Journal) {
	j.DB = nil
	// Delete state entries using tracked addresses (avoids map iteration).
	for _, addr := range j.stateAddrs {
		delete(j.State, addr)
	}
	j.stateAddrs = j.stateAddrs[:0]
	// Reset arena: clears code refs and storage maps, retains allocations.
	j.accountArena.reset()
	clear(j.TransientStorage)
	j.Entries = j.Entries[:0]
	j.Logs = j.Logs[:0]
	j.SelfdestructedAddresses = j.SelfdestructedAddresses[:0]
	j.Depth = 0
	j.TransactionID = 0
	j.WarmAddresses.hasCoinbase = false
	j.WarmAddresses.precompiles = nil // drop reference to shared map (don't clear)
	clear(j.WarmAddresses.accessList)
	j.Cfg = JournalCfg{}
	j.cachedAcc = nil
	j.slotCacheLen = 0
	j.slotArena.reset()
	journalPool.Put(j)
}

// SetForkID sets the spec ID.
func (j *Journal) SetForkID(forkID spec.ForkID) {
	j.Cfg.Spec = forkID
}

// --- Checkpoint / Revert / Commit ---

// Checkpoint creates a snapshot of the current journal state.
func (j *Journal) Checkpoint() JournalCheckpoint {
	cp := JournalCheckpoint{
		LogI:            len(j.Logs),
		JournalI:        len(j.Entries),
		SelfdestructedI: len(j.SelfdestructedAddresses),
	}
	j.Depth++
	return cp
}

// CheckpointCommit commits the checkpoint (just decrements depth).
func (j *Journal) CheckpointCommit() {
	if j.Depth > 0 {
		j.Depth--
	}
}

// CheckpointRevert reverts all changes since the given checkpoint.
func (j *Journal) CheckpointRevert(checkpoint JournalCheckpoint) {
	isSpuriousDragon := j.Cfg.Spec.IsEnabledIn(spec.SpuriousDragon)
	if j.Depth > 0 {
		j.Depth--
	}
	j.Logs = j.Logs[:checkpoint.LogI]
	j.SelfdestructedAddresses = j.SelfdestructedAddresses[:checkpoint.SelfdestructedI]

	// Iterate over journal entries in reverse and revert each one.
	if checkpoint.JournalI < len(j.Entries) {
		for i := len(j.Entries) - 1; i >= checkpoint.JournalI; i-- {
			j.Entries[i].Revert(j.State, j.TransientStorage, isSpuriousDragon)
		}
		j.Entries = j.Entries[:checkpoint.JournalI]
	}
}

// CommitTx prepares for the next transaction.
func (j *Journal) CommitTx() {
	clear(j.TransientStorage)
	j.Depth = 0
	j.Entries = j.Entries[:0]
	j.WarmAddresses.ClearCoinbaseAndAccessList()
	j.TransactionID++
	j.Logs = j.Logs[:0]
	j.SelfdestructedAddresses = j.SelfdestructedAddresses[:0]
	j.slotCacheLen = 0
}

// DiscardTx discards the current transaction by reverting all journal entries.
func (j *Journal) DiscardTx() {
	isSpuriousDragon := j.Cfg.Spec.IsEnabledIn(spec.SpuriousDragon)
	for i := len(j.Entries) - 1; i >= 0; i-- {
		j.Entries[i].Revert(j.State, nil, isSpuriousDragon)
	}
	clear(j.TransientStorage)
	j.Depth = 0
	j.Logs = j.Logs[:0]
	j.SelfdestructedAddresses = j.SelfdestructedAddresses[:0]
	j.TransactionID++
	j.Entries = j.Entries[:0]
	j.WarmAddresses.ClearCoinbaseAndAccessList()
	j.slotCacheLen = 0
}

// Finalize takes the EvmState and resets the journal to initial state.
func (j *Journal) Finalize() EvmState {
	j.WarmAddresses.ClearCoinbaseAndAccessList()
	j.SelfdestructedAddresses = j.SelfdestructedAddresses[:0]
	state := j.State
	j.State = make(EvmState)
	j.Logs = j.Logs[:0]
	clear(j.TransientStorage)
	j.Entries = j.Entries[:0]
	j.Depth = 0
	j.TransactionID = 0
	j.cachedAcc = nil
	j.slotCacheLen = 0
	j.stateAddrs = j.stateAddrs[:0]
	return state
}

// TakeLogs returns and clears the logs.
func (j *Journal) TakeLogs() []Log {
	logs := j.Logs
	j.Logs = nil
	return logs
}

// --- Account Loading ---

// touchAccount marks an account as touched if not already, and adds a journal entry.
func (j *Journal) touchAccount(address types.Address, acc *Account) {
	if !acc.IsTouched() {
		j.Entries = append(j.Entries, JournalEntryAccountTouched(address))
		acc.MarkTouch()
	}
}

// stateAccount returns the *Account for address from the single-entry cache
// or falls back to the State map. Updates the cache on miss.
func (j *Journal) stateAccount(address types.Address) (*Account, bool) {
	if j.cachedAcc != nil && j.cachedAddr == address {
		return j.cachedAcc, true
	}
	acc, ok := j.State[address]
	if ok {
		j.cachedAddr = address
		j.cachedAcc = acc
	}
	return acc, ok
}

// cacheAccount stores an account in the single-entry cache.
func (j *Journal) cacheAccount(address types.Address, acc *Account) {
	j.cachedAddr = address
	j.cachedAcc = acc
}

// cachedSlot looks up a storage slot in the 2-entry cache.
// Returns the slot and true if found.
func (j *Journal) cachedSlot(address types.Address, key types.Uint256) (*EvmStorageSlot, bool) {
	if j.slotCacheLen == 0 || j.slotCacheAddr != address {
		return nil, false
	}
	if j.slotCacheKeys[0] == key {
		return j.slotCacheVals[0], true
	}
	if j.slotCacheLen > 1 && j.slotCacheKeys[1] == key {
		return j.slotCacheVals[1], true
	}
	return nil, false
}

// cacheSlot stores a storage slot in the 2-entry round-robin cache.
func (j *Journal) cacheSlot(address types.Address, key types.Uint256, slot *EvmStorageSlot) {
	if j.slotCacheAddr != address || j.slotCacheLen == 0 {
		// New address or empty cache: initialize slot 0
		j.slotCacheAddr = address
		j.slotCacheKeys[0] = key
		j.slotCacheVals[0] = slot
		j.slotCacheLen = 1
		return
	}
	// Same address: check for duplicate
	if j.slotCacheLen > 0 && j.slotCacheKeys[0] == key {
		j.slotCacheVals[0] = slot
		return
	}
	if j.slotCacheLen > 1 && j.slotCacheKeys[1] == key {
		j.slotCacheVals[1] = slot
		return
	}
	// Add to cache (fill or evict slot 1)
	if j.slotCacheLen < 2 {
		j.slotCacheKeys[1] = key
		j.slotCacheVals[1] = slot
		j.slotCacheLen = 2
	} else {
		// Evict slot 1 (keeps slot 0 = most recently SLOADed first key)
		j.slotCacheKeys[1] = key
		j.slotCacheVals[1] = slot
	}
}

// WarmPrecompileAccounts is a fast path for warming precompile addresses.
// Inserts NotExisting if not in state,
// skip DB calls and journal entries (precompiles are always warm via WarmAddresses).
// Touch marks an already-loaded account as touched.
func (j *Journal) Touch(address types.Address) {
	if acc, ok := j.stateAccount(address); ok {
		j.touchAccount(address, acc)
	}
}

// LoadAccount loads an account into the journal state, marking it warm.
// Returns the account and whether it was cold-loaded.
func (j *Journal) LoadAccount(address types.Address) (StateLoad[*Account], error) {
	acc, isCold, err := j.loadAccountMutInternal(address)
	if err != nil {
		return StateLoad[*Account]{}, err
	}
	return NewStateLoad(acc, isCold), nil
}

// loadAccountMutInternal is the core account loading logic.
// It handles warm/cold tracking, DB loading, and journal entry creation.
func (j *Journal) loadAccountMutInternal(address types.Address) (*Account, bool, error) {
	if acc, ok := j.stateAccount(address); ok {
		// Account already in state - check if cold for this transaction.
		isCold := acc.IsColdTransactionID(j.TransactionID)
		if isCold {
			isCold = !j.WarmAddresses.IsWarm(address)
			// Mark warm.
			acc.MarkWarmWithTransactionID(j.TransactionID)

			// If cold-loaded and locally selfdestructed, clear from previous tx.
			if acc.IsSelfdestructedLocally() {
				acc.Selfdestruct()
				acc.UnmarkSelfdestructedLocally()
			}
			// Set original info to current info.
			acc.OriginalInfo = acc.Info.Clone()
			// Unmark locally created.
			acc.UnmarkCreatedLocally()
			// Journal loading of cold account.
			j.Entries = append(j.Entries, JournalEntryAccountWarmed(address))
		}
		return acc, isCold, nil
	}

	// Account not in state - load from DB.
	isCold := !j.WarmAddresses.IsWarm(address)
	acc := j.accountArena.alloc()
	if j.DB != nil {
		info, exists, err := j.DB.Basic(address)
		if err != nil {
			return nil, false, err
		}
		if exists {
			acc.Info = info
			acc.OriginalInfo = info // shallow copy; Code slice shared (safe: never mutated)
			acc.TransactionID = j.TransactionID
			acc.Status = 0
		} else {
			acc.Info = DefaultAccountInfo()
			acc.OriginalInfo = DefaultAccountInfo()
			acc.TransactionID = j.TransactionID
			acc.Status = AccountStatusLoadedAsNotExist
		}
	} else {
		acc.Info = DefaultAccountInfo()
		acc.OriginalInfo = DefaultAccountInfo()
		acc.TransactionID = j.TransactionID
		acc.Status = AccountStatusLoadedAsNotExist
	}

	// Journal loading of cold account.
	if isCold {
		j.Entries = append(j.Entries, JournalEntryAccountWarmed(address))
	}

	j.State[address] = acc
	j.stateAddrs = append(j.stateAddrs, address)
	j.cacheAccount(address, acc)
	return acc, isCold, nil
}

// --- Storage Operations ---

// SLoad reads a storage slot with warm/cold tracking.
// Returns the storage value and whether the slot was cold.
func (j *Journal) SLoad(address types.Address, key types.Uint256) (StateLoad[types.Uint256], error) {
	var out types.Uint256
	isCold, err := j.SLoadInto(address, &key, &out)
	if err != nil {
		return StateLoad[types.Uint256]{}, err
	}
	return NewStateLoad(out, isCold), nil
}

// SLoadInto reads a storage slot with warm/cold tracking, writing the value
// directly into *out. key and out may alias (both point to the same stack slot).
// Returns (isCold, error).
func (j *Journal) SLoadInto(address types.Address, key *types.Uint256, out *types.Uint256) (bool, error) {
	k := *key // local copy: safe if key==out (aliasing)

	// Ensure account is loaded. Use single-entry cache for repeated same-address lookups.
	acc, ok := j.stateAccount(address)
	if !ok {
		_, _, err := j.loadAccountMutInternal(address)
		if err != nil {
			return false, err
		}
		acc = j.State[address]
		j.cacheAccount(address, acc)
	}

	// Fast path: check slot cache first (avoids map[Uint256] lookup for warm re-access).
	if slot, hit := j.cachedSlot(address, k); hit {
		isCold := false
		if slot.IsColdTransactionID(j.TransactionID) {
			isCold = !j.WarmAddresses.IsStorageWarm(address, k)
		}
		slot.MarkWarmWithTransactionID(j.TransactionID)
		if isCold {
			j.Entries = append(j.Entries, JournalEntryStorageWarmed(address, k))
		}
		*out = slot.PresentValue
		return isCold, nil
	}

	isNewlyCreated := acc.IsCreated()

	slot, slotExists := acc.Storage[k]
	var isCold bool

	if slotExists {
		if slot.IsColdTransactionID(j.TransactionID) {
			// Check if it's in the access list.
			isCold = !j.WarmAddresses.IsStorageWarm(address, k)
		}
		slot.MarkWarmWithTransactionID(j.TransactionID)
	} else {
		// Check if it's in the access list.
		isCold = !j.WarmAddresses.IsStorageWarm(address, k)

		// Load from DB if not newly created.
		var value types.Uint256
		if !isNewlyCreated && j.DB != nil {
			var err error
			value, err = j.DB.Storage(address, k)
			if err != nil {
				return false, err
			}
		}

		slot = j.slotArena.allocSlot()
		slot.OriginalValue = value
		slot.PresentValue = value
		slot.TransactionID = j.TransactionID
		slot.IsCold = false
		acc.EnsureStorage()
		acc.Storage[k] = slot
	}

	j.cacheSlot(address, k, slot)

	if isCold {
		j.Entries = append(j.Entries, JournalEntryStorageWarmed(address, k))
	}

	*out = slot.PresentValue
	return isCold, nil
}

// SStoreResult holds the result of an SSTORE operation.
type SStoreResult struct {
	OriginalValue types.Uint256
	PresentValue  types.Uint256
	NewValue      types.Uint256
}

// SelfDestructResult holds the result of a SELFDESTRUCT operation.
type SelfDestructResult struct {
	HadValue            bool
	TargetExists        bool
	PreviouslyDestroyed bool
}

// SStore writes a storage slot with warm/cold tracking.
// Returns SStoreResult and whether the slot was cold.
// Inlines Touch + SLoad to avoid redundant map lookups (5 → 2).
func (j *Journal) SStore(address types.Address, key, newValue types.Uint256) (StateLoad[SStoreResult], error) {
	var result SStoreResult
	isCold, err := j.sstoreInner(address, &key, &newValue, &result.OriginalValue, &result.PresentValue, &result.NewValue)
	if err != nil {
		return StateLoad[SStoreResult]{}, err
	}
	return NewStateLoad(result, isCold), nil
}

// SStoreInto writes a storage slot and writes the result directly into outOriginal/outPresent/outNew/outCold.
// Key and newValue are passed by pointer to avoid 64B copy through the Host interface.
// Avoids the ~97-byte StateLoad[SStoreResult] return copy.
func (j *Journal) SStoreInto(address types.Address, key *types.Uint256, newValue *types.Uint256,
	outOriginal, outPresent, outNew *types.Uint256, outCold *bool) error {
	isCold, err := j.sstoreInner(address, key, newValue, outOriginal, outPresent, outNew)
	if err != nil {
		return err
	}
	*outCold = isCold
	return nil
}

// sstoreInner is the shared SSTORE implementation.
// Key and newValue passed by pointer; local copies made for aliasing safety.
// Writes directly into outOriginal/outPresent/outNew to avoid intermediate struct copies.
func (j *Journal) sstoreInner(address types.Address, key *types.Uint256, newValue *types.Uint256,
	outOriginal, outPresent, outNew *types.Uint256) (bool, error) {
	k := *key       // local copy: safe even if key aliases outOriginal/outPresent/outNew
	v := *newValue  // local copy

	// Load account once (combines Touch + SLoad account lookup).
	acc, ok := j.stateAccount(address)
	if !ok {
		_, _, err := j.loadAccountMutInternal(address)
		if err != nil {
			return false, err
		}
		acc = j.State[address]
		j.cacheAccount(address, acc)
	}

	// Touch (inlined from touchAccount).
	if !acc.IsTouched() {
		j.Entries = append(j.Entries, JournalEntryAccountTouched(address))
		acc.MarkTouch()
	}

	// Fast path: check slot cache first (avoids map[Uint256] lookup for warm re-access).
	var slot *EvmStorageSlot
	var isCold bool
	if cached, hit := j.cachedSlot(address, k); hit {
		slot = cached
		if slot.IsColdTransactionID(j.TransactionID) {
			isCold = !j.WarmAddresses.IsStorageWarm(address, k)
		}
		slot.MarkWarmWithTransactionID(j.TransactionID)
		if isCold {
			j.Entries = append(j.Entries, JournalEntryStorageWarmed(address, k))
		}
	} else {
		// Slow path: map lookup.
		isNewlyCreated := acc.IsCreated()
		var slotExists bool
		slot, slotExists = acc.Storage[k]

		if slotExists {
			if slot.IsColdTransactionID(j.TransactionID) {
				isCold = !j.WarmAddresses.IsStorageWarm(address, k)
			}
			slot.MarkWarmWithTransactionID(j.TransactionID)
		} else {
			isCold = !j.WarmAddresses.IsStorageWarm(address, k)

			var value types.Uint256
			if !isNewlyCreated && j.DB != nil {
				var err error
				value, err = j.DB.Storage(address, k)
				if err != nil {
					return false, err
				}
			}

			slot = j.slotArena.allocSlot()
			slot.OriginalValue = value
			slot.PresentValue = value
			slot.TransactionID = j.TransactionID
			slot.IsCold = false
			acc.EnsureStorage()
			acc.Storage[k] = slot
		}

		j.cacheSlot(address, k, slot)

		if isCold {
			j.Entries = append(j.Entries, JournalEntryStorageWarmed(address, k))
		}
	}

	// SStore-specific: write result fields and update slot.
	*outOriginal = slot.OriginalValue
	*outPresent = slot.PresentValue
	*outNew = v

	if slot.PresentValue != v {
		previousValue := slot.PresentValue
		slot.PresentValue = v
		j.Entries = append(j.Entries, JournalEntryStorageChanged(address, k, previousValue))
	}

	return isCold, nil
}

// --- Transient Storage (EIP-1153) ---

// TLoad reads a transient storage value.
func (j *Journal) TLoad(address types.Address, key types.Uint256) types.Uint256 {
	tkey := TransientKey{Address: address, Key: key}
	if val, ok := j.TransientStorage[tkey]; ok {
		return val
	}
	return types.U256Zero
}

// TStore writes a transient storage value.
func (j *Journal) TStore(address types.Address, key, newValue types.Uint256) {
	tkey := TransientKey{Address: address, Key: key}

	if newValue == types.U256Zero {
		// Remove entry if new value is zero.
		if old, ok := j.TransientStorage[tkey]; ok {
			delete(j.TransientStorage, tkey)
			j.Entries = append(j.Entries, JournalEntryTransientStorageChange(address, key, old))
		}
	} else {
		old, existed := j.TransientStorage[tkey]
		if !existed {
			old = types.U256Zero
		}
		j.TransientStorage[tkey] = newValue
		if old != newValue {
			j.Entries = append(j.Entries, JournalEntryTransientStorageChange(address, key, old))
		}
	}
}

// --- Logging ---

// AppendLog adds a log entry.
func (j *Journal) AppendLog(log Log) {
	j.Logs = append(j.Logs, log)
}

// AllocLog grows the log slice by one and returns a pointer to the new entry.
// The caller fills the fields in-place, avoiding a 173-byte struct copy.
func (j *Journal) AllocLog() *Log {
	n := len(j.Logs)
	if n < cap(j.Logs) {
		j.Logs = j.Logs[:n+1]
	} else {
		j.Logs = append(j.Logs, Log{})
	}
	return &j.Logs[n]
}

// --- Transfer ---

// Transfer transfers balance between two accounts, loading them from DB if needed.
// Returns nil on success, or a TransferError.
func (j *Journal) Transfer(from, to types.Address, balance types.Uint256) (*TransferError, error) {
	if _, _, err := j.loadAccountMutInternal(from); err != nil {
		return nil, err
	}
	if _, _, err := j.loadAccountMutInternal(to); err != nil {
		return nil, err
	}
	result := j.TransferLoaded(from, to, balance)
	return result, nil
}

// TransferLoaded transfers balance between two already-loaded accounts.
// Returns nil on success, or a pointer to a TransferError.
func (j *Journal) TransferLoaded(from, to types.Address, balance types.Uint256) *TransferError {
	if from == to {
		fromBalance := j.State[to].Info.Balance
		if balance.Gt(fromBalance) {
			e := TransferErrorOutOfFunds
			return &e
		}
		return nil
	}

	if balance == types.U256Zero {
		j.touchAccount(to, j.State[to])
		return nil
	}

	// Sub balance from sender.
	fromAcc := j.State[from]
	j.touchAccount(from, fromAcc)
	newFromBalance, underflow := fromAcc.Info.Balance.OverflowingSub(balance)
	if underflow {
		e := TransferErrorOutOfFunds
		return &e
	}
	fromAcc.Info.Balance = newFromBalance

	// Add balance to receiver.
	toAcc := j.State[to]
	j.touchAccount(to, toAcc)
	newToBalance, overflow := toAcc.Info.Balance.OverflowingAdd(balance)
	if overflow {
		e := TransferErrorOverflowPayment
		return &e
	}
	toAcc.Info.Balance = newToBalance

	// Journal entry.
	j.Entries = append(j.Entries, JournalEntryBalanceTransfer(from, to, balance))

	return nil
}

// --- Self Destruct ---

// Selfdestruct performs the selfdestruct operation.
func (j *Journal) Selfdestruct(address, target types.Address) (StateLoad[SelfDestructResult], error) {
	forkID := j.Cfg.Spec

	// Load target account.
	targetLoad, err := j.LoadAccount(target)
	if err != nil {
		return StateLoad[SelfDestructResult]{}, err
	}
	isCold := targetLoad.IsCold
	isEmpty := targetLoad.Data.StateClearAwareIsEmpty(forkID)

	if address != target {
		accBalance := j.State[address].Info.Balance
		targetAcc := j.State[target]
		j.touchAccount(target, targetAcc)
		targetAcc.Info.Balance = targetAcc.Info.Balance.Add(accBalance)
	}

	acc := j.State[address]
	balance := acc.Info.Balance

	var destroyedStatus SelfdestructionRevertStatus
	if !acc.IsSelfdestructed() {
		destroyedStatus = SelfdestructGlobal
	} else if !acc.IsSelfdestructedLocally() {
		destroyedStatus = SelfdestructLocal
	} else {
		destroyedStatus = SelfdestructRepeated
	}

	isCancunEnabled := forkID.IsEnabledIn(spec.Cancun)

	// EIP-6780: selfdestruct only if created in same tx (post-Cancun).
	if acc.IsCreatedLocally() || !isCancunEnabled {
		acc.MarkSelfdestructedLocally()
		acc.Info.Balance = types.U256Zero
		j.Entries = append(j.Entries, JournalEntryAccountDestroyed(address, target, destroyedStatus, balance))
	} else if address != target {
		acc.Info.Balance = types.U256Zero
		j.Entries = append(j.Entries, JournalEntryBalanceTransfer(address, target, balance))
	}
	// else: post-Cancun, not created locally, target == self → no state change.

	return NewStateLoad(SelfDestructResult{
		HadValue:            balance != types.U256Zero,
		TargetExists:        !isEmpty,
		PreviouslyDestroyed: destroyedStatus == SelfdestructRepeated,
	}, isCold), nil
}

// --- Create Account Checkpoint ---

// CreateAccountCheckpoint sets up a new account for CREATE/CREATE2.
func (j *Journal) CreateAccountCheckpoint(caller, targetAddress types.Address, balance types.Uint256, forkID spec.ForkID) (JournalCheckpoint, *TransferError) {
	checkpoint := j.Checkpoint()

	targetAcc := j.State[targetAddress]

	// Collision check (EIP-7610): nonce != 0, non-empty code, or non-empty storage.
	if targetAcc.Info.CodeHash != types.KeccakEmpty || targetAcc.Info.Nonce != 0 || j.hasStorage(targetAddress, targetAcc) {
		j.CheckpointRevert(checkpoint)
		e := TransferErrorCreateCollision
		return JournalCheckpoint{}, &e
	}

	// Mark as created locally.
	isCreatedGlobally := targetAcc.MarkCreatedLocally()
	j.Entries = append(j.Entries, JournalEntryAccountCreated(targetAddress, isCreatedGlobally))
	targetAcc.Info.Code = nil

	// EIP-161: set nonce to 1 if Spurious Dragon.
	if forkID.IsEnabledIn(spec.SpuriousDragon) {
		targetAcc.Info.Nonce = 1
	}

	j.touchAccount(targetAddress, targetAcc)

	if balance == types.U256Zero {
		return checkpoint, nil
	}

	// Add balance to target.
	newBalance, overflow := targetAcc.Info.Balance.OverflowingAdd(balance)
	if overflow {
		j.CheckpointRevert(checkpoint)
		e := TransferErrorOverflowPayment
		return JournalCheckpoint{}, &e
	}
	targetAcc.Info.Balance = newBalance

	// Subtract from caller.
	callerAcc := j.State[caller]
	callerAcc.Info.Balance = callerAcc.Info.Balance.Sub(balance)

	j.Entries = append(j.Entries, JournalEntryBalanceTransfer(caller, targetAddress, balance))

	return checkpoint, nil
}

// hasStorage checks if the account has any non-empty storage, checking
// journal dirty slots first then falling back to the underlying DB.
func (j *Journal) hasStorage(address types.Address, acc *Account) bool {
	// Check dirty journal storage.
	if acc.Storage != nil {
		for _, slot := range acc.Storage {
			if !slot.PresentValue.IsZero() {
				return true
			}
		}
	}
	// Fall back to DB.
	if j.DB != nil {
		has, _ := j.DB.HasStorage(address)
		return has
	}
	return false
}

// --- Code Setting ---

// SetCodeWithHash sets account code and hash. Assumes account is warm.
func (j *Journal) SetCodeWithHash(address types.Address, code types.Bytes, hash types.B256) {
	acc := j.State[address]
	j.touchAccount(address, acc)
	j.Entries = append(j.Entries, JournalEntryCodeChange(address))
	acc.Info.CodeHash = hash
	acc.Info.Code = code
}

// --- Balance Increment ---

// BalanceIncr increments the balance of an account, loading it if needed.
func (j *Journal) BalanceIncr(address types.Address, balance types.Uint256) error {
	_, _, err := j.loadAccountMutInternal(address)
	if err != nil {
		return err
	}
	acc := j.State[address]
	j.touchAccount(address, acc)
	oldBalance := acc.Info.Balance
	acc.Info.Balance = acc.Info.Balance.Add(balance)
	if acc.Info.Balance != oldBalance {
		j.Entries = append(j.Entries, JournalEntryBalanceChange(address, oldBalance))
	}
	return nil
}
