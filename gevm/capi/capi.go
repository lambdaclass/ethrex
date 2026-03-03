package main

/*
#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>

typedef struct {
	uint8_t beneficiary[20];
	uint8_t timestamp[32];
	uint8_t block_number[32];
	uint8_t gas_limit[32];
	uint8_t base_fee[32];
	uint8_t has_prevrandao;
	uint8_t prevrandao[32];
	uint8_t blob_gas_price[32];
} GevmBlockEnv;

typedef struct {
	uint8_t chain_id[32];
} GevmCfgEnv;

typedef struct {
	uint8_t  address[20];
	uint8_t* storage_keys;
	uintptr_t n_keys;
} GevmAccessListEntry;

typedef struct {
	uint8_t chain_id[32];
	uint8_t address[20];
	uint64_t nonce;
	uint8_t y_parity;
	uint8_t r[32];
	uint8_t s[32];
} GevmAuthorization;

typedef struct {
	uint8_t  kind;
	uint8_t  tx_type;
	uint8_t  caller[20];
	uint8_t  to[20];
	uint8_t  value[32];
	uint8_t* input;
	uintptr_t input_len;
	uint64_t gas_limit;
	uint8_t  gas_price[32];
	uint8_t  max_fee_per_gas[32];
	uint8_t  max_priority_fee_per_gas[32];
	uint8_t  max_fee_per_blob_gas[32];
	uint64_t nonce;
	GevmAccessListEntry* access_list;
	uintptr_t n_access_entries;
	uint8_t* blob_hashes;
	uintptr_t n_blob_hashes;
	GevmAuthorization* auth_list;
	uintptr_t n_auth_entries;
} GevmTxInput;

typedef struct {
	uint8_t  address[20];
	uint8_t  topics[4][32];
	uint8_t  n_topics;
	uint8_t* data;
	uintptr_t data_len;
} GevmLog;

typedef struct {
	uint8_t key[32];
	uint8_t value[32];
} GevmStorageEntry;

typedef struct {
	uint8_t  address[20];
	uint8_t  removed;
	uint8_t  has_info;
	uint8_t  balance[32];
	uint64_t nonce;
	uint8_t  code_hash[32];
	uint8_t* code;
	uintptr_t code_len;
	GevmStorageEntry* storage;
	uintptr_t n_storage;
} GevmAccountUpdate;

typedef struct {
	uint8_t  status;
	uint64_t gas_used;
	int64_t  gas_refund;
	uint8_t* output;
	uintptr_t output_len;
	GevmLog* logs;
	uintptr_t n_logs;
	uint8_t  has_created_addr;
	uint8_t  created_addr[20];
	GevmAccountUpdate* updates;
	uintptr_t n_updates;
	uint8_t  is_validation_error;
	char*    error_msg;
} GevmExecResult;

typedef int32_t (*gevm_basic_fn)(void* handle, const uint8_t addr[20], uint8_t balance_out[32], uint64_t* nonce_out, uint8_t code_hash_out[32], int32_t* exists_out);
typedef int32_t (*gevm_code_by_hash_fn)(void* handle, const uint8_t code_hash[32], uint8_t** code_out, uintptr_t* len_out);
typedef int32_t (*gevm_storage_fn)(void* handle, const uint8_t addr[20], const uint8_t key[32], uint8_t value_out[32]);
typedef int32_t (*gevm_has_storage_fn)(void* handle, const uint8_t addr[20], int32_t* has_storage_out);
typedef int32_t (*gevm_block_hash_fn)(void* handle, uint64_t block_number, uint8_t hash_out[32]);

static int32_t call_basic(gevm_basic_fn fn, void* handle, const uint8_t addr[20], uint8_t balance_out[32], uint64_t* nonce_out, uint8_t code_hash_out[32], int32_t* exists_out) {
	return fn(handle, addr, balance_out, nonce_out, code_hash_out, exists_out);
}
static int32_t call_code_by_hash(gevm_code_by_hash_fn fn, void* handle, const uint8_t code_hash[32], uint8_t** code_out, uintptr_t* len_out) {
	return fn(handle, code_hash, code_out, len_out);
}
static int32_t call_storage(gevm_storage_fn fn, void* handle, const uint8_t addr[20], const uint8_t key[32], uint8_t value_out[32]) {
	return fn(handle, addr, key, value_out);
}
static int32_t call_has_storage(gevm_has_storage_fn fn, void* handle, const uint8_t addr[20], int32_t* has_storage_out) {
	return fn(handle, addr, has_storage_out);
}
static int32_t call_block_hash(gevm_block_hash_fn fn, void* handle, uint64_t block_number, uint8_t hash_out[32]) {
	return fn(handle, block_number, hash_out);
}
*/
import "C"

