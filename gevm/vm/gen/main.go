// Code generator for table_gen.go — the EVM switch dispatch.
// Reads opXXX function bodies from inst_*.go files and emits Run() methods
// with gas counter accumulation, stack checks, fork gates, and inlined bodies.
//
// The emitter is parameterised by a `tracing` flag. When false it produces
// Run() — the fast path with a gas-counter accumulator and zero tracing
// overhead. When true it produces RunWithTracing() — per-opcode gas
// deduction, OnOpcode/OnFault hooks, and a DebugGasTable lookup for cost.
//
// Usage: go generate ./internal/vm/...
package main

import (
	"bytes"
	"fmt"
	"go/ast"
	"go/format"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"strings"
)

// ---------------------------------------------------------------------------
// Opcode definition types
// ---------------------------------------------------------------------------

type gasMode int

const (
	modeAccumulate gasMode = iota // gas counter accumulated, flushed on error only
	modeFlush                     // gas counter flushed before body
)

type shape int

const (
	shapeBinaryOp  shape = iota // pop 2, push 1: s.top < 2, s.top--
	shapeUnaryOp                // pop 1, push 1 in-place: s.top == 0
	shapeTernaryOp              // pop 3, push 1: s.top < 3, s.top -= 2
	shapePushVal                // push 1: s.top >= StackLimit, body does s.top++
	shapePop1                   // pop 1: s.top == 0, body does s.top--
	shapeCustom                 // body handles everything
)

type opDef struct {
	name      string  // "ADD", "SSTORE"
	code      byte    // 0x01, 0x55
	gas       string  // "spec.GasVerylow", "interp.ForkGas.Balance", "" (no static gas)
	mode      gasMode // Accumulate or Flush
	fork      string  // min fork: "spec.Constantinople", "" = always
	shape     shape
	funcName  string // "opAdd", "logNImpl", ""
	inline    bool   // true → inline function body; false → emit function call
	needsHost bool   // function takes Host parameter
	needsOp   bool   // function takes op byte parameter (opPushN)
}

// ---------------------------------------------------------------------------
// Opcode table — single source of truth for code generation
// ---------------------------------------------------------------------------

