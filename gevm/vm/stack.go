package vm

import (
	"fmt"
	"strings"

	"github.com/Giulio2002/gevm/types"
)

// StackLimit is the maximum stack depth (1024 words).
const StackLimit = 1024

// Stack is the EVM stack with StackLimit capacity.
// Uses a fixed-size array with a top pointer to avoid slice header manipulation.
type Stack struct {
	data [StackLimit]types.Uint256
	top  int
}

// NewStack creates a new empty stack.
func NewStack() *Stack {
	return &Stack{}
}

// Len returns the number of items on the stack.
func (s *Stack) Len() int {
	return s.top
}

// IsEmpty returns true if the stack is empty.
func (s *Stack) IsEmpty() bool {
	return s.top == 0
}

// Data returns a slice of the active portion of the stack.
func (s *Stack) Data() []types.Uint256 {
	return s.data[:s.top]
}

// Clear removes all items from the stack.
func (s *Stack) Clear() {
	s.top = 0
}

// Push pushes a value onto the stack. Returns false if stack overflow.
func (s *Stack) Push(value types.Uint256) bool {
	if s.top == StackLimit {
		return false
	}
	s.data[s.top] = value
	s.top++
	return true
}

// Pop removes and returns the top value. Returns false if empty.
func (s *Stack) Pop() (types.Uint256, bool) {
	if s.top == 0 {
		return types.U256Zero, false
	}
	s.top--
	return s.data[s.top], true
}

// PopUnsafe pops the top value without bounds checking.
// Caller must ensure the stack is non-empty.
func (s *Stack) PopUnsafe() types.Uint256 {
	s.top--
	return s.data[s.top]
}

// Top returns a pointer to the top of the stack.
// Returns nil if the stack is empty.
func (s *Stack) Top() *types.Uint256 {
	if s.top == 0 {
		return nil
	}
	return &s.data[s.top-1]
}

// TopUnsafe returns a pointer to the top of the stack without bounds checking.
// Caller must ensure the stack is non-empty.
func (s *Stack) TopUnsafe() *types.Uint256 {
	return &s.data[s.top-1]
}

// Popn pops N values from the stack and returns them.
// Values are returned in pop order: [0] = top, [1] = second, etc.
// Returns nil if there are fewer than N items.
func (s *Stack) Popn(n int) []types.Uint256 {
	if s.top < n {
		return nil
	}
	result := make([]types.Uint256, n)
	for i := 0; i < n; i++ {
		result[i] = s.PopUnsafe()
	}
	return result
}

// Pop1 pops 1 value from the stack. Returns false if underflow.
func (s *Stack) Pop1() (types.Uint256, bool) {
	if s.top < 1 {
		return types.U256Zero, false
	}
	return s.PopUnsafe(), true
}

// Pop2 pops 2 values from the stack. Returns false if underflow.
func (s *Stack) Pop2() (types.Uint256, types.Uint256, bool) {
	if s.top < 2 {
		return types.U256Zero, types.U256Zero, false
	}
	a := s.PopUnsafe()
	b := s.PopUnsafe()
	return a, b, true
}

// Pop3 pops 3 values from the stack. Returns false if underflow.
func (s *Stack) Pop3() (types.Uint256, types.Uint256, types.Uint256, bool) {
	if s.top < 3 {
		return types.U256Zero, types.U256Zero, types.U256Zero, false
	}
	a := s.PopUnsafe()
	b := s.PopUnsafe()
	c := s.PopUnsafe()
	return a, b, c, true
}

// PopnTop pops N values and returns a pointer to the new top.
// Returns nil if there are fewer than N+1 items.
func (s *Stack) PopnTop(n int) ([]types.Uint256, *types.Uint256) {
	if s.top < n+1 {
		return nil, nil
	}
	result := make([]types.Uint256, n)
	for i := 0; i < n; i++ {
		result[i] = s.PopUnsafe()
	}
	return result, s.TopUnsafe()
}

// Pop1Top pops 1 value and returns a pointer to the new top.
func (s *Stack) Pop1Top() (types.Uint256, *types.Uint256, bool) {
	if s.top < 2 {
		return types.U256Zero, nil, false
	}
	a := s.PopUnsafe()
	return a, s.TopUnsafe(), true
}

// Pop2Top pops 2 values and returns a pointer to the new top.
func (s *Stack) Pop2Top() (types.Uint256, types.Uint256, *types.Uint256, bool) {
	if s.top < 3 {
		return types.U256Zero, types.U256Zero, nil, false
	}
	a := s.PopUnsafe()
	b := s.PopUnsafe()
	return a, b, s.TopUnsafe(), true
}

// Peek returns the value at the given index from the top (0 = top).
func (s *Stack) Peek(noFromTop int) (types.Uint256, bool) {
	if s.top <= noFromTop {
		return types.U256Zero, false
	}
	return s.data[s.top-noFromTop-1], true
}

// Dup duplicates the Nth value from the top (1-indexed: DUP1 = n=1).
// Returns false if underflow or overflow.
func (s *Stack) Dup(n int) bool {
	if n == 0 {
		panic("attempted to dup 0")
	}
	if s.top < n || s.top+1 > StackLimit {
		return false
	}
	s.data[s.top] = s.data[s.top-n]
	s.top++
	return true
}

// Swap swaps the top value with the Nth value from the top (1-indexed: SWAP1 = n=1).
func (s *Stack) Swap(n int) bool {
	return s.Exchange(0, n)
}

// Exchange swaps two values on the stack.
// n is the first index from top, the second is n+m from top.
func (s *Stack) Exchange(n, m int) bool {
	if m == 0 {
		panic("overlapping exchange")
	}
	nmIndex := n + m
	if nmIndex >= s.top {
		return false
	}
	topIdx := s.top - 1
	s.data[topIdx-n], s.data[topIdx-nmIndex] = s.data[topIdx-nmIndex], s.data[topIdx-n]
	return true
}

// Set sets the value at the given index from the top.
func (s *Stack) Set(noFromTop int, val types.Uint256) bool {
	if s.top <= noFromTop {
		return false
	}
	s.data[s.top-noFromTop-1] = val
	return true
}

// PushSlice pushes a big-endian byte slice onto the stack as a single Uint256 word.
// Returns false if stack overflow.
func (s *Stack) PushSlice(slice []byte) bool {
	if len(slice) == 0 {
		return true
	}
	if s.top+1 > StackLimit {
		return false
	}
	val := types.U256FromBytes(slice)
	s.data[s.top] = val
	s.top++
	return true
}

// String returns a string representation of the stack.
func (s *Stack) String() string {
	var sb strings.Builder
	sb.WriteString("[")
	for i := 0; i < s.top; i++ {
		if i > 0 {
			sb.WriteString(", ")
		}
		fmt.Fprintf(&sb, "%s", s.data[i].Hex())
	}
	sb.WriteString("]")
	return sb.String()
}