import (
	"fmt"
	"runtime/cgo"
	"unsafe"

	"github.com/Giulio2002/gevm/host"
	"github.com/Giulio2002/gevm/spec"
	"github.com/Giulio2002/gevm/state"
	"github.com/Giulio2002/gevm/types"
)

// callbackDatabase implements state.Database by calling back into C.
type callbackDatabase struct {
	handle        unsafe.Pointer
	basicFn       C.gevm_basic_fn
	codeByHashFn  C.gevm_code_by_hash_fn
	storageFn     C.gevm_storage_fn
	hasStorageFn  C.gevm_has_storage_fn
	blockHashFn   C.gevm_block_hash_fn
}

func (db *callbackDatabase) Basic(address types.Address) (state.AccountInfo, bool, error) {
	var balanceOut [32]byte
	var nonceOut C.uint64_t
	var codeHashOut [32]byte
	var existsOut C.int32_t

	rc := C.call_basic(
		db.basicFn,
		db.handle,
		(*C.uint8_t)(unsafe.Pointer(&address[0])),
		(*C.uint8_t)(unsafe.Pointer(&balanceOut[0])),
		&nonceOut,
		(*C.uint8_t)(unsafe.Pointer(&codeHashOut[0])),
		&existsOut,
	)
	if rc != 0 {
		return state.AccountInfo{}, false, fmt.Errorf("gevm_basic_fn returned %d", rc)
	}
	if existsOut == 0 {
		return state.AccountInfo{}, false, nil
	}
	info := state.AccountInfo{
		Balance:  types.U256FromBytes32(balanceOut),
		Nonce:    uint64(nonceOut),
		CodeHash: types.B256(codeHashOut),
	}
	return info, true, nil
}

func (db *callbackDatabase) CodeByHash(codeHash types.B256) (types.Bytes, error) {
	var codeOut *C.uint8_t
	var lenOut C.uintptr_t

	rc := C.call_code_by_hash(
		db.codeByHashFn,
		db.handle,
		(*C.uint8_t)(unsafe.Pointer(&codeHash[0])),
		&codeOut,
		&lenOut,
	)
	if rc != 0 {
		return nil, fmt.Errorf("gevm_code_by_hash_fn returned %d", rc)
	}
	if lenOut == 0 || codeOut == nil {
		return nil, nil
	}
	// Copy into a Go slice and free the C-allocated buffer.
	code := C.GoBytes(unsafe.Pointer(codeOut), C.int(lenOut))
	C.free(unsafe.Pointer(codeOut))
	return types.Bytes(code), nil
}

func (db *callbackDatabase) Storage(address types.Address, index types.Uint256) (types.Uint256, error) {
	keyBytes := index.ToBytes32()
	var valueOut [32]byte

	rc := C.call_storage(
		db.storageFn,
		db.handle,
		(*C.uint8_t)(unsafe.Pointer(&address[0])),
		(*C.uint8_t)(unsafe.Pointer(&keyBytes[0])),
		(*C.uint8_t)(unsafe.Pointer(&valueOut[0])),
	)
	if rc != 0 {
		return types.U256Zero, fmt.Errorf("gevm_storage_fn returned %d", rc)
	}
	return types.U256FromBytes32(valueOut), nil
}

func (db *callbackDatabase) HasStorage(address types.Address) (bool, error) {
	var hasStorage C.int32_t

	rc := C.call_has_storage(
		db.hasStorageFn,
		db.handle,
		(*C.uint8_t)(unsafe.Pointer(&address[0])),
		&hasStorage,
	)
	if rc != 0 {
		return false, fmt.Errorf("gevm_has_storage_fn returned %d", rc)
	}
	return hasStorage != 0, nil
}

func (db *callbackDatabase) BlockHash(number uint64) (types.B256, error) {
	var hashOut [32]byte

	rc := C.call_block_hash(
		db.blockHashFn,
		db.handle,
		C.uint64_t(number),
		(*C.uint8_t)(unsafe.Pointer(&hashOut[0])),
	)
	if rc != 0 {
		return types.B256Zero, fmt.Errorf("gevm_block_hash_fn returned %d", rc)
	}
	return types.B256(hashOut), nil
}

