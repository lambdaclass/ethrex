// JSON deserialization types for Ethereum BlockchainTest fixtures.
package spec

import (
	"encoding/json"

	"github.com/Giulio2002/gevm/types"
	gevmspec "github.com/Giulio2002/gevm/spec"
)

// BlockchainTestSuite is the top-level map: test name -> BlockchainTestCase.
type BlockchainTestSuite map[string]*BlockchainTestCase

// BlockchainTestCase is a single blockchain test case.
type BlockchainTestCase struct {
	GenesisBlockHeader BlockHeader                  `json:"genesisBlockHeader"`
	Blocks             []BlockData                  `json:"blocks"`
	PostState          map[HexAddr]*TestAccountInfo `json:"postState"`
	Pre                map[HexAddr]*TestAccountInfo `json:"pre"`
	LastBlockHash      HexB256                      `json:"lastblockhash"`
	Network            string                       `json:"network"`
	SealEngine         string                       `json:"sealEngine"`
}

// BlockHeader holds all block header fields.
type BlockHeader struct {
	Bloom                 HexBytes `json:"bloom"`
	Coinbase              HexAddr  `json:"coinbase"`
	Difficulty            HexU256  `json:"difficulty"`
	ExtraData             HexBytes `json:"extraData"`
	GasLimit              HexU256  `json:"gasLimit"`
	GasUsed               HexU256  `json:"gasUsed"`
	Hash                  HexB256  `json:"hash"`
	MixHash               HexB256  `json:"mixHash"`
	Nonce                 HexBytes `json:"nonce"`
	Number                HexU256  `json:"number"`
	ParentHash            HexB256  `json:"parentHash"`
	ReceiptTrie           HexB256  `json:"receiptTrie"`
	StateRoot             HexB256  `json:"stateRoot"`
	Timestamp             HexU256  `json:"timestamp"`
	TransactionsTrie      HexB256  `json:"transactionsTrie"`
	UncleHash             HexB256  `json:"uncleHash"`
	BaseFeePerGas         *HexU256 `json:"baseFeePerGas"`
	WithdrawalsRoot       *HexB256 `json:"withdrawalsRoot"`
	BlobGasUsed           *HexU256 `json:"blobGasUsed"`
	ExcessBlobGas         *HexU256 `json:"excessBlobGas"`
	ParentBeaconBlockRoot *HexB256 `json:"parentBeaconBlockRoot"`
	RequestsHash          *HexB256 `json:"requestsHash"`
	TargetBlobsPerBlock   *HexU256 `json:"targetBlobsPerBlock"`
	SlotNumber            *HexU256 `json:"slotNumber"`
}

// BlockData is a single block in a blockchain test.
type BlockData struct {
	BlockHeader     *BlockHeader    `json:"blockHeader"`
	ExpectException *string         `json:"expectException"`
	Transactions    []BlockTx       `json:"transactions"`
	Withdrawals     []Withdrawal    `json:"withdrawals"`
	UncleHeaders    json.RawMessage `json:"uncleHeaders"`
}

// BlockTx is a transaction within a blockchain test block.
type BlockTx struct {
	Type                 *HexU256        `json:"type"`
	Sender               *HexAddr        `json:"sender"`
	Data                 HexBytes        `json:"data"`
	GasLimit             HexU256         `json:"gasLimit"`
	GasPrice             *HexU256        `json:"gasPrice"`
	Nonce                HexU256         `json:"nonce"`
	Value                HexU256         `json:"value"`
	To                   *string         `json:"to"`
	ChainID              *HexU256        `json:"chainId"`
	AccessList           json.RawMessage `json:"accessList"`
	MaxFeePerGas         *HexU256        `json:"maxFeePerGas"`
	MaxPriorityFeePerGas *HexU256        `json:"maxPriorityFeePerGas"`
	MaxFeePerBlobGas     *HexU256        `json:"maxFeePerBlobGas"`
	BlobVersionedHashes  []HexB256       `json:"blobVersionedHashes"`
	R                    HexU256         `json:"r"`
	S                    HexU256         `json:"s"`
	V                    HexU256         `json:"v"`
}

// Withdrawal represents a validator withdrawal (post-Shanghai).
type Withdrawal struct {
	Index          HexU256 `json:"index"`
	ValidatorIndex HexU256 `json:"validatorIndex"`
	Address        HexAddr `json:"address"`
	Amount         HexU256 `json:"amount"`
}

// BlockchainForkToForkID maps blockchain test fork names to ForkID values.
func BlockchainForkToForkID(network string) (gevmspec.ForkID, bool) {
	switch network {
	case "Frontier":
		return gevmspec.Frontier, true
	case "FrontierToHomesteadAt5":
		return gevmspec.Homestead, true
	case "Homestead":
		return gevmspec.Homestead, true
	case "HomesteadToDaoAt5", "HomesteadToEIP150At5":
		return gevmspec.Tangerine, true
	case "EIP150":
		return gevmspec.Tangerine, true
	case "EIP158":
		return gevmspec.SpuriousDragon, true
	case "EIP158ToByzantiumAt5":
		return gevmspec.Byzantium, true
	case "Byzantium":
		return gevmspec.Byzantium, true
	case "ByzantiumToConstantinopleAt5":
		return gevmspec.Byzantium, true
	case "ByzantiumToConstantinopleFixAt5":
		return gevmspec.Petersburg, true
	case "Constantinople":
		return gevmspec.Constantinople, true
	case "ConstantinopleFix":
		return gevmspec.Petersburg, true
	case "Istanbul":
		return gevmspec.Istanbul, true
	case "Berlin":
		return gevmspec.Berlin, true
	case "BerlinToLondonAt5":
		return gevmspec.London, true
	case "London":
		return gevmspec.London, true
	case "Paris", "Merge":
		return gevmspec.Merge, true
	case "Shanghai":
		return gevmspec.Shanghai, true
	case "Cancun":
		return gevmspec.Cancun, true
	case "Prague":
		return gevmspec.Prague, true
	case "Osaka":
		return gevmspec.Osaka, true
	case "Amsterdam":
		return gevmspec.Amsterdam, true
	// Transition forks: skip
	case "ParisToShanghaiAtTime15k",
		"ShanghaiToCancunAtTime15k",
		"CancunToPragueAtTime15k",
		"PragueToOsakaAtTime15k",
		"BPO1ToBPO2AtTime15k",
		"BPO2ToAmsterdamAtTime15k",
		"Merge+3540+3670",
		"Merge+3860",
		"Merge+3855",
		"MergeEOF",
		"MergeMeterInitCode",
		"MergePush0":
		return 0, false
	default:
		return 0, false
	}
}

// BlockHeaderToBlockEnv creates a BlockEnv from a blockchain test block header.
func BlockHeaderToBlockEnv(hdr *BlockHeader, forkID gevmspec.ForkID) types.Uint256 {
	// This just returns the excess blob gas for the caller to compute blob gas price.
	// The actual BlockEnv construction happens in the runner.
	if hdr.ExcessBlobGas != nil {
		return hdr.ExcessBlobGas.V
	}
	return types.U256Zero
}
