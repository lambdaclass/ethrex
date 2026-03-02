package spec

import "fmt"

// ForkID represents an Ethereum specification/hardfork identifier.
// The ordering is significant: higher values represent later forks.
type ForkID uint8

const (
	Frontier         ForkID = 0
	FrontierThawing ForkID = 1
	Homestead        ForkID = 2
	DaoFork         ForkID = 3
	Tangerine        ForkID = 4
	SpuriousDragon  ForkID = 5
	Byzantium        ForkID = 6
	Constantinople   ForkID = 7
	Petersburg       ForkID = 8
	Istanbul         ForkID = 9
	MuirGlacier     ForkID = 10
	Berlin           ForkID = 11
	London           ForkID = 12
	ArrowGlacier    ForkID = 13
	GrayGlacier     ForkID = 14
	Merge            ForkID = 15
	Shanghai         ForkID = 16
	Cancun           ForkID = 17
	Prague           ForkID = 18
	Osaka            ForkID = 19
	Amsterdam        ForkID = 20
)

// LatestForkID is the default spec (Osaka).
const LatestForkID = Osaka

// IsEnabledIn returns true if fork `other` is enabled (i.e., `other` <= `s`).
func (s ForkID) IsEnabledIn(other ForkID) bool {
	return s >= other
}

// String returns the human-readable name for the spec.
func (s ForkID) String() string {
	switch s {
	case Frontier:
		return "Frontier"
	case FrontierThawing:
		return "Frontier Thawing"
	case Homestead:
		return "Homestead"
	case DaoFork:
		return "DAO Fork"
	case Tangerine:
		return "Tangerine"
	case SpuriousDragon:
		return "Spurious"
	case Byzantium:
		return "Byzantium"
	case Constantinople:
		return "Constantinople"
	case Petersburg:
		return "Petersburg"
	case Istanbul:
		return "Istanbul"
	case MuirGlacier:
		return "MuirGlacier"
	case Berlin:
		return "Berlin"
	case London:
		return "London"
	case ArrowGlacier:
		return "Arrow Glacier"
	case GrayGlacier:
		return "Gray Glacier"
	case Merge:
		return "Merge"
	case Shanghai:
		return "Shanghai"
	case Cancun:
		return "Cancun"
	case Prague:
		return "Prague"
	case Osaka:
		return "Osaka"
	case Amsterdam:
		return "Amsterdam"
	default:
		return fmt.Sprintf("Unknown(%d)", s)
	}
}

// ForkIDFromString parses a spec ID from its string name.
func ForkIDFromString(s string) (ForkID, error) {
	switch s {
	case "Frontier":
		return Frontier, nil
	case "Frontier Thawing":
		return FrontierThawing, nil
	case "Homestead":
		return Homestead, nil
	case "DAO Fork":
		return DaoFork, nil
	case "Tangerine":
		return Tangerine, nil
	case "Spurious":
		return SpuriousDragon, nil
	case "Byzantium":
		return Byzantium, nil
	case "Constantinople":
		return Constantinople, nil
	case "Petersburg":
		return Petersburg, nil
	case "Istanbul":
		return Istanbul, nil
	case "MuirGlacier":
		return MuirGlacier, nil
	case "Berlin":
		return Berlin, nil
	case "London":
		return London, nil
	case "Arrow Glacier":
		return ArrowGlacier, nil
	case "Gray Glacier":
		return GrayGlacier, nil
	case "Merge":
		return Merge, nil
	case "Shanghai":
		return Shanghai, nil
	case "Cancun":
		return Cancun, nil
	case "Prague":
		return Prague, nil
	case "Osaka":
		return Osaka, nil
	case "Amsterdam":
		return Amsterdam, nil
	case "Latest":
		return LatestForkID, nil
	default:
		return 0, fmt.Errorf("unknown hardfork: %s", s)
	}
}

// ForkIDFromByte converts a u8 to a ForkID, returning an error if out of range.
func ForkIDFromByte(v uint8) (ForkID, error) {
	if v > uint8(Amsterdam) {
		return 0, fmt.Errorf("invalid spec id: %d", v)
	}
	return ForkID(v), nil
}