// u256FromCBytes reads a 32-byte big-endian C array into a Uint256.
func u256FromCBytes(p *C.uint8_t) types.Uint256 {
	var b [32]byte
	copy(b[:], (*[32]byte)(unsafe.Pointer(p))[:])
	return types.U256FromBytes32(b)
}

// b256FromCBytes reads a 32-byte C array into a B256.
func b256FromCBytes(p *C.uint8_t) types.B256 {
	return types.B256(*(*[32]byte)(unsafe.Pointer(p)))
}

// addrFromCBytes reads a 20-byte C array into an Address.
func addrFromCBytes(p *C.uint8_t) types.Address {
	return types.Address(*(*[20]byte)(unsafe.Pointer(p)))
}

// ======================== Shared Helpers ========================

func parseBlockEnv(block *C.GevmBlockEnv) host.BlockEnv {
	blockEnv := host.BlockEnv{
		Beneficiary:  addrFromCBytes(&block.beneficiary[0]),
		Timestamp:    u256FromCBytes(&block.timestamp[0]),
		Number:       u256FromCBytes(&block.block_number[0]),
		GasLimit:     u256FromCBytes(&block.gas_limit[0]),
		BaseFee:      u256FromCBytes(&block.base_fee[0]),
		BlobGasPrice: u256FromCBytes(&block.blob_gas_price[0]),
	}
	if block.has_prevrandao != 0 {
		pr := u256FromCBytes(&block.prevrandao[0])
		blockEnv.Prevrandao = &pr
	}
	return blockEnv
}

func parseCfgEnv(cfg *C.GevmCfgEnv) host.CfgEnv {
	return host.CfgEnv{
		ChainId: u256FromCBytes(&cfg.chain_id[0]),
	}
}

func parseTxInput(tx *C.GevmTxInput) *host.Transaction {
	goTx := &host.Transaction{
		Kind:                 host.TxKind(tx.kind),
		TxType:               host.TxType(tx.tx_type),
		Caller:               addrFromCBytes(&tx.caller[0]),
		To:                   addrFromCBytes(&tx.to[0]),
		Value:                u256FromCBytes(&tx.value[0]),
		GasLimit:             uint64(tx.gas_limit),
		GasPrice:             u256FromCBytes(&tx.gas_price[0]),
		MaxFeePerGas:         u256FromCBytes(&tx.max_fee_per_gas[0]),
		MaxPriorityFeePerGas: u256FromCBytes(&tx.max_priority_fee_per_gas[0]),
		MaxFeePerBlobGas:     u256FromCBytes(&tx.max_fee_per_blob_gas[0]),
		Nonce:                uint64(tx.nonce),
	}

	// Input bytes.
	if tx.input_len > 0 && tx.input != nil {
		goTx.Input = C.GoBytes(unsafe.Pointer(tx.input), C.int(tx.input_len))
	}

	// Access list.
	if tx.n_access_entries > 0 && tx.access_list != nil {
		entries := (*[1 << 28]C.GevmAccessListEntry)(unsafe.Pointer(tx.access_list))[:tx.n_access_entries:tx.n_access_entries]
		goTx.AccessList = make([]host.AccessListItem, len(entries))
		for i, e := range entries {
			item := host.AccessListItem{
				Address: addrFromCBytes(&e.address[0]),
			}
			if e.n_keys > 0 && e.storage_keys != nil {
				keyBytes := (*[1 << 28]C.uint8_t)(unsafe.Pointer(e.storage_keys))[:e.n_keys*32 : e.n_keys*32]
				item.StorageKeys = make([]types.Uint256, e.n_keys)
				for k := C.uintptr_t(0); k < e.n_keys; k++ {
					var b32 [32]byte
					for j := 0; j < 32; j++ {
						b32[j] = byte(keyBytes[k*32+C.uintptr_t(j)])
					}
					item.StorageKeys[k] = types.U256FromBytes32(b32)
				}
			}
			goTx.AccessList[i] = item
		}
	}

	// Blob hashes (each 32 bytes, packed).
	if tx.n_blob_hashes > 0 && tx.blob_hashes != nil {
		blobBytes := (*[1 << 28]C.uint8_t)(unsafe.Pointer(tx.blob_hashes))[:tx.n_blob_hashes*32 : tx.n_blob_hashes*32]
		goTx.BlobHashes = make([]types.Uint256, tx.n_blob_hashes)
		for k := C.uintptr_t(0); k < tx.n_blob_hashes; k++ {
			var b32 [32]byte
			for j := 0; j < 32; j++ {
				b32[j] = byte(blobBytes[k*32+C.uintptr_t(j)])
			}
			goTx.BlobHashes[k] = types.U256FromBytes32(b32)
		}
	}

	// Authorization list.
	if tx.n_auth_entries > 0 && tx.auth_list != nil {
		auths := (*[1 << 20]C.GevmAuthorization)(unsafe.Pointer(tx.auth_list))[:tx.n_auth_entries:tx.n_auth_entries]
		goTx.AuthorizationList = make([]host.Authorization, len(auths))
		for i, a := range auths {
			goTx.AuthorizationList[i] = host.Authorization{
				ChainId: u256FromCBytes(&a.chain_id[0]),
				Address: addrFromCBytes(&a.address[0]),
				Nonce:   uint64(a.nonce),
				YParity: uint8(a.y_parity),
				R:       b256FromCBytes(&a.r[0]),
				S:       b256FromCBytes(&a.s[0]),
			}
		}
	}

	return goTx
}

