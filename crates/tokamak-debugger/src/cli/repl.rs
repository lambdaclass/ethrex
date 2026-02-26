//! Interactive REPL loop for the debugger.

use std::collections::BTreeSet;

use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::{Config, Editor};

use crate::cli::commands::{Action, DebuggerState};
use crate::cli::{commands, formatter};
use crate::engine::ReplayEngine;
use crate::error::DebuggerError;

/// Start the interactive debugger REPL.
pub fn start(mut engine: ReplayEngine) -> Result<(), DebuggerError> {
    let config = Config::builder().auto_add_history(true).build();
    let mut rl: Editor<(), DefaultHistory> =
        Editor::with_config(config).map_err(|e| DebuggerError::Cli(e.to_string()))?;
    let mut state = DebuggerState {
        breakpoints: BTreeSet::new(),
    };

    let total = engine.len();

    if let Some(step) = engine.current_step() {
        println!("{}", formatter::format_step(step, total));
    }
    println!("Type 'help' for available commands.\n");

    loop {
        let prompt = format!("(dbg {}/{}) ", engine.position(), engine.len());
        match rl.readline(&prompt) {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(cmd) = commands::parse(trimmed) {
                    match commands::execute(&cmd, &mut engine, &mut state) {
                        Action::Print(s) => println!("{s}"),
                        Action::Quit => break,
                        Action::Silent => {}
                    }
                }
            }
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(e) => {
                eprintln!("Readline error: {e}");
                break;
            }
        }
    }

    Ok(())
}
