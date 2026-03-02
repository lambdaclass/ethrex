// Defines frame input/output types for CALL and CREATE instructions.
package vm

import (
	"github.com/Giulio2002/gevm/types"
)

// CallScheme represents the type of call instruction.
type CallScheme int

const (
	CallSchemeCall         CallScheme = iota // CALL
	CallSchemeCallCode                       // CALLCODE
	CallSchemeDelegateCall                   // DELEGATECALL
	CallSchemeStaticCall                     // STATICCALL
)

// CallValueKind distinguishes Transfer from Apparent call values.
type CallValueKind int

const (
	CallValueTransfer CallValueKind = iota // Actual value transfer
	CallValueApparent                      // Apparent value (DELEGATECALL)
)

// CallValue represents the value associated with a call.
type CallValue struct {
	Kind  CallValueKind
	Value types.Uint256
}

// NewCallValueTransfer creates a Transfer call value.
func NewCallValueTransfer(value types.Uint256) CallValue {
	return CallValue{Kind: CallValueTransfer, Value: value}
}

// NewCallValueApparent creates an Apparent call value (DELEGATECALL).
func NewCallValueApparent(value types.Uint256) CallValue {
	return CallValue{Kind: CallValueApparent, Value: value}
}

// IsTransfer returns true if this is a real value transfer.
func (cv CallValue) IsTransfer() bool { return cv.Kind == CallValueTransfer }

// IsApparent returns true if this is an apparent value (not transferred).
func (cv CallValue) IsApparent() bool { return cv.Kind == CallValueApparent }

// TransferValue returns the transfer amount, or nil if apparent.
func (cv CallValue) TransferValue() *types.Uint256 {
	if cv.Kind == CallValueTransfer {
		v := cv.Value
		return &v
	}
	return nil
}

// TransfersValue returns true if there is a non-zero value transfer.
func (cv CallValue) TransfersValue() bool {
	return cv.Kind == CallValueTransfer && !cv.Value.IsZero()
}

// CreateSchemeKind distinguishes Create from Create2.
type CreateSchemeKind int

const (
	CreateSchemeCreate  CreateSchemeKind = iota // CREATE
	CreateSchemeCreate2                         // CREATE2
)

// CreateScheme represents the creation scheme with optional salt.
type CreateScheme struct {
	Kind CreateSchemeKind
	Salt types.Uint256 // Only meaningful for Create2
}

// NewCreateSchemeCreate creates a CREATE scheme.
func NewCreateSchemeCreate() CreateScheme {
	return CreateScheme{Kind: CreateSchemeCreate}
}

// NewCreateSchemeCreate2 creates a CREATE2 scheme with salt.
func NewCreateSchemeCreate2(salt types.Uint256) CreateScheme {
	return CreateScheme{Kind: CreateSchemeCreate2, Salt: salt}
}

// CallInputs holds all inputs for a CALL-family instruction.
type CallInputs struct {
	// The call data (input to callee).
	Input types.Bytes
	// Return memory offset range in the caller's memory.
	ReturnMemoryOffset MemoryRange
	// Gas limit of the call.
	GasLimit uint64
	// Address whose bytecode will be executed.
	BytecodeAddress types.Address
	// Target address (where storage is modified).
	TargetAddress types.Address
	// Caller address.
	Caller types.Address
	// Call value.
	Value CallValue
	// Call scheme (CALL, CALLCODE, DELEGATECALL, STATICCALL).
	Scheme CallScheme
	// Whether this is a static (read-only) call.
	IsStatic bool
}

// MemoryRange represents a range [Offset, Offset+Length) in memory.
type MemoryRange struct {
	Offset int
	Length int
}

// CreateInputs holds all inputs for a CREATE/CREATE2 instruction.
type CreateInputs struct {
	// Caller address.
	Caller types.Address
	// Create scheme (Create or Create2 with salt).
	Scheme CreateScheme
	// Value to transfer to the new contract.
	Value types.Uint256
	// Init code of the contract.
	InitCode types.Bytes
	// Gas limit of the create call.
	GasLimit uint64
}

// FrameInputKind distinguishes Call from Create frame inputs.
type FrameInputKind int

const (
	FrameInputCall   FrameInputKind = iota // Call frame
	FrameInputCreate                       // Create frame
)

// FrameInput is a tagged union holding either CallInputs or CreateInputs.
type FrameInput struct {
	Kind   FrameInputKind
	Call   CallInputs   // Valid when Kind == FrameInputCall
	Create CreateInputs // Valid when Kind == FrameInputCreate
}

// NewFrameInputCall creates a FrameInput for a call.
func NewFrameInputCall(inputs CallInputs) FrameInput {
	return FrameInput{Kind: FrameInputCall, Call: inputs}
}

// NewFrameInputCreate creates a FrameInput for a create.
func NewFrameInputCreate(inputs CreateInputs) FrameInput {
	return FrameInput{Kind: FrameInputCreate, Create: inputs}
}

// CallOutcome holds the result of a sub-call.
type CallOutcome struct {
	Result       InterpreterResult
	MemoryOffset MemoryRange
}

// NewCallOutcome creates a CallOutcome.
func NewCallOutcome(result InterpreterResult, memoryOffset MemoryRange) CallOutcome {
	return CallOutcome{
		Result:       result,
		MemoryOffset: memoryOffset,
	}
}

// CreateOutcome holds the result of a sub-create.
type CreateOutcome struct {
	Result  InterpreterResult
	Address *types.Address // nil if create failed
}

// NewCreateOutcome creates a CreateOutcome.
func NewCreateOutcome(result InterpreterResult, address *types.Address) CreateOutcome {
	return CreateOutcome{
		Result:  result,
		Address: address,
	}
}

// FrameResultKind distinguishes Call from Create results.
type FrameResultKind int

const (
	FrameResultCall FrameResultKind = iota
	FrameResultCreate
)

// FrameResult is a tagged union holding either a CallOutcome or CreateOutcome.
type FrameResult struct {
	Kind   FrameResultKind
	Call   CallOutcome
	Create CreateOutcome
}

// NewFrameResultCall creates a FrameResult for a call outcome.
func NewFrameResultCall(outcome CallOutcome) FrameResult {
	return FrameResult{Kind: FrameResultCall, Call: outcome}
}

// NewFrameResultCreate creates a FrameResult for a create outcome.
func NewFrameResultCreate(outcome CreateOutcome) FrameResult {
	return FrameResult{Kind: FrameResultCreate, Create: outcome}
}