func buildCResult(result host.ExecutionResult, updates []accountUpdate) *C.GevmExecResult {
	res := (*C.GevmExecResult)(C.calloc(1, C.size_t(unsafe.Sizeof(C.GevmExecResult{}))))

	// Status: 0 = success, 1 = revert, 2 = halt.
	switch result.Kind {
	case host.ResultSuccess:
		res.status = 0
	case host.ResultRevert:
		res.status = 1
	default:
		res.status = 2
	}

	res.gas_used = C.uint64_t(result.GasUsed)
	res.gas_refund = C.int64_t(result.GasRefund)

	// Output.
	if len(result.Output) > 0 {
		res.output = (*C.uint8_t)(C.malloc(C.size_t(len(result.Output))))
		copy((*[1 << 30]byte)(unsafe.Pointer(res.output))[:len(result.Output)], result.Output)
		res.output_len = C.uintptr_t(len(result.Output))
	}

	// Logs.
	if len(result.Logs) > 0 {
		res.n_logs = C.uintptr_t(len(result.Logs))
		res.logs = (*C.GevmLog)(C.calloc(C.size_t(len(result.Logs)), C.size_t(unsafe.Sizeof(C.GevmLog{}))))
		logs := (*[1 << 20]C.GevmLog)(unsafe.Pointer(res.logs))[:len(result.Logs):len(result.Logs)]
		for i, l := range result.Logs {
			addr := l.Address
			copy((*[20]byte)(unsafe.Pointer(&logs[i].address[0]))[:], addr[:])
			logs[i].n_topics = C.uint8_t(l.NumTopics)
			for t := uint8(0); t < l.NumTopics; t++ {
				copy((*[32]byte)(unsafe.Pointer(&logs[i].topics[t][0]))[:], l.Topics[t][:])
			}
			if len(l.Data) > 0 {
				logs[i].data = (*C.uint8_t)(C.malloc(C.size_t(len(l.Data))))
				copy((*[1 << 30]byte)(unsafe.Pointer(logs[i].data))[:len(l.Data)], l.Data)
				logs[i].data_len = C.uintptr_t(len(l.Data))
			}
		}
	}

	// Created address.
	if result.CreatedAddr != nil {
		res.has_created_addr = 1
		copy((*[20]byte)(unsafe.Pointer(&res.created_addr[0]))[:], result.CreatedAddr[:])
	}

	// Account updates.
	if len(updates) > 0 {
		res.n_updates = C.uintptr_t(len(updates))
		res.updates = (*C.GevmAccountUpdate)(C.calloc(C.size_t(len(updates)), C.size_t(unsafe.Sizeof(C.GevmAccountUpdate{}))))
		updSlice := (*[1 << 20]C.GevmAccountUpdate)(unsafe.Pointer(res.updates))[:len(updates):len(updates)]
		for i, u := range updates {
			copy((*[20]byte)(unsafe.Pointer(&updSlice[i].address[0]))[:], u.address[:])
			if u.removed {
				updSlice[i].removed = 1
			}
			if u.hasInfo {
				updSlice[i].has_info = 1
				bal := u.balance.ToBytes32()
				copy((*[32]byte)(unsafe.Pointer(&updSlice[i].balance[0]))[:], bal[:])
				updSlice[i].nonce = C.uint64_t(u.nonce)
				copy((*[32]byte)(unsafe.Pointer(&updSlice[i].code_hash[0]))[:], u.codeHash[:])
				if len(u.code) > 0 {
					updSlice[i].code = (*C.uint8_t)(C.malloc(C.size_t(len(u.code))))
					copy((*[1 << 30]byte)(unsafe.Pointer(updSlice[i].code))[:len(u.code)], u.code)
					updSlice[i].code_len = C.uintptr_t(len(u.code))
				}
			}
			if len(u.storage) > 0 {
				updSlice[i].n_storage = C.uintptr_t(len(u.storage))
				updSlice[i].storage = (*C.GevmStorageEntry)(C.calloc(C.size_t(len(u.storage)), C.size_t(unsafe.Sizeof(C.GevmStorageEntry{}))))
				storSlice := (*[1 << 20]C.GevmStorageEntry)(unsafe.Pointer(updSlice[i].storage))[:len(u.storage):len(u.storage)]
				for j, s := range u.storage {
					kBytes := s.key.ToBytes32()
					vBytes := s.value.ToBytes32()
					copy((*[32]byte)(unsafe.Pointer(&storSlice[j].key[0]))[:], kBytes[:])
					copy((*[32]byte)(unsafe.Pointer(&storSlice[j].value[0]))[:], vBytes[:])
				}
			}
		}
	}

	if result.ValidationError {
		res.is_validation_error = 1
	}

	return res
}

