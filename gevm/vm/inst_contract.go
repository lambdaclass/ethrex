// Implements CREATE, CREATE2, CALL, CALLCODE, DELEGATECALL, STATICCALL.
package vm

import (
	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
)

// Maximum initcode size: 2 * MAX_CODE_SIZE (24576) = 49152
const maxInitcodeSize = 2 * 24576

// createInner is the shared logic for CREATE and CREATE2.
func createInner(interp *Interpreter, host Host, isCreate2 bool) {
	if interp.RuntimeFlag.IsStatic {
		interp.Halt(InstructionResultStateChangeDuringStaticCall)
		return
	}

	// Pop [value, code_offset, len]
	value, codeOffset, length, ok := interp.Stack.Pop3()
	if !ok {
		interp.HaltUnderflow()
		return
	}

	codeLen, ok := interp.asUsizeOrFail(length)
	if !ok {
		return
	}

	var initCode types.Bytes
	if codeLen != 0 {
		// EIP-3860: limit and meter initcode
		if interp.RuntimeFlag.ForkID.IsEnabledIn(spec.Shanghai) {
			if codeLen > maxInitcodeSize {
				interp.Halt(InstructionResultCreateInitCodeSizeLimit)
				return
			}
			initcodeCost := interp.GasParams.InitcodeCost(uint64(codeLen))
			if !interp.Gas.RecordCost(initcodeCost) {
				interp.HaltOOG()
				return
			}
		}

		offset, ok := interp.asUsizeOrFail(codeOffset)
		if !ok {
			return
		}
		if !interp.ResizeMemory(offset, codeLen) {
			return
		}
		// Pass memory slice directly — Bytecode.Reset() copies it before any
		// buffer growth can invalidate the reference.
		initCode = interp.Memory.Slice(offset, codeLen)
	}

	// For CREATE2, pop the salt
	var scheme CreateScheme
	if isCreate2 {
		salt, ok := interp.Stack.Pop()
		if !ok {
			interp.HaltUnderflow()
			return
		}
		scheme = NewCreateSchemeCreate2(salt)
	} else {
		scheme = NewCreateSchemeCreate()
	}

	// Gas cost: create_cost() or create2_cost(len)
	var gasCost uint64
	if isCreate2 {
		gasCost = interp.GasParams.Create2Cost(uint64(codeLen))
	} else {
		gasCost = interp.GasParams.CreateCost()
	}
	if !interp.Gas.RecordCost(gasCost) {
		interp.HaltOOG()
		return
	}

	// EIP-150: 63/64 gas reduction for sub-call
	var gasLimit uint64
	if interp.RuntimeFlag.ForkID.IsEnabledIn(spec.Tangerine) {
		gasLimit = interp.GasParams.CallStipendReduction(interp.Gas.Remaining())
	} else {
		gasLimit = interp.Gas.Remaining()
	}
	if !interp.Gas.RecordCost(gasLimit) {
		interp.HaltOOG()
		return
	}

	interp.SetCreateAction(CreateInputs{
		Caller:   interp.Input.TargetAddress,
		Scheme:   scheme,
		Value:    value,
		InitCode: initCode,
		GasLimit: gasLimit,
	})
}

// --- Helper functions ---

// getMemoryInputAndOutRanges pops 4 stack values and resizes memory for
// the input and output ranges used by CALL-family instructions.
// Returns (inputRange, outputRange, ok).
func getMemoryInputAndOutRanges(interp *Interpreter) (MemoryRange, MemoryRange, bool) {
	inOffsetVal, inLenVal, ok := interp.Stack.Pop2()
	if !ok {
		interp.HaltUnderflow()
		return MemoryRange{}, MemoryRange{}, false
	}
	outOffsetVal, outLenVal, ok := interp.Stack.Pop2()
	if !ok {
		interp.HaltUnderflow()
		return MemoryRange{}, MemoryRange{}, false
	}

	inputRange, ok := resizeMemoryRange(interp, inOffsetVal, inLenVal)
	if !ok {
		return MemoryRange{}, MemoryRange{}, false
	}
	outputRange, ok := resizeMemoryRange(interp, outOffsetVal, outLenVal)
	if !ok {
		return MemoryRange{}, MemoryRange{}, false
	}

	return inputRange, outputRange, true
}

// resizeMemoryRange converts Uint256 offset/len to a MemoryRange and resizes
// memory if needed. Returns (range, ok). If len is 0, returns a zero range
// without resizing.
func resizeMemoryRange(interp *Interpreter, offsetVal, lenVal types.Uint256) (MemoryRange, bool) {
	length, ok := interp.asUsizeOrFail(lenVal)
	if !ok {
		return MemoryRange{}, false
	}
	if length == 0 {
		return MemoryRange{Offset: 0, Length: 0}, true
	}
	offset, ok := interp.asUsizeOrFail(offsetVal)
	if !ok {
		return MemoryRange{}, false
	}
	if !interp.ResizeMemory(offset, length) {
		return MemoryRange{}, false
	}
	return MemoryRange{Offset: offset, Length: length}, true
}

