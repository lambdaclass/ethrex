package spec

import (
	"math/bits"

	"github.com/Giulio2002/gevm/types"
)

// GasId is an index into the 256-entry gas parameter table.
type GasId uint8

// All 39 GasId constants (IDs 1-39).
const (
	GasIdExpByteGas                     GasId = 1
	GasIdExtcodecopyPerWord             GasId = 2
	GasIdCopyPerWord                    GasId = 3
	GasIdLogdata                        GasId = 4
	GasIdLogtopic                       GasId = 5
	GasIdMcopyPerWord                   GasId = 6
	GasIdKeccak256PerWord               GasId = 7
	GasIdMemoryLinearCost               GasId = 8
	GasIdMemoryQuadraticReduction       GasId = 9
	GasIdInitcodePerWord                GasId = 10
	GasIdCreate                         GasId = 11
	GasIdCallStipendReduction           GasId = 12
	GasIdTransferValueCost              GasId = 13
	GasIdColdAccountAdditionalCost      GasId = 14
	GasIdNewAccountCost                 GasId = 15
	GasIdWarmStorageReadCost            GasId = 16
	GasIdSstoreStatic                   GasId = 17
	GasIdSstoreSetWithoutLoadCost       GasId = 18
	GasIdSstoreResetWithoutColdLoadCost GasId = 19
	GasIdSstoreClearingSlotRefund       GasId = 20
	GasIdSelfdestructRefund             GasId = 21
	GasIdCallStipend                    GasId = 22
	GasIdColdStorageAdditionalCost      GasId = 23
	GasIdColdStorageCost                GasId = 24
	GasIdNewAccountCostForSelfdestruct  GasId = 25
	GasIdCodeDepositCost                GasId = 26
	GasIdTxEip7702PerEmptyAccountCost   GasId = 27
	GasIdTxTokenNonZeroByteMultiplier   GasId = 28
	GasIdTxTokenCost                    GasId = 29
	GasIdTxFloorCostPerToken            GasId = 30
	GasIdTxFloorCostBaseGas             GasId = 31
	GasIdTxAccessListAddressCost        GasId = 32
	GasIdTxAccessListStorageKeyCost     GasId = 33
	GasIdTxBaseStipend                  GasId = 34
	GasIdTxCreateCost                   GasId = 35
	GasIdTxInitcodeCost                 GasId = 36
	GasIdSstoreSetRefund                GasId = 37
	GasIdSstoreResetRefund              GasId = 38
	GasIdTxEip7702AuthRefund            GasId = 39
)

// GasParams holds a 256-entry gas cost table, fork-specific.
type GasParams struct {
	table [256]uint64
}

// Get returns the gas cost for the given GasId.
func (g *GasParams) Get(id GasId) uint64 {
	return g.table[id]
}

// gasParamsCache pre-computes GasParams for every ForkID at init time.
// Since GasParams is read-only during execution, all interpreters sharing the
// same ForkID can safely share a single pointer.
var gasParamsCache [20]*GasParams

func init() {
	for i := ForkID(0); i <= Osaka; i++ {
		gasParamsCache[i] = newGasParams(i)
	}
}

// NewGasParams returns the cached GasParams for the given spec.
func NewGasParams(spec ForkID) *GasParams {
	return gasParamsCache[spec]
}

