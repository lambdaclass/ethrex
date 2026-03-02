// Object pooling for hot EVM execution objects.
// Uses sync.Pool to reduce GC pressure from frequent allocations
// during nested CALL/CREATE frame execution.
package vm

import (
	"sync"

	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
)

// returnBufPool pools byte slices for RETURN/REVERT output buffers.
var returnBufPool = sync.Pool{
	New: func() any {
		b := make([]byte, 0, 4096)
		return &b
	},
}

// ReturnDataArena accumulates return data buffers during a transaction
// and releases them all at the end. This avoids per-frame allocations
// after the first transaction warms the pool.
type ReturnDataArena struct {
	bufs []*[]byte // tracked pooled buffer pointers
}

// Alloc returns a []byte of the given size from the pool.
// The returned slice is valid until Reset is called.
func (a *ReturnDataArena) Alloc(size int) []byte {
	bp := returnBufPool.Get().(*[]byte)
	buf := *bp
	if cap(buf) >= size {
		buf = buf[:size]
	} else {
		buf = make([]byte, size)
	}
	*bp = buf
	a.bufs = append(a.bufs, bp)
	return buf
}

// Reset returns all accumulated buffers to the pool.
// After Reset, any slices returned by Alloc must not be used.
func (a *ReturnDataArena) Reset() {
	for i, bp := range a.bufs {
		*bp = (*bp)[:0]
		returnBufPool.Put(bp)
		a.bufs[i] = nil
	}
	a.bufs = a.bufs[:0]
}

// stackPool pools Stack objects (each ~32KB: 1024 × 32-byte Uint256 words).
var stackPool = sync.Pool{
	New: func() any { return NewStack() },
}

// AcquireStack returns a Stack from the pool, ready for use.
func AcquireStack() *Stack {
	s := stackPool.Get().(*Stack)
	s.Clear()
	return s
}

// ReleaseStack returns a Stack to the pool.
func ReleaseStack(s *Stack) {
	if s == nil {
		return
	}
	s.Clear()
	stackPool.Put(s)
}

// interpreterPool pools Interpreter objects.
var interpreterPool = sync.Pool{
	New: func() any {
		return &Interpreter{
			Stack: NewStack(),
		}
	},
}

// AcquireInterpreter returns a pooled Interpreter and initializes it with the given params.
func AcquireInterpreter(
	memory *Memory,
	bytecode *Bytecode,
	input Inputs,
	isStatic bool,
	forkID spec.ForkID,
	gasLimit uint64,
) *Interpreter {
	interp := interpreterPool.Get().(*Interpreter)
	interp.Clear(memory, bytecode, input, isStatic, forkID, gasLimit)
	return interp
}

// ReleaseInterpreter returns an Interpreter to the pool.
func ReleaseInterpreter(interp *Interpreter) {
	if interp == nil {
		return
	}
	// Only nil out pointer/slice fields for GC. Skip full struct zeroing of
	// ActionData (~300B) and Input (~132B) — Clear() overwrites all fields
	// before next use, and SetCallAction/SetCreateAction overwrite ActionData.
	interp.Bytecode = nil
	interp.Memory = nil
	interp.ReturnData = nil
	interp.HasAction = false
	interp.ActionData.Call.Input = nil
	interp.ActionData.Create.InitCode = nil
	interp.Input.Input = nil
	interp.ReturnAlloc = nil
	interp.Journal = nil
	interp.Stack.Clear()
	interpreterPool.Put(interp)
}

// memoryPool pools root Memory objects (each starts with a 4KB buffer).
var memoryPool = sync.Pool{
	New: func() any { return NewMemory() },
}

// AcquireMemory returns a Memory from the pool, reset to empty state.
func AcquireMemory() *Memory {
	m := memoryPool.Get().(*Memory)
	m.Reset()
	return m
}

// ReleaseMemory returns a Memory to the pool.
func ReleaseMemory(m *Memory) {
	if m == nil {
		return
	}
	m.Reset()
	memoryPool.Put(m)
}

// bytecodePool pools Bytecode objects to reuse their backing slices.
var bytecodePool = sync.Pool{
	New: func() any { return &Bytecode{} },
}

// AcquireBytecode returns a pooled Bytecode initialized with the given code.
func AcquireBytecode(code []byte) *Bytecode {
	bc := bytecodePool.Get().(*Bytecode)
	bc.Reset(code)
	return bc
}

// AcquireBytecodeWithHash returns a pooled Bytecode, skipping jump table
// analysis if the code hash matches the previous code in this pooled object.
// If jumpTable is non-nil, it is set directly, skipping analysis entirely.
func AcquireBytecodeWithHash(code []byte, hash types.B256, jumpTable []byte) *Bytecode {
	bc := bytecodePool.Get().(*Bytecode)
	bc.ResetWithHash(code, hash)
	if jumpTable != nil {
		bc.SetJumpTable(jumpTable)
	}
	return bc
}

// ReleaseBytecode returns a Bytecode to the pool.
// The hash is preserved so that AcquireBytecodeWithHash can skip
// analysis when the same contract code is loaded again.
func ReleaseBytecode(bc *Bytecode) {
	if bc == nil {
		return
	}
	bytecodePool.Put(bc)
}

// InitEmbeddedMemory initializes an embedded Memory struct's buffer on first use.
// Subsequent calls just reset it for reuse.
func InitEmbeddedMemory(m *Memory) {
	if m.buffer == nil {
		buf := make([]byte, 0, 4096)
		m.buffer = &buf
	}
	m.Reset()
}

// InitEmbeddedInterpreter sets up an embedded Interpreter's Stack pointer on first use.
func InitEmbeddedInterpreter(interp *Interpreter, stack *Stack) {
	if interp.Stack == nil {
		interp.Stack = stack
	}
}

// ResetEmbeddedBytecodeWithHash resets an embedded Bytecode with code and hash,
// optionally using a cached jump table. Same semantics as AcquireBytecodeWithHash
// but operates on an existing (non-pooled) Bytecode.
func ResetEmbeddedBytecodeWithHash(bc *Bytecode, code []byte, hash types.B256, jumpTable []byte) {
	bc.ResetWithHash(code, hash)
	if jumpTable != nil {
		bc.SetJumpTable(jumpTable)
	}
}
