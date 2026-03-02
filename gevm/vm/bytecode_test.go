package vm

import "testing"

func TestBytecodeNew(t *testing.T) {
	code := []byte{0x60, 0x01, 0x60, 0x02, 0x01} // PUSH1 1 PUSH1 2 ADD
	bc := NewBytecode(code)

	if bc.Len() != 5 {
		t.Errorf("len: got %d, want 5", bc.Len())
	}
	if bc.PC() != 0 {
		t.Errorf("pc: got %d, want 0", bc.PC())
	}
	if bc.Opcode() != 0x60 {
		t.Errorf("opcode: got %x, want 60", bc.Opcode())
	}
	if !bc.IsRunning() {
		t.Error("should be running")
	}
}

func TestBytecodeJumps(t *testing.T) {
	code := []byte{0x60, 0x01, 0x60, 0x02, 0x01} // PUSH1 1 PUSH1 2 ADD
	bc := NewBytecode(code)

	// Relative jump
	bc.RelativeJump(2)
	if bc.PC() != 2 {
		t.Errorf("pc after relative jump: got %d, want 2", bc.PC())
	}
	if bc.Opcode() != 0x60 {
		t.Errorf("opcode at 2: got %x, want 60", bc.Opcode())
	}

	// Absolute jump
	bc.AbsoluteJump(4)
	if bc.PC() != 4 {
		t.Errorf("pc after absolute jump: got %d, want 4", bc.PC())
	}
	if bc.Opcode() != 0x01 {
		t.Errorf("opcode at 4: got %x, want 01", bc.Opcode())
	}
}

func TestBytecodeJumpDest(t *testing.T) {
	// Code: PUSH1 0x5b, JUMPDEST (0x5b at position 2), STOP
	code := []byte{0x60, 0x5b, 0x5b, 0x00}
	bc := NewBytecode(code)

	// Position 0: PUSH1 - not a valid jump dest
	if bc.IsValidJump(0) {
		t.Error("position 0 (PUSH1) should not be valid jump dest")
	}

	// Position 1: 0x5b but it's PUSH1 data - not a valid jump dest
	if bc.IsValidJump(1) {
		t.Error("position 1 (PUSH data) should not be valid jump dest")
	}

	// Position 2: JUMPDEST - valid
	if !bc.IsValidJump(2) {
		t.Error("position 2 (JUMPDEST) should be valid jump dest")
	}

	// Position 3: STOP - not a valid jump dest
	if bc.IsValidJump(3) {
		t.Error("position 3 (STOP) should not be valid jump dest")
	}

	// Out of bounds
	if bc.IsValidJump(10) {
		t.Error("position 10 should not be valid jump dest")
	}
}

func TestBytecodeImmediates(t *testing.T) {
	code := []byte{0x61, 0x01, 0x00} // PUSH2 0x0100
	bc := NewBytecode(code)

	bc.RelativeJump(1) // move to immediates
	val := bc.ReadU16()
	if val != 0x0100 {
		t.Errorf("read_u16: got %x, want 100", val)
	}

	bc.AbsoluteJump(1)
	b := bc.ReadU8()
	if b != 0x01 {
		t.Errorf("read_u8: got %x, want 01", b)
	}
}

func TestBytecodeReadSlice(t *testing.T) {
	code := []byte{0x63, 0xDE, 0xAD, 0xBE, 0xEF} // PUSH4 DEADBEEF
	bc := NewBytecode(code)
	bc.RelativeJump(1)

	slice := bc.ReadSlice(4)
	expected := []byte{0xDE, 0xAD, 0xBE, 0xEF}
	for i, b := range slice {
		if b != expected[i] {
			t.Errorf("slice[%d]: got %x, want %x", i, b, expected[i])
		}
	}
}

func TestBytecodeLoopControl(t *testing.T) {
	code := []byte{0x60, 0x01, 0x00} // PUSH1 1, STOP
	bc := NewBytecode(code)

	if !bc.IsRunning() {
		t.Error("should be running initially")
	}
	bc.Stop()
	if bc.IsRunning() {
		t.Error("should not be running after stop")
	}
}

func TestBytecodeEmpty(t *testing.T) {
	bc := NewBytecode(nil)
	if bc.Len() != 0 {
		t.Errorf("empty bytecode len: got %d, want 0", bc.Len())
	}
	// Should still be safe to read opcode (padded STOP)
	if bc.Opcode() != 0x00 {
		t.Errorf("empty bytecode opcode: got %x, want 00", bc.Opcode())
	}
}

func TestBytecodeOriginalSlice(t *testing.T) {
	code := []byte{0x60, 0x01, 0x00}
	bc := NewBytecode(code)

	slice := bc.BytecodeSlice()
	if len(slice) != 3 {
		t.Errorf("original slice len: got %d, want 3", len(slice))
	}
	for i, b := range code {
		if slice[i] != b {
			t.Errorf("original slice[%d]: got %x, want %x", i, slice[i], b)
		}
	}
}

func TestBytecodePUSH32(t *testing.T) {
	// PUSH32 followed by 32 bytes, then a JUMPDEST
	code := make([]byte, 34)
	code[0] = 0x7f // PUSH32
	for i := 1; i <= 32; i++ {
		code[i] = 0x5b // These bytes are data, not JUMPDEST
	}
	code[33] = 0x5b // This is a real JUMPDEST

	bc := NewBytecode(code)

	// Positions 1-32: data bytes should NOT be valid jump dests
	for i := 1; i <= 32; i++ {
		if bc.IsValidJump(i) {
			t.Errorf("position %d (PUSH32 data) should not be valid jump dest", i)
		}
	}

	// Position 33: real JUMPDEST
	if !bc.IsValidJump(33) {
		t.Error("position 33 (JUMPDEST) should be valid")
	}
}
