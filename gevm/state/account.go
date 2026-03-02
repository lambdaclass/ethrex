package state

import (
	"sync"

	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
)

// AccountStatus is a bitflag tracking account state.
type AccountStatus uint8

const (
	AccountStatusCreated           AccountStatus = 0b00000001
	AccountStatusCreatedLocal      AccountStatus = 0b10000000
	AccountStatusSelfDestructed    AccountStatus = 0b00000010
	AccountStatusSelfDestructLocal AccountStatus = 0b01000000
	AccountStatusTouched           AccountStatus = 0b00000100
	AccountStatusLoadedAsNotExist  AccountStatus = 0b00001000
	AccountStatusCold              AccountStatus = 0b00010000
)

// AccountInfo holds balance, nonce, code hash, and optional code.
type AccountInfo struct {
	Balance  types.Uint256
	Nonce    uint64
	CodeHash types.B256
	Code     types.Bytes // nil means code not loaded yet
}

// DefaultAccountInfo returns an AccountInfo with default values (KECCAK_EMPTY code hash).
func DefaultAccountInfo() AccountInfo {
	return AccountInfo{
		CodeHash: types.KeccakEmpty,
	}
}

// IsEmpty returns true if balance is zero, nonce is zero, and code hash is empty/zero.
func (info *AccountInfo) IsEmpty() bool {
	codeEmpty := info.CodeHash == types.KeccakEmpty || info.CodeHash.IsZero()
	return codeEmpty && info.Balance == types.U256Zero && info.Nonce == 0
}

// IsEmptyCodeHash returns true if the code hash is KECCAK_EMPTY.
func (info *AccountInfo) IsEmptyCodeHash() bool {
	return info.CodeHash == types.KeccakEmpty
}

// HasNoCodeAndNonce returns true if nonce is 0 and code hash is KECCAK_EMPTY.
func (info *AccountInfo) HasNoCodeAndNonce() bool {
	return info.IsEmptyCodeHash() && info.Nonce == 0
}

// Clone returns a deep copy of the AccountInfo.
func (info *AccountInfo) Clone() AccountInfo {
	clone := *info
	if info.Code != nil {
		clone.Code = make(types.Bytes, len(info.Code))
		copy(clone.Code, info.Code)
	}
	return clone
}

// Account is the main account type stored inside the journal.
type Account struct {
	Info          AccountInfo
	OriginalInfo  AccountInfo
	TransactionID int
	Storage       EvmStorage
	Status        AccountStatus
}

// DefaultAccount returns a zero-value Account with empty status.
func DefaultAccount() *Account {
	return &Account{
		Info:    DefaultAccountInfo(),
		Storage: make(EvmStorage),
	}
}

// NewAccountFromInfo creates an Account from AccountInfo.
// Storage is nil initially and lazily allocated on first write (SLoad/SStore).
func NewAccountFromInfo(info AccountInfo) *Account {
	return &Account{
		Info:         info,
		OriginalInfo: info.Clone(),
	}
}

// NewAccountNotExisting creates an account marked as not existing.
// Storage is nil initially; most "not existing" accounts (precompiles) never need storage.
func NewAccountNotExisting(transactionID int) *Account {
	return &Account{
		Info:          DefaultAccountInfo(),
		OriginalInfo:  DefaultAccountInfo(),
		TransactionID: transactionID,
		Status:        AccountStatusLoadedAsNotExist,
	}
}

// EnsureStorage initializes the Storage map if nil.
// Must be called before writing to Storage.
func (a *Account) EnsureStorage() {
	if a.Storage == nil {
		a.Storage = make(EvmStorage)
	}
}

// --- Account Pool ---

var accountPool = sync.Pool{
	New: func() any { return &Account{} },
}

// AcquireAccountFromInfo gets an Account from the pool, initialized from info.
// Code slice is shared between Info and OriginalInfo (never mutated in-place).
func AcquireAccountFromInfo(info AccountInfo) *Account {
	acc := accountPool.Get().(*Account)
	acc.Info = info
	acc.OriginalInfo = info // shallow copy; Code slice shared (safe: never mutated)
	acc.TransactionID = 0
	// Keep Storage map for reuse (cleared on release); EnsureStorage handles nil case.
	acc.Status = 0
	return acc
}

// AcquireAccountNotExisting gets an Account from the pool, marked as not existing.
func AcquireAccountNotExisting(transactionID int) *Account {
	acc := accountPool.Get().(*Account)
	acc.Info = DefaultAccountInfo()
	acc.OriginalInfo = DefaultAccountInfo()
	acc.TransactionID = transactionID
	// Keep Storage map for reuse (cleared on release); EnsureStorage handles nil case.
	acc.Status = AccountStatusLoadedAsNotExist
	return acc
}

// ReleaseAccount returns an Account to the pool.
func ReleaseAccount(acc *Account) {
	acc.Info = AccountInfo{}
	acc.OriginalInfo = AccountInfo{}
	// Clear map but keep it allocated for reuse by next Acquire.
	if acc.Storage != nil {
		clear(acc.Storage)
	}
	acc.Status = 0
	acc.TransactionID = 0
	accountPool.Put(acc)
}

// --- AccountStatus flag methods ---

func (a *Account) MarkTouch()   { a.Status |= AccountStatusTouched }
func (a *Account) UnmarkTouch() { a.Status &^= AccountStatusTouched }
func (a *Account) IsTouched() bool {
	return a.Status&AccountStatusTouched != 0
}

