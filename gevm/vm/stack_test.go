package vm

import (
	"testing"

	"github.com/Giulio2002/gevm/types"
)

func TestStackPushPop(t *testing.T) {
	s := NewStack()

	if !s.Push(types.U256From(42)) {
		t.Fatal("push should succeed")
	}
	if s.Len() != 1 {
		t.Errorf("len: got %d, want 1", s.Len())
	}

	val, ok := s.Pop()
	if !ok {
		t.Fatal("pop should succeed")
	}
	if !val.Eq(types.U256From(42)) {
		t.Errorf("pop: got %s, want 42", val.Hex())
	}
	if s.Len() != 0 {
		t.Errorf("len after pop: got %d, want 0", s.Len())
	}
}

func TestStackPopEmpty(t *testing.T) {
	s := NewStack()
	_, ok := s.Pop()
	if ok {
		t.Error("pop on empty should fail")
	}
}

func TestStackOverflow(t *testing.T) {
	s := NewStack()
	for i := 0; i < StackLimit; i++ {
		if !s.Push(types.U256From(uint64(i))) {
			t.Fatalf("push %d should succeed", i)
		}
	}
	if s.Push(types.U256One) {
		t.Error("push beyond limit should fail")
	}
	if s.Len() != StackLimit {
		t.Errorf("len: got %d, want %d", s.Len(), StackLimit)
	}
}

func TestStackPeek(t *testing.T) {
	s := NewStack()
	s.Push(types.U256From(10))
	s.Push(types.U256From(20))
	s.Push(types.U256From(30))

	val, ok := s.Peek(0) // top
	if !ok || !val.Eq(types.U256From(30)) {
		t.Errorf("peek(0): got %s, want 30", val.Hex())
	}
	val, ok = s.Peek(1)
	if !ok || !val.Eq(types.U256From(20)) {
		t.Errorf("peek(1): got %s, want 20", val.Hex())
	}
	val, ok = s.Peek(2)
	if !ok || !val.Eq(types.U256From(10)) {
		t.Errorf("peek(2): got %s, want 10", val.Hex())
	}
	_, ok = s.Peek(3)
	if ok {
		t.Error("peek(3) on 3-deep stack should fail")
	}
}

func TestStackDup(t *testing.T) {
	s := NewStack()
	s.Push(types.U256From(10))
	s.Push(types.U256From(20))
	s.Push(types.U256From(30))

	// DUP1 duplicates the top
	if !s.Dup(1) {
		t.Error("dup(1) should succeed")
	}
	if s.Len() != 4 {
		t.Errorf("len after dup: got %d, want 4", s.Len())
	}
	val, _ := s.Peek(0)
	if !val.Eq(types.U256From(30)) {
		t.Errorf("dup(1) top: got %s, want 30", val.Hex())
	}

	// DUP from deeper
	s2 := NewStack()
	s2.Push(types.U256From(1))
	s2.Push(types.U256From(2))
	s2.Push(types.U256From(3))
	if !s2.Dup(3) {
		t.Error("dup(3) should succeed")
	}
	val, _ = s2.Peek(0)
	if !val.Eq(types.U256From(1)) {
		t.Errorf("dup(3) top: got %s, want 1", val.Hex())
	}
}

func TestStackDupUnderflow(t *testing.T) {
	s := NewStack()
	s.Push(types.U256From(1))
	if s.Dup(2) {
		t.Error("dup(2) on 1-deep stack should fail")
	}
}

func TestStackSwap(t *testing.T) {
	s := NewStack()
	s.Push(types.U256From(10))
	s.Push(types.U256From(20))
	s.Push(types.U256From(30))

	// SWAP1: swap top with second
	if !s.Swap(1) {
		t.Error("swap(1) should succeed")
	}
	top, _ := s.Peek(0)
	second, _ := s.Peek(1)
	if !top.Eq(types.U256From(20)) {
		t.Errorf("after swap(1) top: got %s, want 20", top.Hex())
	}
	if !second.Eq(types.U256From(30)) {
		t.Errorf("after swap(1) second: got %s, want 30", second.Hex())
	}

	// SWAP2: swap top with third (now: [10, 30, 20])
	if !s.Swap(2) {
		t.Error("swap(2) should succeed")
	}
	top, _ = s.Peek(0)
	third, _ := s.Peek(2)
	if !top.Eq(types.U256From(10)) {
		t.Errorf("after swap(2) top: got %s, want 10", top.Hex())
	}
	if !third.Eq(types.U256From(20)) {
		t.Errorf("after swap(2) third: got %s, want 20", third.Hex())
	}
}

func TestStackSwapUnderflow(t *testing.T) {
	s := NewStack()
	s.Push(types.U256From(1))
	if s.Swap(1) {
		t.Error("swap(1) on 1-deep stack should fail")
	}
}

