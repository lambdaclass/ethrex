package types

import (
	"testing"
)

func TestAddressZero(t *testing.T) {
	var a Address
	if !a.IsZero() {
		t.Error("zero address should be zero")
	}
	if a.Hex() != "0x0000000000000000000000000000000000000000" {
		t.Errorf("zero address hex: got %s", a.Hex())
	}
}

func TestAddressFrom(t *testing.T) {
	b := make([]byte, 20)
	b[19] = 0x42
	addr := AddressFrom(b)
	if addr[19] != 0x42 {
		t.Errorf("AddressFrom[19]: got %x, want 42", addr[19])
	}

	b2 := []byte{0xab, 0xcd}
	addr2 := AddressFrom(b2)
	if addr2[18] != 0xab || addr2[19] != 0xcd {
		t.Errorf("AddressFrom short: got %s", addr2.Hex())
	}
}

func TestHexToAddress(t *testing.T) {
	addr, err := HexToAddress("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef")
	if err != nil {
		t.Fatal(err)
	}
	expected := "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
	if addr.Hex() != expected {
		t.Errorf("got %s, want %s", addr.Hex(), expected)
	}
}

func TestB256Zero(t *testing.T) {
	var b B256
	if !b.IsZero() {
		t.Error("zero B256 should be zero")
	}
}

func TestB256FromSlice(t *testing.T) {
	data := make([]byte, 32)
	data[0] = 0xab
	data[31] = 0xcd
	h := B256From(data)
	if h[0] != 0xab || h[31] != 0xcd {
		t.Errorf("B256From: got %s", h.Hex())
	}
}

func TestB256U256Roundtrip(t *testing.T) {
	val := Uint256{0xdeadbeef, 0xcafebabe, 0x12345678, 0x9abcdef0}
	b := B256FromU256(val)
	got := b.ToU256()
	if !got.Eq(val) {
		t.Errorf("B256-Uint256 roundtrip failed: got %s, want %s", got.Hex(), val.Hex())
	}
}

func TestKeccakEmpty(t *testing.T) {
	expected := "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
	if KeccakEmpty.Hex() != expected {
		t.Errorf("KeccakEmpty: got %s, want %s", KeccakEmpty.Hex(), expected)
	}
}

func TestBytes(t *testing.T) {
	b := BytesFrom([]byte{0x01, 0x02, 0x03})
	if b.Len() != 3 {
		t.Errorf("Bytes len: got %d, want 3", b.Len())
	}
	if b.IsEmpty() {
		t.Error("Bytes should not be empty")
	}
	if b.Hex() != "0x010203" {
		t.Errorf("Bytes hex: got %s", b.Hex())
	}

	empty := BytesFrom(nil)
	if !empty.IsEmpty() {
		t.Error("nil Bytes should be empty")
	}
}
