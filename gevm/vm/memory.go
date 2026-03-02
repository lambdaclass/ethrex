package vm

import (
	"encoding/binary"
	"sync"

	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
)

// childMemPool reuses Memory structs for child contexts.
var childMemPool = sync.Pool{
	New: func() any { return &Memory{} },
}

// Memory represents EVM memory, a byte-addressable, word-aligned growable buffer.
// In Go we use a simpler design with a shared byte slice and checkpoint-based context nesting.
type Memory struct {
	// The underlying buffer, shared across call contexts via pointer.
	buffer *[]byte
	// Checkpoint offset for the current context.
	checkpoint int
	// Child checkpoint to free on return (if any).
	childCheckpoint int
	// Whether a child context is active.
	hasChild bool
}

// NewMemory creates a new Memory with 4KiB initial capacity.
func NewMemory() *Memory {
	buf := make([]byte, 0, 4096)
	return &Memory{
		buffer:     &buf,
		checkpoint: 0,
	}
}

// Reset resets the Memory to an empty state, retaining the underlying buffer capacity.
func (m *Memory) Reset() {
	*m.buffer = (*m.buffer)[:0]
	m.checkpoint = 0
	m.childCheckpoint = 0
	m.hasChild = false
}

// NewMemoryWithCapacity creates a new Memory with the given initial capacity.
func NewMemoryWithCapacity(capacity int) *Memory {
	buf := make([]byte, 0, capacity)
	return &Memory{
		buffer:     &buf,
		checkpoint: 0,
	}
}

// Len returns the length of the current context's memory.
func (m *Memory) Len() int {
	return len(*m.buffer) - m.checkpoint
}

// IsEmpty returns true if the current context's memory is empty.
func (m *Memory) IsEmpty() bool {
	return m.Len() == 0
}

// Resize resizes the memory so that the current context has newSize bytes.
func (m *Memory) Resize(newSize int) {
	targetLen := m.checkpoint + newSize
	buf := *m.buffer
	if targetLen <= len(buf) {
		return
	}
	if targetLen <= cap(buf) {
		// Zero-extend within existing capacity.
		// clear() compiles to memclr (SIMD-optimized on arm64).
		oldLen := len(buf)
		buf = buf[:targetLen]
		clear(buf[oldLen:])
	} else {
		// Allocate new buffer with extra capacity.
		newBuf := make([]byte, targetLen, targetLen*2)
		copy(newBuf, buf)
		buf = newBuf
	}
	*m.buffer = buf
}

// Slice returns a slice of the current context's memory at the given range.
// Caller must ensure memory has been resized to accommodate offset+size.
func (m *Memory) Slice(offset, size int) []byte {
	if size == 0 {
		return nil
	}
	start := m.checkpoint + offset
	end := start + size
	buf := *m.buffer
	if start < 0 || end > len(buf) {
		return make([]byte, size) // safety fallback: return zeroed slice
	}
	return buf[start:end]
}

// SliceRange returns a slice of the current context's memory.
func (m *Memory) SliceRange(start, end int) []byte {
	return (*m.buffer)[m.checkpoint+start : m.checkpoint+end]
}

// GetByte returns the byte at the given offset in the current context.
func (m *Memory) GetByte(offset int) byte {
	return (*m.buffer)[m.checkpoint+offset]
}

// GetWord returns a 32-byte B256 from memory at the given offset.
func (m *Memory) GetWord(offset int) types.B256 {
	var b types.B256
	copy(b[:], (*m.buffer)[m.checkpoint+offset:m.checkpoint+offset+32])
	return b
}

// GetU256 returns a Uint256 from memory at the given offset (big-endian).
// Reads directly from the buffer to avoid an intermediate B256 copy.
func (m *Memory) GetU256(offset int) types.Uint256 {
	p := m.checkpoint + offset
	b := (*m.buffer)[p : p+32 : p+32]
	return types.Uint256{
		binary.BigEndian.Uint64(b[24:]),
		binary.BigEndian.Uint64(b[16:]),
		binary.BigEndian.Uint64(b[8:]),
		binary.BigEndian.Uint64(b[0:]),
	}
}

// SetByte sets a single byte at the given offset.
func (m *Memory) SetByte(offset int, b byte) {
	(*m.buffer)[m.checkpoint+offset] = b
}

// SetWord sets a 32-byte B256 at the given offset.
func (m *Memory) SetWord(offset int, value types.B256) {
	copy((*m.buffer)[m.checkpoint+offset:], value[:])
}

// SetU256 sets a Uint256 at the given offset (big-endian).
func (m *Memory) SetU256(offset int, value types.Uint256) {
	value.PutBytes32((*m.buffer)[m.checkpoint+offset:])
}

