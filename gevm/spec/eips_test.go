package spec

import "testing"

// TestFakeExponential verifies the blob gas price calculation.
// Test vectors for blob gas price calculation.
func TestFakeExponential(t *testing.T) {
	tests := []struct {
		factor, numerator, denominator uint64
		expected                       uint64
	}{
		{1, 0, 1, 1},
		{38493, 0, 1000, 38493},
		{0, 1234, 2345, 0},
		{1, 2, 1, 6},
		{1, 4, 2, 6},
		{1, 3, 1, 16},
		{1, 6, 2, 18},
		{1, 4, 1, 49},
		{1, 8, 2, 50},
		{10, 8, 2, 542},
		{11, 8, 2, 596},
		{1, 5, 1, 136},
		{1, 5, 2, 11},
		{2, 5, 2, 23},
		{1, 50000000, 2225652, 5709098764},
		{1, 380928, BlobBaseFeeUpdateFractionCancun, 1},
	}

	for _, tt := range tests {
		got := FakeExponential(tt.factor, tt.numerator, tt.denominator)
		if got != tt.expected {
			t.Errorf("FakeExponential(%d, %d, %d) = %d, want %d",
				tt.factor, tt.numerator, tt.denominator, got, tt.expected)
		}
	}
}

func TestCalcBlobGasPrice(t *testing.T) {
	// Cancun: excess_blob_gas=0x240000, price=2
	price := CalcBlobGasPrice(0x240000, Cancun)
	if price != 2 {
		t.Errorf("CalcBlobGasPrice(0x240000, Cancun) = %d, want 2", price)
	}

	// Prague: excess_blob_gas=0x240000, price=1
	price = CalcBlobGasPrice(0x240000, Prague)
	if price != 1 {
		t.Errorf("CalcBlobGasPrice(0x240000, Prague) = %d, want 1", price)
	}
}
