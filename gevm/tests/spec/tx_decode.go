// Transaction decoding from RLP for TransactionTests.
// Handles legacy, EIP-2930, EIP-1559, and EIP-4844 transaction formats.
package spec

import (
	"fmt"

	"github.com/Giulio2002/gevm/precompiles"
	"github.com/Giulio2002/gevm/types"
)

// DecodedTx represents a decoded Ethereum transaction.
type DecodedTx struct {
	TxType               int // 0=legacy, 1=EIP-2930, 2=EIP-1559, 3=EIP-4844
	ChainId              *uint64
	Nonce                uint64
	GasPrice             types.Uint256 // legacy/EIP-2930
	MaxPriorityFeePerGas types.Uint256 // EIP-1559+
	MaxFeePerGas         types.Uint256 // EIP-1559+
	GasLimit             uint64
	To                   *types.Address // nil for CREATE
	Value                types.Uint256
	Data                 []byte
	AccessList           []DecodedAccessListEntry // EIP-2930+
	MaxFeePerBlobGas     types.Uint256          // EIP-4844
	BlobHashes           []types.B256        // EIP-4844
	V                    types.Uint256
	R                    types.Uint256
	S                    types.Uint256
}

// DecodedAccessListEntry is one entry in a decoded access list.
type DecodedAccessListEntry struct {
	Address     types.Address
	StorageKeys []types.B256
}

// TxDecodeError wraps a decoding error with a kind string.
type TxDecodeError struct {
	Kind    string
	Message string
}

func (e *TxDecodeError) Error() string {
	return fmt.Sprintf("%s: %s", e.Kind, e.Message)
}

func txErr(kind, msg string) error {
	return &TxDecodeError{Kind: kind, Message: msg}
}

// DecodeTx decodes a raw transaction from bytes.
func DecodeTx(txbytes []byte) (*DecodedTx, error) {
	if len(txbytes) == 0 {
		return nil, txErr("RLP_ERROR_SIZE", "empty transaction bytes")
	}

	firstByte := txbytes[0]

	if firstByte >= 0xc0 {
		// Legacy transaction: entire bytes are an RLP list
		item, err := RlpDecodeComplete(txbytes)
		if err != nil {
			return nil, txErr("RLP_ERROR_SIZE", err.Error())
		}
		if item.Kind != RlpList {
			return nil, txErr("RLP_ERROR_SIZE", "expected RLP list for legacy tx")
		}
		return decodeLegacyTx(item.Items)
	}

	// Typed transaction: first byte is the type, rest is RLP
	if firstByte > 0x03 {
		return nil, txErr("RLP_ERROR_SIZE", fmt.Sprintf("unsupported tx type %d", firstByte))
	}

	payload := txbytes[1:]
	item, err := RlpDecodeComplete(payload)
	if err != nil {
		return nil, txErr("RLP_ERROR_SIZE", err.Error())
	}
	if item.Kind != RlpList {
		return nil, txErr("RLP_ERROR_SIZE", "expected RLP list for typed tx")
	}

	switch firstByte {
	case 0x01:
		return decodeEIP2930Tx(item.Items)
	case 0x02:
		return decodeEIP1559Tx(item.Items)
	case 0x03:
		return decodeEIP4844Tx(item.Items)
	default:
		return nil, txErr("RLP_ERROR_SIZE", fmt.Sprintf("unsupported tx type %d", firstByte))
	}
}