// newGasParams creates a GasParams for the given spec.
func newGasParams(spec ForkID) *GasParams {
	var t [256]uint64

	// Frontier defaults
	t[GasIdExpByteGas] = 10
	t[GasIdLogdata] = GasLogdata
	t[GasIdLogtopic] = GasLogtopic
	t[GasIdCopyPerWord] = GasCopy
	t[GasIdExtcodecopyPerWord] = GasCopy
	t[GasIdMcopyPerWord] = GasCopy
	t[GasIdKeccak256PerWord] = GasKeccak256Word
	t[GasIdMemoryLinearCost] = GasMemory
	t[GasIdMemoryQuadraticReduction] = 512
	t[GasIdInitcodePerWord] = GasInitcodeWordCost
	t[GasIdCreate] = GasCreate
	t[GasIdCallStipendReduction] = 64
	t[GasIdTransferValueCost] = GasCallvalue
	t[GasIdColdAccountAdditionalCost] = 0
	t[GasIdNewAccountCost] = GasNewaccount
	t[GasIdWarmStorageReadCost] = 0

	// Frontier: fixed 5k SSTORE cost
	t[GasIdSstoreStatic] = GasSstoreReset
	t[GasIdSstoreSetWithoutLoadCost] = GasSstoreSet - GasSstoreReset
	t[GasIdSstoreResetWithoutColdLoadCost] = 0
	t[GasIdSstoreSetRefund] = t[GasIdSstoreSetWithoutLoadCost]
	t[GasIdSstoreResetRefund] = t[GasIdSstoreResetWithoutColdLoadCost]
	t[GasIdSstoreClearingSlotRefund] = 15000
	t[GasIdSelfdestructRefund] = 24000
	t[GasIdCallStipend] = GasCallStipend
	t[GasIdColdStorageAdditionalCost] = 0
	t[GasIdColdStorageCost] = 0
	t[GasIdNewAccountCostForSelfdestruct] = 0
	t[GasIdCodeDepositCost] = GasCodedeposit
	t[GasIdTxTokenNonZeroByteMultiplier] = GasNonZeroByteMultiplier
	t[GasIdTxTokenCost] = GasStandardTokenCost
	t[GasIdTxBaseStipend] = 21000

	// Homestead: transaction creation cost
	if spec.IsEnabledIn(Homestead) {
		t[GasIdTxCreateCost] = GasCreate
	}

	// Tangerine: new account cost for selfdestruct
	if spec.IsEnabledIn(Tangerine) {
		t[GasIdNewAccountCostForSelfdestruct] = GasNewaccount
	}

	// Spurious Dragon: EXP cost increase
	if spec.IsEnabledIn(SpuriousDragon) {
		t[GasIdExpByteGas] = 50
	}

	// Istanbul: SSTORE gas changes
	if spec.IsEnabledIn(Istanbul) {
		t[GasIdSstoreStatic] = GasIstanbulSloadGas
		t[GasIdSstoreSetWithoutLoadCost] = GasSstoreSet - GasIstanbulSloadGas
		t[GasIdSstoreResetWithoutColdLoadCost] = GasSstoreReset - GasIstanbulSloadGas
		t[GasIdSstoreSetRefund] = t[GasIdSstoreSetWithoutLoadCost]
		t[GasIdSstoreResetRefund] = t[GasIdSstoreResetWithoutColdLoadCost]
		t[GasIdTxTokenNonZeroByteMultiplier] = GasNonZeroByteMultiplierIstanbul
	}

	// Berlin: warm/cold state access (EIP-2929/2930)
	if spec.IsEnabledIn(Berlin) {
		t[GasIdSstoreStatic] = GasWarmStorageReadCost
		t[GasIdColdAccountAdditionalCost] = GasColdAccountAccessAdditional
		t[GasIdColdStorageAdditionalCost] = GasColdSloadCost - GasWarmStorageReadCost
		t[GasIdColdStorageCost] = GasColdSloadCost
		t[GasIdWarmStorageReadCost] = GasWarmStorageReadCost

		t[GasIdSstoreResetWithoutColdLoadCost] = GasWarmSstoreReset - GasWarmStorageReadCost
		t[GasIdSstoreSetWithoutLoadCost] = GasSstoreSet - GasWarmStorageReadCost
		t[GasIdSstoreSetRefund] = t[GasIdSstoreSetWithoutLoadCost]
		t[GasIdSstoreResetRefund] = t[GasIdSstoreResetWithoutColdLoadCost]

		t[GasIdTxAccessListAddressCost] = GasAccessListAddress
		t[GasIdTxAccessListStorageKeyCost] = GasAccessListStorageKey
	}

	// London: refund reduction (EIP-3529)
	if spec.IsEnabledIn(London) {
		t[GasIdSstoreClearingSlotRefund] = GasWarmSstoreReset + GasAccessListStorageKey
		t[GasIdSelfdestructRefund] = 0
	}

	// Shanghai: initcode metering (EIP-3860)
	if spec.IsEnabledIn(Shanghai) {
		t[GasIdTxInitcodeCost] = GasInitcodeWordCost
	}

	// Prague: EIP-7702 + EIP-7623
	if spec.IsEnabledIn(Prague) {
		t[GasIdTxEip7702PerEmptyAccountCost] = EIP7702PerEmptyAccountCost
		t[GasIdTxEip7702AuthRefund] = EIP7702PerEmptyAccountCost - EIP7702PerAuthBaseCost
		t[GasIdTxFloorCostPerToken] = GasTotalCostFloorPerToken
		t[GasIdTxFloorCostBaseGas] = 21000
	}

	return &GasParams{table: t}
}

