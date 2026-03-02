// Provides a concrete Host implementation that bridges the Journal/state
// system to the VM's Host interface.
package host

import (
	"github.com/Giulio2002/gevm/spec"
	"github.com/Giulio2002/gevm/state"
	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/vm"
)

// BlockEnv holds block-level environment data.
type BlockEnv struct {
	Beneficiary  types.Address
	Timestamp    types.Uint256
	Number       types.Uint256
	Difficulty   types.Uint256
	Prevrandao   *types.Uint256
	GasLimit     types.Uint256
	BaseFee      types.Uint256
	BlobGasPrice types.Uint256
	SlotNum      types.Uint256
}

// TxEnv holds transaction-level environment data.
type TxEnv struct {
	Caller            types.Address
	EffectiveGasPrice types.Uint256
	BlobHashes        []types.Uint256
}

// CfgEnv holds chain configuration.
type CfgEnv struct {
	ChainId types.Uint256
}

// EvmHost is the concrete Host implementation that delegates to Journal.
// It implements the vm.Host interface.
// Block and Cfg are stored by pointer to avoid ~316 bytes of struct copy
// when setting up the host in Transact.
type EvmHost struct {
	Block   *BlockEnv
	Tx      TxEnv
	Cfg     *CfgEnv
	Journal *state.Journal
	ForkID  spec.ForkID // for fork-specific behavior (e.g. BLOCKHASH window)
}

// NewEvmHost creates a new EvmHost.
func NewEvmHost(journal *state.Journal, block *BlockEnv, tx TxEnv, cfg *CfgEnv) *EvmHost {
	return &EvmHost{
		Block:   block,
		Tx:      tx,
		Cfg:     cfg,
		Journal: journal,
	}
}

// Compile-time check that EvmHost implements vm.Host.
var _ vm.Host = (*EvmHost)(nil)

// --- Block info ---

func (h *EvmHost) Beneficiary() types.Address { return h.Block.Beneficiary }
func (h *EvmHost) Timestamp() types.Uint256      { return h.Block.Timestamp }
func (h *EvmHost) BlockNumber() types.Uint256    { return h.Block.Number }
func (h *EvmHost) Difficulty() types.Uint256     { return h.Block.Difficulty }
func (h *EvmHost) Prevrandao() *types.Uint256    { return h.Block.Prevrandao }
func (h *EvmHost) GasLimit() types.Uint256       { return h.Block.GasLimit }
func (h *EvmHost) BaseFee() types.Uint256        { return h.Block.BaseFee }
func (h *EvmHost) BlobGasPrice() types.Uint256   { return h.Block.BlobGasPrice }
func (h *EvmHost) SlotNum() types.Uint256        { return h.Block.SlotNum }
func (h *EvmHost) ChainId() types.Uint256        { return h.Cfg.ChainId }

// --- Tx info ---

func (h *EvmHost) Caller() types.Address         { return h.Tx.Caller }
func (h *EvmHost) EffectiveGasPrice() types.Uint256 { return h.Tx.EffectiveGasPrice }
func (h *EvmHost) BlobHash(index int) *types.Uint256 {
	if index < 0 || index >= len(h.Tx.BlobHashes) {
		return nil
	}
	v := h.Tx.BlobHashes[index]
	return &v
}

// --- Account access (delegated to Journal) ---

func (h *EvmHost) Balance(addr types.Address) (types.Uint256, bool) {
	result, err := h.Journal.LoadAccount(addr)
	if err != nil {
		return types.U256Zero, false
	}
	return result.Data.Info.Balance, result.IsCold
}

func (h *EvmHost) CodeSize(addr types.Address) (int, bool) {
	result, err := h.Journal.LoadAccount(addr)
	if err != nil {
		return 0, false
	}
	acc := result.Data
	if acc.Info.Code != nil {
		return len(acc.Info.Code), result.IsCold
	}
	// Code not loaded; load from DB
	if h.Journal.DB != nil && acc.Info.CodeHash != types.KeccakEmpty {
		code, dbErr := h.Journal.DB.CodeByHash(acc.Info.CodeHash)
		if dbErr == nil {
			acc.Info.Code = code
			return len(code), result.IsCold
		}
	}
	return 0, result.IsCold
}

func (h *EvmHost) CodeHash(addr types.Address) (types.B256, bool) {
	result, err := h.Journal.LoadAccount(addr)
	if err != nil {
		return types.B256Zero, false
	}
	acc := result.Data
	if acc.StateClearAwareIsEmpty(h.Journal.Cfg.Spec) {
		return types.B256Zero, result.IsCold
	}
	return acc.Info.CodeHash, result.IsCold
}