// decodeLegacyTx decodes a legacy (type 0) transaction from RLP list items.
// Format: [nonce, gasPrice, gasLimit, to, value, data, v, r, s]
func decodeLegacyTx(items []RlpItem) (*DecodedTx, error) {
	if len(items) != 9 {
		return nil, txErr("RLP_ERROR_SIZE", fmt.Sprintf("legacy tx: expected 9 items, got %d", len(items)))
	}

	tx := &DecodedTx{TxType: 0}

	var err error
	// nonce
	tx.Nonce, err = items[0].AsUint64()
	if err != nil {
		return nil, txErr("NONCE_OVERFLOW", err.Error())
	}

	// gasPrice
	tx.GasPrice, err = items[1].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_GAS_PRICE", err.Error())
	}

	// gasLimit
	tx.GasLimit, err = items[2].AsUint64()
	if err != nil {
		return nil, txErr("GAS_LIMIT_OVERFLOW", err.Error())
	}

	// to
	toBytes := items[3].AsBytes()
	if len(toBytes) == 0 {
		// CREATE
		tx.To = nil
	} else if len(toBytes) == 20 {
		var addr types.Address
		copy(addr[:], toBytes)
		tx.To = &addr
	} else if len(toBytes) > 20 {
		return nil, txErr("ADDRESS_TOO_LONG", fmt.Sprintf("to field: %d bytes", len(toBytes)))
	} else {
		return nil, txErr("ADDRESS_TOO_SHORT", fmt.Sprintf("to field: %d bytes", len(toBytes)))
	}

	// value
	tx.Value, err = items[4].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_VALUE", err.Error())
	}

	// data
	tx.Data = items[5].AsBytes()

	// v, r, s
	tx.V, err = items[6].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_V", err.Error())
	}
	tx.R, err = items[7].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_R", err.Error())
	}
	tx.S, err = items[8].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_S", err.Error())
	}

	// Derive chainId from V for EIP-155
	// Legacy pre-EIP-155: V = 27 or 28
	// EIP-155: V = 2*chainId + 35 + {0,1}
	vU64 := tx.V.AsUsize()
	if vU64 != 27 && vU64 != 28 {
		// EIP-155
		if vU64 >= 35 {
			chainId := (vU64 - 35) / 2
			tx.ChainId = &chainId
		}
		// else: invalid V, let signing hash/recovery handle it
	}

	return tx, nil
}

// decodeEIP2930Tx decodes a type 1 (EIP-2930) transaction.
// Format: [chainId, nonce, gasPrice, gasLimit, to, value, data, accessList, signatureYParity, signatureR, signatureS]
func decodeEIP2930Tx(items []RlpItem) (*DecodedTx, error) {
	if len(items) != 11 {
		return nil, txErr("RLP_ERROR_SIZE", fmt.Sprintf("EIP-2930 tx: expected 11 items, got %d", len(items)))
	}

	tx := &DecodedTx{TxType: 1}

	var err error
	// chainId
	chainId, err := items[0].AsUint64()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_CHAIN_ID", err.Error())
	}
	tx.ChainId = &chainId

	// nonce
	tx.Nonce, err = items[1].AsUint64()
	if err != nil {
		return nil, txErr("NONCE_OVERFLOW", err.Error())
	}

	// gasPrice
	tx.GasPrice, err = items[2].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_GAS_PRICE", err.Error())
	}

	// gasLimit
	tx.GasLimit, err = items[3].AsUint64()
	if err != nil {
		return nil, txErr("GAS_LIMIT_OVERFLOW", err.Error())
	}

	// to
	toBytes := items[4].AsBytes()
	if len(toBytes) == 0 {
		tx.To = nil
	} else if len(toBytes) == 20 {
		var addr types.Address
		copy(addr[:], toBytes)
		tx.To = &addr
	} else if len(toBytes) > 20 {
		return nil, txErr("ADDRESS_TOO_LONG", fmt.Sprintf("to field: %d bytes", len(toBytes)))
	} else {
		return nil, txErr("ADDRESS_TOO_SHORT", fmt.Sprintf("to field: %d bytes", len(toBytes)))
	}

	// value
	tx.Value, err = items[5].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_VALUE", err.Error())
	}

	// data
	tx.Data = items[6].AsBytes()

	// accessList
	tx.AccessList, err = decodeAccessList(&items[7])
	if err != nil {
		return nil, err
	}

	// signatureYParity, signatureR, signatureS
	tx.V, err = items[8].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_V", err.Error())
	}
	tx.R, err = items[9].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_R", err.Error())
	}
	tx.S, err = items[10].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_S", err.Error())
	}

	return tx, nil
}

