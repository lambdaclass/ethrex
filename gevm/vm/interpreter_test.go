package vm

import (
	"testing"

	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
)

func TestInterpreterNew(t *testing.T) {
	interp := NewInterpreter(
		NewMemory(),
		NewBytecode([]byte{0x00}), // STOP
		Inputs{},
		false,
		spec.Frontier,
		1000,
	)

	if interp.Gas.Limit() != 1000 {
		t.Errorf("gas limit: got %d, want 1000", interp.Gas.Limit())
	}
	if interp.Gas.Remaining() != 1000 {
		t.Errorf("gas remaining: got %d, want 1000", interp.Gas.Remaining())
	}
	if interp.Stack.Len() != 0 {
		t.Errorf("stack len: got %d, want 0", interp.Stack.Len())
	}
	if interp.Memory.Len() != 0 {
		t.Errorf("memory len: got %d, want 0", interp.Memory.Len())
	}
	if interp.RuntimeFlag.IsStatic {
		t.Error("should not be static")
	}
	if interp.RuntimeFlag.ForkID != spec.Frontier {
		t.Errorf("spec_id: got %v, want Frontier", interp.RuntimeFlag.ForkID)
	}
}

func TestInterpreterDefault(t *testing.T) {
	interp := DefaultInterpreter()
	if interp.Gas.Limit() != ^uint64(0) {
		t.Errorf("default gas limit: got %d, want max", interp.Gas.Limit())
	}
	if interp.RuntimeFlag.ForkID != spec.LatestForkID {
		t.Errorf("default spec_id: got %v, want %v", interp.RuntimeFlag.ForkID, spec.LatestForkID)
	}
}

func TestInterpreterClear(t *testing.T) {
	interp := DefaultInterpreter()
	interp.Stack.Push(types.U256From(42))
	interp.Gas.RecordCost(100)

	interp.Clear(
		NewMemory(),
		NewBytecode([]byte{0x01}),
		Inputs{},
		true,
		spec.London,
		500,
	)

	if interp.Stack.Len() != 0 {
		t.Errorf("stack after clear: got %d, want 0", interp.Stack.Len())
	}
	if interp.Gas.Limit() != 500 {
		t.Errorf("gas limit after clear: got %d, want 500", interp.Gas.Limit())
	}
	if interp.Gas.Remaining() != 500 {
		t.Errorf("gas remaining after clear: got %d, want 500", interp.Gas.Remaining())
	}
	if !interp.RuntimeFlag.IsStatic {
		t.Error("should be static after clear")
	}
	if interp.RuntimeFlag.ForkID != spec.London {
		t.Errorf("spec_id after clear: got %v, want London", interp.RuntimeFlag.ForkID)
	}
}

func TestInterpreterHalt(t *testing.T) {
	interp := DefaultInterpreter()
	interp.Halt(InstructionResultOutOfGas)
	if interp.Bytecode.IsRunning() {
		t.Error("should not be running after halt")
	}
}

func TestInterpreterHaltOOG(t *testing.T) {
	interp := NewInterpreter(
		NewMemory(),
		NewBytecode([]byte{0x00}),
		Inputs{},
		false,
		spec.Frontier,
		1000,
	)

	interp.HaltOOG()
	if interp.Gas.Remaining() != 0 {
		t.Errorf("remaining after oog: got %d, want 0", interp.Gas.Remaining())
	}
	if interp.Bytecode.IsRunning() {
		t.Error("should not be running after oog")
	}
}

func TestInterpreterResizeMemory(t *testing.T) {
	interp := NewInterpreter(
		NewMemory(),
		NewBytecode([]byte{0x00}),
		Inputs{},
		false,
		spec.Frontier,
		10000,
	)

	// Resize to 32 bytes: should succeed, cost 3 gas
	if !interp.ResizeMemory(0, 32) {
		t.Error("resize to 32 should succeed")
	}
	if interp.Memory.Len() != 32 {
		t.Errorf("memory len: got %d, want 32", interp.Memory.Len())
	}
	if interp.Gas.Spent() != 3 {
		t.Errorf("gas spent: got %d, want 3", interp.Gas.Spent())
	}
}

func TestInterpreterResizeMemoryOOG(t *testing.T) {
	interp := NewInterpreter(
		NewMemory(),
		NewBytecode([]byte{0x00}),
		Inputs{},
		false,
		spec.Frontier,
		2, // Only 2 gas, 1 word costs 3
	)

	if interp.ResizeMemory(0, 32) {
		t.Error("resize should fail with OOG")
	}
	if interp.Bytecode.IsRunning() {
		t.Error("should be halted after OOG")
	}
}

func TestInterpreterResultNew(t *testing.T) {
	r := NewInterpreterResult(InstructionResultReturn, types.Bytes{0x01}, NewGas(100))
	if !r.IsOk() {
		t.Error("result should be ok")
	}
	if r.IsRevert() {
		t.Error("result should not be revert")
	}
	if r.IsError() {
		t.Error("result should not be error")
	}
	if len(r.Output) != 1 || r.Output[0] != 0x01 {
		t.Error("output mismatch")
	}
}

func TestInterpreterResultOOG(t *testing.T) {
	r := NewInterpreterResultOOG(500)
	if r.IsOk() {
		t.Error("oog result should not be ok")
	}
	if !r.IsError() {
		t.Error("oog result should be error")
	}
	if r.Gas.Remaining() != 0 {
		t.Errorf("oog remaining: got %d, want 0", r.Gas.Remaining())
	}
	if r.Gas.Spent() != 500 {
		t.Errorf("oog spent: got %d, want 500", r.Gas.Spent())
	}
}