// --- Gas calculation methods ---

// ExpCost calculates the EXP opcode gas cost for a given exponent.
func (g *GasParams) ExpCost(power types.Uint256) uint64 {
	if power.IsZero() {
		return 0
	}
	return satMul(g.Get(GasIdExpByteGas), log2floor(power)/8+1)
}

// SelfdestructRefund returns the SELFDESTRUCT refund.
func (g *GasParams) SelfdestructRefund() int64 {
	return int64(g.Get(GasIdSelfdestructRefund))
}

// SelfdestructColdCost returns SELFDESTRUCT cold cost (cold + warm).
func (g *GasParams) SelfdestructColdCost() uint64 {
	return g.ColdAccountAdditionalCost() + g.WarmStorageReadCost()
}

// SelfdestructCost calculates SELFDESTRUCT gas cost.
func (g *GasParams) SelfdestructCost(shouldChargeTopup bool, isCold bool) uint64 {
	var gas uint64
	if shouldChargeTopup {
		gas += g.NewAccountCostForSelfdestruct()
	}
	if isCold {
		gas += g.SelfdestructColdCost()
	}
	return gas
}

// ExtcodecopyGas calculates EXTCODECOPY gas cost.
func (g *GasParams) ExtcodecopyGas(length uint64) uint64 {
	return satMul(g.Get(GasIdExtcodecopyPerWord), NumWords(length))
}

// McopyGas calculates MCOPY gas cost.
func (g *GasParams) McopyGas(length uint64) uint64 {
	return satMul(g.Get(GasIdMcopyPerWord), NumWords(length))
}

// SstoreStaticGas returns the static SSTORE gas cost.
func (g *GasParams) SstoreStaticGas() uint64 {
	return g.Get(GasIdSstoreStatic)
}

// SstoreSetWithoutLoadCost returns the SSTORE set cost (additional above static).
func (g *GasParams) SstoreSetWithoutLoadCost() uint64 {
	return g.Get(GasIdSstoreSetWithoutLoadCost)
}

// SstoreResetWithoutColdLoadCost returns the SSTORE reset cost.
func (g *GasParams) SstoreResetWithoutColdLoadCost() uint64 {
	return g.Get(GasIdSstoreResetWithoutColdLoadCost)
}

// SstoreClearingSlotRefund returns the SSTORE clearing slot refund.
func (g *GasParams) SstoreClearingSlotRefund() uint64 {
	return g.Get(GasIdSstoreClearingSlotRefund)
}

// SstoreSetRefund returns the SSTORE set refund.
func (g *GasParams) SstoreSetRefund() uint64 {
	return g.Get(GasIdSstoreSetRefund)
}

// SstoreResetRefund returns the SSTORE reset refund.
func (g *GasParams) SstoreResetRefund() uint64 {
	return g.Get(GasIdSstoreResetRefund)
}

// SStoreResult holds the values needed for SSTORE gas/refund calculation.
type SStoreResult struct {
	OriginalValue types.Uint256
	PresentValue  types.Uint256
	NewValue      types.Uint256
}

