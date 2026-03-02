package vm

import "testing"

func TestInstructionResultExhaustiveness(t *testing.T) {
	// All values must classify as exactly one of ok, revert, or error.
	allResults := []InstructionResult{
		// OK
		InstructionResultStop,
		InstructionResultReturn,
		InstructionResultSelfDestruct,
		// Revert
		InstructionResultRevert,
		InstructionResultCallTooDeep,
		InstructionResultOutOfFunds,
		InstructionResultCreateInitCodeStartingEF00,
		InstructionResultInvalidEOFInitCode,
		InstructionResultInvalidExtDelegateCallTarget,
		// Error
		InstructionResultOutOfGas,
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
		// Transaction validation errors
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
		InstructionResultNonceMismatch,
	}

	for _, r := range allResults {
		ok := r.IsOk()
		rev := r.IsRevert()
		err := r.IsError()
		count := 0
		if ok {
			count++
		}
		if rev {
			count++
		}
		if err {
			count++
		}
		if count != 1 {
			t.Errorf("%s: classified as ok=%v revert=%v error=%v (expected exactly one)", r, ok, rev, err)
		}
	}
}

func TestInstructionResultOk(t *testing.T) {
	okResults := []InstructionResult{
		InstructionResultStop,
		InstructionResultReturn,
		InstructionResultSelfDestruct,
	}
	for _, r := range okResults {
		if !r.IsOk() {
			t.Errorf("%s should be ok", r)
		}
		if r.IsRevert() {
			t.Errorf("%s should not be revert", r)
		}
		if r.IsError() {
			t.Errorf("%s should not be error", r)
		}
	}
}

func TestInstructionResultRevert(t *testing.T) {
	revertResults := []InstructionResult{
		InstructionResultRevert,
		InstructionResultCallTooDeep,
		InstructionResultOutOfFunds,
	}
	for _, r := range revertResults {
		if r.IsOk() {
			t.Errorf("%s should not be ok", r)
		}
		if !r.IsRevert() {
			t.Errorf("%s should be revert", r)
		}
		if r.IsError() {
			t.Errorf("%s should not be error", r)
		}
	}
}

func TestInstructionResultError(t *testing.T) {
	errorResults := []InstructionResult{
		InstructionResultOutOfGas,
		InstructionResultMemoryOOG,
		InstructionResultMemoryLimitOOG,
		InstructionResultPrecompileOOG,
		InstructionResultInvalidOperandOOG,
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
		InstructionResultNonceMismatch,
	}
	for _, r := range errorResults {
		if r.IsOk() {
			t.Errorf("%s should not be ok", r)
		}
		if r.IsRevert() {
			t.Errorf("%s should not be revert", r)
		}
		if !r.IsError() {
			t.Errorf("%s should be error", r)
		}
	}
}

func TestInstructionResultString(t *testing.T) {
	if InstructionResultStop.String() != "Stop" {
		t.Errorf("Stop string: got %s", InstructionResultStop.String())
	}
	if InstructionResultOutOfGas.String() != "OutOfGas" {
		t.Errorf("OutOfGas string: got %s", InstructionResultOutOfGas.String())
	}
}