// loadAccountAndCalcGas loads the target account and calculates the
// gas cost and gas limit for a CALL-family instruction.
// Returns (gasLimit, ok).
func loadAccountAndCalcGas(
	interp *Interpreter,
	host Host,
	to types.Address,
	transfersValue bool,
	createEmptyAccount bool,
	stackGasLimit uint64,
) (uint64, bool) {
	forkID := interp.RuntimeFlag.ForkID

	// 1. Transfer value cost
	if transfersValue {
		cost := interp.GasParams.TransferValueCost()
		if !interp.Gas.RecordCost(cost) {
			interp.HaltOOG()
			return 0, false
		}
	}

	// 2. Load account and calculate access gas
	accountGas := loadAccountDelegated(interp, host, to, transfersValue, createEmptyAccount)
	if !interp.Gas.RecordCost(accountGas) {
		interp.HaltOOG()
		return 0, false
	}

	// 3. Gas limit calculation with EIP-150 63/64 rule
	var gasLimit uint64
	if forkID.IsEnabledIn(spec.Tangerine) {
		// EIP-150: child gets at most 63/64 of parent's remaining gas
		reduced := interp.GasParams.CallStipendReduction(interp.Gas.Remaining())
		if reduced < stackGasLimit {
			gasLimit = reduced
		} else {
			gasLimit = stackGasLimit
		}
	} else {
		gasLimit = stackGasLimit
	}

	// Deduct gas limit from parent
	if !interp.Gas.RecordCost(gasLimit) {
		interp.HaltOOG()
		return 0, false
	}

	// 4. Add call stipend if value transferred
	if transfersValue {
		gasLimit += interp.GasParams.CallStipend()
	}

	return gasLimit, true
}

// loadAccountDelegated loads the account and returns the gas cost for
// accessing it. Handles cold/warm access (EIP-2929), new account
// creation costs, and EIP-7702 delegation resolution.
func loadAccountDelegated(
	interp *Interpreter,
	host Host,
	addr types.Address,
	transfersValue bool,
	createEmptyAccount bool,
) uint64 {
	forkID := interp.RuntimeFlag.ForkID
	isBerlin := forkID.IsEnabledIn(spec.Berlin)
	isSpuriousDragon := forkID.IsEnabledIn(spec.SpuriousDragon)

	var cost uint64

	// Load the account
	acl := host.LoadAccountCode(addr)

	// Cold account access cost (EIP-2929)
	if isBerlin && acl.IsCold {
		cost += interp.GasParams.ColdAccountAdditionalCost()
	}

	// EIP-7702: if the code is a delegation designator, charge warm read (100)
	// for the indirection and cold access (2600) if the delegate target is cold.
	if isEIP7702Code(acl.Code) {
		var delegateAddr types.Address
		copy(delegateAddr[:], acl.Code[3:23])
		delegateACL := host.LoadAccountCode(delegateAddr)
		cost += interp.GasParams.WarmStorageReadCost()
		if delegateACL.IsCold {
			cost += interp.GasParams.ColdAccountAdditionalCost()
		}
	}

	// New account cost: if account is empty and we're creating an empty account
	// (i.e., transferring value to a new account)
	if acl.IsEmpty && createEmptyAccount {
		cost += interp.GasParams.NewAccountCost(isSpuriousDragon, transfersValue)
	}

	return cost
}

// isEIP7702Code returns true if code is an EIP-7702 delegation designator.
func isEIP7702Code(code []byte) bool {
	return len(code) == 23 && code[0] == 0xef && code[1] == 0x01 && code[2] == 0x00
}

// opCreate — Custom flush handler (needs Host). Wraps createInner.
func opCreate(interp *Interpreter, host Host) {
	createInner(interp, host, false)
}

// opCreate2 — Custom flush handler (needs Host). Fork gate (Petersburg) checked by generator.
func opCreate2(interp *Interpreter, host Host) {
	createInner(interp, host, true)
}