// decodeEIP1559Tx decodes a type 2 (EIP-1559) transaction.
// Format: [chainId, nonce, maxPriorityFeePerGas, maxFeePerGas, gasLimit, to, value, data, accessList, signatureYParity, signatureR, signatureS]
func decodeEIP1559Tx(items []RlpItem) (*DecodedTx, error) {
	if len(items) != 12 {
		return nil, txErr("RLP_ERROR_SIZE", fmt.Sprintf("EIP-1559 tx: expected 12 items, got %d", len(items)))
	}

	tx := &DecodedTx{TxType: 2}

	var err error
	// chainId
	chainId, err := items[0].AsUint64()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_CHAIN_ID", err.Error())
	}
	tx.ChainId = &chainId

	// nonce
	tx.Nonce, err = items[1].AsUint64()
	if err != nil {
		return nil, txErr("NONCE_OVERFLOW", err.Error())
	}

	// maxPriorityFeePerGas
	tx.MaxPriorityFeePerGas, err = items[2].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_MAX_PRIORITY_FEE", err.Error())
	}

	// maxFeePerGas
	tx.MaxFeePerGas, err = items[3].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_MAX_FEE", err.Error())
	}

	// gasLimit
	tx.GasLimit, err = items[4].AsUint64()
	if err != nil {
		return nil, txErr("GAS_LIMIT_OVERFLOW", err.Error())
	}

	// to
	toBytes := items[5].AsBytes()
	if len(toBytes) == 0 {
		tx.To = nil
	} else if len(toBytes) == 20 {
		var addr types.Address
		copy(addr[:], toBytes)
		tx.To = &addr
	} else if len(toBytes) > 20 {
		return nil, txErr("ADDRESS_TOO_LONG", fmt.Sprintf("to field: %d bytes", len(toBytes)))
	} else {
		return nil, txErr("ADDRESS_TOO_SHORT", fmt.Sprintf("to field: %d bytes", len(toBytes)))
	}

	// value
	tx.Value, err = items[6].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_VALUE", err.Error())
	}

	// data
	tx.Data = items[7].AsBytes()

	// accessList
	tx.AccessList, err = decodeAccessList(&items[8])
	if err != nil {
		return nil, err
	}

	// signatureYParity, signatureR, signatureS
	tx.V, err = items[9].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_V", err.Error())
	}
	tx.R, err = items[10].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_R", err.Error())
	}
	tx.S, err = items[11].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_S", err.Error())
	}

	return tx, nil
}