func (a *Account) MarkSelfdestruct()   { a.Status |= AccountStatusSelfDestructed }
func (a *Account) UnmarkSelfdestruct() { a.Status &^= AccountStatusSelfDestructed }
func (a *Account) IsSelfdestructed() bool {
	return a.Status&AccountStatusSelfDestructed != 0
}

func (a *Account) MarkCreated()   { a.Status |= AccountStatusCreated }
func (a *Account) UnmarkCreated() { a.Status &^= AccountStatusCreated }
func (a *Account) IsCreated() bool {
	return a.Status&AccountStatusCreated != 0
}

func (a *Account) MarkCold() { a.Status |= AccountStatusCold }

// IsColdTransactionID returns true if the account is cold for the given transaction ID.
func (a *Account) IsColdTransactionID(transactionID int) bool {
	return a.TransactionID != transactionID || a.Status&AccountStatusCold != 0
}

// MarkWarmWithTransactionID marks the account as warm and returns true if it was previously cold.
func (a *Account) MarkWarmWithTransactionID(transactionID int) bool {
	isCold := a.IsColdTransactionID(transactionID)
	a.Status &^= AccountStatusCold
	a.TransactionID = transactionID
	return isCold
}

// IsCreatedLocally returns true if the account was created locally in this transaction.
func (a *Account) IsCreatedLocally() bool {
	return a.Status&AccountStatusCreatedLocal != 0
}

// MarkCreatedLocally marks the account as locally created and sets the global Created flag.
// Returns true if it was created globally for the first time.
func (a *Account) MarkCreatedLocally() bool {
	return a.markLocalAndGlobal(AccountStatusCreatedLocal, AccountStatusCreated)
}

// UnmarkCreatedLocally removes the local created flag.
func (a *Account) UnmarkCreatedLocally() {
	a.Status &^= AccountStatusCreatedLocal
}

// IsSelfdestructedLocally returns true if the account was selfdestructed locally.
func (a *Account) IsSelfdestructedLocally() bool {
	return a.Status&AccountStatusSelfDestructLocal != 0
}

// MarkSelfdestructedLocally marks the account as locally selfdestructed and sets the global flag.
// Returns true if it was selfdestructed globally for the first time.
func (a *Account) MarkSelfdestructedLocally() bool {
	return a.markLocalAndGlobal(AccountStatusSelfDestructLocal, AccountStatusSelfDestructed)
}

// UnmarkSelfdestructedLocally removes the local selfdestruct flag.
func (a *Account) UnmarkSelfdestructedLocally() {
	a.Status &^= AccountStatusSelfDestructLocal
}

func (a *Account) markLocalAndGlobal(local, global AccountStatus) bool {
	a.Status |= local
	isGlobalFirstTime := a.Status&global == 0
	a.Status |= global
	return isGlobalFirstTime
}

// IsLoadedAsNotExisting returns true if the account was loaded as not existing from DB.
func (a *Account) IsLoadedAsNotExisting() bool {
	return a.Status&AccountStatusLoadedAsNotExist != 0
}

// IsLoadedAsNotExistingNotTouched checks both not-existing and not-touched.
func (a *Account) IsLoadedAsNotExistingNotTouched() bool {
	return a.IsLoadedAsNotExisting() && !a.IsTouched()
}

// IsEmpty returns true if the account info is empty.
func (a *Account) IsEmpty() bool {
	return a.Info.IsEmpty()
}

// StateClearAwareIsEmpty checks emptiness accounting for the Spurious Dragon fork.
func (a *Account) StateClearAwareIsEmpty(forkID spec.ForkID) bool {
	if forkID.IsEnabledIn(spec.SpuriousDragon) {
		return a.IsEmpty()
	}
	return a.IsLoadedAsNotExistingNotTouched()
}

// Selfdestruct clears the account's storage and resets info.
func (a *Account) Selfdestruct() {
	a.Storage = make(EvmStorage)
	a.Info = DefaultAccountInfo()
}

// EvmStorageSlot tracks the current value of a storage slot.
type EvmStorageSlot struct {
	OriginalValue types.Uint256
	PresentValue  types.Uint256
	TransactionID int
	IsCold        bool
}

// NewEvmStorageSlot creates an unchanged slot for the given value.
func NewEvmStorageSlot(original types.Uint256, transactionID int) *EvmStorageSlot {
	return &EvmStorageSlot{
		OriginalValue: original,
		PresentValue:  original,
		TransactionID: transactionID,
	}
}

// NewEvmStorageSlotChanged creates a changed slot.
func NewEvmStorageSlotChanged(original, present types.Uint256, transactionID int) *EvmStorageSlot {
	return &EvmStorageSlot{
		OriginalValue: original,
		PresentValue:  present,
		TransactionID: transactionID,
	}
}

// IsChanged returns true if the present value differs from the original.
func (s *EvmStorageSlot) IsChanged() bool {
	return s.OriginalValue != s.PresentValue
}

// MarkCold marks the storage slot as cold.
func (s *EvmStorageSlot) MarkCold() {
	s.IsCold = true
}

// IsColdTransactionID returns true if the slot is cold for the given transaction ID.
func (s *EvmStorageSlot) IsColdTransactionID(transactionID int) bool {
	return s.TransactionID != transactionID || s.IsCold
}

// MarkWarmWithTransactionID marks the slot as warm and returns true if it was cold.
// When the slot transitions from cold to warm, original_value is reset to present_value.
func (s *EvmStorageSlot) MarkWarmWithTransactionID(transactionID int) bool {
	isCold := s.IsColdTransactionID(transactionID)
	if isCold {
		// if slot is cold original value should be reset to present value.
		s.OriginalValue = s.PresentValue
	}
	s.TransactionID = transactionID
	s.IsCold = false
	return isCold
}
