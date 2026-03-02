package vm

// InstructionResult represents all possible outcomes when executing an EVM instruction.
type InstructionResult uint8

const (
	// Success results (0x01-0x03)
	InstructionResultStop        InstructionResult = 0x01
	InstructionResultReturn      InstructionResult = 0x02
	InstructionResultSelfDestruct InstructionResult = 0x03

	// Revert results (0x10-0x15)
	InstructionResultRevert                       InstructionResult = 0x10
	InstructionResultCallTooDeep                  InstructionResult = 0x11
	InstructionResultOutOfFunds                   InstructionResult = 0x12
	InstructionResultCreateInitCodeStartingEF00   InstructionResult = 0x13
	InstructionResultInvalidEOFInitCode           InstructionResult = 0x14
	InstructionResultInvalidExtDelegateCallTarget InstructionResult = 0x15

	// Error results (0x20-0x37)
	InstructionResultOutOfGas                    InstructionResult = 0x20
	InstructionResultMemoryOOG                   InstructionResult = 0x21
	InstructionResultMemoryLimitOOG              InstructionResult = 0x22
	InstructionResultPrecompileOOG               InstructionResult = 0x23
	InstructionResultInvalidOperandOOG           InstructionResult = 0x24
	InstructionResultReentrancySentryOOG         InstructionResult = 0x25
	InstructionResultOpcodeNotFound              InstructionResult = 0x26
	InstructionResultCallNotAllowedInsideStatic  InstructionResult = 0x27
	InstructionResultStateChangeDuringStaticCall InstructionResult = 0x28
	InstructionResultInvalidFEOpcode             InstructionResult = 0x29
	InstructionResultInvalidJump                 InstructionResult = 0x2a
	InstructionResultNotActivated                InstructionResult = 0x2b
	InstructionResultStackUnderflow              InstructionResult = 0x2c
	InstructionResultStackOverflow               InstructionResult = 0x2d
	InstructionResultOutOfOffset                 InstructionResult = 0x2e
	InstructionResultCreateCollision             InstructionResult = 0x2f
	InstructionResultOverflowPayment             InstructionResult = 0x30
	InstructionResultPrecompileError             InstructionResult = 0x31
	InstructionResultNonceOverflow               InstructionResult = 0x32
	InstructionResultCreateContractSizeLimit     InstructionResult = 0x33
	InstructionResultCreateContractStartingWithEF InstructionResult = 0x34
	InstructionResultCreateInitCodeSizeLimit     InstructionResult = 0x35
	InstructionResultFatalExternalError          InstructionResult = 0x36
	InstructionResultInvalidImmediateEncoding    InstructionResult = 0x37

	// Transaction validation errors (0x40-0x4B)
	InstructionResultInvalidTxType          InstructionResult = 0x40
	InstructionResultGasPriceBelowBaseFee   InstructionResult = 0x41
	InstructionResultPriorityFeeTooHigh     InstructionResult = 0x42
	InstructionResultBlobGasPriceTooHigh    InstructionResult = 0x43
	InstructionResultEmptyBlobs             InstructionResult = 0x44
	InstructionResultTooManyBlobs           InstructionResult = 0x45
	InstructionResultInvalidBlobVersion     InstructionResult = 0x46
	InstructionResultCreateNotAllowed       InstructionResult = 0x47
	InstructionResultEmptyAuthorizationList InstructionResult = 0x48
	InstructionResultGasLimitTooHigh        InstructionResult = 0x49
	InstructionResultSenderNotEOA           InstructionResult = 0x4a
	InstructionResultNonceMismatch          InstructionResult = 0x4b
)

// IsOk returns true if the result is a success (Stop, Return, SelfDestruct).
func (r InstructionResult) IsOk() bool {
	switch r {
	case InstructionResultStop, InstructionResultReturn, InstructionResultSelfDestruct:
		return true
	}
	return false
}

// IsRevert returns true if the result is a revert.
func (r InstructionResult) IsRevert() bool {
	switch r {
	case InstructionResultRevert,
		InstructionResultCallTooDeep,
		InstructionResultOutOfFunds,
		InstructionResultInvalidEOFInitCode,
		InstructionResultCreateInitCodeStartingEF00,
		InstructionResultInvalidExtDelegateCallTarget:
		return true
	}
	return false
}

