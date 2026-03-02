package state

import (
	"github.com/Giulio2002/gevm/types"
)

// EvmState is the main state map: Address -> Account.
type EvmState map[types.Address]*Account

// EvmStorage is per-account storage: Uint256 key -> EvmStorageSlot.
type EvmStorage map[types.Uint256]*EvmStorageSlot

// TransientKey is a composite key for transient storage (address + storage key).
type TransientKey struct {
	Address types.Address
	Key     types.Uint256
}

// TransientStorage is EIP-1153 transient storage, cleared after each transaction.
type TransientStorage map[TransientKey]types.Uint256

// StateLoad wraps state access results with a cold/warm indicator for gas metering.
type StateLoad[T any] struct {
	Data   T
	IsCold bool
}

// NewStateLoad creates a new StateLoad with the given data and cold status.
func NewStateLoad[T any](data T, isCold bool) StateLoad[T] {
	return StateLoad[T]{Data: data, IsCold: isCold}
}

// TransferError represents errors during balance transfers.
type TransferError int

const (
	TransferErrorOutOfFunds TransferError = iota
	TransferErrorOverflowPayment
	TransferErrorCreateCollision
)

// JournalCheckpoint saves a snapshot of journal state for reverting.
type JournalCheckpoint struct {
	LogI            int
	JournalI        int
	SelfdestructedI int
}

// Log represents an EVM log entry (LOG0-LOG4 output).
// Topics uses a fixed array to avoid heap allocation (LOG can have at most 4 topics).
type Log struct {
	Address   types.Address
	Topics    [4]types.B256
	NumTopics uint8
	Data      types.Bytes
}

// TopicSlice returns the active topics as a slice.
func (l *Log) TopicSlice() []types.B256 {
	return l.Topics[:l.NumTopics]
}