var opcodes = []opDef{
	// === Arithmetic (0x00-0x0B) ===
	{name: "STOP", code: 0x00, mode: modeFlush, shape: shapeCustom, funcName: "opStop", inline: true},
	{name: "ADD", code: 0x01, gas: "spec.GasVerylow", mode: modeAccumulate, shape: shapeBinaryOp, funcName: "opAdd", inline: true},
	{name: "MUL", code: 0x02, gas: "spec.GasLow", mode: modeAccumulate, shape: shapeBinaryOp, funcName: "opMul", inline: true},
	{name: "SUB", code: 0x03, gas: "spec.GasVerylow", mode: modeAccumulate, shape: shapeBinaryOp, funcName: "opSub", inline: true},
	{name: "DIV", code: 0x04, gas: "spec.GasLow", mode: modeAccumulate, shape: shapeBinaryOp, funcName: "opDiv", inline: true},
	{name: "SDIV", code: 0x05, gas: "spec.GasLow", mode: modeAccumulate, shape: shapeBinaryOp, funcName: "opSdiv", inline: true},
	{name: "MOD", code: 0x06, gas: "spec.GasLow", mode: modeAccumulate, shape: shapeBinaryOp, funcName: "opMod", inline: true},
	{name: "SMOD", code: 0x07, gas: "spec.GasLow", mode: modeAccumulate, shape: shapeBinaryOp, funcName: "opSmod", inline: true},
	{name: "ADDMOD", code: 0x08, gas: "spec.GasMid", mode: modeAccumulate, shape: shapeTernaryOp, funcName: "opAddmod", inline: true},
	{name: "MULMOD", code: 0x09, gas: "spec.GasMid", mode: modeAccumulate, shape: shapeTernaryOp, funcName: "opMulmod", inline: true},
	{name: "EXP", code: 0x0A, gas: "spec.GasHigh", mode: modeFlush, shape: shapeCustom, funcName: "opExp"},
	{name: "SIGNEXTEND", code: 0x0B, gas: "spec.GasLow", mode: modeAccumulate, shape: shapeBinaryOp, funcName: "opSignextend", inline: true},

	// === Comparison & Bitwise (0x10-0x1E) ===
	{name: "LT", code: 0x10, gas: "spec.GasVerylow", mode: modeAccumulate, shape: shapeBinaryOp, funcName: "opLt", inline: true},
	{name: "GT", code: 0x11, gas: "spec.GasVerylow", mode: modeAccumulate, shape: shapeBinaryOp, funcName: "opGt", inline: true},
	{name: "SLT", code: 0x12, gas: "spec.GasVerylow", mode: modeAccumulate, shape: shapeBinaryOp, funcName: "opSlt", inline: true},
	{name: "SGT", code: 0x13, gas: "spec.GasVerylow", mode: modeAccumulate, shape: shapeBinaryOp, funcName: "opSgt", inline: true},
	{name: "EQ", code: 0x14, gas: "spec.GasVerylow", mode: modeAccumulate, shape: shapeBinaryOp, funcName: "opEq", inline: true},
	{name: "ISZERO", code: 0x15, gas: "spec.GasVerylow", mode: modeAccumulate, shape: shapeUnaryOp, funcName: "opIszero", inline: true},
	{name: "AND", code: 0x16, gas: "spec.GasVerylow", mode: modeAccumulate, shape: shapeBinaryOp, funcName: "opAnd", inline: true},
	{name: "OR", code: 0x17, gas: "spec.GasVerylow", mode: modeAccumulate, shape: shapeBinaryOp, funcName: "opOr", inline: true},
	{name: "XOR", code: 0x18, gas: "spec.GasVerylow", mode: modeAccumulate, shape: shapeBinaryOp, funcName: "opXor", inline: true},
	{name: "NOT", code: 0x19, gas: "spec.GasVerylow", mode: modeAccumulate, shape: shapeUnaryOp, funcName: "opNot", inline: true},
	{name: "BYTE", code: 0x1A, gas: "spec.GasVerylow", mode: modeAccumulate, shape: shapeBinaryOp, funcName: "opByte", inline: true},
	{name: "SHL", code: 0x1B, gas: "spec.GasVerylow", mode: modeAccumulate, fork: "spec.Constantinople", shape: shapeBinaryOp, funcName: "opShl", inline: true},
	{name: "SHR", code: 0x1C, gas: "spec.GasVerylow", mode: modeAccumulate, fork: "spec.Constantinople", shape: shapeBinaryOp, funcName: "opShr", inline: true},
	{name: "SAR", code: 0x1D, gas: "spec.GasVerylow", mode: modeAccumulate, fork: "spec.Constantinople", shape: shapeBinaryOp, funcName: "opSar", inline: true},
	{name: "CLZ", code: 0x1E, gas: "spec.GasLow", mode: modeAccumulate, fork: "spec.Osaka", shape: shapeUnaryOp, funcName: "opClz", inline: true},

	// === Keccak (0x20) ===
	{name: "KECCAK256", code: 0x20, gas: "spec.GasKeccak256", mode: modeFlush, shape: shapeCustom, funcName: "opKeccak256"},

	// === Environment (0x30-0x3F) ===
	{name: "ADDRESS", code: 0x30, gas: "spec.GasBase", mode: modeAccumulate, shape: shapePushVal, funcName: "opAddress", inline: true},
	{name: "BALANCE", code: 0x31, gas: "interp.ForkGas.Balance", mode: modeFlush, shape: shapeCustom, funcName: "opBalance", needsHost: true},
	{name: "ORIGIN", code: 0x32, gas: "spec.GasBase", mode: modeAccumulate, shape: shapePushVal, funcName: "opOrigin", inline: true, needsHost: true},
	{name: "CALLER", code: 0x33, gas: "spec.GasBase", mode: modeAccumulate, shape: shapePushVal, funcName: "opCaller", inline: true},
	{name: "CALLVALUE", code: 0x34, gas: "spec.GasBase", mode: modeAccumulate, shape: shapePushVal, funcName: "opCallvalue", inline: true},
	{name: "CALLDATALOAD", code: 0x35, gas: "spec.GasVerylow", mode: modeAccumulate, shape: shapeUnaryOp, funcName: "opCalldataload", inline: true},
	{name: "CALLDATASIZE", code: 0x36, gas: "spec.GasBase", mode: modeAccumulate, shape: shapePushVal, funcName: "opCalldatasize", inline: true},
	{name: "CALLDATACOPY", code: 0x37, gas: "spec.GasVerylow", mode: modeFlush, shape: shapeCustom, funcName: "opCalldatacopy"},
	{name: "CODESIZE", code: 0x38, gas: "spec.GasBase", mode: modeAccumulate, shape: shapePushVal, funcName: "opCodesize", inline: true},
	{name: "CODECOPY", code: 0x39, gas: "spec.GasVerylow", mode: modeFlush, shape: shapeCustom, funcName: "opCodecopy"},
	{name: "GASPRICE", code: 0x3A, gas: "spec.GasBase", mode: modeAccumulate, shape: shapePushVal, funcName: "opGasprice", inline: true, needsHost: true},
	{name: "EXTCODESIZE", code: 0x3B, gas: "interp.ForkGas.ExtCodeSize", mode: modeFlush, shape: shapeCustom, funcName: "opExtcodesize", needsHost: true},
	{name: "EXTCODECOPY", code: 0x3C, gas: "interp.ForkGas.ExtCodeSize", mode: modeFlush, shape: shapeCustom, funcName: "opExtcodecopy", needsHost: true},
	{name: "RETURNDATASIZE", code: 0x3D, gas: "spec.GasBase", mode: modeAccumulate, fork: "spec.Byzantium", shape: shapePushVal, funcName: "opReturndatasize", inline: true},
	{name: "RETURNDATACOPY", code: 0x3E, gas: "spec.GasVerylow", mode: modeFlush, fork: "spec.Byzantium", shape: shapeCustom, funcName: "opReturndatacopy"},
	{name: "EXTCODEHASH", code: 0x3F, gas: "interp.ForkGas.ExtCodeHash", mode: modeFlush, fork: "spec.Constantinople", shape: shapeCustom, funcName: "opExtcodehash", needsHost: true},

	// === Block info (0x40-0x4B) ===
	{name: "BLOCKHASH", code: 0x40, gas: "spec.GasBlockhash", mode: modeAccumulate, shape: shapeUnaryOp, funcName: "opBlockhash", inline: true, needsHost: true},
	{name: "COINBASE", code: 0x41, gas: "spec.GasBase", mode: modeAccumulate, shape: shapePushVal, funcName: "opCoinbase", inline: true, needsHost: true},
	{name: "TIMESTAMP", code: 0x42, gas: "spec.GasBase", mode: modeAccumulate, shape: shapePushVal, funcName: "opTimestamp", inline: true, needsHost: true},
	{name: "NUMBER", code: 0x43, gas: "spec.GasBase", mode: modeAccumulate, shape: shapePushVal, funcName: "opNumber", inline: true, needsHost: true},
	{name: "DIFFICULTY", code: 0x44, gas: "spec.GasBase", mode: modeFlush, shape: shapeCustom, funcName: "opDifficulty", needsHost: true},
	{name: "GASLIMIT", code: 0x45, gas: "spec.GasBase", mode: modeAccumulate, shape: shapePushVal, funcName: "opGaslimit", inline: true, needsHost: true},
	{name: "CHAINID", code: 0x46, gas: "spec.GasBase", mode: modeAccumulate, fork: "spec.Istanbul", shape: shapePushVal, funcName: "opChainid", needsHost: true},
	{name: "SELFBALANCE", code: 0x47, gas: "spec.GasLow", mode: modeAccumulate, fork: "spec.Istanbul", shape: shapePushVal, funcName: "opSelfbalance", needsHost: true},
	{name: "BASEFEE", code: 0x48, gas: "spec.GasBase", mode: modeAccumulate, fork: "spec.London", shape: shapePushVal, funcName: "opBasefee", needsHost: true},
	{name: "BLOBHASH", code: 0x49, gas: "spec.GasVerylow", mode: modeFlush, fork: "spec.Cancun", shape: shapeCustom, funcName: "opBlobhash", needsHost: true},
	{name: "BLOBBASEFEE", code: 0x4A, gas: "spec.GasBase", mode: modeAccumulate, fork: "spec.Cancun", shape: shapePushVal, funcName: "opBlobbasefee", needsHost: true},
	{name: "SLOTNUM", code: 0x4B, gas: "spec.GasBase", mode: modeAccumulate, fork: "spec.Amsterdam", shape: shapePushVal, funcName: "opSlotnum", needsHost: true},

	// === Stack/Memory/Storage (0x50-0x5E) ===
	{name: "POP", code: 0x50, gas: "spec.GasBase", mode: modeAccumulate, shape: shapePop1, funcName: "opPop", inline: true},
	{name: "MLOAD", code: 0x51, gas: "spec.GasVerylow", mode: modeFlush, shape: shapeCustom, funcName: "opMload", inline: true},
	{name: "MSTORE", code: 0x52, gas: "spec.GasVerylow", mode: modeFlush, shape: shapeCustom, funcName: "opMstore", inline: true},
	{name: "MSTORE8", code: 0x53, gas: "spec.GasVerylow", mode: modeFlush, shape: shapeCustom, funcName: "opMstore8"},
	{name: "SLOAD", code: 0x54, gas: "interp.ForkGas.Sload", mode: modeFlush, shape: shapeCustom, funcName: "opSload", needsHost: true},
	{name: "SSTORE", code: 0x55, mode: modeFlush, shape: shapeCustom, funcName: "opSstore", needsHost: true},
	{name: "JUMP", code: 0x56, gas: "spec.GasMid", mode: modeFlush, shape: shapeCustom, funcName: "opJump", inline: true},
	{name: "JUMPI", code: 0x57, gas: "spec.GasHigh", mode: modeFlush, shape: shapeCustom, funcName: "opJumpi", inline: true},
	{name: "PC", code: 0x58, gas: "spec.GasBase", mode: modeAccumulate, shape: shapePushVal, funcName: "opPc", inline: true},
	{name: "MSIZE", code: 0x59, gas: "spec.GasBase", mode: modeAccumulate, shape: shapePushVal, funcName: "opMsize", inline: true},
	{name: "GAS", code: 0x5A, gas: "spec.GasBase", mode: modeFlush, shape: shapeCustom, funcName: "opGas"},
	{name: "JUMPDEST", code: 0x5B, gas: "spec.GasJumpdest", mode: modeFlush, shape: shapeCustom}, // no body
	{name: "TLOAD", code: 0x5C, gas: "spec.GasWarmStorageReadCost", mode: modeFlush, fork: "spec.Cancun", shape: shapeCustom, funcName: "opTload", needsHost: true},
	{name: "TSTORE", code: 0x5D, gas: "spec.GasWarmStorageReadCost", mode: modeFlush, fork: "spec.Cancun", shape: shapeCustom, funcName: "opTstore", needsHost: true},
	{name: "MCOPY", code: 0x5E, gas: "spec.GasVerylow", mode: modeFlush, fork: "spec.Cancun", shape: shapeCustom, funcName: "opMcopy"},

	// === Push (0x5F-0x7F) — handled specially ===
	// PUSH0, PUSH1-4, PUSH20, PUSH32 are in the table.
	// PUSH5-19, PUSH21-31 are a combined case.

	// === EOF stack (0xE6-0xE8) ===
	{name: "DUPN", code: 0xE6, gas: "spec.GasVerylow", mode: modeFlush, fork: "spec.Amsterdam", shape: shapeCustom, funcName: "opDupN"},
	{name: "SWAPN", code: 0xE7, gas: "spec.GasVerylow", mode: modeFlush, fork: "spec.Amsterdam", shape: shapeCustom, funcName: "opSwapN"},
	{name: "EXCHANGE", code: 0xE8, gas: "spec.GasVerylow", mode: modeFlush, fork: "spec.Amsterdam", shape: shapeCustom, funcName: "opExchange"},

	// === Contract (0xF0-0xFF) ===
	{name: "CREATE", code: 0xF0, mode: modeFlush, shape: shapeCustom, funcName: "opCreate", needsHost: true},
	{name: "CALL", code: 0xF1, gas: "interp.ForkGas.Call", mode: modeFlush, shape: shapeCustom, funcName: "opCall", needsHost: true},
	{name: "CALLCODE", code: 0xF2, gas: "interp.ForkGas.Call", mode: modeFlush, shape: shapeCustom, funcName: "opCallcode", needsHost: true},
	{name: "RETURN", code: 0xF3, mode: modeFlush, shape: shapeCustom, funcName: "opReturn"},
	{name: "DELEGATECALL", code: 0xF4, gas: "interp.ForkGas.Call", mode: modeFlush, fork: "spec.Homestead", shape: shapeCustom, funcName: "opDelegatecall", needsHost: true},
	{name: "CREATE2", code: 0xF5, mode: modeFlush, fork: "spec.Petersburg", shape: shapeCustom, funcName: "opCreate2", needsHost: true},
	{name: "STATICCALL", code: 0xFA, gas: "interp.ForkGas.Call", mode: modeFlush, fork: "spec.Byzantium", shape: shapeCustom, funcName: "opStaticcall", needsHost: true},
	{name: "REVERT", code: 0xFD, mode: modeFlush, fork: "spec.Byzantium", shape: shapeCustom, funcName: "opRevert"},
	{name: "INVALID", code: 0xFE, mode: modeFlush, shape: shapeCustom, funcName: "opInvalid", inline: true},
	{name: "SELFDESTRUCT", code: 0xFF, gas: "interp.ForkGas.Selfdestruct", mode: modeFlush, shape: shapeCustom, funcName: "opSelfdestruct", needsHost: true},
}

