package vm

import (
	"testing"

	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
)

func TestMemoryNew(t *testing.T) {
	m := NewMemory()
	if m.Len() != 0 {
		t.Errorf("len: got %d, want 0", m.Len())
	}
	if !m.IsEmpty() {
		t.Error("should be empty")
	}
}

func TestMemoryResize(t *testing.T) {
	m := NewMemory()
	m.Resize(32)
	if m.Len() != 32 {
		t.Errorf("len after resize: got %d, want 32", m.Len())
	}

	// All bytes should be zero
	for i := 0; i < 32; i++ {
		if m.GetByte(i) != 0 {
			t.Errorf("byte %d: got %d, want 0", i, m.GetByte(i))
		}
	}
}

func TestMemorySetGet(t *testing.T) {
	m := NewMemory()
	m.Resize(64)

	// Set/get byte
	m.SetByte(5, 0xAB)
	if m.GetByte(5) != 0xAB {
		t.Errorf("get_byte: got %x, want ab", m.GetByte(5))
	}

	// Set/get word
	word := types.B256{}
	for i := 0; i < 32; i++ {
		word[i] = byte(i)
	}
	m.SetWord(0, word)
	got := m.GetWord(0)
	if got != word {
		t.Error("get_word mismatch")
	}

	// Set/get Uint256
	val := types.U256From(0xDEADBEEF)
	m.SetU256(32, val)
	gotU256 := m.GetU256(32)
	if !gotU256.Eq(val) {
		t.Errorf("get_u256: got %s, want %s", gotU256.Hex(), val.Hex())
	}
}

func TestMemorySet(t *testing.T) {
	m := NewMemory()
	m.Resize(32)

	data := []byte{1, 2, 3, 4, 5}
	m.Set(10, data)

	for i, b := range data {
		if m.GetByte(10+i) != b {
			t.Errorf("byte %d: got %d, want %d", 10+i, m.GetByte(10+i), b)
		}
	}
}

func TestMemorySetData(t *testing.T) {
	m := NewMemory()
	m.Resize(32)

	// Normal copy
	src := []byte{0xAA, 0xBB, 0xCC, 0xDD}
	m.SetData(4, 1, 2, src) // copy src[1..3] (BB, CC) to mem[4..6]
	if m.GetByte(4) != 0xBB || m.GetByte(5) != 0xCC {
		t.Errorf("set_data: got %x %x, want BB CC", m.GetByte(4), m.GetByte(5))
	}

	// dataOffset beyond src: zero-fill
	m.SetData(0, 100, 4, src)
	for i := 0; i < 4; i++ {
		if m.GetByte(i) != 0 {
			t.Errorf("set_data oob: byte %d got %x, want 0", i, m.GetByte(i))
		}
	}

	// Partial: src too short
	m.SetData(10, 2, 4, src) // src[2..4]=CC,DD -> 2 bytes, then 2 zeros
	if m.GetByte(10) != 0xCC || m.GetByte(11) != 0xDD || m.GetByte(12) != 0 || m.GetByte(13) != 0 {
		t.Errorf("set_data partial: got %x %x %x %x",
			m.GetByte(10), m.GetByte(11), m.GetByte(12), m.GetByte(13))
	}
}

func TestMemoryCopy(t *testing.T) {
	m := NewMemory()
	m.Resize(64)
	m.Set(0, []byte{1, 2, 3, 4, 5})

	// Copy 5 bytes from offset 0 to offset 10
	m.Copy(10, 0, 5)
	for i := 0; i < 5; i++ {
		if m.GetByte(10+i) != byte(i+1) {
			t.Errorf("copy byte %d: got %d, want %d", 10+i, m.GetByte(10+i), i+1)
		}
	}
}

func TestMemorySlice(t *testing.T) {
	m := NewMemory()
	m.Resize(32)
	m.Set(0, []byte{10, 20, 30, 40})

	s := m.Slice(1, 2)
	if s[0] != 20 || s[1] != 30 {
		t.Errorf("slice: got %v, want [20, 30]", s)
	}
}