// ======================== Single-shot API ========================

//export gevm_execute
func gevm_execute(
	forkID C.uint8_t,
	block *C.GevmBlockEnv,
	cfg *C.GevmCfgEnv,
	tx *C.GevmTxInput,
	dbHandle unsafe.Pointer,
	basicFn C.gevm_basic_fn,
	codeByHashFn C.gevm_code_by_hash_fn,
	storageFn C.gevm_storage_fn,
	hasStorageFn C.gevm_has_storage_fn,
	blockHashFn C.gevm_block_hash_fn,
) *C.GevmExecResult {
	fid, err := spec.ForkIDFromByte(uint8(forkID))
	if err != nil {
		return allocErrorResult(fmt.Sprintf("invalid fork id: %v", err))
	}

	db := &callbackDatabase{
		handle:       dbHandle,
		basicFn:      basicFn,
		codeByHashFn: codeByHashFn,
		storageFn:    storageFn,
		hasStorageFn: hasStorageFn,
		blockHashFn:  blockHashFn,
	}

	blockEnv := parseBlockEnv(block)
	cfgEnv := parseCfgEnv(cfg)
	goTx := parseTxInput(tx)

	evm := host.NewEvm(db, fid, blockEnv, cfgEnv)
	result := evm.Transact(goTx)
	updates := buildAccountUpdates(evm, result)
	evm.ReleaseEvm()

	return buildCResult(result, updates)
}

// ======================== Persistent Context API ========================

// GevmContext holds a persistent Go EVM across multiple transactions.
type GevmContext struct {
	evm *host.Evm
}

//export gevm_create_context
func gevm_create_context(
	forkID C.uint8_t,
	block *C.GevmBlockEnv,
	cfg *C.GevmCfgEnv,
	dbHandle unsafe.Pointer,
	basicFn C.gevm_basic_fn,
	codeByHashFn C.gevm_code_by_hash_fn,
	storageFn C.gevm_storage_fn,
	hasStorageFn C.gevm_has_storage_fn,
	blockHashFn C.gevm_block_hash_fn,
) C.uintptr_t {
	fid, err := spec.ForkIDFromByte(uint8(forkID))
	if err != nil {
		return 0
	}

	db := &callbackDatabase{
		handle:       dbHandle,
		basicFn:      basicFn,
		codeByHashFn: codeByHashFn,
		storageFn:    storageFn,
		hasStorageFn: hasStorageFn,
		blockHashFn:  blockHashFn,
	}

	blockEnv := parseBlockEnv(block)
	cfgEnv := parseCfgEnv(cfg)

	evm := host.NewEvm(db, fid, blockEnv, cfgEnv)
	ctx := &GevmContext{evm: evm}
	h := cgo.NewHandle(ctx)
	return C.uintptr_t(h)
}