// pushOps are handled separately from the main table with specialised inlining.
var pushOps = []opDef{
	{name: "PUSH0", code: 0x5F, gas: "spec.GasBase", fork: "spec.Shanghai", funcName: "opPush0"},
	{name: "PUSH1", code: 0x60, gas: "spec.GasVerylow", funcName: "opPush1"},
	{name: "PUSH2", code: 0x61, gas: "spec.GasVerylow", funcName: "opPush2"},
	{name: "PUSH3", code: 0x62, gas: "spec.GasVerylow", funcName: "opPush3"},
	{name: "PUSH4", code: 0x63, gas: "spec.GasVerylow", funcName: "opPush4"},
	{name: "PUSH20", code: 0x73, gas: "spec.GasVerylow", funcName: "opPush20"},
	{name: "PUSH32", code: 0x7F, gas: "spec.GasVerylow", funcName: "opPush32"},
}

// ---------------------------------------------------------------------------
// Function body extraction
// ---------------------------------------------------------------------------

// parseFuncBodies parses all inst_*.go files in dir and returns a map
// from function name to the raw source text of its body (between { and }).
func parseFuncBodies(dir string) map[string]string {
	bodies := make(map[string]string)
	matches, _ := filepath.Glob(filepath.Join(dir, "inst_*.go"))
	fset := token.NewFileSet()
	for _, path := range matches {
		src, err := os.ReadFile(path)
		if err != nil {
			fmt.Fprintf(os.Stderr, "warning: cannot read %s: %v\n", path, err)
			continue
		}
		f, err := parser.ParseFile(fset, path, src, 0)
		if err != nil {
			fmt.Fprintf(os.Stderr, "warning: cannot parse %s: %v\n", path, err)
			continue
		}
		for _, decl := range f.Decls {
			fn, ok := decl.(*ast.FuncDecl)
			if !ok || fn.Recv != nil || fn.Body == nil {
				continue
			}
			name := fn.Name.Name
			if !strings.HasPrefix(name, "op") && name != "logNImpl" {
				continue
			}
			// Extract body text between { and }
			start := fset.Position(fn.Body.Lbrace).Offset + 1
			end := fset.Position(fn.Body.Rbrace).Offset
			bodyText := string(src[start:end])
			bodies[name] = bodyText
		}
	}
	return bodies
}

