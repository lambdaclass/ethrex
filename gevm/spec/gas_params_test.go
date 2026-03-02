package spec

import (
	"testing"

	"github.com/Giulio2002/gevm/types"
)

func TestNewGasParamsFrontier(t *testing.T) {
	g := NewGasParams(Frontier)
	if g.Get(GasIdExpByteGas) != 10 {
		t.Errorf("frontier exp_byte_gas: got %d, want 10", g.Get(GasIdExpByteGas))
	}
	if g.Get(GasIdSstoreStatic) != 5000 {
		t.Errorf("frontier sstore_static: got %d, want 5000", g.Get(GasIdSstoreStatic))
	}
	if g.Get(GasIdSelfdestructRefund) != 24000 {
		t.Errorf("frontier selfdestruct_refund: got %d, want 24000", g.Get(GasIdSelfdestructRefund))
	}
	if g.Get(GasIdTxCreateCost) != 0 {
		t.Errorf("frontier tx_create_cost: got %d, want 0", g.Get(GasIdTxCreateCost))
	}
	if g.Get(GasIdNewAccountCostForSelfdestruct) != 0 {
		t.Errorf("frontier selfdestruct_new_account: got %d, want 0", g.Get(GasIdNewAccountCostForSelfdestruct))
	}
}

func TestNewGasParamsHomestead(t *testing.T) {
	g := NewGasParams(Homestead)
	if g.Get(GasIdTxCreateCost) != 32000 {
		t.Errorf("homestead tx_create_cost: got %d, want 32000", g.Get(GasIdTxCreateCost))
	}
}

func TestNewGasParamsTangerine(t *testing.T) {
	g := NewGasParams(Tangerine)
	if g.Get(GasIdNewAccountCostForSelfdestruct) != 25000 {
		t.Errorf("tangerine selfdestruct_new_account: got %d, want 25000", g.Get(GasIdNewAccountCostForSelfdestruct))
	}
}

func TestNewGasParamsSpuriousDragon(t *testing.T) {
	g := NewGasParams(SpuriousDragon)
	if g.Get(GasIdExpByteGas) != 50 {
		t.Errorf("spurious exp_byte_gas: got %d, want 50", g.Get(GasIdExpByteGas))
	}
}

func TestNewGasParamsIstanbul(t *testing.T) {
	g := NewGasParams(Istanbul)
	if g.Get(GasIdSstoreStatic) != 800 {
		t.Errorf("istanbul sstore_static: got %d, want 800", g.Get(GasIdSstoreStatic))
	}
	if g.Get(GasIdTxTokenNonZeroByteMultiplier) != 4 {
		t.Errorf("istanbul tx_non_zero_multiplier: got %d, want 4", g.Get(GasIdTxTokenNonZeroByteMultiplier))
	}
}

func TestNewGasParamsBerlin(t *testing.T) {
	g := NewGasParams(Berlin)
	if g.Get(GasIdSstoreStatic) != 100 {
		t.Errorf("berlin sstore_static: got %d, want 100", g.Get(GasIdSstoreStatic))
	}
	if g.Get(GasIdColdStorageCost) != 2100 {
		t.Errorf("berlin cold_storage_cost: got %d, want 2100", g.Get(GasIdColdStorageCost))
	}
	if g.Get(GasIdWarmStorageReadCost) != 100 {
		t.Errorf("berlin warm_storage_read: got %d, want 100", g.Get(GasIdWarmStorageReadCost))
	}
	if g.Get(GasIdTxAccessListAddressCost) != 2400 {
		t.Errorf("berlin access_list_address: got %d, want 2400", g.Get(GasIdTxAccessListAddressCost))
	}
}

func TestNewGasParamsLondon(t *testing.T) {
	g := NewGasParams(London)
	if g.Get(GasIdSelfdestructRefund) != 0 {
		t.Errorf("london selfdestruct_refund: got %d, want 0", g.Get(GasIdSelfdestructRefund))
	}
	// 2900 + 1900 = 4800
	if g.Get(GasIdSstoreClearingSlotRefund) != 4800 {
		t.Errorf("london clearing_slot_refund: got %d, want 4800", g.Get(GasIdSstoreClearingSlotRefund))
	}
}

