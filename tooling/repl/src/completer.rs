use std::sync::Arc;

use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Context, Helper};

use crate::commands::CommandRegistry;
use crate::parser::UTILITY_NAMES;
use crate::variables::VariableStore;

pub struct ReplHelper {
    registry: Arc<CommandRegistry>,
    variables: VariableStore,
}

impl ReplHelper {
    pub fn new(registry: Arc<CommandRegistry>, variables: VariableStore) -> Self {
        Self {
            registry,
            variables,
        }
    }
}

impl ReplHelper {
    /// Complete variable references starting with `$`.
    ///
    /// Handles three cases:
    /// - `$` or `$par` → complete variable names
    /// - `$name.` or `$name.fi` → complete top-level field names
    /// - `$name.field.` or `$name.field.sub` → complete nested field names
    ///
    /// Returns `None` if the current word doesn't contain a `$` reference.
    fn complete_variable(&self, input: &str) -> Option<Vec<Pair>> {
        // Find the last '$' that could be the start of a variable reference.
        // It must be preceded by whitespace, '(' , ',' , or be at position 0
        // of the current "word" (i.e. not inside a quoted string).
        let dollar_pos = input.rfind('$')?;
        if dollar_pos > 0 {
            let before = input.as_bytes()[dollar_pos - 1];
            if before != b' ' && before != b'(' && before != b',' && before != b'\t' {
                return None;
            }
        }

        let var_text = &input[dollar_pos + 1..]; // everything after '$'

        // Split into segments by '.'
        let segments: Vec<&str> = var_text.split('.').collect();

        match segments.len() {
            // `$` or `$par` → complete variable names
            0 | 1 => {
                let partial = segments.first().unwrap_or(&"");
                let names = self.variables.names();
                let matches: Vec<Pair> = names
                    .iter()
                    .filter(|n| n.starts_with(partial))
                    .map(|n| Pair {
                        display: format!("${n}"),
                        replacement: format!("${n}"),
                    })
                    .collect();
                if matches.is_empty() {
                    None
                } else {
                    Some(matches)
                }
            }
            // `$name.field...partial` → complete field at the deepest level
            _ => {
                let var_name = segments[0];
                let partial = segments.last().unwrap_or(&"");
                let path: Vec<&str> = segments[1..segments.len() - 1].to_vec();

                let field_names = self.variables.nested_field_names(var_name, &path);
                let prefix = if path.is_empty() {
                    format!("${var_name}.")
                } else {
                    format!("${var_name}.{}.", path.join("."))
                };

                let matches: Vec<Pair> = field_names
                    .iter()
                    .filter(|f| f.starts_with(partial))
                    .map(|f| Pair {
                        display: format!("{prefix}{f}"),
                        replacement: format!("{prefix}{f}"),
                    })
                    .collect();
                if matches.is_empty() {
                    None
                } else {
                    Some(matches)
                }
            }
        }
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
            let builtins = [
                ".help", ".exit", ".quit", ".clear", ".connect", ".history", ".vars",
            ];
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

        // Variable completion: find the current word containing '$'
        if let Some(var_completions) = self.complete_variable(input) {
            // The replacement starts at the '$' position
            let dollar_pos = input.rfind('$').unwrap_or(0);
            return Ok((dollar_pos, var_completions));
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
            if name.starts_with(input) || name.to_lowercase().starts_with(&input.to_lowercase()) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_helper() -> ReplHelper {
        ReplHelper::new(Arc::new(CommandRegistry::new()), VariableStore::new())
    }

    fn make_helper_with_vars() -> ReplHelper {
        let vars = VariableStore::new();
        vars.insert(
            "head".to_string(),
            serde_json::json!({
                "hash": "0xabc123",
                "number": "0x1",
                "nested": { "deep": "value" }
            }),
        );
        vars.insert(
            "payload".to_string(),
            serde_json::json!({
                "executionPayload": {
                    "blockHash": "0xdef456",
                    "stateRoot": "0x789"
                }
            }),
        );
        vars.insert("ts".to_string(), serde_json::json!("0x69b9a63d"));
        ReplHelper::new(Arc::new(CommandRegistry::new()), vars)
    }

    /// Get completions without needing a rustyline Context.
    /// Replicates the completion logic since Context is hard to construct.
    fn get_completions(helper: &ReplHelper, input: &str) -> Vec<String> {
        // Dot commands
        if input.starts_with('.') {
            let builtins = [
                ".help", ".exit", ".quit", ".clear", ".connect", ".history", ".vars",
            ];
            return builtins
                .iter()
                .filter(|b| b.starts_with(input))
                .map(|b| b.to_string())
                .collect();
        }

        // Variable completion
        if let Some(completions) = helper.complete_variable(input) {
            return completions.into_iter().map(|p| p.replacement).collect();
        }

        // Method completion after namespace.
        if let Some(dot_pos) = input.rfind('.') {
            let namespace = &input[..dot_pos];
            let partial_method = &input[dot_pos + 1..];
            let methods = helper.registry.methods_in_namespace(namespace);
            return methods
                .iter()
                .filter(|m| m.name.starts_with(partial_method))
                .map(|m| format!("{}.{}", namespace, m.name))
                .collect();
        }

        // Namespace + utility completion
        let mut matches = Vec::new();
        for ns in helper.registry.namespaces() {
            if ns.starts_with(input) {
                matches.push(format!("{}.", ns));
            }
        }
        for name in UTILITY_NAMES {
            if name.starts_with(input) || name.to_lowercase().starts_with(&input.to_lowercase()) {
                matches.push(name.to_string());
            }
        }
        matches
    }

    /// Get hints without needing a rustyline Context.
    fn get_hint(helper: &ReplHelper, line: &str) -> Option<String> {
        if let Some(dot_pos) = line.rfind('.') {
            let namespace = &line[..dot_pos];
            let method = &line[dot_pos + 1..];
            if let Some(cmd) = helper.registry.find(namespace, method)
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

    // --- Dot command tests ---

    #[test]
    fn complete_dot_h() {
        let helper = make_helper();
        let matches = get_completions(&helper, ".h");
        assert!(matches.contains(&".help".to_string()));
        assert!(matches.contains(&".history".to_string()));
        assert!(!matches.contains(&".exit".to_string()));
    }

    #[test]
    fn complete_dot_e() {
        let helper = make_helper();
        let matches = get_completions(&helper, ".e");
        assert!(matches.contains(&".exit".to_string()));
        assert!(!matches.contains(&".help".to_string()));
    }

    #[test]
    fn complete_dot_x_no_matches() {
        let helper = make_helper();
        let matches = get_completions(&helper, ".x");
        assert!(matches.is_empty());
    }

    // --- Namespace completion tests ---

    #[test]
    fn complete_namespace_eth() {
        let helper = make_helper();
        let matches = get_completions(&helper, "et");
        assert!(matches.contains(&"eth.".to_string()));
    }

    #[test]
    fn complete_namespace_net() {
        let helper = make_helper();
        let matches = get_completions(&helper, "ne");
        assert!(matches.contains(&"net.".to_string()));
    }

    #[test]
    fn complete_empty_input_shows_all() {
        let helper = make_helper();
        let matches = get_completions(&helper, "");
        // Should include all namespaces
        for ns in helper.registry.namespaces() {
            assert!(
                matches.contains(&format!("{}.", ns)),
                "missing namespace: {ns}"
            );
        }
        // Should include all utilities
        for name in UTILITY_NAMES {
            assert!(
                matches.contains(&name.to_string()),
                "missing utility: {name}"
            );
        }
    }

    // --- Method completion tests ---

    #[test]
    fn complete_eth_get_methods() {
        let helper = make_helper();
        let matches = get_completions(&helper, "eth.get");
        // All results should start with "eth.get"
        assert!(!matches.is_empty());
        for m in &matches {
            assert!(m.starts_with("eth.get"), "unexpected match: {m}");
        }
    }

    #[test]
    fn complete_eth_block_number_exact() {
        let helper = make_helper();
        let matches = get_completions(&helper, "eth.blockNumber");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0], "eth.blockNumber");
    }

    #[test]
    fn complete_eth_zzz_no_match() {
        let helper = make_helper();
        let matches = get_completions(&helper, "eth.zzz");
        assert!(matches.is_empty());
    }

    // --- Utility completion tests ---

    #[test]
    fn complete_utility_to() {
        let helper = make_helper();
        let matches = get_completions(&helper, "to");
        assert!(matches.contains(&"toWei".to_string()));
        assert!(matches.contains(&"toHex".to_string()));
        assert!(matches.contains(&"toChecksumAddress".to_string()));
    }

    #[test]
    fn complete_utility_keccak() {
        let helper = make_helper();
        let matches = get_completions(&helper, "keccak");
        assert!(matches.contains(&"keccak256".to_string()));
    }

    #[test]
    fn complete_utility_case_insensitive() {
        let helper = make_helper();
        let matches = get_completions(&helper, "towei");
        // Case-insensitive matching should find "toWei"
        assert!(matches.contains(&"toWei".to_string()));
    }

    #[test]
    fn complete_utility_towel_no_match() {
        let helper = make_helper();
        let matches = get_completions(&helper, "towel");
        assert!(matches.is_empty());
    }

    // --- Hinter tests ---

    #[test]
    fn hint_eth_get_balance_shows_params() {
        let helper = make_helper();
        let hint = get_hint(&helper, "eth.getBalance");
        // eth.getBalance takes <address> [block], so hint should be present
        assert!(hint.is_some());
        let hint_str = hint.unwrap();
        assert!(
            hint_str.contains("address"),
            "hint should contain address param"
        );
    }

    #[test]
    fn hint_eth_block_number_no_params() {
        let helper = make_helper();
        let hint = get_hint(&helper, "eth.blockNumber");
        // blockNumber takes no params, so no hint
        assert!(hint.is_none());
    }

    #[test]
    fn hint_unknown_method_returns_none() {
        let helper = make_helper();
        let hint = get_hint(&helper, "unknown.method");
        assert!(hint.is_none());
    }

    // --- Variable completion tests ---

    #[test]
    fn complete_dollar_lists_all_vars() {
        let helper = make_helper_with_vars();
        let matches = get_completions(&helper, "$");
        assert!(matches.contains(&"$head".to_string()));
        assert!(matches.contains(&"$payload".to_string()));
        assert!(matches.contains(&"$ts".to_string()));
    }

    #[test]
    fn complete_dollar_partial_name() {
        let helper = make_helper_with_vars();
        let matches = get_completions(&helper, "$he");
        assert_eq!(matches, vec!["$head"]);
    }

    #[test]
    fn complete_dollar_name_dot_lists_fields() {
        let helper = make_helper_with_vars();
        let matches = get_completions(&helper, "$head.");
        assert!(matches.contains(&"$head.hash".to_string()));
        assert!(matches.contains(&"$head.number".to_string()));
        assert!(matches.contains(&"$head.nested".to_string()));
    }

    #[test]
    fn complete_dollar_name_dot_partial_field() {
        let helper = make_helper_with_vars();
        let matches = get_completions(&helper, "$head.ha");
        assert_eq!(matches, vec!["$head.hash"]);
    }

    #[test]
    fn complete_nested_field() {
        let helper = make_helper_with_vars();
        let matches = get_completions(&helper, "$payload.executionPayload.");
        assert!(matches.contains(&"$payload.executionPayload.blockHash".to_string()));
        assert!(matches.contains(&"$payload.executionPayload.stateRoot".to_string()));
    }

    #[test]
    fn complete_deeply_nested() {
        let helper = make_helper_with_vars();
        let matches = get_completions(&helper, "$head.nested.");
        assert_eq!(matches, vec!["$head.nested.deep"]);
    }

    #[test]
    fn complete_var_no_fields_on_string() {
        let helper = make_helper_with_vars();
        // $ts is a string, not an object — no field completions
        let matches = get_completions(&helper, "$ts.");
        assert!(matches.is_empty());
    }

    #[test]
    fn complete_var_unknown_name() {
        let helper = make_helper_with_vars();
        let matches = get_completions(&helper, "$zzz");
        assert!(matches.is_empty());
    }

    #[test]
    fn complete_var_mid_command() {
        let helper = make_helper_with_vars();
        // Variable completion in the middle of a command line
        let matches = get_completions(&helper, "engine.newPayloadV4 $pay");
        assert_eq!(matches, vec!["$payload"]);
    }

    #[test]
    fn complete_var_mid_command_field() {
        let helper = make_helper_with_vars();
        let matches = get_completions(&helper, "engine.newPayloadV4 $payload.execution");
        assert_eq!(matches, vec!["$payload.executionPayload"]);
    }
}