// stripStackDecl removes `s := interp.Stack` from the beginning of a body.
func stripStackDecl(body string) string {
	lines := strings.Split(body, "\n")
	for i, line := range lines {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			continue
		}
		if trimmed == "s := interp.Stack" {
			// Remove this line
			lines = append(lines[:i], lines[i+1:]...)
			return strings.Join(lines, "\n")
		}
		break // first non-empty line is not the stack decl
	}
	return body
}

// substituteLocals replaces interp.Bytecode → bc in inlined bodies
// so that hot paths (PUSH1-4, JUMP, etc.) use the local variable
// from the Run() method rather than an extra pointer dereference.
func substituteLocals(body string) string {
	body = strings.ReplaceAll(body, "interp.Bytecode.", "bc.")
	body = strings.ReplaceAll(body, "interp.Bytecode\n", "bc\n")
	return body
}

// ---------------------------------------------------------------------------
// Code emission
// ---------------------------------------------------------------------------

type emitter struct {
	buf     *bytes.Buffer
	bodies  map[string]string
	tracing bool // true → per-opcode gas deduction, hooks; false → gas accumulator
}

func (e *emitter) p(format string, args ...interface{}) {
	fmt.Fprintf(e.buf, format, args...)
}

// emitGas emits gas charging for a single opcode.
// In accumulator mode: gasCounter += expr.
// In tracing mode: immediate check + deduction from gas.remaining.
func (e *emitter) emitGas(gasExpr string) {
	if e.tracing {
		e.p("if gas.remaining < %s {\n", gasExpr)
		e.p("interp.HaltOOG()\n")
		e.p("return\n")
		e.p("}\n")
		e.p("gas.remaining -= %s\n", gasExpr)
	} else {
		e.p("gasCounter += %s\n", gasExpr)
	}
}

