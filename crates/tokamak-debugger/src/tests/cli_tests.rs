//! Tests for the CLI module: command parsing, formatter, and execution.

use std::collections::BTreeSet;

use ethrex_common::{Address, U256};

use crate::cli::commands::{self, Command, DebuggerState};
use crate::cli::formatter;
use crate::engine::ReplayEngine;
use crate::tests::helpers;
use crate::types::{ReplayConfig, StepRecord};

// ─── Command Parsing ────────────────────────────────────────────────

#[test]
fn parse_step() {
    assert_eq!(commands::parse("step"), Some(Command::Step));
    assert_eq!(commands::parse("s"), Some(Command::Step));
}

#[test]
fn parse_step_back() {
    assert_eq!(commands::parse("step-back"), Some(Command::StepBack));
    assert_eq!(commands::parse("sb"), Some(Command::StepBack));
}

#[test]
fn parse_continue() {
    assert_eq!(commands::parse("continue"), Some(Command::Continue));
    assert_eq!(commands::parse("c"), Some(Command::Continue));
}

#[test]
fn parse_reverse_continue() {
    assert_eq!(
        commands::parse("reverse-continue"),
        Some(Command::ReverseContinue)
    );
    assert_eq!(commands::parse("rc"), Some(Command::ReverseContinue));
}

#[test]
fn parse_break_decimal() {
    assert_eq!(commands::parse("break 10"), Some(Command::Break { pc: 10 }));
    assert_eq!(commands::parse("b 10"), Some(Command::Break { pc: 10 }));
}

#[test]
fn parse_break_hex() {
    assert_eq!(
        commands::parse("break 0x0a"),
        Some(Command::Break { pc: 10 })
    );
    assert_eq!(commands::parse("b 0X0A"), Some(Command::Break { pc: 10 }));
}

#[test]
fn parse_delete() {
    assert_eq!(commands::parse("delete 5"), Some(Command::Delete { pc: 5 }));
    assert_eq!(commands::parse("d 0x05"), Some(Command::Delete { pc: 5 }));
}

#[test]
fn parse_goto() {
    assert_eq!(commands::parse("goto 42"), Some(Command::Goto { step: 42 }));
    assert_eq!(commands::parse("g 42"), Some(Command::Goto { step: 42 }));
}

#[test]
fn parse_list_default() {
    assert_eq!(commands::parse("list"), Some(Command::List { count: 5 }));
    assert_eq!(commands::parse("l"), Some(Command::List { count: 5 }));
}

#[test]
fn parse_list_with_count() {
    assert_eq!(
        commands::parse("list 10"),
        Some(Command::List { count: 10 })
    );
    assert_eq!(commands::parse("l 3"), Some(Command::List { count: 3 }));
}

#[test]
fn parse_info_stack_bp_help_quit() {
    assert_eq!(commands::parse("info"), Some(Command::Info));
    assert_eq!(commands::parse("i"), Some(Command::Info));
    assert_eq!(commands::parse("stack"), Some(Command::Stack));
    assert_eq!(commands::parse("st"), Some(Command::Stack));
    assert_eq!(commands::parse("breakpoints"), Some(Command::Breakpoints));
    assert_eq!(commands::parse("bp"), Some(Command::Breakpoints));
    assert_eq!(commands::parse("help"), Some(Command::Help));
    assert_eq!(commands::parse("h"), Some(Command::Help));
    assert_eq!(commands::parse("quit"), Some(Command::Quit));
    assert_eq!(commands::parse("q"), Some(Command::Quit));
}

#[test]
fn parse_empty_returns_none() {
    assert_eq!(commands::parse(""), None);
    assert_eq!(commands::parse("   "), None);
}

#[test]
fn parse_unknown_returns_none() {
    assert_eq!(commands::parse("xyz"), None);
    assert_eq!(commands::parse("break"), None); // missing arg
}

// ─── Formatter ──────────────────────────────────────────────────────

fn make_sample_step(step_index: usize, pc: usize, opcode: u8, gas: i64) -> StepRecord {
    StepRecord {
        step_index,
        pc,
        opcode,
        depth: 0,
        gas_remaining: gas,
        stack_top: vec![U256::from(7), U256::from(3)],
        stack_depth: 2,
        memory_size: 0,
        code_address: Address::zero(),
    }
}

#[test]
fn opcode_name_known() {
    assert_eq!(formatter::opcode_name(0x01), "ADD");
    assert_eq!(formatter::opcode_name(0x60), "PUSH1");
    assert_eq!(formatter::opcode_name(0x00), "STOP");
}

#[test]
fn format_step_contains_key_fields() {
    let step = make_sample_step(42, 0x0a, 0x01, 99994);
    let output = formatter::format_step(&step, 1337);
    assert!(output.contains("[42/1337]"));
    assert!(output.contains("0x000a"));
    assert!(output.contains("ADD"));
    assert!(output.contains("gas=99994"));
    assert!(output.contains("stack(2)"));
}

#[test]
fn format_step_compact_cursor_marker() {
    let step = make_sample_step(5, 0x02, 0x60, 999);
    let with_cursor = formatter::format_step_compact(&step, 10, true);
    let without_cursor = formatter::format_step_compact(&step, 10, false);
    assert!(with_cursor.starts_with('>'));
    assert!(without_cursor.starts_with(' '));
}

