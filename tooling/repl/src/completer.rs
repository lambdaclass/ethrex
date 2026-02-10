use std::sync::Arc;

use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Context, Helper};

use crate::commands::CommandRegistry;

const UTILITY_NAMES: &[&str] = &[
    "toWei",
    "fromWei",
    "toHex",
    "fromHex",
    "keccak256",
    "toChecksumAddress",
    "isAddress",
];

pub struct ReplHelper {
    registry: Arc<CommandRegistry>,
}

impl ReplHelper {
    pub fn new(registry: Arc<CommandRegistry>) -> Self {
        Self { registry }
    }
}

impl Helper for ReplHelper {}

impl Completer for ReplHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let input = &line[..pos];

        // Built-in dot commands
        if input.starts_with('.') {
            let builtins = [".help", ".exit", ".quit", ".clear", ".connect", ".history"];
            let matches: Vec<Pair> = builtins
                .iter()
                .filter(|b| b.starts_with(input))
                .map(|b| Pair {
                    display: b.to_string(),
                    replacement: b.to_string(),
                })
                .collect();
            return Ok((0, matches));
        }

        // If we have "namespace." pattern, complete methods
        if let Some(dot_pos) = input.rfind('.') {
            let namespace = &input[..dot_pos];
            let partial_method = &input[dot_pos + 1..];
            let methods = self.registry.methods_in_namespace(namespace);
            let matches: Vec<Pair> = methods
                .iter()
                .filter(|m| m.name.starts_with(partial_method))
                .map(|m| {
                    let replacement = format!("{}.{}", namespace, m.name);
                    let display = format!("{}.{} - {}", namespace, m.name, m.description);
                    Pair {
                        display,
                        replacement,
                    }
                })
                .collect();
            return Ok((0, matches));
        }

        // Complete namespaces and utilities
        let mut matches: Vec<Pair> = Vec::new();

        for ns in self.registry.namespaces() {
            if ns.starts_with(input) {
                matches.push(Pair {
                    display: format!("{}.", ns),
                    replacement: format!("{}.", ns),
                });
            }
        }

        for name in UTILITY_NAMES {
            if name.starts_with(input)
                || name.to_lowercase().starts_with(&input.to_lowercase())
            {
                matches.push(Pair {
                    display: name.to_string(),
                    replacement: name.to_string(),
                });
            }
        }

        Ok((0, matches))
    }
}

impl Hinter for ReplHelper {
    type Hint = String;

    fn hint(&self, line: &str, _pos: usize, _ctx: &Context<'_>) -> Option<String> {
        // Show method signature as hint when a full namespace.method is typed
        if let Some(dot_pos) = line.rfind('.') {
            let namespace = &line[..dot_pos];
            let method = &line[dot_pos + 1..];
            if let Some(cmd) = self.registry.find(namespace, method)
                && !cmd.params.is_empty()
            {
                let params: Vec<String> = cmd
                    .params
                    .iter()
                    .map(|p| {
                        if p.required {
                            format!("<{}>", p.name)
                        } else {
                            format!("[{}]", p.name)
                        }
                    })
                    .collect();
                return Some(format!(" {}", params.join(" ")));
            }
        }
        None
    }
}

impl Highlighter for ReplHelper {}
impl Validator for ReplHelper {}