func TestMemoryChildContext(t *testing.T) {
	m1 := NewMemory()
	m1.Resize(32)
	if m1.Len() != 32 {
		t.Errorf("m1 len: got %d, want 32", m1.Len())
	}

	// Create child context
	m2 := m1.NewChildContext()
	if m2.Len() != 0 {
		t.Errorf("m2 initial len: got %d, want 0", m2.Len())
	}
	if m2.checkpoint != 32 {
		t.Errorf("m2 checkpoint: got %d, want 32", m2.checkpoint)
	}

	// Resize child
	m2.Resize(64)
	if m2.Len() != 64 {
		t.Errorf("m2 len after resize: got %d, want 64", m2.Len())
	}
	// Total buffer should be 32 + 64 = 96
	if len(*m2.buffer) != 96 {
		t.Errorf("total buffer: got %d, want 96", len(*m2.buffer))
	}

	// Create another child
	m3 := m2.NewChildContext()
	m3.Resize(16)
	if m3.Len() != 16 {
		t.Errorf("m3 len: got %d, want 16", m3.Len())
	}

	// Free m3
	m2.FreeChildContext()
	if m2.Len() != 64 {
		t.Errorf("m2 len after free m3: got %d, want 64", m2.Len())
	}

	// Free m2
	m1.FreeChildContext()
	if m1.Len() != 32 {
		t.Errorf("m1 len after free m2: got %d, want 32", m1.Len())
	}
}

func TestNumWords(t *testing.T) {
	cases := []struct {
		input int
		want  uint64
	}{
		{0, 0},
		{1, 1},
		{31, 1},
		{32, 1},
		{33, 2},
		{63, 2},
		{64, 2},
		{65, 3},
	}
	for _, tc := range cases {
		got := numWords(tc.input)
		if got != tc.want {
			t.Errorf("numWords(%d): got %d, want %d", tc.input, got, tc.want)
		}
	}
}

func TestResizeMemory(t *testing.T) {
	mem := NewMemory()
	gas := NewGas(10000)
	gasParams := spec.NewGasParams(spec.Frontier)

	// Resize to 32 bytes = 1 word. Cost = 3*1 + 1*1/512 = 3
	result := ResizeMemory(&gas, mem, gasParams, 0, 32)
	if result != 0 {
		t.Errorf("resize to 32: got %s", result.String())
	}
	if mem.Len() != 32 {
		t.Errorf("mem len: got %d, want 32", mem.Len())
	}
	if gas.Spent() != 3 {
		t.Errorf("gas spent: got %d, want 3", gas.Spent())
	}

	// Resize to 64 bytes = 2 words. Cost = 3*2 + 2*2/512 = 6 (incremental: 6-3=3)
	result = ResizeMemory(&gas, mem, gasParams, 0, 64)
	if result != 0 {
		t.Errorf("resize to 64: got %s", result.String())
	}
	if mem.Len() != 64 {
		t.Errorf("mem len: got %d, want 64", mem.Len())
	}
	if gas.Spent() != 6 {
		t.Errorf("gas spent: got %d, want 6", gas.Spent())
	}

	// Same size: no additional cost
	result = ResizeMemory(&gas, mem, gasParams, 0, 64)
	if result != 0 {
		t.Errorf("resize same: got %s", result.String())
	}
	if gas.Spent() != 6 {
		t.Errorf("gas spent unchanged: got %d, want 6", gas.Spent())
	}
}

func TestResizeMemoryOOG(t *testing.T) {
	mem := NewMemory()
	gas := NewGas(2) // Only 2 gas
	gasParams := spec.NewGasParams(spec.Frontier)

	// 1 word costs 3 gas, should fail
	result := ResizeMemory(&gas, mem, gasParams, 0, 32)
	if result != InstructionResultMemoryOOG {
		t.Errorf("resize oog: got %s, want MemoryOOG", result.String())
	}
}
