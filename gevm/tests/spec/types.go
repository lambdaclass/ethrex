// JSON deserialization types for Ethereum GeneralStateTest fixtures.
package spec

import (
	"encoding/json"

	"github.com/Giulio2002/gevm/types"
	gevmspec "github.com/Giulio2002/gevm/spec"
)

// TestSuite is the top-level test file: a map of test name → TestUnit.
type TestSuite map[string]*TestUnit

// TestConfig holds chain/blob configuration (new fixture format).
type TestConfig struct {
	ChainId *HexU256 `json:"chainid"`
}

// TestUnit is a single test unit containing env, pre-state, transaction, and expected results.
type TestUnit struct {
	Info        json.RawMessage              `json:"_info"`
	Config      *TestConfig                  `json:"config"`
	Env         TestEnv                      `json:"env"`
	Pre         map[HexAddr]*TestAccountInfo `json:"pre"`
	Post        map[string][]TestCase        `json:"post"`
	Transaction TransactionParts             `json:"transaction"`
	Out         *HexBytes                    `json:"out"`
}

// ChainId returns the chain ID from config or env (new format uses config.chainid).
func (u *TestUnit) ChainId() types.Uint256 {
	if u.Config != nil && u.Config.ChainId != nil {
		return u.Config.ChainId.V
	}
	if u.Env.CurrentChainID != nil {
		return u.Env.CurrentChainID.V
	}
	return types.U256From(1) // default mainnet
}

// TestEnv holds block environment variables from the test JSON.
type TestEnv struct {
	CurrentChainID         *HexU256 `json:"currentChainID"`
	CurrentCoinbase        HexAddr  `json:"currentCoinbase"`
	CurrentDifficulty      HexU256  `json:"currentDifficulty"`
	CurrentGasLimit        HexU256  `json:"currentGasLimit"`
	CurrentNumber          HexU256  `json:"currentNumber"`
	CurrentTimestamp       HexU256  `json:"currentTimestamp"`
	CurrentBaseFee         *HexU256 `json:"currentBaseFee"`
	PreviousHash           *HexB256 `json:"previousHash"`
	CurrentRandom          *HexB256 `json:"currentRandom"`
	CurrentBeaconRoot      *HexB256 `json:"currentBeaconRoot"`
	CurrentWithdrawalsRoot *HexB256 `json:"currentWithdrawalsRoot"`
	CurrentExcessBlobGas   *HexU256 `json:"currentExcessBlobGas"`
	SlotNumber             *HexU256 `json:"slotNumber"`
}

// TestAccountInfo holds pre-state account information.
type TestAccountInfo struct {
	Balance HexU256             `json:"balance"`
	Code    HexBytes            `json:"code"`
	Nonce   HexU64              `json:"nonce"`
	Storage map[HexB256]HexU256 `json:"storage"`
}

// TestCase is a single expected test result for a specific fork.
type TestCase struct {
	ExpectException *string                      `json:"expectException"`
	Indexes         TxPartIndices                `json:"indexes"`
	Hash            HexB256                      `json:"hash"`
	PostState       map[HexAddr]*TestAccountInfo `json:"postState"`
	State           map[HexAddr]*TestAccountInfo `json:"state"` // new format alias
	Logs            HexB256                      `json:"logs"`
	TxBytes         *HexBytes                    `json:"txbytes"`
}

// TransactionParts holds the transaction template with multiple variants.
type TransactionParts struct {
	TxType               *HexU64           `json:"type"`
	Data                 []HexBytes        `json:"data"`
	GasLimit             []HexU256         `json:"gasLimit"`
	GasPrice             *HexU256          `json:"gasPrice"`
	Nonce                HexU256           `json:"nonce"`
	SecretKey            HexB256           `json:"secretKey"`
	Sender               *HexAddr          `json:"sender"`
	To                   *string           `json:"to"` // empty string or hex address; nil for CREATE
	Value                []HexU256         `json:"value"`
	MaxFeePerGas         *HexU256          `json:"maxFeePerGas"`
	MaxPriorityFeePerGas *HexU256          `json:"maxPriorityFeePerGas"`
	AccessLists          []json.RawMessage `json:"accessLists"`
	AuthorizationList    json.RawMessage   `json:"authorizationList"`
	BlobVersionedHashes  []HexB256         `json:"blobVersionedHashes"`
	MaxFeePerBlobGas     *HexU256          `json:"maxFeePerBlobGas"`
}

// TestAuthorization is an EIP-7702 authorization entry in test fixtures.
type TestAuthorization struct {
	ChainId HexU256 `json:"chainId"`
	Address HexAddr `json:"address"`
	Nonce   HexU256 `json:"nonce"`
	V       HexU256 `json:"v"`
	R       HexB256 `json:"r"`
	S       HexB256 `json:"s"`
	Signer  *HexAddr `json:"signer"`
	YParity *HexU256 `json:"yParity"`
}

// TxPartIndices selects which variant of data/gas/value to use.
type TxPartIndices struct {
	Data  int `json:"data"`
	Gas   int `json:"gas"`
	Value int `json:"value"`
}

// SpecNameToForkID maps Ethereum test spec names to GEVM ForkID values.
func SpecNameToForkID(name string) (gevmspec.ForkID, bool) {
	switch name {
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
	case "Byzantium", "EIP158ToByzantiumAt5":
		return gevmspec.Byzantium, true
	case "ConstantinopleFix", "ByzantiumToConstantinopleFixAt5":
		return gevmspec.Petersburg, true
	case "Istanbul":
		return gevmspec.Istanbul, true
	case "Berlin":
		return gevmspec.Berlin, true
	case "London", "BerlinToLondonAt5":
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
	// Constantinople is skipped (uses Petersburg)
	case "Constantinople", "ByzantiumToConstantinopleAt5":
		return 0, false
	default:
		return 0, false
	}
}

// ParseTo parses the "to" field: empty string or missing means CREATE.
func (t *TransactionParts) ParseTo() (types.Address, bool) {
	if t.To == nil || *t.To == "" {
		return types.AddressZero, false // CREATE
	}
	addr, err := types.HexToAddress(*t.To)
	if err != nil {
		return types.AddressZero, false
	}
	return addr, true
}
