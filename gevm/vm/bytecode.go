package vm

import (
	"encoding/binary"

	"github.com/Giulio2002/gevm/types"
)

// Bytecode wraps raw EVM bytecode with execution metadata: program counter,
// jump table validation, and loop control.
type Bytecode struct {
	// The raw bytecode bytes (padded with a trailing STOP).
	code []byte
	// The original bytecode length (before padding).
	originalLen int
	// Current program counter.
	pc int
	// Whether execution should continue.
	running bool
	// The jump table for legacy bytecode (nil if not analyzed).
	// Bit i is set if code[i] is a valid JUMPDEST.
	jumpTable []byte
	// Whether the jump table has been built for the current code.
	jumpTableReady bool
	// Whether the jump table was set externally (must not be mutated by ensureJumpTable).
	jumpTableExternal bool
	// The hash of the bytecode, lazily computed.
	hash *types.B256
}

// bytecodeEndPadding is the number of zero bytes appended after bytecode.
// Must be >= 33 to safely read PUSH32 immediates (1 opcode + 32 data bytes)
// and ReadU16 (2 bytes) at any position without bounds checking.
const bytecodeEndPadding = 33

// NewBytecode creates a Bytecode from raw code bytes.
// It analyzes the code to build a jump table and pads with trailing zeros
// to ensure safe reading of immediate operands past the end.
func NewBytecode(code []byte) *Bytecode {
	originalLen := len(code)

	// Pad with zeros to allow safe reads of PUSH32 immediates past end.
	padded := make([]byte, originalLen+bytecodeEndPadding)
	copy(padded, code)
	// All padding bytes are already zero (STOP opcode).

	bc := &Bytecode{
		code:        padded,
		originalLen: originalLen,
		pc:          0,
		running:     true,
	}
	bc.ensureJumpTable()
	return bc
}

// ResetWithHash reinitializes from new code, skipping jump table analysis
// if the code hash matches the previously cached hash (same contract called again).
func (b *Bytecode) ResetWithHash(code []byte, hash types.B256) {
	originalLen := len(code)

	// If same hash as previous call on this pooled bytecode, skip analysis
	if b.hash != nil && *b.hash == hash && b.originalLen == originalLen {
		needed := originalLen + bytecodeEndPadding
		if cap(b.code) >= needed {
			b.code = b.code[:needed]
		} else {
			b.code = make([]byte, needed)
		}
		copy(b.code, code)
		clear(b.code[originalLen:needed])
		b.pc = 0
		b.running = true
		// jumpTable + jumpTableReady still valid from previous Reset
		return
	}

	b.Reset(code)
	h := hash
	b.hash = &h
}

// Reset reinitializes the Bytecode from new code, reusing existing slice capacity.
func (b *Bytecode) Reset(code []byte) {
	originalLen := len(code)
	needed := originalLen + bytecodeEndPadding

	if cap(b.code) >= needed {
		b.code = b.code[:needed]
	} else {
		b.code = make([]byte, needed)
	}
	copy(b.code, code)
	// Zero padding bytes
	clear(b.code[originalLen:needed])

	b.originalLen = originalLen
	b.pc = 0
	b.running = true
	b.hash = nil
	b.jumpTableReady = false // defer analysis until first IsValidJump
}

// SetJumpTable sets an externally-provided jump table, skipping analysis.
// The table must be a valid JUMPDEST bitmap for the current code.
// The slice is borrowed (not copied) and must not be mutated by the caller.
func (b *Bytecode) SetJumpTable(jt []byte) {
	b.jumpTable = jt
	b.jumpTableReady = true
	b.jumpTableExternal = true
}

// GetJumpTable returns the jump table, building it lazily if needed.
// Callers can cache this externally and pass it via SetJumpTable on
// future Bytecode instances with the same code, avoiding re-analysis.
// Marks the table as external so ensureJumpTable won't reuse the slice
// when this Bytecode is later recycled for different code.
func (b *Bytecode) GetJumpTable() []byte {
	b.ensureJumpTable()
	b.jumpTableExternal = true
	return b.jumpTable
}