// emitFlush emits the gasCounter flush check + deduction.
// No-op in tracing mode (gas is deducted per-opcode).
func (e *emitter) emitFlush() {
	if e.tracing {
		return
	}
	e.p("if gas.remaining < gasCounter {\n")
	e.p("interp.HaltOOG()\n")
	e.p("return\n")
	e.p("}\n")
	e.p("gas.remaining -= gasCounter\n")
	e.p("gasCounter = 0\n")
}

// emitFlushOnError emits flush-on-error inside a stack check failure branch.
// No-op in tracing mode (gas already deducted).
func (e *emitter) emitFlushOnError() {
	if e.tracing {
		return
	}
	e.p("if gas.remaining < gasCounter {\n")
	e.p("interp.HaltOOG()\n")
	e.p("return\n")
	e.p("}\n")
	e.p("gas.remaining -= gasCounter\n")
	e.p("gasCounter = 0\n")
}

// emitFuncCall emits a function call to the opXXX handler.
func (e *emitter) emitFuncCall(op opDef) {
	if op.needsHost && op.needsOp {
		e.p("%s(interp, host, op)\n", op.funcName)
	} else if op.needsHost {
		e.p("%s(interp, host)\n", op.funcName)
	} else if op.needsOp {
		e.p("%s(interp, op)\n", op.funcName)
	} else {
		e.p("%s(interp)\n", op.funcName)
	}
}

// emitInlineBody emits the inlined body of a function. For shaped opcodes,
// strips `s := interp.Stack` since it's already declared by the boilerplate.
// All inlined bodies get interp.Bytecode → bc substitution for hot paths.
func (e *emitter) emitInlineBody(op opDef) {
	body, ok := e.bodies[op.funcName]
	if !ok {
		fmt.Fprintf(os.Stderr, "ERROR: no body found for %s\n", op.funcName)
		os.Exit(1)
	}
	if op.shape != shapeCustom {
		body = stripStackDecl(body)
	}
	body = substituteLocals(body)
	e.p("%s\n", body)
}

// emitBody emits either the inlined body or a function call.
func (e *emitter) emitBody(op opDef) {
	if op.inline && op.funcName != "" {
		e.emitInlineBody(op)
	} else if op.funcName != "" {
		e.emitFuncCall(op)
	}
}

// emitCase emits the complete case for an opcode.
func (e *emitter) emitCase(op opDef) {
	e.p("case opcode.%s:\n", op.name)

	// Gas charging
	if op.gas != "" {
		e.emitGas(op.gas)
	}

	if op.mode == modeFlush {
		e.emitFlushCase(op)
	} else {
		e.emitAccumulateCase(op)
	}
}

