package vm

// decodeSingle decodes a single immediate byte for DUPN/SWAPN.
func decodeSingle(x int) (int, bool) {
	if x <= 90 {
		return x + 17, true
	} else if x >= 128 {
		return x - 20, true
	}
	return 0, false
}

// decodePair decodes a pair of indices from a single immediate byte for EXCHANGE.
func decodePair(x int) (int, int, bool) {
	var k int
	if x <= 79 {
		k = x
	} else if x >= 128 {
		k = x - 48
	} else {
		return 0, 0, false
	}
	q := k / 16
	r := k % 16
	if q < r {
		return q + 1, r + 1, true
	}
	return r + 1, 29 - q, true
}
