package spec

// Gas cost constants.
const (
	GasZero     uint64 = 0
	GasBase     uint64 = 2
	GasVerylow  uint64 = 3
	GasLow      uint64 = 5
	GasMid      uint64 = 8
	GasHigh     uint64 = 10
	GasJumpdest uint64 = 1

	GasDataLoadN      uint64 = 3
	GasConditionJump  uint64 = 4
	GasRetf           uint64 = 3
	GasDataLoad       uint64 = 4

	GasSelfdestructRefund int64  = 24000
	GasCreate             uint64 = 32000
	GasCallvalue          uint64 = 9000
	GasNewaccount         uint64 = 25000
	GasExp                uint64 = 10
	GasMemory             uint64 = 3
	GasLog                uint64 = 375
	GasLogdata            uint64 = 8
	GasLogtopic           uint64 = 375
	GasKeccak256          uint64 = 30
	GasKeccak256Word      uint64 = 6
	GasCopy               uint64 = 3
	GasBlockhash          uint64 = 20
	GasCodedeposit        uint64 = 200

	// EIP-1884
	GasIstanbulSloadGas uint64 = 800
	GasSstoreSet        uint64 = 20000
	GasSstoreReset      uint64 = 5000
	GasRefundSstoreClears int64 = 15000

	// Calldata costs
	GasStandardTokenCost              uint64 = 4
	GasNonZeroByteDataCost            uint64 = 68
	GasNonZeroByteMultiplier          uint64 = 68 / 4 // 17
	GasNonZeroByteDataCostIstanbul    uint64 = 16
	GasNonZeroByteMultiplierIstanbul  uint64 = 16 / 4 // 4
	GasTotalCostFloorPerToken         uint64 = 10

	GasEofCreate uint64 = 32000

	// Berlin EIP-2929/EIP-2930
	GasAccessListAddress          uint64 = 2400
	GasAccessListStorageKey       uint64 = 1900
	GasColdSloadCost              uint64 = 2100
	GasColdAccountAccessCost      uint64 = 2600
	GasColdAccountAccessAdditional uint64 = 2600 - 100 // 2500
	GasWarmStorageReadCost        uint64 = 100
	GasWarmSstoreReset            uint64 = 5000 - 2100 // 2900

	// EIP-3860
	GasInitcodeWordCost uint64 = 2

	// Call stipend
	GasCallStipend uint64 = 2300
)

// InitialAndFloorGas holds the initial gas and floor gas for a transaction.
type InitialAndFloorGas struct {
	InitialGas uint64
	FloorGas   uint64
}

// NumWords returns the number of 32-byte words needed for len bytes (rounds up).
func NumWords(length uint64) uint64 {
	return (length + 31) / 32
}

// GetTokensInCalldata computes the total number of tokens in calldata.
func GetTokensInCalldata(input []byte, nonZeroMultiplier uint64) uint64 {
	var zeroCount uint64
	for _, b := range input {
		if b == 0 {
			zeroCount++
		}
	}
	nonZeroCount := uint64(len(input)) - zeroCount
	return zeroCount + nonZeroCount*nonZeroMultiplier
}