// emitFlushCase: flush gasCounter, then fork gate, then body.
func (e *emitter) emitFlushCase(op opDef) {
	e.emitFlush()

	if op.fork != "" {
		e.p("if !interp.RuntimeFlag.ForkID.IsEnabledIn(%s) {\n", op.fork)
		e.p("interp.HaltNotActivated()\n")
		if op.funcName != "" {
			e.p("} else {\n")
			e.emitBody(op)
			e.p("}\n")
		} else {
			e.p("}\n")
		}
	} else {
		e.emitBody(op)
	}
}

// emitAccumulateCase: accumulate gas, then fork gate + shape boilerplate.
func (e *emitter) emitAccumulateCase(op opDef) {
	if op.fork != "" {
		e.emitAccumulateForkGated(op)
	} else {
		e.emitShapedBody(op)
	}
}

// emitAccumulateForkGated wraps shaped body with fork gate.
func (e *emitter) emitAccumulateForkGated(op opDef) {
	e.p("if !interp.RuntimeFlag.ForkID.IsEnabledIn(%s) {\n", op.fork)
	e.emitFlushOnError()
	e.p("interp.HaltNotActivated()\n")
	e.p("} else {\n")
	e.emitShapedBody(op)
	e.p("}\n")
}

// emitShapedBody emits the stack check + body for shaped opcodes.
func (e *emitter) emitShapedBody(op opDef) {
	switch op.shape {
	case shapeBinaryOp:
		e.p("s := interp.Stack\n")
		e.p("if s.top < 2 {\n")
		e.emitFlushOnError()
		e.p("interp.HaltUnderflow()\n")
		e.p("} else {\n")
		e.p("s.top--\n")
		e.emitBody(op)
		e.p("}\n")

	case shapeUnaryOp:
		e.p("s := interp.Stack\n")
		e.p("if s.top == 0 {\n")
		e.emitFlushOnError()
		e.p("interp.HaltUnderflow()\n")
		e.p("} else {\n")
		e.emitBody(op)
		e.p("}\n")

	case shapeTernaryOp:
		e.p("s := interp.Stack\n")
		e.p("if s.top < 3 {\n")
		e.emitFlushOnError()
		e.p("interp.HaltUnderflow()\n")
		e.p("} else {\n")
		e.p("s.top -= 2\n")
		e.emitBody(op)
		e.p("}\n")

	case shapePushVal:
		e.p("s := interp.Stack\n")
		e.p("if s.top >= StackLimit {\n")
		e.emitFlushOnError()
		e.p("interp.HaltOverflow()\n")
		e.p("} else {\n")
		e.emitBody(op)
		e.p("}\n")

	case shapePop1:
		e.p("if interp.Stack.top == 0 {\n")
		e.emitFlushOnError()
		e.p("interp.HaltUnderflow()\n")
		e.p("} else {\n")
		e.emitBody(op)
		e.p("}\n")

	case shapeCustom:
		// Custom shape in accumulate mode shouldn't happen for non-trivial ops,
		// but handle gracefully — flush before body.
		e.emitFlush()
		e.emitBody(op)
	}
}

// emitDup emits a DUP<n> case (n=1..16).
func (e *emitter) emitDup(n int) {
	e.p("case opcode.DUP%d:\n", n)
	e.emitGas("spec.GasVerylow")
	e.p("s := interp.Stack\n")
	e.p("if s.top < %d || s.top >= StackLimit {\n", n)
	e.emitFlushOnError()
	e.p("interp.HaltOverflow()\n")
	e.p("} else {\n")
	e.p("s.data[s.top] = s.data[s.top-%d]\n", n)
	e.p("s.top++\n")
	e.p("}\n")
}

// emitSwap emits a SWAP<n> case (n=1..16).
func (e *emitter) emitSwap(n int) {
	e.p("case opcode.SWAP%d:\n", n)
	e.emitGas("spec.GasVerylow")
	e.p("s := interp.Stack\n")
	e.p("t := s.top - 1\n")
	e.p("if t < %d {\n", n)
	e.emitFlushOnError()
	e.p("interp.HaltOverflow()\n")
	e.p("} else {\n")
	e.p("s.data[t], s.data[t-%d] = s.data[t-%d], s.data[t]\n", n, n)
	e.p("}\n")
}

// emitLog emits a LOG<n> case (n=0..4).
func (e *emitter) emitLog(n int) {
	e.p("case opcode.LOG%d:\n", n)
	e.emitFlush()
	e.p("logNImpl(interp, host, %d)\n", n)
}

// emitPushGeneric emits the combined PUSH5-PUSH31 case (excluding PUSH20 and PUSH32).
func (e *emitter) emitPushGeneric() {
	e.p("case ")
	first := true
	for i := 5; i <= 31; i++ {
		if i == 20 {
			continue // PUSH20 has its own case
		}
		if !first {
			e.p(", ")
		}
		if i <= 16 || i == 17 || i == 18 || i == 19 || i == 21 || i == 22 || i == 23 || i == 24 ||
			i == 25 || i == 26 || i == 27 || i == 28 || i == 29 || i == 30 || i == 31 {
			e.p("opcode.PUSH%d", i)
		}
		first = false
	}
	e.p(":\n")
	e.emitGas("spec.GasVerylow")
	e.p("s := interp.Stack\n")
	e.p("if s.top >= StackLimit {\n")
	e.emitFlushOnError()
	e.p("interp.HaltOverflow()\n")
	e.p("} else {\n")
	e.p("n := int(op - opcode.PUSH0)\n")
	e.p("s.data[s.top] = types.U256FromBytes(bc.code[bc.pc : bc.pc+n])\n")
	e.p("bc.pc += n\n")
	e.p("s.top++\n")
	e.p("}\n")
}

