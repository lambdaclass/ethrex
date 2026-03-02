package types

import "testing"

func TestCreateAddressNonce0(t *testing.T) {
	// Known test vector
	sender, _ := HexToAddress("6ac7ea33f8831ea9dcc53393aaa88b25a785dbf0")
	addr := CreateAddress(sender, 0)
	expected, _ := HexToAddress("cd234a471b72ba2f1ccf0a70fcaba648a5eecd8d")
	if addr != expected {
		t.Fatalf("CREATE nonce=0: got %s, want %s", addr.Hex(), expected.Hex())
	}
}

func TestCreateAddressNonce1(t *testing.T) {
	sender, _ := HexToAddress("6ac7ea33f8831ea9dcc53393aaa88b25a785dbf0")
	addr := CreateAddress(sender, 1)
	expected, _ := HexToAddress("343c43a37d37dff08ae8c4a11544c718abb4fcf8")
	if addr != expected {
		t.Fatalf("CREATE nonce=1: got %s, want %s", addr.Hex(), expected.Hex())
	}
}

func TestCreate2Address(t *testing.T) {
	// EIP-1014 test vector: sender=0x00..00, salt=0x00..00, initCodeHash=keccak256(0x00)
	var zeroAddr Address
	var zeroSalt [32]byte
	codeHash := Keccak256([]byte{0x00})
	addr := Create2Address(zeroAddr, zeroSalt, codeHash)
	expected, _ := HexToAddress("4D1A2e2bB4F88F0250f26Ffff098B0b30B26BF38")
	if addr != expected {
		t.Fatalf("CREATE2 zero: got %s, want %s", addr.Hex(), expected.Hex())
	}
}

func TestCreate2AddressWithSalt(t *testing.T) {
	// EIP-1014 test vector #4: sender=0xdeadbeef..., salt=0x00...cafebabe, code=0xdeadbeef
	sender, _ := HexToAddress("00000000000000000000000000000000deadbeef")
	var salt [32]byte
	salt[28] = 0xca
	salt[29] = 0xfe
	salt[30] = 0xba
	salt[31] = 0xbe
	codeHash := Keccak256([]byte{0xde, 0xad, 0xbe, 0xef})
	addr := Create2Address(sender, salt, codeHash)
	expected, _ := HexToAddress("60f3f640a8508fC6a86d45DF051962668E1e8AC7")
	if addr != expected {
		t.Fatalf("CREATE2 deadbeef: got %s, want %s", addr.Hex(), expected.Hex())
	}
}

func TestRlpEncodeUint64(t *testing.T) {
	tests := []struct {
		v    uint64
		want []byte
	}{
		{0, []byte{0x80}},
		{1, []byte{0x01}},
		{0x7F, []byte{0x7F}},
		{0x80, []byte{0x81, 0x80}},
		{0xFF, []byte{0x81, 0xFF}},
		{0x100, []byte{0x82, 0x01, 0x00}},
	}
	for _, tt := range tests {
		got := rlpEncodeUint64(tt.v)
		if len(got) != len(tt.want) {
			t.Errorf("rlpEncodeUint64(%d): len=%d, want %d", tt.v, len(got), len(tt.want))
			continue
		}
		for i := range got {
			if got[i] != tt.want[i] {
				t.Errorf("rlpEncodeUint64(%d): byte %d: got %x, want %x", tt.v, i, got[i], tt.want[i])
			}
		}
	}
}