// decodeEIP4844Tx decodes a type 3 (EIP-4844) transaction.
// Format: [chainId, nonce, maxPriorityFeePerGas, maxFeePerGas, gasLimit, to, value, data, accessList, maxFeePerBlobGas, blobVersionedHashes, signatureYParity, signatureR, signatureS]
func decodeEIP4844Tx(items []RlpItem) (*DecodedTx, error) {
	if len(items) != 14 {
		return nil, txErr("RLP_ERROR_SIZE", fmt.Sprintf("EIP-4844 tx: expected 14 items, got %d", len(items)))
	}

	tx := &DecodedTx{TxType: 3}

	var err error
	// chainId
	chainId, err := items[0].AsUint64()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_CHAIN_ID", err.Error())
	}
	tx.ChainId = &chainId

	// nonce
	tx.Nonce, err = items[1].AsUint64()
	if err != nil {
		return nil, txErr("NONCE_OVERFLOW", err.Error())
	}

	// maxPriorityFeePerGas
	tx.MaxPriorityFeePerGas, err = items[2].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_MAX_PRIORITY_FEE", err.Error())
	}

	// maxFeePerGas
	tx.MaxFeePerGas, err = items[3].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_MAX_FEE", err.Error())
	}

	// gasLimit
	tx.GasLimit, err = items[4].AsUint64()
	if err != nil {
		return nil, txErr("GAS_LIMIT_OVERFLOW", err.Error())
	}

	// to
	toBytes := items[5].AsBytes()
	if len(toBytes) == 0 {
		tx.To = nil
	} else if len(toBytes) == 20 {
		var addr types.Address
		copy(addr[:], toBytes)
		tx.To = &addr
	} else if len(toBytes) > 20 {
		return nil, txErr("ADDRESS_TOO_LONG", fmt.Sprintf("to field: %d bytes", len(toBytes)))
	} else {
		return nil, txErr("ADDRESS_TOO_SHORT", fmt.Sprintf("to field: %d bytes", len(toBytes)))
	}

	// value
	tx.Value, err = items[6].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_VALUE", err.Error())
	}

	// data
	tx.Data = items[7].AsBytes()

	// accessList
	tx.AccessList, err = decodeAccessList(&items[8])
	if err != nil {
		return nil, err
	}

	// maxFeePerBlobGas
	tx.MaxFeePerBlobGas, err = items[9].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_MAX_FEE_PER_BLOB_GAS", err.Error())
	}

	// blobVersionedHashes
	if items[10].Kind != RlpList {
		return nil, txErr("RLP_ERROR_SIZE", "blobVersionedHashes must be a list")
	}
	for _, hashItem := range items[10].Items {
		b := hashItem.AsBytes()
		if len(b) != 32 {
			return nil, txErr("RLP_ERROR_SIZE", fmt.Sprintf("blob hash: expected 32 bytes, got %d", len(b)))
		}
		var h types.B256
		copy(h[:], b)
		tx.BlobHashes = append(tx.BlobHashes, h)
	}

	// signatureYParity, signatureR, signatureS
	tx.V, err = items[11].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_V", err.Error())
	}
	tx.R, err = items[12].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_R", err.Error())
	}
	tx.S, err = items[13].AsU256()
	if err != nil {
		return nil, txErr("RLP_LEADING_ZEROS_S", err.Error())
	}

	return tx, nil
}

// decodeAccessList decodes an access list from an RLP item.
func decodeAccessList(item *RlpItem) ([]DecodedAccessListEntry, error) {
	if item.Kind != RlpList {
		return nil, txErr("RLP_ERROR_SIZE", "access list must be a list")
	}
	var result []DecodedAccessListEntry
	for _, entryItem := range item.Items {
		if entryItem.Kind != RlpList || len(entryItem.Items) != 2 {
			return nil, txErr("RLP_ERROR_SIZE", "access list entry must be [address, storageKeys]")
		}
		addrBytes := entryItem.Items[0].AsBytes()
		if len(addrBytes) > 20 {
			return nil, txErr("ADDRESS_TOO_LONG", fmt.Sprintf("access list address: %d bytes", len(addrBytes)))
		}
		if len(addrBytes) != 0 && len(addrBytes) != 20 {
			return nil, txErr("ADDRESS_TOO_SHORT", fmt.Sprintf("access list address: %d bytes", len(addrBytes)))
		}
		var addr types.Address
		copy(addr[:], addrBytes)

		keysItem := entryItem.Items[1]
		if keysItem.Kind != RlpList {
			return nil, txErr("RLP_ERROR_SIZE", "storage keys must be a list")
		}
		var keys []types.B256
		for _, kItem := range keysItem.Items {
			kb := kItem.AsBytes()
			if len(kb) > 32 {
				return nil, txErr("RLP_INVALID_ACCESS_LIST_STORAGE_TOO_LONG", fmt.Sprintf("storage key: %d bytes", len(kb)))
			}
			if len(kb) != 0 && len(kb) != 32 {
				return nil, txErr("RLP_INVALID_ACCESS_LIST_STORAGE_TOO_SHORT", fmt.Sprintf("storage key: %d bytes", len(kb)))
			}
			var k types.B256
			copy(k[:], kb)
			keys = append(keys, k)
		}

		result = append(result, DecodedAccessListEntry{
			Address:     addr,
			StorageKeys: keys,
		})
	}
	return result, nil
}