// emitPush emits a PUSH<n> case with specialized inline code.
func (e *emitter) emitPush(op opDef) {
	e.p("case opcode.%s:\n", op.name)
	e.emitGas(op.gas)
	if op.fork != "" {
		e.p("if !interp.RuntimeFlag.ForkID.IsEnabledIn(%s) {\n", op.fork)
		e.emitFlushOnError()
		e.p("interp.HaltNotActivated()\n")
		e.p("} else {\n")
	}
	e.p("s := interp.Stack\n")
	e.p("if s.top >= StackLimit {\n")
	e.emitFlushOnError()
	e.p("interp.HaltOverflow()\n")
	e.p("} else {\n")
	e.emitInlineBody(op)
	e.p("}\n")
	if op.fork != "" {
		e.p("}\n")
	}
}

// ---------------------------------------------------------------------------
// Shared case emission (used by both Run and RunWithTracing)
// ---------------------------------------------------------------------------

func (e *emitter) emitAllCases() {
	for _, op := range opcodes {
		e.emitCase(op)
	}
	for _, op := range pushOps {
		op.inline = true
		op.shape = shapePushVal
		op.mode = modeAccumulate
		e.emitPush(op)
	}
	e.emitPushGeneric()
	for i := 1; i <= 16; i++ {
		e.emitDup(i)
	}
	for i := 1; i <= 16; i++ {
		e.emitSwap(i)
	}
	for i := 0; i <= 4; i++ {
		e.emitLog(i)
	}
	e.p("default:\n")
	e.emitFlush()
	e.p("interp.Halt(InstructionResultOpcodeNotFound)\n")
}

// ---------------------------------------------------------------------------
// Run() — fast path with gas accumulator, zero tracing overhead
// ---------------------------------------------------------------------------

func (e *emitter) emitRunFunc() {
	e.tracing = false
	e.p(`// Run executes bytecode until halted using direct switch dispatch.
// Static gas is accumulated in a local gasCounter variable. Instead of
// checking and deducting gas per-instruction, static gas costs are summed
// across a basic block and flushed (checked + deducted) at block boundaries
// (jumps, dynamic-gas opcodes, halting opcodes). This eliminates one branch
// + one memory write per static-gas instruction in the hot loop.
func (DefaultRunner) Run(interp *Interpreter, host Host) {
bc := interp.Bytecode
gas := &interp.Gas
var gasCounter uint64
for bc.running {
op := bc.code[bc.pc]
bc.pc++

switch op {
`)
	e.emitAllCases()
	e.p("}\n") // switch
	e.p("}\n") // for
	e.p("}\n") // func
}

// ---------------------------------------------------------------------------
// RunWithTracing() — per-opcode gas, OnOpcode/OnFault hooks
// ---------------------------------------------------------------------------

func (e *emitter) emitRunWithTracingFunc() {
	e.tracing = true
	e.p(`// Run executes bytecode with per-opcode tracing hooks.
// Unlike DefaultRunner.Run, there is no gas accumulator — each opcode
// deducts its static gas immediately, so the tracer receives accurate
// gas and cost values in OnOpcode callbacks.
// If Hooks.OnOpcode is nil, delegates to DefaultRunner for the fast path.
func (r *TracingRunner) Run(interp *Interpreter, host Host) {
if r.Hooks == nil || r.Hooks.OnOpcode == nil {
	DefaultRunner{}.Run(interp, host)
	return
}
bc := interp.Bytecode
gas := &interp.Gas
hooks := r.Hooks
dgt := r.DebugGasTable
for bc.running {
op := bc.code[bc.pc]
bc.pc++

// OnOpcode: report to tracer before execution.
// gas.remaining is pre-deduction; debugCost is the base static cost.
var debugCost uint64
if dgt != nil {
	debugCost = dgt[op]
}
hooks.OnOpcode(uint64(bc.pc-1), op, gas.remaining, debugCost,
	interp, interp.ReturnData, interp.Depth+1, nil)

switch op {
`)
	e.emitAllCases()
	e.p("}\n") // switch

	// OnFault: if interpreter halted with an error, call OnFault.
	e.p("if !bc.running && interp.HaltResult.IsError() {\n")
	e.p("if hooks.OnFault != nil {\n")
	e.p("var faultCost uint64\n")
	e.p("if dgt != nil {\n")
	e.p("faultCost = dgt[op]\n")
	e.p("}\n")
	e.p("hooks.OnFault(uint64(bc.pc-1), op, gas.remaining, faultCost,\n")
	e.p("interp, interp.Depth+1, nil)\n")
	e.p("}\n")
	e.p("}\n")

	e.p("}\n") // for
	e.p("}\n") // func
	e.tracing = false
}

