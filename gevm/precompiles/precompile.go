// Provides precompile infrastructure: types, registry, and fork-aware set management.
package precompiles

import (
	"github.com/Giulio2002/gevm/types"
	"github.com/Giulio2002/gevm/spec"
)

// PrecompileOutput holds the result of a successful precompile execution.
type PrecompileOutput struct {
	GasUsed   uint64
	GasRefund int64
	Bytes     types.Bytes
	Reverted  bool
}

// NewPrecompileOutput creates a successful output.
func NewPrecompileOutput(gasUsed uint64, output types.Bytes) PrecompileOutput {
	return PrecompileOutput{GasUsed: gasUsed, Bytes: output}
}

// PrecompileError represents precompile execution errors.
type PrecompileError int

const (
	PrecompileErrorOutOfGas PrecompileError = iota
	PrecompileErrorBlake2WrongLength
	PrecompileErrorBlake2WrongFinalIndicatorFlag
	PrecompileErrorModexpExpOverflow
	PrecompileErrorModexpBaseOverflow
	PrecompileErrorModexpModOverflow
	PrecompileErrorBn254FieldPointNotAMember
	PrecompileErrorBn254AffineGFailedToCreate
	PrecompileErrorBn254PairLength
	PrecompileErrorFatal
)

// PrecompileResult is the result of executing a precompile.
type PrecompileResult struct {
	Output PrecompileOutput
	Err    *PrecompileError
}

// PrecompileOk creates a successful result.
func PrecompileOk(output PrecompileOutput) PrecompileResult {
	return PrecompileResult{Output: output}
}

// PrecompileErr creates an error result.
func PrecompileErr(err PrecompileError) PrecompileResult {
	return PrecompileResult{Err: &err}
}

// IsOk returns true if the precompile succeeded.
func (r *PrecompileResult) IsOk() bool { return r.Err == nil }

// IsErr returns true if the precompile failed.
func (r *PrecompileResult) IsErr() bool { return r.Err != nil }

// PrecompileFn is the function signature for all precompile implementations.
type PrecompileFn func(input []byte, gasLimit uint64) PrecompileResult

// Precompile represents a single precompile contract.
type Precompile struct {
	Address types.Address
	Fn      PrecompileFn
}

// Execute runs the precompile with the given input and gas limit.
func (p *Precompile) Execute(input []byte, gasLimit uint64) PrecompileResult {
	return p.Fn(input, gasLimit)
}

// maxShortAddress is the maximum precompile address stored in the optimized array.
// Addresses 0x01 through 0x11 (17 decimal) are stored in a flat array for O(1) lookup.
const maxShortAddress = 0x11

// PrecompileSet holds a fork-specific set of precompile contracts.
type PrecompileSet struct {
	// Short addresses (0x01-0x11) for O(1) lookup.
	short [maxShortAddress + 1]*Precompile
	// Extended addresses for longer precompile addresses (e.g., 0x0100 for P256VERIFY).
	extended map[types.Address]*Precompile
	// Cached warm addresses, computed once via cacheWarmAddresses().
	warmAddrs   []types.Address
	warmAddrMap map[types.Address]struct{} // shared immutable map for Journal warm tracking
}

// NewPrecompileSet creates an empty PrecompileSet.
func NewPrecompileSet() *PrecompileSet {
	return &PrecompileSet{
		extended: make(map[types.Address]*Precompile),
	}
}

// Add registers a precompile in the set.
func (ps *PrecompileSet) Add(p *Precompile) {
	idx := shortIndex(p.Address)
	if idx >= 0 && idx <= maxShortAddress {
		ps.short[idx] = p
	} else {
		ps.extended[p.Address] = p
	}
}

// Get returns the precompile at the given address, or nil if not found.
func (ps *PrecompileSet) Get(addr types.Address) *Precompile {
	idx := shortIndex(addr)
	if idx >= 0 && idx <= maxShortAddress {
		return ps.short[idx]
	}
	return ps.extended[addr]
}

// Contains returns true if the address is a precompile.
func (ps *PrecompileSet) Contains(addr types.Address) bool {
	return ps.Get(addr) != nil
}