#[test]
fn format_stack_shows_values() {
    let step = make_sample_step(0, 0, 0x01, 100);
    let output = formatter::format_stack(&step);
    assert!(output.contains("Stack depth: 2"));
    assert!(output.contains("[0]: 0x7"));
    assert!(output.contains("[1]: 0x3"));
}

#[test]
fn format_stack_empty() {
    let step = StepRecord {
        step_index: 0,
        pc: 0,
        opcode: 0x00,
        depth: 0,
        gas_remaining: 100,
        stack_top: vec![],
        stack_depth: 0,
        memory_size: 0,
        code_address: Address::zero(),
    };
    let output = formatter::format_stack(&step);
    assert!(output.contains("(empty)"));
}

#[test]
fn format_breakpoints_empty_and_populated() {
    let empty = BTreeSet::new();
    assert!(formatter::format_breakpoints(&empty).contains("No breakpoints"));

    let mut bps = BTreeSet::new();
    bps.insert(10);
    bps.insert(20);
    let output = formatter::format_breakpoints(&bps);
    assert!(output.contains("Breakpoints (2)"));
    assert!(output.contains("0x000a"));
    assert!(output.contains("0x0014"));
}

// ─── Command Execution (with ReplayEngine) ──────────────────────────

/// PUSH1 3, PUSH1 4, ADD, STOP → 4 recorded steps
fn make_test_engine() -> ReplayEngine {
    let bytecode = vec![0x60, 0x03, 0x60, 0x04, 0x01, 0x00];
    let (contract, sender, mut db) = helpers::setup_contract(bytecode);
    let env = helpers::make_test_env(sender);
    let tx = helpers::make_test_tx(contract);
    ReplayEngine::record(&mut db, env, &tx, ReplayConfig::default()).expect("record")
}

fn make_state() -> DebuggerState {
    DebuggerState {
        breakpoints: BTreeSet::new(),
    }
}

#[test]
fn exec_step_forward() {
    let mut engine = make_test_engine();
    let mut state = make_state();
    assert_eq!(engine.position(), 0);

    let action = commands::execute(&Command::Step, &mut engine, &mut state);
    assert_eq!(engine.position(), 1);
    assert!(matches!(action, commands::Action::Print(s) if s.contains("PUSH1")));
}

#[test]
fn exec_step_back() {
    let mut engine = make_test_engine();
    let mut state = make_state();

    // Move forward then back
    commands::execute(&Command::Step, &mut engine, &mut state);
    assert_eq!(engine.position(), 1);

    let action = commands::execute(&Command::StepBack, &mut engine, &mut state);
    assert_eq!(engine.position(), 0);
    assert!(matches!(action, commands::Action::Print(s) if s.contains("PUSH1")));
}

#[test]
fn exec_step_back_at_start() {
    let mut engine = make_test_engine();
    let mut state = make_state();

    let action = commands::execute(&Command::StepBack, &mut engine, &mut state);
    assert!(matches!(action, commands::Action::Print(s) if s.contains("Already at first")));
}

#[test]
fn exec_continue_no_breakpoints() {
    let mut engine = make_test_engine();
    let mut state = make_state();

    let action = commands::execute(&Command::Continue, &mut engine, &mut state);
    assert!(matches!(action, commands::Action::Print(s) if s.contains("Reached end")));
    // Should be at last step
    assert_eq!(engine.position(), engine.len() - 1);
}

#[test]
fn exec_continue_with_breakpoint() {
    let mut engine = make_test_engine();
    let mut state = make_state();

    // ADD opcode is at PC=4 (PUSH1 3 [PC=0,1], PUSH1 4 [PC=2,3], ADD [PC=4])
    state.breakpoints.insert(4);

    let action = commands::execute(&Command::Continue, &mut engine, &mut state);
    assert!(matches!(action, commands::Action::Print(s) if s.contains("Breakpoint hit")));
    // Current step should be at the ADD opcode
    let step = engine.current_step().unwrap();
    assert_eq!(step.opcode, 0x01); // ADD
}

#[test]
fn exec_goto() {
    let mut engine = make_test_engine();
    let mut state = make_state();
    let last = engine.len() - 1;

    let action = commands::execute(&Command::Goto { step: last }, &mut engine, &mut state);
    assert_eq!(engine.position(), last);
    assert!(matches!(action, commands::Action::Print(s) if s.contains("STOP")));
}

#[test]
fn exec_goto_out_of_range() {
    let mut engine = make_test_engine();
    let mut state = make_state();

    let action = commands::execute(&Command::Goto { step: 9999 }, &mut engine, &mut state);
    assert!(matches!(action, commands::Action::Print(s) if s.contains("out of range")));
}

#[test]
fn exec_break_and_breakpoints() {
    let mut engine = make_test_engine();
    let mut state = make_state();

    commands::execute(&Command::Break { pc: 10 }, &mut engine, &mut state);
    commands::execute(&Command::Break { pc: 20 }, &mut engine, &mut state);
    assert_eq!(state.breakpoints.len(), 2);

    let action = commands::execute(&Command::Breakpoints, &mut engine, &mut state);
    assert!(
        matches!(action, commands::Action::Print(s) if s.contains("0x000a") && s.contains("0x0014"))
    );

    commands::execute(&Command::Delete { pc: 10 }, &mut engine, &mut state);
    assert_eq!(state.breakpoints.len(), 1);
}