// SigningHash computes the signing hash for a decoded transaction.
func SigningHash(tx *DecodedTx) types.B256 {
	switch tx.TxType {
	case 0:
		return legacySigningHash(tx)
	case 1:
		return typedSigningHash(0x01, eip2930SigningPayload(tx))
	case 2:
		return typedSigningHash(0x02, eip1559SigningPayload(tx))
	case 3:
		return typedSigningHash(0x03, eip4844SigningPayload(tx))
	default:
		return types.B256Zero
	}
}

// legacySigningHash computes the signing hash for legacy transactions.
func legacySigningHash(tx *DecodedTx) types.B256 {
	// Determine if EIP-155 from V
	vU64 := tx.V.AsUsize()

	var items [][]byte

	items = append(items, RlpEncodeUint64(tx.Nonce))
	items = append(items, RlpEncodeU256(tx.GasPrice))
	items = append(items, RlpEncodeUint64(tx.GasLimit))
	items = append(items, rlpEncodeTo(tx.To))
	items = append(items, RlpEncodeU256(tx.Value))
	items = append(items, RlpEncodeBytes(tx.Data))

	if vU64 != 27 && vU64 != 28 {
		// EIP-155: append [chainId, 0, 0]
		if tx.ChainId != nil {
			items = append(items, RlpEncodeUint64(*tx.ChainId))
		} else {
			items = append(items, RlpEncodeUint64(0))
		}
		items = append(items, RlpEncodeBytes(nil)) // 0 as empty bytes
		items = append(items, RlpEncodeBytes(nil)) // 0 as empty bytes
	}

	encoded := RlpEncodeList(items)
	return types.Keccak256(encoded)
}

// eip2930SigningPayload returns the RLP payload (without type byte) for EIP-2930 signing.
func eip2930SigningPayload(tx *DecodedTx) []byte {
	var items [][]byte
	chainId := uint64(0)
	if tx.ChainId != nil {
		chainId = *tx.ChainId
	}
	items = append(items, RlpEncodeUint64(chainId))
	items = append(items, RlpEncodeUint64(tx.Nonce))
	items = append(items, RlpEncodeU256(tx.GasPrice))
	items = append(items, RlpEncodeUint64(tx.GasLimit))
	items = append(items, rlpEncodeTo(tx.To))
	items = append(items, RlpEncodeU256(tx.Value))
	items = append(items, RlpEncodeBytes(tx.Data))
	items = append(items, encodeAccessListRLP(tx.AccessList))
	return RlpEncodeList(items)
}

// eip1559SigningPayload returns the RLP payload for EIP-1559 signing.
func eip1559SigningPayload(tx *DecodedTx) []byte {
	var items [][]byte
	chainId := uint64(0)
	if tx.ChainId != nil {
		chainId = *tx.ChainId
	}
	items = append(items, RlpEncodeUint64(chainId))
	items = append(items, RlpEncodeUint64(tx.Nonce))
	items = append(items, RlpEncodeU256(tx.MaxPriorityFeePerGas))
	items = append(items, RlpEncodeU256(tx.MaxFeePerGas))
	items = append(items, RlpEncodeUint64(tx.GasLimit))
	items = append(items, rlpEncodeTo(tx.To))
	items = append(items, RlpEncodeU256(tx.Value))
	items = append(items, RlpEncodeBytes(tx.Data))
	items = append(items, encodeAccessListRLP(tx.AccessList))
	return RlpEncodeList(items)
}