// WarmAddresses returns all precompile addresses for EIP-2929 warm list injection.
// For cached sets (from ForSpec), returns a pre-computed slice (zero allocation).
func (ps *PrecompileSet) WarmAddresses() []types.Address {
	if ps.warmAddrs != nil {
		return ps.warmAddrs
	}
	return ps.buildWarmAddresses()
}

// buildWarmAddresses constructs the warm addresses slice from scratch.
func (ps *PrecompileSet) buildWarmAddresses() []types.Address {
	var addrs []types.Address
	for i := 1; i <= maxShortAddress; i++ {
		if ps.short[i] != nil {
			addrs = append(addrs, ps.short[i].Address)
		}
	}
	for _, p := range ps.extended {
		addrs = append(addrs, p.Address)
	}
	return addrs
}

// WarmAddressMap returns the pre-computed warm address map (shared, immutable).
// Used by Journal to set precompile warm addresses without creating Account objects.
func (ps *PrecompileSet) WarmAddressMap() map[types.Address]struct{} {
	return ps.warmAddrMap
}

// cacheWarmAddresses pre-computes and stores the warm address list and map.
func (ps *PrecompileSet) cacheWarmAddresses() {
	ps.warmAddrs = ps.buildWarmAddresses()
	ps.warmAddrMap = make(map[types.Address]struct{}, len(ps.warmAddrs))
	for _, addr := range ps.warmAddrs {
		ps.warmAddrMap[addr] = struct{}{}
	}
}

// shortIndex returns the single-byte index of a short address, or -1 if not short.
func shortIndex(addr types.Address) int {
	// Short precompile addresses are 0x00..01 through 0x00..11 (last byte only, rest zero).
	for i := 0; i < 19; i++ {
		if addr[i] != 0 {
			return -1
		}
	}
	return int(addr[19])
}

// precompileAddr creates an address from a single byte (for 0x01-0x11).
func precompileAddr(b byte) types.Address {
	var addr types.Address
	addr[19] = b
	return addr
}

// CalcLinearCost computes gas for precompiles with linear cost model.
// gas = ceil(len / 32) * wordCost + baseCost
func CalcLinearCost(dataLen int, baseCost, wordCost uint64) uint64 {
	words := (uint64(dataLen) + 31) / 32
	return words*wordCost + baseCost
}

// RightPad pads input to exactly size bytes with zeros on the right.
// If input is already >= size, returns input[:size].
func RightPad(input []byte, size int) []byte {
	if len(input) >= size {
		return input[:size]
	}
	padded := make([]byte, size)
	copy(padded, input)
	return padded
}

// --- Fork-specific precompile sets ---

// Cached precompile sets built once at init time.
// ForSpec returns these cached pointers (zero allocation per call).
var (
	cachedHomestead *PrecompileSet
	cachedByzantium *PrecompileSet
	cachedIstanbul  *PrecompileSet
	cachedBerlin    *PrecompileSet
	cachedCancun    *PrecompileSet
	cachedPrague    *PrecompileSet
	cachedOsaka     *PrecompileSet
)

func init() {
	cachedHomestead = Homestead()
	cachedByzantium = Byzantium()
	cachedIstanbul = Istanbul()
	cachedBerlin = Berlin()
	cachedCancun = Cancun()
	cachedPrague = Prague()
	cachedOsaka = Osaka()

	// Pre-compute warm addresses for each cached set.
	for _, ps := range []*PrecompileSet{
		cachedHomestead, cachedByzantium, cachedIstanbul,
		cachedBerlin, cachedCancun, cachedPrague, cachedOsaka,
	} {
		ps.cacheWarmAddresses()
	}
}

// ForSpec returns the cached precompile set for a given spec (zero allocation).
func ForSpec(forkID spec.ForkID) *PrecompileSet {
	switch {
	case forkID.IsEnabledIn(spec.Osaka):
		return cachedOsaka
	case forkID.IsEnabledIn(spec.Prague):
		return cachedPrague
	case forkID.IsEnabledIn(spec.Cancun):
		return cachedCancun
	case forkID.IsEnabledIn(spec.Berlin):
		return cachedBerlin
	case forkID.IsEnabledIn(spec.Istanbul):
		return cachedIstanbul
	case forkID.IsEnabledIn(spec.Byzantium):
		return cachedByzantium
	default:
		return cachedHomestead
	}
}