func (s *SStoreResult) IsOriginalZero() bool      { return types.IsZeroPtr(&s.OriginalValue) }
func (s *SStoreResult) IsPresentZero() bool       { return types.IsZeroPtr(&s.PresentValue) }
func (s *SStoreResult) IsNewZero() bool           { return types.IsZeroPtr(&s.NewValue) }
func (s *SStoreResult) IsOriginalEqPresent() bool { return types.EqPtr(&s.OriginalValue, &s.PresentValue) }
func (s *SStoreResult) IsOriginalEqNew() bool     { return types.EqPtr(&s.OriginalValue, &s.NewValue) }
func (s *SStoreResult) IsNewEqPresent() bool      { return types.EqPtr(&s.NewValue, &s.PresentValue) }
func (s *SStoreResult) NewValuesChangesPresent() bool { return !types.EqPtr(&s.NewValue, &s.PresentValue) }

// SstoreDynamicGas calculates the dynamic SSTORE gas cost.
func (g *GasParams) SstoreDynamicGas(isIstanbul bool, vals *SStoreResult, isCold bool) uint64 {
	if !isIstanbul {
		if vals.IsPresentZero() && !vals.IsNewZero() {
			return g.SstoreSetWithoutLoadCost()
		}
		return g.SstoreResetWithoutColdLoadCost()
	}

	var gas uint64
	if isCold {
		gas += g.ColdStorageCost()
	}

	if vals.NewValuesChangesPresent() && vals.IsOriginalEqPresent() {
		if vals.IsOriginalZero() {
			gas += g.SstoreSetWithoutLoadCost()
		} else {
			gas += g.SstoreResetWithoutColdLoadCost()
		}
	}
	return gas
}

// SstoreRefund calculates the SSTORE refund.
func (g *GasParams) SstoreRefund(isIstanbul bool, vals *SStoreResult) int64 {
	clearRefund := int64(g.SstoreClearingSlotRefund())

	if !isIstanbul {
		if !vals.IsPresentZero() && vals.IsNewZero() {
			return clearRefund
		}
		return 0
	}

	if vals.IsNewEqPresent() {
		return 0
	}

	if vals.IsOriginalEqPresent() && vals.IsNewZero() {
		return clearRefund
	}

	var refund int64

	if !vals.IsOriginalZero() {
		if vals.IsPresentZero() {
			refund -= clearRefund
		} else if vals.IsNewZero() {
			refund += clearRefund
		}
	}

	if vals.IsOriginalEqNew() {
		if vals.IsOriginalZero() {
			refund += int64(g.SstoreSetRefund())
		} else {
			refund += int64(g.SstoreResetRefund())
		}
	}
	return refund
}

// LogCost calculates LOG opcode gas cost.
func (g *GasParams) LogCost(numTopics uint8, dataLen uint64) uint64 {
	return satMul(g.Get(GasIdLogdata), dataLen) + g.Get(GasIdLogtopic)*uint64(numTopics)
}

// Keccak256Cost calculates KECCAK256 gas cost for data of given length.
func (g *GasParams) Keccak256Cost(length uint64) uint64 {
	return satMul(g.Get(GasIdKeccak256PerWord), NumWords(length))
}

// MemoryCost calculates memory expansion gas cost for numWords words.
func (g *GasParams) MemoryCost(numWords uint64) uint64 {
	linear := satMul(g.Get(GasIdMemoryLinearCost), numWords)
	quadratic := satMul(numWords, numWords) / g.Get(GasIdMemoryQuadraticReduction)
	return satAdd(linear, quadratic)
}

// InitcodeCost calculates initcode metering gas cost.
func (g *GasParams) InitcodeCost(length uint64) uint64 {
	return satMul(g.Get(GasIdInitcodePerWord), NumWords(length))
}

// CreateCost returns the CREATE instruction gas cost.
func (g *GasParams) CreateCost() uint64 {
	return g.Get(GasIdCreate)
}

// Create2Cost returns the CREATE2 instruction gas cost.
func (g *GasParams) Create2Cost(length uint64) uint64 {
	return satAdd(g.Get(GasIdCreate), satMul(g.Get(GasIdKeccak256PerWord), NumWords(length)))
}

// CallStipend returns the call stipend gas.
func (g *GasParams) CallStipend() uint64 {
	return g.Get(GasIdCallStipend)
}

// CallStipendReduction returns gas_limit - gas_limit/64.
func (g *GasParams) CallStipendReduction(gasLimit uint64) uint64 {
	return gasLimit - gasLimit/g.Get(GasIdCallStipendReduction)
}