func TestStackExchange(t *testing.T) {
	s := NewStack()
	// bottom -> top: 1, 2, 3, 4, 5
	for i := 1; i <= 5; i++ {
		s.Push(types.U256From(uint64(i)))
	}

	// Exchange(1, 2): swap position 1 from top with position 1+2=3 from top
	// Stack: [1, 2, 3, 4, 5] -> swap idx 4-1=3 and 4-3=1 -> [1, 4, 3, 2, 5]
	if !s.Exchange(1, 2) {
		t.Error("exchange(1,2) should succeed")
	}
	v1, _ := s.Peek(1)
	v3, _ := s.Peek(3)
	if !v1.Eq(types.U256From(2)) {
		t.Errorf("after exchange(1,2) pos 1: got %s, want 2", v1.Hex())
	}
	if !v3.Eq(types.U256From(4)) {
		t.Errorf("after exchange(1,2) pos 3: got %s, want 4", v3.Hex())
	}
}

func TestStackPop2(t *testing.T) {
	s := NewStack()
	s.Push(types.U256From(10))
	s.Push(types.U256From(20))

	a, b, ok := s.Pop2()
	if !ok {
		t.Fatal("pop2 should succeed")
	}
	if !a.Eq(types.U256From(20)) {
		t.Errorf("pop2 a: got %s, want 20", a.Hex())
	}
	if !b.Eq(types.U256From(10)) {
		t.Errorf("pop2 b: got %s, want 10", b.Hex())
	}
}

func TestStackPop3(t *testing.T) {
	s := NewStack()
	s.Push(types.U256From(10))
	s.Push(types.U256From(20))
	s.Push(types.U256From(30))

	a, b, c, ok := s.Pop3()
	if !ok {
		t.Fatal("pop3 should succeed")
	}
	if !a.Eq(types.U256From(30)) || !b.Eq(types.U256From(20)) || !c.Eq(types.U256From(10)) {
		t.Errorf("pop3: got %s %s %s", a.Hex(), b.Hex(), c.Hex())
	}
}

func TestStackPop1Top(t *testing.T) {
	s := NewStack()
	s.Push(types.U256From(10))
	s.Push(types.U256From(20))

	a, top, ok := s.Pop1Top()
	if !ok {
		t.Fatal("pop1top should succeed")
	}
	if !a.Eq(types.U256From(20)) {
		t.Errorf("pop1top a: got %s, want 20", a.Hex())
	}
	if !top.Eq(types.U256From(10)) {
		t.Errorf("pop1top top: got %s, want 10", top.Hex())
	}

	// Modify through pointer
	*top = types.U256From(99)
	val, _ := s.Peek(0)
	if !val.Eq(types.U256From(99)) {
		t.Errorf("after modify top: got %s, want 99", val.Hex())
	}
}

func TestStackSet(t *testing.T) {
	s := NewStack()
	s.Push(types.U256From(10))
	s.Push(types.U256From(20))

	if !s.Set(0, types.U256From(30)) {
		t.Error("set(0) should succeed")
	}
	top, _ := s.Peek(0)
	if !top.Eq(types.U256From(30)) {
		t.Errorf("set top: got %s, want 30", top.Hex())
	}

	if s.Set(5, types.U256One) {
		t.Error("set(5) on 2-deep stack should fail")
	}
}

func TestStackPushSlice(t *testing.T) {
	s := NewStack()

	// Empty slice: no-op
	if !s.PushSlice(nil) {
		t.Error("push empty should succeed")
	}
	if s.Len() != 0 {
		t.Error("empty push should not add items")
	}

	// Single byte
	if !s.PushSlice([]byte{42}) {
		t.Error("push [42] should succeed")
	}
	val, _ := s.Pop()
	if !val.Eq(types.U256From(42)) {
		t.Errorf("push [42]: got %s, want 42", val.Hex())
	}
}

func TestStackClone(t *testing.T) {
	s := NewStack()
	s.Push(types.U256From(10))
	s.Push(types.U256From(20))

	// Clone by creating a new stack and copying
	s2 := NewStack()
	for _, v := range s.Data() {
		s2.Push(v)
	}

	if s2.Len() != 2 {
		t.Errorf("clone len: got %d, want 2", s2.Len())
	}

	// Modify clone, original unchanged
	s2.Push(types.U256From(30))
	if s.Len() != 2 {
		t.Errorf("original len after clone modify: got %d, want 2", s.Len())
	}
}

func TestStackString(t *testing.T) {
	s := NewStack()
	s.Push(types.U256From(1))
	s.Push(types.U256From(255))
	str := s.String()
	if str != "[0x1, 0xff]" {
		t.Errorf("string: got %s", str)
	}
}