// opCall — Custom flush handler (needs Host).
func opCall(interp *Interpreter, host Host) {
	stackGasLimit, toVal, value, ok := interp.Stack.Pop3()
	if !ok {
		interp.HaltUnderflow()
		return
	}
	to := toVal.ToAddress()
	transfersValue := !value.IsZero()
	if interp.RuntimeFlag.IsStatic && transfersValue {
		interp.Halt(InstructionResultCallNotAllowedInsideStatic)
		return
	}
	inputRange, outputRange, ok := getMemoryInputAndOutRanges(interp)
	if !ok {
		return
	}
	gasLimitOnStack := stackGasLimit.AsUsizeSaturated()
	gasLimit, ok := loadAccountAndCalcGas(interp, host, to, transfersValue, true, gasLimitOnStack)
	if !ok {
		return
	}
	callInput := make([]byte, inputRange.Length)
	if inputRange.Length > 0 {
		copy(callInput, interp.Memory.Slice(inputRange.Offset, inputRange.Length))
	}
	interp.SetCallAction(CallInputs{
		Input:              callInput,
		ReturnMemoryOffset: outputRange,
		GasLimit:           gasLimit,
		BytecodeAddress:    to,
		TargetAddress:      to,
		Caller:             interp.Input.TargetAddress,
		Value:              NewCallValueTransfer(value),
		Scheme:             CallSchemeCall,
		IsStatic:           interp.RuntimeFlag.IsStatic,
	})
}

// opCallcode — Custom flush handler (needs Host).
func opCallcode(interp *Interpreter, host Host) {
	stackGasLimit, toVal, value, ok := interp.Stack.Pop3()
	if !ok {
		interp.HaltUnderflow()
		return
	}
	to := toVal.ToAddress()
	transfersValue := !value.IsZero()
	inputRange, outputRange, ok := getMemoryInputAndOutRanges(interp)
	if !ok {
		return
	}
	gasLimitOnStack := stackGasLimit.AsUsizeSaturated()
	gasLimit, ok := loadAccountAndCalcGas(interp, host, to, transfersValue, false, gasLimitOnStack)
	if !ok {
		return
	}
	callInput := make([]byte, inputRange.Length)
	if inputRange.Length > 0 {
		copy(callInput, interp.Memory.Slice(inputRange.Offset, inputRange.Length))
	}
	interp.SetCallAction(CallInputs{
		Input:              callInput,
		ReturnMemoryOffset: outputRange,
		GasLimit:           gasLimit,
		BytecodeAddress:    to,
		TargetAddress:      interp.Input.TargetAddress,
		Caller:             interp.Input.TargetAddress,
		Value:              NewCallValueTransfer(value),
		Scheme:             CallSchemeCallCode,
		IsStatic:           interp.RuntimeFlag.IsStatic,
	})
}

// opDelegatecall — Custom flush handler (needs Host). Fork gate (Homestead) checked by generator.
func opDelegatecall(interp *Interpreter, host Host) {
	stackGasLimit, toVal, ok := interp.Stack.Pop2()
	if !ok {
		interp.HaltUnderflow()
		return
	}
	to := toVal.ToAddress()
	inputRange, outputRange, ok := getMemoryInputAndOutRanges(interp)
	if !ok {
		return
	}
	gasLimitOnStack := stackGasLimit.AsUsizeSaturated()
	gasLimit, ok := loadAccountAndCalcGas(interp, host, to, false, false, gasLimitOnStack)
	if !ok {
		return
	}
	callInput := make([]byte, inputRange.Length)
	if inputRange.Length > 0 {
		copy(callInput, interp.Memory.Slice(inputRange.Offset, inputRange.Length))
	}
	interp.SetCallAction(CallInputs{
		Input:              callInput,
		ReturnMemoryOffset: outputRange,
		GasLimit:           gasLimit,
		BytecodeAddress:    to,
		TargetAddress:      interp.Input.TargetAddress,
		Caller:             interp.Input.CallerAddress,
		Value:              NewCallValueApparent(interp.Input.CallValue),
		Scheme:             CallSchemeDelegateCall,
		IsStatic:           interp.RuntimeFlag.IsStatic,
	})
}

// opStaticcall — Custom flush handler (needs Host). Fork gate (Byzantium) checked by generator.
func opStaticcall(interp *Interpreter, host Host) {
	stackGasLimit, toVal, ok := interp.Stack.Pop2()
	if !ok {
		interp.HaltUnderflow()
		return
	}
	to := toVal.ToAddress()
	inputRange, outputRange, ok := getMemoryInputAndOutRanges(interp)
	if !ok {
		return
	}
	gasLimitOnStack := stackGasLimit.AsUsizeSaturated()
	gasLimit, ok := loadAccountAndCalcGas(interp, host, to, false, false, gasLimitOnStack)
	if !ok {
		return
	}
	callInput := make([]byte, inputRange.Length)
	if inputRange.Length > 0 {
		copy(callInput, interp.Memory.Slice(inputRange.Offset, inputRange.Length))
	}
	interp.SetCallAction(CallInputs{
		Input:              callInput,
		ReturnMemoryOffset: outputRange,
		GasLimit:           gasLimit,
		BytecodeAddress:    to,
		TargetAddress:      to,
		Caller:             interp.Input.TargetAddress,
		Value:              NewCallValueTransfer(types.U256Zero),
		Scheme:             CallSchemeStaticCall,
		IsStatic:           true,
	})
}