// IsError returns true if the result is an error.
func (r InstructionResult) IsError() bool {
	switch r {
	case InstructionResultOutOfGas,
		InstructionResultMemoryOOG,
		InstructionResultMemoryLimitOOG,
		InstructionResultPrecompileOOG,
		InstructionResultInvalidOperandOOG,
		InstructionResultReentrancySentryOOG,
		InstructionResultOpcodeNotFound,
		InstructionResultCallNotAllowedInsideStatic,
		InstructionResultStateChangeDuringStaticCall,
		InstructionResultInvalidFEOpcode,
		InstructionResultInvalidJump,
		InstructionResultNotActivated,
		InstructionResultStackUnderflow,
		InstructionResultStackOverflow,
		InstructionResultOutOfOffset,
		InstructionResultCreateCollision,
		InstructionResultOverflowPayment,
		InstructionResultPrecompileError,
		InstructionResultNonceOverflow,
		InstructionResultCreateContractSizeLimit,
		InstructionResultCreateContractStartingWithEF,
		InstructionResultCreateInitCodeSizeLimit,
		InstructionResultFatalExternalError,
		InstructionResultInvalidImmediateEncoding,
		InstructionResultInvalidTxType,
		InstructionResultGasPriceBelowBaseFee,
		InstructionResultPriorityFeeTooHigh,
		InstructionResultBlobGasPriceTooHigh,
		InstructionResultEmptyBlobs,
		InstructionResultTooManyBlobs,
		InstructionResultInvalidBlobVersion,
		InstructionResultCreateNotAllowed,
		InstructionResultEmptyAuthorizationList,
		InstructionResultGasLimitTooHigh,
		InstructionResultSenderNotEOA,
		InstructionResultNonceMismatch:
		return true
	}
	return false
}

// IsOkOrRevert returns true if the result is a success or revert (not an error).
func (r InstructionResult) IsOkOrRevert() bool {
	return r.IsOk() || r.IsRevert()
}

// Error implements the error interface for use in tracer callbacks.
func (r InstructionResult) Error() string { return r.String() }

// String returns the name of the instruction result.
func (r InstructionResult) String() string {
	switch r {
	case InstructionResultStop:
		return "Stop"
	case InstructionResultReturn:
		return "Return"
	case InstructionResultSelfDestruct:
		return "SelfDestruct"
	case InstructionResultRevert:
		return "Revert"
	case InstructionResultCallTooDeep:
		return "CallTooDeep"
	case InstructionResultOutOfFunds:
		return "OutOfFunds"
	case InstructionResultCreateInitCodeStartingEF00:
		return "CreateInitCodeStartingEF00"
	case InstructionResultInvalidEOFInitCode:
		return "InvalidEOFInitCode"
	case InstructionResultInvalidExtDelegateCallTarget:
		return "InvalidExtDelegateCallTarget"
	case InstructionResultOutOfGas:
		return "OutOfGas"
	case InstructionResultMemoryOOG:
		return "MemoryOOG"
	case InstructionResultMemoryLimitOOG:
		return "MemoryLimitOOG"
	case InstructionResultPrecompileOOG:
		return "PrecompileOOG"
	case InstructionResultInvalidOperandOOG:
		return "InvalidOperandOOG"
	case InstructionResultReentrancySentryOOG:
		return "ReentrancySentryOOG"
	case InstructionResultOpcodeNotFound:
		return "OpcodeNotFound"
	case InstructionResultCallNotAllowedInsideStatic:
		return "CallNotAllowedInsideStatic"
	case InstructionResultStateChangeDuringStaticCall:
		return "StateChangeDuringStaticCall"
	case InstructionResultInvalidFEOpcode:
		return "InvalidFEOpcode"
	case InstructionResultInvalidJump:
		return "InvalidJump"
	case InstructionResultNotActivated:
		return "NotActivated"
	case InstructionResultStackUnderflow:
		return "StackUnderflow"
	case InstructionResultStackOverflow:
		return "StackOverflow"
	case InstructionResultOutOfOffset:
		return "OutOfOffset"
	case InstructionResultCreateCollision:
		return "CreateCollision"
	case InstructionResultOverflowPayment:
		return "OverflowPayment"
	case InstructionResultPrecompileError:
		return "PrecompileError"
	case InstructionResultNonceOverflow:
		return "NonceOverflow"
	case InstructionResultCreateContractSizeLimit:
		return "CreateContractSizeLimit"
	case InstructionResultCreateContractStartingWithEF:
		return "CreateContractStartingWithEF"
	case InstructionResultCreateInitCodeSizeLimit:
		return "CreateInitCodeSizeLimit"
	case InstructionResultFatalExternalError:
		return "FatalExternalError"
	case InstructionResultInvalidImmediateEncoding:
		return "InvalidImmediateEncoding"
	case InstructionResultInvalidTxType:
		return "InvalidTxType"
	case InstructionResultGasPriceBelowBaseFee:
		return "GasPriceBelowBaseFee"
	case InstructionResultPriorityFeeTooHigh:
		return "PriorityFeeTooHigh"
	case InstructionResultBlobGasPriceTooHigh:
		return "BlobGasPriceTooHigh"
	case InstructionResultEmptyBlobs:
		return "EmptyBlobs"
	case InstructionResultTooManyBlobs:
		return "TooManyBlobs"
	case InstructionResultInvalidBlobVersion:
		return "InvalidBlobVersion"
	case InstructionResultCreateNotAllowed:
		return "CreateNotAllowed"
	case InstructionResultEmptyAuthorizationList:
		return "EmptyAuthorizationList"
	case InstructionResultGasLimitTooHigh:
		return "GasLimitTooHigh"
	case InstructionResultSenderNotEOA:
		return "SenderNotEOA"
	case InstructionResultNonceMismatch:
		return "NonceMismatch"
	default:
		return "Unknown"
	}
}
