//go:build arm64

package types

//go:noescape
func u256Add(result, a, b *Uint256)

//go:noescape
func u256Sub(result, a, b *Uint256)

//go:noescape
func u256And(result, a, b *Uint256)

//go:noescape
func u256Or(result, a, b *Uint256)

//go:noescape
func u256Xor(result, a, b *Uint256)

//go:noescape
func u256Not(result, a *Uint256)

//go:noescape
func u256Eq(a, b *Uint256) bool

//go:noescape
func u256Lt(a, b *Uint256) bool

//go:noescape
func u256Gt(a, b *Uint256) bool

//go:noescape
func u256IsZero(a *Uint256) bool

// Exported pointer-based wrappers. These should inline into callers.

func AddTo(result, a, b *Uint256)  { u256Add(result, a, b) }
func SubTo(result, a, b *Uint256)  { u256Sub(result, a, b) }
func AndTo(result, a, b *Uint256)  { u256And(result, a, b) }
func OrTo(result, a, b *Uint256)   { u256Or(result, a, b) }
func XorTo(result, a, b *Uint256)  { u256Xor(result, a, b) }
func NotTo(result, a *Uint256)     { u256Not(result, a) }
func EqPtr(a, b *Uint256) bool     { return u256Eq(a, b) }
func LtPtr(a, b *Uint256) bool     { return u256Lt(a, b) }
func GtPtr(a, b *Uint256) bool     { return u256Gt(a, b) }
func IsZeroPtr(a *Uint256) bool    { return u256IsZero(a) }
