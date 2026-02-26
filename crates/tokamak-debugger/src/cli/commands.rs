//! Command parsing and execution for the debugger REPL.

use std::collections::BTreeSet;

use crate::cli::formatter;
use crate::engine::ReplayEngine;

/// A parsed debugger command.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Step,
    StepBack,
    Continue,
    ReverseContinue,
    Break { pc: usize },
    Delete { pc: usize },
    Goto { step: usize },
    Info,
    Stack,
    List { count: usize },
    Breakpoints,
    Help,
    Quit,
}

/// Result of executing a command.
pub enum Action {
    Print(String),
    Quit,
    Silent,
}

/// Mutable state for the debugger session.
pub struct DebuggerState {
    pub breakpoints: BTreeSet<usize>,
}

/// Parse user input into a command. Returns `None` for empty or unrecognized input.
pub fn parse(input: &str) -> Option<Command> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut parts = trimmed.splitn(2, ' ');
    let cmd = parts.next().unwrap_or("");
    let arg = parts.next().map(str::trim);

    match cmd {
        "s" | "step" => Some(Command::Step),
        "sb" | "step-back" => Some(Command::StepBack),
        "c" | "continue" => Some(Command::Continue),
        "rc" | "reverse-continue" => Some(Command::ReverseContinue),
        "b" | "break" => Some(Command::Break {
            pc: parse_number(arg?)?,
        }),
        "d" | "delete" => Some(Command::Delete {
            pc: parse_number(arg?)?,
        }),
        "g" | "goto" => Some(Command::Goto {
            step: parse_number(arg?)?,
        }),
        "i" | "info" => Some(Command::Info),
        "st" | "stack" => Some(Command::Stack),
        "l" | "list" => {
            let count = arg.and_then(|a| a.parse::<usize>().ok()).unwrap_or(5);
            Some(Command::List { count })
        }
        "bp" | "breakpoints" => Some(Command::Breakpoints),
        "h" | "help" => Some(Command::Help),
        "q" | "quit" => Some(Command::Quit),
        _ => {
            eprintln!("Unknown command: '{cmd}'. Type 'help' for available commands.");
            None
        }
    }
}

/// Execute a command against the engine and debugger state.
pub fn execute(cmd: &Command, engine: &mut ReplayEngine, state: &mut DebuggerState) -> Action {
    let total = engine.len();
    match cmd {
        Command::Step => match engine.forward() {
            Some(step) => Action::Print(formatter::format_step(step, total)),
            None => Action::Print("Already at last step.".to_string()),
        },
        Command::StepBack => match engine.backward() {
            Some(step) => Action::Print(formatter::format_step(step, total)),
            None => Action::Print("Already at first step.".to_string()),
        },
        Command::Continue => execute_continue(engine, state, total),
        Command::ReverseContinue => execute_reverse_continue(engine, state, total),
        Command::Break { pc } => {
            state.breakpoints.insert(*pc);
            Action::Print(format!("Breakpoint set at PC={:#06x} ({}).", pc, pc))
        }
        Command::Delete { pc } => {
            if state.breakpoints.remove(pc) {
                Action::Print(format!("Breakpoint removed at PC={:#06x} ({}).", pc, pc))
            } else {
                Action::Print(format!("No breakpoint at PC={:#06x} ({}).", pc, pc))
            }
        }
        Command::Goto { step } => match engine.goto(*step) {
            Some(s) => Action::Print(formatter::format_step(s, total)),
            None => Action::Print(format!(
                "Step {} out of range (0..{}).",
                step,
                total.saturating_sub(1)
            )),
        },
        Command::Info => Action::Print(formatter::format_info(engine.trace(), engine.position())),
        Command::Stack => match engine.current_step() {
            Some(step) => Action::Print(formatter::format_stack(step)),
            None => Action::Print("No steps recorded.".to_string()),
        },
        Command::List { count } => execute_list(engine, total, *count),
        Command::Breakpoints => Action::Print(formatter::format_breakpoints(&state.breakpoints)),
        Command::Help => Action::Print(formatter::format_help()),
        Command::Quit => Action::Quit,
    }
}

fn execute_continue(engine: &mut ReplayEngine, state: &DebuggerState, total: usize) -> Action {
    loop {
        match engine.forward() {
            Some(step) => {
                if state.breakpoints.contains(&step.pc) {
                    return Action::Print(format!(
                        "Breakpoint hit at PC={:#06x}\n{}",
                        step.pc,
                        formatter::format_step(step, total)
                    ));
                }
            }
            None => {
                return Action::Print(format!(
                    "Reached end of trace.\n{}",
                    engine
                        .current_step()
                        .map(|s| formatter::format_step(s, total))
                        .unwrap_or_default()
                ));
            }
        }
    }
}

fn execute_reverse_continue(
    engine: &mut ReplayEngine,
    state: &DebuggerState,
    total: usize,
) -> Action {
    loop {
        match engine.backward() {
            Some(step) => {
                if state.breakpoints.contains(&step.pc) {
                    return Action::Print(format!(
                        "Breakpoint hit at PC={:#06x}\n{}",
                        step.pc,
                        formatter::format_step(step, total)
                    ));
                }
            }
            None => {
                return Action::Print(format!(
                    "Reached start of trace.\n{}",
                    engine
                        .current_step()
                        .map(|s| formatter::format_step(s, total))
                        .unwrap_or_default()
                ));
            }
        }
    }
}

fn execute_list(engine: &ReplayEngine, total: usize, count: usize) -> Action {
    let pos = engine.position();
    let half = count / 2;
    let start = pos.saturating_sub(half);
    let steps = engine.steps_range(start, count);
    if steps.is_empty() {
        return Action::Print("No steps recorded.".to_string());
    }
    let lines: Vec<String> = steps
        .iter()
        .map(|s| formatter::format_step_compact(s, total, s.step_index == pos))
        .collect();
    Action::Print(lines.join("\n"))
}

/// Parse a number supporting hex (0x prefix) and decimal.
fn parse_number(s: &str) -> Option<usize> {
    let s = s.trim();
    if let Some(hex_str) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        usize::from_str_radix(hex_str, 16).ok()
    } else {
        s.parse::<usize>().ok()
    }
}