// eip4844SigningPayload returns the RLP payload for EIP-4844 signing.
func eip4844SigningPayload(tx *DecodedTx) []byte {
	var items [][]byte
	chainId := uint64(0)
	if tx.ChainId != nil {
		chainId = *tx.ChainId
	}
	items = append(items, RlpEncodeUint64(chainId))
	items = append(items, RlpEncodeUint64(tx.Nonce))
	items = append(items, RlpEncodeU256(tx.MaxPriorityFeePerGas))
	items = append(items, RlpEncodeU256(tx.MaxFeePerGas))
	items = append(items, RlpEncodeUint64(tx.GasLimit))
	items = append(items, rlpEncodeTo(tx.To))
	items = append(items, RlpEncodeU256(tx.Value))
	items = append(items, RlpEncodeBytes(tx.Data))
	items = append(items, encodeAccessListRLP(tx.AccessList))
	items = append(items, RlpEncodeU256(tx.MaxFeePerBlobGas))
	// blob versioned hashes
	var hashItems [][]byte
	for _, h := range tx.BlobHashes {
		hashItems = append(hashItems, RlpEncodeBytes(h[:]))
	}
	items = append(items, RlpEncodeList(hashItems))
	return RlpEncodeList(items)
}

// typedSigningHash computes Keccak256(type_byte || rlp_payload).
func typedSigningHash(typeByte byte, rlpPayload []byte) types.B256 {
	buf := make([]byte, 1+len(rlpPayload))
	buf[0] = typeByte
	copy(buf[1:], rlpPayload)
	return types.Keccak256(buf)
}

// rlpEncodeTo encodes the "to" address field:
// nil (CREATE) => empty bytes, non-nil => 20-byte address.
func rlpEncodeTo(to *types.Address) []byte {
	if to == nil {
		return RlpEncodeBytes(nil)
	}
	return RlpEncodeBytes(to[:])
}

// encodeAccessListRLP encodes an access list as RLP.
func encodeAccessListRLP(al []DecodedAccessListEntry) []byte {
	var entries [][]byte
	for _, entry := range al {
		var keyItems [][]byte
		for _, k := range entry.StorageKeys {
			keyItems = append(keyItems, RlpEncodeBytes(k[:]))
		}
		entryItems := [][]byte{
			RlpEncodeBytes(entry.Address[:]),
			RlpEncodeList(keyItems),
		}
		entries = append(entries, RlpEncodeList(entryItems))
	}
	return RlpEncodeList(entries)
}

// RecoverSender recovers the sender address from a decoded transaction.
func RecoverSender(tx *DecodedTx) (types.Address, error) {
	sigHash := SigningHash(tx)

	// Extract recid from V
	var recid byte
	switch tx.TxType {
	case 0:
		// Legacy: V=27/28 (pre-EIP-155) or V=2*chainId+35+{0,1} (EIP-155)
		vU64 := tx.V.AsUsize()
		if vU64 == 27 || vU64 == 28 {
			recid = byte(vU64 - 27)
		} else if vU64 >= 35 {
			recid = byte((vU64 - 35) % 2)
		} else {
			return types.AddressZero, fmt.Errorf("invalid legacy V value: %d", vU64)
		}
	case 1, 2, 3:
		// Typed txs: V is the yParity (0 or 1)
		vU64 := tx.V.AsUsize()
		if vU64 > 1 {
			return types.AddressZero, fmt.Errorf("invalid yParity: %d", vU64)
		}
		recid = byte(vU64)
	default:
		return types.AddressZero, fmt.Errorf("unsupported tx type for recovery: %d", tx.TxType)
	}

	// Build 64-byte signature [R(32) || S(32)]
	rBytes := tx.R.ToBytes32()
	sBytes := tx.S.ToBytes32()
	var sig [64]byte
	copy(sig[0:32], rBytes[:])
	copy(sig[32:64], sBytes[:])

	// Use precompiles.Ecrecover
	hashBytes := [32]byte(sigHash)
	padded, ok := precompiles.Ecrecover(sig, recid, hashBytes)
	if !ok {
		return types.AddressZero, fmt.Errorf("ecrecover failed")
	}

	// padded is [12 zero bytes || 20-byte address]
	var addr types.Address
	copy(addr[:], padded[12:])
	return addr, nil
}
