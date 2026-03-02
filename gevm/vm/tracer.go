// StructLogger is a default EVM tracer that collects per-opcode execution logs.
package vm

import "github.com/Giulio2002/gevm/types"

// StructLog records a single opcode execution step.
type StructLog struct {
	Pc      uint64               `json:"pc"`
	Op      byte                 `json:"op"`
	Gas     uint64               `json:"gas"`
	GasCost uint64               `json:"gasCost"`
	Depth   int                  `json:"depth"`
	Stack   []types.Uint256 `json:"stack,omitempty"`
	Memory  []byte               `json:"memory,omitempty"`
	Err     error                `json:"-"`
	ErrStr  string               `json:"error,omitempty"`
}

// LogConfig controls what data the StructLogger captures.
type LogConfig struct {
	DisableMemory bool
	DisableStack  bool
}

// StructLogger collects per-opcode execution logs.
type StructLogger struct {
	cfg     LogConfig
	logs    []StructLog
	output  []byte
	err     error
	gasUsed uint64
}

// NewStructLogger creates a StructLogger with the given configuration.
func NewStructLogger(cfg LogConfig) *StructLogger {
	return &StructLogger{cfg: cfg}
}

// Hooks returns a Hooks struct wired to this StructLogger.
func (l *StructLogger) Hooks() *Hooks {
	return &Hooks{
		OnOpcode: l.captureState,
		OnFault:  l.captureFault,
		OnTxEnd:  l.captureTxEnd,
	}
}

// Logs returns the collected execution logs.
func (l *StructLogger) Logs() []StructLog { return l.logs }

// Error returns any error recorded during execution.
func (l *StructLogger) Error() error { return l.err }

// Output returns the transaction output recorded by OnTxEnd.
func (l *StructLogger) Output() []byte { return l.output }

// GasUsed returns the gas used recorded by OnTxEnd.
func (l *StructLogger) GasUsed() uint64 { return l.gasUsed }

// Reset clears all captured state for reuse.
func (l *StructLogger) Reset() {
	l.logs = l.logs[:0]
	l.output = nil
	l.err = nil
	l.gasUsed = 0
}

func (l *StructLogger) captureState(pc uint64, op byte, gas, cost uint64,
	scope OpContext, rData []byte, depth int, err error) {

	entry := StructLog{
		Pc:    pc,
		Op:    op,
		Gas:   gas,
		Depth: depth, // Already 1-indexed from Run() (matches geth convention)
	}

	if !l.cfg.DisableStack {
		src := scope.StackData()
		if len(src) > 0 {
			stack := make([]types.Uint256, len(src))
			copy(stack, src)
			entry.Stack = stack
		}
	}

	if !l.cfg.DisableMemory {
		mem := scope.MemoryData()
		if len(mem) > 0 {
			m := make([]byte, len(mem))
			copy(m, mem)
			entry.Memory = m
		}
	}

	if err != nil {
		entry.Err = err
		entry.ErrStr = err.Error()
	}

	l.logs = append(l.logs, entry)
}

func (l *StructLogger) captureFault(pc uint64, op byte, gas, cost uint64,
	scope OpContext, depth int, err error) {
	if len(l.logs) > 0 {
		last := &l.logs[len(l.logs)-1]
		if err != nil {
			last.Err = err
			last.ErrStr = err.Error()
		}
	}
}

func (l *StructLogger) captureTxEnd(gasUsed uint64, output []byte, err error) {
	l.gasUsed = gasUsed
	l.err = err
	if len(output) > 0 {
		l.output = make([]byte, len(output))
		copy(l.output, output)
	}
}