func TestNewGasParamsPrague(t *testing.T) {
	g := NewGasParams(Prague)
	if g.Get(GasIdTxEip7702PerEmptyAccountCost) != 25000 {
		t.Errorf("prague eip7702_empty_account: got %d, want 25000", g.Get(GasIdTxEip7702PerEmptyAccountCost))
	}
	if g.Get(GasIdTxEip7702AuthRefund) != 12500 {
		t.Errorf("prague eip7702_auth_refund: got %d, want 12500", g.Get(GasIdTxEip7702AuthRefund))
	}
	if g.Get(GasIdTxFloorCostPerToken) != 10 {
		t.Errorf("prague floor_cost_per_token: got %d, want 10", g.Get(GasIdTxFloorCostPerToken))
	}
}

func TestExpCost(t *testing.T) {
	g := NewGasParams(SpuriousDragon)

	// power = 0 → cost = 0
	if g.ExpCost(types.U256Zero) != 0 {
		t.Errorf("exp_cost(0): got %d, want 0", g.ExpCost(types.U256Zero))
	}

	// power = 1 → log2floor = 0, cost = 50 * (0/8 + 1) = 50
	if g.ExpCost(types.U256One) != 50 {
		t.Errorf("exp_cost(1): got %d, want 50", g.ExpCost(types.U256One))
	}

	// power = 255 → log2floor = 7, cost = 50 * (7/8 + 1) = 50
	if g.ExpCost(types.U256From(255)) != 50 {
		t.Errorf("exp_cost(255): got %d, want 50", g.ExpCost(types.U256From(255)))
	}

	// power = 256 → log2floor = 8, cost = 50 * (8/8 + 1) = 100
	if g.ExpCost(types.U256From(256)) != 100 {
		t.Errorf("exp_cost(256): got %d, want 100", g.ExpCost(types.U256From(256)))
	}
}

func TestMemoryCost(t *testing.T) {
	g := NewGasParams(Frontier)

	// 0 words → 0
	if g.MemoryCost(0) != 0 {
		t.Errorf("memory_cost(0): got %d, want 0", g.MemoryCost(0))
	}

	// 1 word → 3*1 + 1*1/512 = 3
	if g.MemoryCost(1) != 3 {
		t.Errorf("memory_cost(1): got %d, want 3", g.MemoryCost(1))
	}

	// 32 words → 3*32 + 32*32/512 = 96 + 2 = 98
	if g.MemoryCost(32) != 98 {
		t.Errorf("memory_cost(32): got %d, want 98", g.MemoryCost(32))
	}
}

func TestInitialTxGas(t *testing.T) {
	// Frontier: 21000 base, 68 per nonzero byte, 4 per zero byte
	g := NewGasParams(Frontier)
	result := g.InitialTxGas([]byte{0x00, 0xff}, false, 0, 0, 0)
	// tokens = 1 (zero) + 1*17 (nonzero) = 18; cost = 18*4 + 21000 = 21072
	expected := uint64(18*4 + 21000)
	if result.InitialGas != expected {
		t.Errorf("frontier initial_tx_gas: got %d, want %d", result.InitialGas, expected)
	}

	// Berlin with access list
	g = NewGasParams(Berlin)
	result = g.InitialTxGas(nil, false, 2, 5, 0)
	// 21000 + 2*2400 + 5*1900 = 21000 + 4800 + 9500 = 35300
	if result.InitialGas != 35300 {
		t.Errorf("berlin access list: got %d, want 35300", result.InitialGas)
	}

	// Pre-Berlin: access list costs should be zero
	g = NewGasParams(Istanbul)
	result = g.InitialTxGas(nil, false, 10, 20, 0)
	if result.InitialGas != 21000 {
		t.Errorf("istanbul no access list: got %d, want 21000", result.InitialGas)
	}
}

func TestLog2floor(t *testing.T) {
	cases := []struct {
		val  types.Uint256
		want uint64
	}{
		{types.U256From(1), 0},
		{types.U256From(2), 1},
		{types.U256From(255), 7},
		{types.U256From(256), 8},
		{types.U256Max, 255},
	}
	for _, tc := range cases {
		got := log2floor(tc.val)
		if got != tc.want {
			t.Errorf("log2floor(%s): got %d, want %d", tc.val.Hex(), got, tc.want)
		}
	}
}

func TestNumWords(t *testing.T) {
	if NumWords(0) != 0 {
		t.Errorf("num_words(0): got %d", NumWords(0))
	}
	if NumWords(1) != 1 {
		t.Errorf("num_words(1): got %d", NumWords(1))
	}
	if NumWords(32) != 1 {
		t.Errorf("num_words(32): got %d", NumWords(32))
	}
	if NumWords(33) != 2 {
		t.Errorf("num_words(33): got %d", NumWords(33))
	}
}