// TransferValueCost returns the CALLVALUE transfer cost.
func (g *GasParams) TransferValueCost() uint64 {
	return g.Get(GasIdTransferValueCost)
}

// ColdAccountAdditionalCost returns the cold account access additional cost.
func (g *GasParams) ColdAccountAdditionalCost() uint64 {
	return g.Get(GasIdColdAccountAdditionalCost)
}

// ColdStorageAdditionalCost returns the cold storage additional cost.
func (g *GasParams) ColdStorageAdditionalCost() uint64 {
	return g.Get(GasIdColdStorageAdditionalCost)
}

// ColdStorageCost returns the cold storage (SLOAD) cost.
func (g *GasParams) ColdStorageCost() uint64 {
	return g.Get(GasIdColdStorageCost)
}

// NewAccountCost returns the new account cost.
func (g *GasParams) NewAccountCost(isSpuriousDragon bool, transfersValue bool) uint64 {
	if !isSpuriousDragon || transfersValue {
		return g.Get(GasIdNewAccountCost)
	}
	return 0
}

// NewAccountCostForSelfdestruct returns the new account cost for SELFDESTRUCT.
func (g *GasParams) NewAccountCostForSelfdestruct() uint64 {
	return g.Get(GasIdNewAccountCostForSelfdestruct)
}

// WarmStorageReadCost returns the warm storage read cost.
func (g *GasParams) WarmStorageReadCost() uint64 {
	return g.Get(GasIdWarmStorageReadCost)
}

// CopyCost calculates the copy gas cost for length bytes.
func (g *GasParams) CopyCost(length uint64) uint64 {
	return satMul(g.Get(GasIdCopyPerWord), NumWords(length))
}

// CodeDepositCost calculates the code deposit gas cost.
func (g *GasParams) CodeDepositCost(length uint64) uint64 {
	return satMul(g.Get(GasIdCodeDepositCost), length)
}

// InitialTxGas calculates the initial gas deducted for a transaction.
func (g *GasParams) InitialTxGas(
	input []byte,
	isCreate bool,
	accessListAccounts uint64,
	accessListStorages uint64,
	authorizationListNum uint64,
) InitialAndFloorGas {
	var gas InitialAndFloorGas

	tokensInCalldata := GetTokensInCalldata(input, g.Get(GasIdTxTokenNonZeroByteMultiplier))

	gas.InitialGas += tokensInCalldata*g.Get(GasIdTxTokenCost) +
		accessListAccounts*g.Get(GasIdTxAccessListAddressCost) +
		accessListStorages*g.Get(GasIdTxAccessListStorageKeyCost) +
		g.Get(GasIdTxBaseStipend) +
		authorizationListNum*g.Get(GasIdTxEip7702PerEmptyAccountCost)

	if isCreate {
		gas.InitialGas += g.Get(GasIdTxCreateCost)
		gas.InitialGas += g.Get(GasIdTxInitcodeCost) * NumWords(uint64(len(input)))
	}

	gas.FloorGas = g.Get(GasIdTxFloorCostPerToken)*tokensInCalldata + g.Get(GasIdTxFloorCostBaseGas)

	return gas
}

// --- Internal helpers ---

// log2floor returns floor(log2(value)). Returns 0 if value is zero.
func log2floor(value types.Uint256) uint64 {
	var l uint64 = 256
	for i := 3; i >= 0; i-- {
		limb := value[i]
		if limb == 0 {
			l -= 64
		} else {
			l -= uint64(bits.LeadingZeros64(limb))
			if l == 0 {
				return l
			}
			return l - 1
		}
	}
	return l
}

// satMul returns a * b, saturating at max uint64.
func satMul(a, b uint64) uint64 {
	hi, lo := bits.Mul64(a, b)
	if hi != 0 {
		return ^uint64(0)
	}
	return lo
}

// satAdd returns a + b, saturating at max uint64.
func satAdd(a, b uint64) uint64 {
	sum, carry := bits.Add64(a, b, 0)
	if carry != 0 {
		return ^uint64(0)
	}
	return sum
}
