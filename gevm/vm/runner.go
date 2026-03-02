// Runner abstracts the interpreter opcode loop, enabling pluggable
// execution modes (fast path vs tracing) without runtime branching.
package vm

import "github.com/Giulio2002/gevm/spec"

// Runner executes the interpreter opcode loop.
// Implementations are code-generated in table_gen.go.
type Runner interface {
	Run(interp *Interpreter, host Host)
}

// DefaultRunner is the fast-path runner with gas accumulator and zero tracing overhead.
// Its Run method is generated in table_gen.go.
type DefaultRunner struct{}

// TracingRunner executes with per-opcode gas deduction and tracing hooks.
// Its Run method is generated in table_gen.go.
// If Hooks.OnOpcode is nil, Run delegates to DefaultRunner for the fast path.
type TracingRunner struct {
	Hooks         *Hooks
	DebugGasTable *[256]uint64
}

// NewTracingRunner creates a TracingRunner for the given hooks and fork.
func NewTracingRunner(hooks *Hooks, forkID spec.ForkID) *TracingRunner {
	return &TracingRunner{
		Hooks:         hooks,
		DebugGasTable: DebugGasTableForFork(forkID),
	}
}
