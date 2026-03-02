package vm

import "testing"

func TestGasNew(t *testing.T) {
	g := NewGas(1000)
	if g.Limit() != 1000 {
		t.Errorf("limit: got %d, want 1000", g.Limit())
	}
	if g.Remaining() != 1000 {
		t.Errorf("remaining: got %d, want 1000", g.Remaining())
	}
	if g.Spent() != 0 {
		t.Errorf("spent: got %d, want 0", g.Spent())
	}
	if g.Refunded() != 0 {
		t.Errorf("refunded: got %d, want 0", g.Refunded())
	}
}

func TestGasNewSpent(t *testing.T) {
	g := NewGasSpent(500)
	if g.Limit() != 500 {
		t.Errorf("limit: got %d, want 500", g.Limit())
	}
	if g.Remaining() != 0 {
		t.Errorf("remaining: got %d, want 0", g.Remaining())
	}
	if g.Spent() != 500 {
		t.Errorf("spent: got %d, want 500", g.Spent())
	}
}

func TestGasRecordCost(t *testing.T) {
	g := NewGas(100)

	if !g.RecordCost(30) {
		t.Error("RecordCost(30) should succeed")
	}
	if g.Remaining() != 70 {
		t.Errorf("remaining after 30: got %d, want 70", g.Remaining())
	}
	if g.Spent() != 30 {
		t.Errorf("spent after 30: got %d, want 30", g.Spent())
	}

	if !g.RecordCost(70) {
		t.Error("RecordCost(70) should succeed")
	}
	if g.Remaining() != 0 {
		t.Errorf("remaining after 100: got %d, want 0", g.Remaining())
	}

	if g.RecordCost(1) {
		t.Error("RecordCost(1) should fail on empty gas")
	}
	if g.Remaining() != 0 {
		t.Errorf("remaining should still be 0: got %d", g.Remaining())
	}
}

func TestGasRecordCostUnsafe(t *testing.T) {
	g := NewGas(100)

	if g.RecordCostUnsafe(50) {
		t.Error("RecordCostUnsafe(50) should not indicate OOG")
	}
	if g.Remaining() != 50 {
		t.Errorf("remaining: got %d, want 50", g.Remaining())
	}

	// This should indicate OOG and wrap
	if !g.RecordCostUnsafe(51) {
		t.Error("RecordCostUnsafe(51) should indicate OOG")
	}
}

func TestGasSpendAll(t *testing.T) {
	g := NewGas(100)
	g.SpendAll()
	if g.Remaining() != 0 {
		t.Errorf("remaining after spend_all: got %d", g.Remaining())
	}
	if g.Spent() != 100 {
		t.Errorf("spent after spend_all: got %d, want 100", g.Spent())
	}
}

func TestGasEraseCost(t *testing.T) {
	g := NewGas(100)
	g.RecordCost(60)
	g.EraseCost(20)
	if g.Remaining() != 60 {
		t.Errorf("remaining after erase: got %d, want 60", g.Remaining())
	}
}

func TestGasRefund(t *testing.T) {
	g := NewGas(100)
	g.RecordRefund(500)
	if g.Refunded() != 500 {
		t.Errorf("refunded: got %d, want 500", g.Refunded())
	}
	g.RecordRefund(-200)
	if g.Refunded() != 300 {
		t.Errorf("refunded after negative: got %d, want 300", g.Refunded())
	}
}

func TestGasSetFinalRefund(t *testing.T) {
	g := NewGas(100)
	g.RecordCost(80) // spent = 80
	g.RecordRefund(50)

	// Pre-London: refund capped at spent/2 = 40
	g2 := g
	g2.SetFinalRefund(false)
	if g2.Refunded() != 40 {
		t.Errorf("pre-london final refund: got %d, want 40", g2.Refunded())
	}

	// London: refund capped at spent/5 = 16
	g3 := g
	g3.SetFinalRefund(true)
	if g3.Refunded() != 16 {
		t.Errorf("london final refund: got %d, want 16", g3.Refunded())
	}
}

func TestGasSetSpent(t *testing.T) {
	g := NewGas(100)
	g.SetSpent(60)
	if g.Remaining() != 40 {
		t.Errorf("remaining: got %d, want 40", g.Remaining())
	}
	if g.Spent() != 60 {
		t.Errorf("spent: got %d, want 60", g.Spent())
	}

	// Saturate
	g.SetSpent(200)
	if g.Remaining() != 0 {
		t.Errorf("remaining saturated: got %d, want 0", g.Remaining())
	}
}

func TestMemoryGasRecordNewLen(t *testing.T) {
	mg := NewMemoryGas()

	// First expansion: 1 word (linear=3, quadratic=512)
	cost, expanded := mg.RecordNewLen(1, 3, 512)
	if !expanded {
		t.Error("should expand for 1 word")
	}
	if cost != 3 {
		t.Errorf("cost for 1 word: got %d, want 3", cost)
	}
	if mg.WordsNum != 1 {
		t.Errorf("words_num: got %d, want 1", mg.WordsNum)
	}

	// Same size: no expansion
	_, expanded = mg.RecordNewLen(1, 3, 512)
	if expanded {
		t.Error("should not expand for same size")
	}

	// Expand to 32 words
	cost, expanded = mg.RecordNewLen(32, 3, 512)
	if !expanded {
		t.Error("should expand to 32 words")
	}
	// Total cost for 32 words: 3*32 + 32*32/512 = 96 + 2 = 98
	// Incremental: 98 - 3 = 95
	if cost != 95 {
		t.Errorf("incremental cost to 32 words: got %d, want 95", cost)
	}
}