//export gevm_context_transact
func gevm_context_transact(handle C.uintptr_t, tx *C.GevmTxInput) *C.GevmExecResult {
	h := cgo.Handle(handle)
	ctx := h.Value().(*GevmContext)

	goTx := parseTxInput(tx)
	result := ctx.evm.Transact(goTx)

	// Extract per-tx account updates BEFORE CommitTx/DiscardTx
	// (OriginalInfo resets on next cold load, so we must capture now).
	updates := buildAccountUpdates(ctx.evm, result)

	if result.ValidationError {
		ctx.evm.Journal.DiscardTx()
	} else {
		ctx.evm.Journal.CommitTx()
	}

	return buildCResult(result, updates)
}

//export gevm_free_context
func gevm_free_context(handle C.uintptr_t) {
	h := cgo.Handle(handle)
	ctx := h.Value().(*GevmContext)
	ctx.evm.ReleaseEvm()
	h.Delete()
}

// ======================== Result Freeing ========================

//export gevm_free_result
func gevm_free_result(res *C.GevmExecResult) {
	if res == nil {
		return
	}
	if res.output != nil {
		C.free(unsafe.Pointer(res.output))
	}
	if res.error_msg != nil {
		C.free(unsafe.Pointer(res.error_msg))
	}
	if res.logs != nil {
		logs := (*[1 << 20]C.GevmLog)(unsafe.Pointer(res.logs))[:res.n_logs:res.n_logs]
		for i := range logs {
			if logs[i].data != nil {
				C.free(unsafe.Pointer(logs[i].data))
			}
		}
		C.free(unsafe.Pointer(res.logs))
	}
	if res.updates != nil {
		updates := (*[1 << 20]C.GevmAccountUpdate)(unsafe.Pointer(res.updates))[:res.n_updates:res.n_updates]
		for i := range updates {
			if updates[i].code != nil {
				C.free(unsafe.Pointer(updates[i].code))
			}
			if updates[i].storage != nil {
				C.free(unsafe.Pointer(updates[i].storage))
			}
		}
		C.free(unsafe.Pointer(res.updates))
	}
	C.free(unsafe.Pointer(res))
}

// ======================== Internal Helpers ========================

// accountUpdate is a Go-side representation of a modified account.
type accountUpdate struct {
	address  types.Address
	removed  bool
	hasInfo  bool
	balance  types.Uint256
	nonce    uint64
	codeHash types.B256
	code     types.Bytes
	storage  []storageEntry
}

type storageEntry struct {
	key   types.Uint256
	value types.Uint256
}

// buildAccountUpdates inspects the journal state and constructs account updates.
//
// NOTE: This function must be called with the journal still alive (before
// ReleaseEvm or CommitTx/DiscardTx).
func buildAccountUpdates(evm *host.Evm, _ host.ExecutionResult) []accountUpdate {
	journal := evm.Journal
	if journal == nil {
		return nil
	}

	var updates []accountUpdate

	for addr, acc := range journal.State {
		if acc.IsSelfdestructedLocally() {
			updates = append(updates, accountUpdate{
				address: addr,
				removed: true,
			})
			continue
		}

		infoChanged := acc.Info.Balance != acc.OriginalInfo.Balance ||
			acc.Info.Nonce != acc.OriginalInfo.Nonce ||
			acc.Info.CodeHash != acc.OriginalInfo.CodeHash

		var changedSlots []storageEntry
		for key, slot := range acc.Storage {
			if slot.IsChanged() {
				changedSlots = append(changedSlots, storageEntry{
					key:   key,
					value: slot.PresentValue,
				})
			}
		}

		if !infoChanged && len(changedSlots) == 0 {
			continue
		}

		u := accountUpdate{
			address: addr,
			hasInfo: infoChanged,
			storage: changedSlots,
		}
		if infoChanged {
			u.balance = acc.Info.Balance
			u.nonce = acc.Info.Nonce
			u.codeHash = acc.Info.CodeHash
			u.code = acc.Info.Code
		}
		updates = append(updates, u)
	}

	return updates
}

// allocErrorResult allocates a GevmExecResult with only the error_msg set and
// status=2 (halt), is_validation_error=1.
func allocErrorResult(msg string) *C.GevmExecResult {
	res := (*C.GevmExecResult)(C.calloc(1, C.size_t(unsafe.Sizeof(C.GevmExecResult{}))))
	res.status = 2
	res.is_validation_error = 1
	cMsg := C.CString(msg)
	res.error_msg = cMsg
	return res
}

func main() {}