func (h *EvmHost) Code(addr types.Address) (types.Bytes, bool) {
	result, err := h.Journal.LoadAccount(addr)
	if err != nil {
		return nil, false
	}
	acc := result.Data
	if acc.Info.Code != nil {
		return acc.Info.Code, result.IsCold
	}
	// Load from DB
	if h.Journal.DB != nil && acc.Info.CodeHash != types.KeccakEmpty {
		code, dbErr := h.Journal.DB.CodeByHash(acc.Info.CodeHash)
		if dbErr == nil {
			acc.Info.Code = code
			return code, result.IsCold
		}
	}
	return nil, result.IsCold
}

func (h *EvmHost) LoadAccountCode(addr types.Address) vm.AccountCodeLoad {
	result, err := h.Journal.LoadAccount(addr)
	if err != nil {
		return vm.AccountCodeLoad{IsEmpty: true}
	}
	acc := result.Data
	isEmpty := acc.StateClearAwareIsEmpty(h.Journal.Cfg.Spec)
	// Load code if needed
	code := acc.Info.Code
	if code == nil && h.Journal.DB != nil && acc.Info.CodeHash != types.KeccakEmpty {
		loaded, dbErr := h.Journal.DB.CodeByHash(acc.Info.CodeHash)
		if dbErr == nil {
			code = loaded
			acc.Info.Code = code
		}
	}
	return vm.AccountCodeLoad{
		Code:     code,
		CodeHash: acc.Info.CodeHash,
		IsCold:   result.IsCold,
		IsEmpty:  isEmpty,
	}
}

func (h *EvmHost) SelfBalance(addr types.Address) types.Uint256 {
	if acc, ok := h.Journal.State[addr]; ok {
		return acc.Info.Balance
	}
	return types.U256Zero
}

// --- Storage access ---

func (h *EvmHost) SLoadInto(addr types.Address, key *types.Uint256, out *types.Uint256) bool {
	isCold, err := h.Journal.SLoadInto(addr, key, out)
	if err != nil {
		*out = types.U256Zero
		return false
	}
	return isCold
}

func (h *EvmHost) SStore(addr types.Address, key *types.Uint256, value *types.Uint256, out *vm.SStoreResult) {
	err := h.Journal.SStoreInto(addr, key, value,
		&out.OriginalValue, &out.PresentValue, &out.NewValue, &out.IsCold)
	if err != nil {
		*out = vm.SStoreResult{}
	}
}

func (h *EvmHost) TLoad(addr types.Address, key types.Uint256) types.Uint256 {
	return h.Journal.TLoad(addr, key)
}

func (h *EvmHost) TStore(addr types.Address, key, value types.Uint256) {
	h.Journal.TStore(addr, key, value)
}

// --- Block hash ---

func (h *EvmHost) BlockHash(number types.Uint256) types.B256 {
	if number[1] != 0 || number[2] != 0 || number[3] != 0 {
		return types.B256Zero
	}
	n := number[0]

	// EVM spec: BLOCKHASH is only valid for block numbers in [current-256, current-1].
	// EIP-2935 does NOT extend the BLOCKHASH opcode window (still 256 blocks);
	// older hashes are accessible only via direct SLOAD on the history storage contract.
	const blockHashWindow = uint64(256)
	current := h.Block.Number
	if current[1] != 0 || current[2] != 0 || current[3] != 0 {
		// Block number too large to fit in uint64; n already fits, so it's out of range.
		return types.B256Zero
	}
	currentN := current[0]
	if n >= currentN || currentN-n > blockHashWindow {
		return types.B256Zero
	}

	if h.Journal.DB == nil {
		return types.B256Zero
	}
	hash, err := h.Journal.DB.BlockHash(n)
	if err != nil {
		return types.B256Zero
	}
	return hash
}

// --- Logging ---

func (h *EvmHost) Log(addr types.Address, topics *[4]types.B256, numTopics int, data types.Bytes) {
	log := h.Journal.AllocLog()
	log.Address = addr
	log.Topics = *topics
	log.NumTopics = uint8(numTopics)
	log.Data = data
}

// --- Self destruct ---

func (h *EvmHost) SelfDestruct(addr, target types.Address) vm.SelfDestructResult {
	result, err := h.Journal.Selfdestruct(addr, target)
	if err != nil {
		return vm.SelfDestructResult{}
	}
	return vm.SelfDestructResult{
		HadValue:            result.Data.HadValue,
		TargetExists:        result.Data.TargetExists,
		IsCold:              result.IsCold,
		PreviouslyDestroyed: result.Data.PreviouslyDestroyed,
	}
}
