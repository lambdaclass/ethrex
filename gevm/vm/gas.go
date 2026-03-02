package vm

// MemoryGas tracks current memory expansion cost to enable incremental gas accounting.
type MemoryGas struct {
	// Current memory length in words.
	WordsNum uint64
	// Current cumulative memory expansion cost.
	ExpansionCost uint64
}

// NewMemoryGas creates a new MemoryGas with zero memory allocation.
func NewMemoryGas() MemoryGas {
	return MemoryGas{}
}

// SetWordsNum sets the number of words and the expansion cost.
// Returns the incremental cost (new - old), or ok=false if new < old (should not happen).
func (m *MemoryGas) SetWordsNum(wordsNum uint64, newExpansionCost uint64) (uint64, bool) {
	m.WordsNum = wordsNum
	oldCost := m.ExpansionCost
	m.ExpansionCost = newExpansionCost
	if newExpansionCost < oldCost {
		return 0, false
	}
	return newExpansionCost - oldCost, true
}

// RecordNewLen records a new memory length (in words) and returns the additional
// gas cost if memory needs to be expanded. Returns 0, false if no expansion needed.
func (m *MemoryGas) RecordNewLen(newNum uint64, linearCost uint64, quadraticCost uint64) (uint64, bool) {
	if newNum <= m.WordsNum {
		return 0, false
	}
	m.WordsNum = newNum
	oldCost := m.ExpansionCost
	m.ExpansionCost = memoryGas(newNum, linearCost, quadraticCost)
	// Safe to subtract because new_len > length means new cost >= old cost.
	return m.ExpansionCost - oldCost, true
}

// memoryGas calculates memory expansion cost for a given number of words.
func memoryGas(numWords uint64, linearCost uint64, quadraticCost uint64) uint64 {
	return satMul(linearCost, numWords) + satMul(numWords, numWords)/quadraticCost
}

// satMul returns a * b, saturating at max uint64.
func satMul(a, b uint64) uint64 {
	hi, lo := mulUint64(a, b)
	if hi != 0 {
		return ^uint64(0)
	}
	return lo
}

// mulUint64 returns the full 128-bit product of a * b as (hi, lo).
func mulUint64(a, b uint64) (uint64, uint64) {
	// Use the same approach as math/bits.Mul64
	const mask32 = 1<<32 - 1
	a0, a1 := a&mask32, a>>32
	b0, b1 := b&mask32, b>>32
	w0 := a0 * b0
	t := a1*b0 + w0>>32
	w1 := t & mask32
	w2 := t >> 32
	w1 += a0 * b1
	return a1*b1 + w2 + w1>>32, a*b
}

// Gas represents the state of gas during EVM execution.
type Gas struct {
	// The initial gas limit. Constant throughout execution.
	limit uint64
	// The remaining gas.
	remaining uint64
	// Refunded gas. Used only at end of execution.
	refunded int64
	// Memoisation of values for memory expansion cost.
	memory MemoryGas
}

// NewGas creates a new Gas with the given gas limit.
func NewGas(limit uint64) Gas {
	return Gas{
		limit:     limit,
		remaining: limit,
		refunded:  0,
		memory:    NewMemoryGas(),
	}
}

// NewGasSpent creates a new Gas with the given limit but no remaining gas.
func NewGasSpent(limit uint64) Gas {
	return Gas{
		limit:     limit,
		remaining: 0,
		refunded:  0,
		memory:    NewMemoryGas(),
	}
}

// Limit returns the gas limit.
func (g *Gas) Limit() uint64 {
	return g.limit
}

// Memory returns a pointer to the memory gas tracker.
func (g *Gas) Memory() *MemoryGas {
	return &g.memory
}

// Refunded returns the total refund.
func (g *Gas) Refunded() int64 {
	return g.refunded
}

// Spent returns the total gas spent (limit - remaining).
func (g *Gas) Spent() uint64 {
	return g.limit - g.remaining
}

// Used returns the final gas used (spent - refund), with saturating subtraction.
func (g *Gas) Used() uint64 {
	r := g.refunded
	if r < 0 {
		return g.Spent()
	}
	spent := g.Spent()
	if uint64(r) > spent {
		return 0
	}
	return spent - uint64(r)
}

// SpentSubRefunded returns spent gas minus refunded gas, saturating.
func (g *Gas) SpentSubRefunded() uint64 {
	spent := g.Spent()
	if g.refunded < 0 || uint64(g.refunded) > spent {
		return spent
	}
	return spent - uint64(g.refunded)
}

// Remaining returns the remaining gas.
func (g *Gas) Remaining() uint64 {
	return g.remaining
}

// EraseCost adds back gas that was previously spent.
func (g *Gas) EraseCost(returned uint64) {
	g.remaining += returned
}

// SpendAll sets remaining gas to 0.
func (g *Gas) SpendAll() {
	g.remaining = 0
}

// RecordRefund records a refund value. Refund can be negative.
func (g *Gas) RecordRefund(refund int64) {
	g.refunded += refund
}

// SetFinalRefund limits refund to Nth part of gas spent (EIP-3529).
func (g *Gas) SetFinalRefund(isLondon bool) {
	maxRefundQuotient := uint64(2)
	if isLondon {
		maxRefundQuotient = 5
	}
	r := uint64(g.refunded)
	maxRefund := g.Spent() / maxRefundQuotient
	if r > maxRefund {
		r = maxRefund
	}
	g.refunded = int64(r)
}

// SetRefund overrides the current refund value.
func (g *Gas) SetRefund(refund int64) {
	g.refunded = refund
}

// SetSpent overrides the spent value by adjusting remaining.
func (g *Gas) SetSpent(spent uint64) {
	if spent > g.limit {
		g.remaining = 0
	} else {
		g.remaining = g.limit - spent
	}
}

// RecordCost records an explicit gas cost.
// Returns true if successful, false if gas limit exceeded.
func (g *Gas) RecordCost(cost uint64) bool {
	if g.remaining >= cost {
		g.remaining -= cost
		return true
	}
	return false
}

// RecordCostUnsafe records a cost using wrapping subtraction.
// Returns true if the gas limit was exceeded (out of gas).
func (g *Gas) RecordCostUnsafe(cost uint64) bool {
	oog := g.remaining < cost
	g.remaining -= cost // wrapping subtraction in Go (uint64 underflow wraps)
	return oog
}