// Homestead returns precompiles available from Homestead (0x01-0x04).
func Homestead() *PrecompileSet {
	ps := NewPrecompileSet()
	ps.Add(&Precompile{Address: precompileAddr(0x01), Fn: EcRecoverRun})
	ps.Add(&Precompile{Address: precompileAddr(0x02), Fn: Sha256Run})
	ps.Add(&Precompile{Address: precompileAddr(0x03), Fn: Ripemd160Run})
	ps.Add(&Precompile{Address: precompileAddr(0x04), Fn: IdentityRun})
	return ps
}

// Byzantium returns precompiles available from Byzantium (0x01-0x08).
func Byzantium() *PrecompileSet {
	ps := Homestead()
	ps.Add(&Precompile{Address: precompileAddr(0x05), Fn: ModExpByzantiumRun})
	ps.Add(&Precompile{Address: precompileAddr(0x06), Fn: Bn254AddByzantiumRun})
	ps.Add(&Precompile{Address: precompileAddr(0x07), Fn: Bn254MulByzantiumRun})
	ps.Add(&Precompile{Address: precompileAddr(0x08), Fn: Bn254PairingByzantiumRun})
	return ps
}

// Istanbul returns precompiles available from Istanbul (0x01-0x09).
func Istanbul() *PrecompileSet {
	ps := Homestead()
	ps.Add(&Precompile{Address: precompileAddr(0x05), Fn: ModExpByzantiumRun})
	ps.Add(&Precompile{Address: precompileAddr(0x06), Fn: Bn254AddIstanbulRun})
	ps.Add(&Precompile{Address: precompileAddr(0x07), Fn: Bn254MulIstanbulRun})
	ps.Add(&Precompile{Address: precompileAddr(0x08), Fn: Bn254PairingIstanbulRun})
	ps.Add(&Precompile{Address: precompileAddr(0x09), Fn: Blake2FRun})
	return ps
}

// Berlin returns precompiles available from Berlin (0x01-0x09, with repriced modexp).
func Berlin() *PrecompileSet {
	ps := Istanbul()
	ps.Add(&Precompile{Address: precompileAddr(0x05), Fn: ModExpBerlinRun})
	return ps
}

// Cancun returns precompiles available from Cancun (0x01-0x0A).
func Cancun() *PrecompileSet {
	ps := Berlin()
	ps.Add(&Precompile{Address: precompileAddr(0x0A), Fn: KzgPointEvaluationRun})
	return ps
}

// Prague returns precompiles available from Prague (0x01-0x11).
func Prague() *PrecompileSet {
	ps := Cancun()
	ps.Add(&Precompile{Address: precompileAddr(0x0B), Fn: Bls12G1AddRun})
	ps.Add(&Precompile{Address: precompileAddr(0x0C), Fn: Bls12G1MsmRun})
	ps.Add(&Precompile{Address: precompileAddr(0x0D), Fn: Bls12G2AddRun})
	ps.Add(&Precompile{Address: precompileAddr(0x0E), Fn: Bls12G2MsmRun})
	ps.Add(&Precompile{Address: precompileAddr(0x0F), Fn: Bls12PairingRun})
	ps.Add(&Precompile{Address: precompileAddr(0x10), Fn: Bls12MapFpToG1Run})
	ps.Add(&Precompile{Address: precompileAddr(0x11), Fn: Bls12MapFp2ToG2Run})
	return ps
}

// Osaka returns precompiles available from Osaka (Prague + repriced ModExp + P256VERIFY).
func Osaka() *PrecompileSet {
	ps := Prague()
	ps.Add(&Precompile{Address: precompileAddr(0x05), Fn: ModExpOsakaRun})
	ps.Add(&Precompile{Address: precompileAddr16(0x0100), Fn: P256VerifyOsakaRun})
	return ps
}

// precompileAddr16 creates an address from a 16-bit value (for addresses like 0x0100).
func precompileAddr16(v uint16) types.Address {
	var addr types.Address
	addr[18] = byte(v >> 8)
	addr[19] = byte(v)
	return addr
}