// Set copies data into memory at the given offset.
// Caller must ensure memory has been resized to accommodate offset+len(data).
func (m *Memory) Set(offset int, data []byte) {
	if len(data) == 0 {
		return
	}
	start := m.checkpoint + offset
	buf := *m.buffer
	if start < 0 || start+len(data) > len(buf) {
		return // safety: should not happen if ResizeMemory was called
	}
	copy(buf[start:], data)
}

// SetData copies data from src into memory, handling out-of-bounds src gracefully.
// Zeroes any bytes in the destination that are beyond the source.
func (m *Memory) SetData(memoryOffset, dataOffset, length int, data []byte) {
	if length == 0 {
		return
	}

	dst := (*m.buffer)[m.checkpoint:]

	// Safety: ensure memoryOffset is valid for the dst slice
	if memoryOffset < 0 || memoryOffset+length > len(dst) {
		return
	}

	if dataOffset < 0 || dataOffset >= len(data) {
		// Zero all destination bytes.
		zeroSlice(dst[memoryOffset : memoryOffset+length])
		return
	}

	srcEnd := dataOffset + length
	if srcEnd > len(data) {
		srcEnd = len(data)
	}
	srcLen := srcEnd - dataOffset

	copy(dst[memoryOffset:memoryOffset+srcLen], data[dataOffset:srcEnd])

	// Zero remaining bytes.
	if srcLen < length {
		zeroSlice(dst[memoryOffset+srcLen : memoryOffset+length])
	}
}

// zeroSlice zeros a byte slice. clear() compiles to SIMD-optimized memclr.
func zeroSlice(b []byte) {
	clear(b)
}

// Copy copies len bytes within the current context's memory from src to dst.
func (m *Memory) Copy(dst, src, length int) {
	if length == 0 {
		return
	}
	buf := (*m.buffer)[m.checkpoint:]
	bufLen := len(buf)
	if dst < 0 || src < 0 || dst+length > bufLen || src+length > bufLen {
		return // safety: should not happen if ResizeMemory was called
	}
	copy(buf[dst:dst+length], buf[src:src+length])
}

// ContextMemory returns the current context's memory as a byte slice.
func (m *Memory) ContextMemory() []byte {
	return (*m.buffer)[m.checkpoint:]
}

// NewChildContext creates a child memory context for a nested CALL/CREATE.
// Returns a pooled Memory that shares the buffer but starts at the current end.
func (m *Memory) NewChildContext() *Memory {
	if m.hasChild {
		panic("new_child_context called while child already active")
	}
	newCheckpoint := len(*m.buffer)
	m.hasChild = true
	m.childCheckpoint = newCheckpoint
	child := childMemPool.Get().(*Memory)
	child.buffer = m.buffer
	child.checkpoint = newCheckpoint
	child.hasChild = false
	child.childCheckpoint = 0
	return child
}

// FreeChildContext truncates the buffer back to the child checkpoint.
func (m *Memory) FreeChildContext() {
	if !m.hasChild {
		return
	}
	m.hasChild = false
	*m.buffer = (*m.buffer)[:m.childCheckpoint]
}

// ReleaseChildContext frees the child context and returns the child Memory to the pool.
func (m *Memory) ReleaseChildContext(child *Memory) {
	m.FreeChildContext()
	child.buffer = nil
	childMemPool.Put(child)
}

// ResizeMemory performs EVM memory resize with gas accounting.
// Returns the InstructionResult error if out of gas, or 0 on success.
func ResizeMemory(gas *Gas, mem *Memory, gasParams *spec.GasParams, offset, length int) InstructionResult {
	// Guard against integer overflow in offset + length
	if offset < 0 || length < 0 || offset > maxInt-length {
		return InstructionResultMemoryOOG
	}
	newNumWords := numWords(offset + length)
	if newNumWords <= gas.memory.WordsNum {
		return 0 // no expansion needed
	}
	return resizeMemoryCold(gas, mem, gasParams, newNumWords)
}

func resizeMemoryCold(gas *Gas, mem *Memory, gasParams *spec.GasParams, newNumWords uint64) InstructionResult {
	cost := gasParams.MemoryCost(newNumWords)
	incrementalCost, ok := gas.memory.SetWordsNum(newNumWords, cost)
	if !ok {
		return InstructionResultMemoryOOG
	}
	if !gas.RecordCost(incrementalCost) {
		return InstructionResultMemoryOOG
	}
	mem.Resize(int(newNumWords) * 32)
	return 0
}

// numWords returns the number of 32-byte words needed for the given byte count (rounds up).
func numWords(length int) uint64 {
	if length <= 0 {
		return 0
	}
	return uint64((length + 31) / 32)
}