// ---------------------------------------------------------------------------
// DebugGasTable — per-opcode static gas lookup for tracers
// ---------------------------------------------------------------------------

// gasToTableExpr converts a gas expression from the opDef table into one
// suitable for buildDebugGasTable (which takes fg ForkGas, not interp).
func gasToTableExpr(gas string) string {
	if strings.HasPrefix(gas, "interp.ForkGas.") {
		return "fg." + strings.TrimPrefix(gas, "interp.ForkGas.")
	}
	return gas
}

func (e *emitter) emitDebugGasTable() {
	e.p("// buildDebugGasTable constructs a per-opcode static gas cost table.\n")
	e.p("// Used by the tracer to report cost in OnOpcode/OnFault hooks.\n")
	e.p("func buildDebugGasTable(fg ForkGas) [256]uint64 {\n")
	e.p("var t [256]uint64\n")

	// Main opcodes
	for _, op := range opcodes {
		if op.gas == "" {
			continue
		}
		e.p("t[0x%02X] = %s // %s\n", op.code, gasToTableExpr(op.gas), op.name)
	}

	// PUSH0
	e.p("t[0x5F] = spec.GasBase // PUSH0\n")
	// PUSH1-PUSH32
	e.p("for i := byte(0x60); i <= 0x7F; i++ { t[i] = spec.GasVerylow } // PUSH1-PUSH32\n")
	// DUP1-DUP16
	e.p("for i := byte(0x80); i <= 0x8F; i++ { t[i] = spec.GasVerylow } // DUP1-DUP16\n")
	// SWAP1-SWAP16
	e.p("for i := byte(0x90); i <= 0x9F; i++ { t[i] = spec.GasVerylow } // SWAP1-SWAP16\n")
	// LOG0-LOG4: base cost charged inside logNImpl
	e.p("for i := byte(0xA0); i <= 0xA4; i++ { t[i] = spec.GasLog } // LOG0-LOG4\n")

	e.p("return t\n")
	e.p("}\n\n")

	// Accessor (computes on the fly — called once per tracing session)
	e.p("// DebugGasTableForFork returns a per-opcode static gas table for the given fork.\n")
	e.p("// Allocated once per tracing session; cost is negligible.\n")
	e.p("func DebugGasTableForFork(forkID spec.ForkID) *[256]uint64 {\n")
	e.p("t := buildDebugGasTable(NewForkGas(forkID))\n")
	e.p("return &t\n")
	e.p("}\n")
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

func (e *emitter) emitHeader() {
	e.p("// Code generated by gen/main.go; DO NOT EDIT.\n\n")
	e.p("package vm\n\n")
	e.p("import (\n")
	e.p("\t\"github.com/Giulio2002/gevm/opcode\"\n")
	e.p("\t\"github.com/Giulio2002/gevm/types\"\n")
	e.p("\t\"github.com/Giulio2002/gevm/spec\"\n")
	e.p(")\n\n")
	// Silence unused import warnings for cases where inline bodies use keccak
	e.p("// Ensure imports are used.\n")
	e.p("var (\n")
	e.p("\t_ = types.U256Zero\n")
	e.p("\t_ = spec.GasVerylow\n")
	e.p("\t_ = opcode.STOP\n")
	e.p(")\n\n")
}

func main() {
	// Determine paths relative to this generator
	genDir, err := os.Getwd()
	if err != nil {
		fmt.Fprintf(os.Stderr, "cannot get working directory: %v\n", err)
		os.Exit(1)
	}
	// The generator is in internal/vm/gen/, parent is internal/vm/
	vmDir := filepath.Dir(genDir)

	// Parse function bodies from inst_*.go files
	bodies := parseFuncBodies(vmDir)
	fmt.Fprintf(os.Stderr, "parsed %d function bodies\n", len(bodies))

	// Generate
	var buf bytes.Buffer
	e := &emitter{buf: &buf, bodies: bodies}
	e.emitHeader()
	e.emitRunFunc()
	e.p("\n")
	e.emitRunWithTracingFunc()
	e.p("\n")
	e.emitDebugGasTable()

	// Format with gofmt
	formatted, err := format.Source(buf.Bytes())
	if err != nil {
		// Write unformatted for debugging
		debugPath := filepath.Join(vmDir, "table_gen_debug.go")
		os.WriteFile(debugPath, buf.Bytes(), 0644)
		fmt.Fprintf(os.Stderr, "format error: %v\nwrote unformatted to %s\n", err, debugPath)
		os.Exit(1)
	}

	// Write output
	outPath := filepath.Join(vmDir, "table_gen.go")
	if err := os.WriteFile(outPath, formatted, 0644); err != nil {
		fmt.Fprintf(os.Stderr, "cannot write %s: %v\n", outPath, err)
		os.Exit(1)
	}
	fmt.Fprintf(os.Stderr, "wrote %s (%d bytes)\n", outPath, len(formatted))
}
