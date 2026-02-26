//! Display formatting for debugger output.

use std::collections::BTreeSet;

use ethrex_common::U256;
use ethrex_levm::opcodes::Opcode;

use crate::types::{ReplayTrace, StepRecord};

/// Format a step for detailed display (after step/goto).
pub fn format_step(step: &StepRecord, total: usize) -> String {
    let name = opcode_name(step.opcode);
    let stack_preview = format_stack_inline(&step.stack_top);
    format!(
        "[{}/{}] PC={:#06x}  {:<14} depth={}  gas={}\n  stack({}): [{}]",
        step.step_index,
        total,
        step.pc,
        name,
        step.depth,
        step.gas_remaining,
        step.stack_depth,
        stack_preview,
    )
}

/// Format a step compactly (for list view).
pub fn format_step_compact(step: &StepRecord, total: usize, is_cursor: bool) -> String {
    let marker = if is_cursor { ">" } else { " " };
    format!(
        "{marker} [{}/{}] PC={:#06x}  {:<14} depth={}  gas={}",
        step.step_index,
        total,
        step.pc,
        opcode_name(step.opcode),
        step.depth,
        step.gas_remaining,
    )
}

/// Format trace info summary.
pub fn format_info(trace: &ReplayTrace, position: usize) -> String {
    let output_hex = if trace.output.is_empty() {
        "0x".to_string()
    } else {
        format!("0x{}", hex::encode(&trace.output))
    };
    format!(
        "Trace: {} steps | gas_used: {} | success: {} | output: {}\nPosition: {}/{}",
        trace.steps.len(),
        trace.gas_used,
        trace.success,
        output_hex,
        position,
        trace.steps.len(),
    )
}

/// Format the full stack of a step.
pub fn format_stack(step: &StepRecord) -> String {
    if step.stack_top.is_empty() {
        return format!("Stack depth: {} (empty)", step.stack_depth);
    }
    let mut lines = vec![format!(
        "Stack depth: {} (showing top {}):",
        step.stack_depth,
        step.stack_top.len()
    )];
    for (i, val) in step.stack_top.iter().enumerate() {
        lines.push(format!("  [{}]: {:#x}", i, val));
    }
    lines.join("\n")
}

/// Format the list of active breakpoints.
pub fn format_breakpoints(breakpoints: &BTreeSet<usize>) -> String {
    if breakpoints.is_empty() {
        return "No breakpoints set.".to_string();
    }
    let mut lines = vec![format!("Breakpoints ({}):", breakpoints.len())];
    for pc in breakpoints {
        lines.push(format!("  PC={:#06x} ({})", pc, pc));
    }
    lines.join("\n")
}

/// Static help text.
pub fn format_help() -> String {
    "\
Commands:
  s, step            Step forward one opcode
  sb, step-back      Step backward one opcode
  c, continue        Continue until breakpoint or end
  rc, reverse-continue  Continue backward until breakpoint or start
  b, break <pc>      Set breakpoint at PC (hex 0x0a or decimal 10)
  d, delete <pc>     Delete breakpoint at PC
  g, goto <step>     Jump to step number
  i, info            Show trace summary
  st, stack          Show current stack
  l, list [n]        List n steps around cursor (default: 5)
  bp, breakpoints    List all breakpoints
  h, help            Show this help
  q, quit            Exit debugger"
        .to_string()
}

/// Convert an opcode byte to its human-readable name.
pub fn opcode_name(byte: u8) -> String {
    format!("{:?}", Opcode::from(byte))
}

fn format_stack_inline(stack_top: &[U256]) -> String {
    stack_top
        .iter()
        .map(|v| format!("{:#x}", v))
        .collect::<Vec<_>>()
        .join(", ")
}