// ensureJumpTable builds the jump table if not yet built for the current code.
func (b *Bytecode) ensureJumpTable() {
	if b.jumpTableReady {
		return
	}
	b.jumpTableReady = true

	tableLen := (b.originalLen + 7) / 8
	// Don't reuse capacity if the slice was externally provided (it belongs to the cache).
	if !b.jumpTableExternal && cap(b.jumpTable) >= tableLen {
		b.jumpTable = b.jumpTable[:tableLen]
		clear(b.jumpTable)
	} else {
		b.jumpTable = make([]byte, tableLen)
		b.jumpTableExternal = false
	}

	// Scan bytecode for JUMPDESTs with fast-path for zero regions.
	// In CREATE scenarios the initcode is often mostly zeros; scanning
	// 8 bytes at a time as a uint64 skips zero regions ~8x faster.
	i := 0
	for i < b.originalLen {
		if i+8 <= b.originalLen && binary.LittleEndian.Uint64(b.code[i:i+8]) == 0 {
			i += 8
			continue
		}
		op := b.code[i]
		if op == 0x5b { // JUMPDEST
			b.jumpTable[i/8] |= 1 << (uint(i) % 8)
		}
		if op >= 0x60 && op <= 0x7f {
			i += int(op-0x60) + 1
		}
		i++
	}
}

// NewBytecodeWithHash creates a Bytecode with a precomputed hash.
func NewBytecodeWithHash(code []byte, hash types.B256) *Bytecode {
	bc := NewBytecode(code)
	bc.hash = &hash
	return bc
}

// analyzeJumps builds the jump table by scanning for JUMPDEST (0x5b) opcodes.
// It correctly skips over PUSH data.
func (b *Bytecode) analyzeJumps() []byte {
	tableLen := (b.originalLen + 7) / 8
	table := make([]byte, tableLen)

	i := 0
	for i < b.originalLen {
		op := b.code[i]
		if op == 0x5b { // JUMPDEST
			table[i/8] |= 1 << (uint(i) % 8)
		}
		// Skip PUSH data: opcodes 0x60-0x7f push 1-32 bytes.
		if op >= 0x60 && op <= 0x7f {
			i += int(op-0x60) + 1
		}
		i++
	}
	return table
}

// --- LoopControl ---

// IsRunning returns true if execution should continue.
func (b *Bytecode) IsRunning() bool {
	return b.running
}

// Stop sets the running flag to false.
func (b *Bytecode) Stop() {
	b.running = false
}

// Resume sets the running flag to true.
// Used to restart execution after processing a sub-frame action.
func (b *Bytecode) Resume() {
	b.running = true
}

// --- Jumps ---

// PC returns the current program counter.
func (b *Bytecode) PC() int {
	return b.pc
}

// Opcode returns the opcode at the current PC.
func (b *Bytecode) Opcode() byte {
	return b.code[b.pc]
}

// RelativeJump advances/retreats the PC by offset.
func (b *Bytecode) RelativeJump(offset int) {
	b.pc += offset
}

// AbsoluteJump sets the PC to the given offset.
func (b *Bytecode) AbsoluteJump(offset int) {
	b.pc = offset
}

// IsValidJump returns true if the given offset is a valid JUMPDEST.
func (b *Bytecode) IsValidJump(offset int) bool {
	if offset >= b.originalLen {
		return false
	}
	if b.code[offset] != 0x5b { // Must be JUMPDEST
		return false
	}
	b.ensureJumpTable()
	return b.jumpTable[offset/8]&(1<<(uint(offset)%8)) != 0
}

// --- Immediates ---

// ReadU8 reads the next byte at the current PC (immediate operand).
func (b *Bytecode) ReadU8() uint8 {
	return b.code[b.pc]
}

// ReadI8 reads the next byte at the current PC as a signed integer.
func (b *Bytecode) ReadI8() int8 {
	return int8(b.code[b.pc])
}

// ReadU16 reads the next 2 bytes at the current PC as big-endian uint16.
func (b *Bytecode) ReadU16() uint16 {
	return binary.BigEndian.Uint16(b.code[b.pc : b.pc+2])
}

// ReadI16 reads the next 2 bytes at the current PC as big-endian int16.
func (b *Bytecode) ReadI16() int16 {
	return int16(b.ReadU16())
}

// ReadSlice reads the next len bytes at the current PC.
func (b *Bytecode) ReadSlice(length int) []byte {
	return b.code[b.pc : b.pc+length]
}

// ReadOffsetU16 reads 2 bytes at the current PC + offset as big-endian uint16.
func (b *Bytecode) ReadOffsetU16(offset int) uint16 {
	pos := b.pc + offset
	return binary.BigEndian.Uint16(b.code[pos : pos+2])
}

// --- LegacyBytecode ---

// Len returns the original bytecode length (before padding).
func (b *Bytecode) Len() int {
	return b.originalLen
}

// BytecodeSlice returns the original bytecode bytes.
func (b *Bytecode) BytecodeSlice() []byte {
	return b.code[:b.originalLen]
}

// Code returns the full code bytes (including trailing STOP).
func (b *Bytecode) Code() []byte {
	return b.code
}
