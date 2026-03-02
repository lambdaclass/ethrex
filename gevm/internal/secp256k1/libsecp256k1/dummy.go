//go:build dummy

// Workaround for go mod vendor not vendoring C files without a Go file.
// See https://github.com/golang/go/issues/26366
package libsecp256k1
