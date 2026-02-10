use std::sync::Arc;

use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::{Config, Editor};

use crate::client::RpcClient;
use crate::commands::{CommandDef, CommandRegistry, ParamType};
use crate::completer::ReplHelper;
use crate::ens;
use crate::formatter;
use crate::parser::{self, ParsedCommand};

pub struct Repl {
    client: RpcClient,
    registry: Arc<CommandRegistry>,
    history_path: String,
}

impl Repl {
    pub fn new(client: RpcClient, history_path: String) -> Self {
        Self {
            client,
            registry: Arc::new(CommandRegistry::new()),
            history_path,
        }
    }

    pub async fn run(&mut self) {
        let config = Config::builder().auto_add_history(true).build();

        let helper = ReplHelper::new(Arc::clone(&self.registry));
        let mut rl: Editor<ReplHelper, DefaultHistory> =
            Editor::with_config(config).expect("Failed to create editor");
        rl.set_helper(Some(helper));

        if let Err(e) = rl.load_history(&self.history_path)
            && !matches!(e, ReadlineError::Io(_))
        {
            eprintln!("Warning: could not load history: {e}");
        }

        println!("Welcome to the ethrex REPL!");
        println!("Connected to {}", self.client.endpoint());
        println!("Type .help for available commands, .exit to quit.\n");

        let mut multiline_buffer = String::new();

        loop {
            let prompt = if multiline_buffer.is_empty() {
                "> "
            } else {
                "... "
            };

            match rl.readline(prompt) {
                Ok(line) => {
                    let line = line.trim_end();

                    // Multi-line support: accumulate if braces/brackets are unbalanced
                    if !multiline_buffer.is_empty() {
                        multiline_buffer.push(' ');
                        multiline_buffer.push_str(line);
                        if !is_balanced(&multiline_buffer) {
                            continue;
                        }
                        let full_input = std::mem::take(&mut multiline_buffer);
                        self.execute_input(&full_input).await;
                    } else if !is_balanced(line) {
                        multiline_buffer = line.to_string();
                        continue;
                    } else {
                        self.execute_input(line).await;
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    multiline_buffer.clear();
                    println!("(Use .exit to quit)");
                }
                Err(ReadlineError::Eof) => {
                    println!("Bye!");
                    break;
                }
                Err(err) => {
                    eprintln!("{}", formatter::format_error(&err.to_string()));
                    break;
                }
            }
        }

        ensure_parent_dir(&self.history_path);
        if let Err(e) = rl.save_history(&self.history_path) {
            eprintln!("Warning: could not save history: {e}");
        }
    }

    /// Execute a single command and return the result as a string (for -x mode).
    pub async fn execute_command(&self, input: &str) -> String {
        match parser::parse(input) {
            Ok(cmd) => match cmd {
                ParsedCommand::RpcCall {
                    namespace,
                    method,
                    args,
                } => self.execute_rpc(&namespace, &method, &args).await,
                ParsedCommand::UtilityCall { name, args } => execute_utility(&name, &args),
                ParsedCommand::BuiltinCommand { name, .. } => {
                    format!("Built-in command .{name} not available in non-interactive mode")
                }
                ParsedCommand::Empty => String::new(),
            },
            Err(e) => formatter::format_error(&e.to_string()),
        }
    }

    async fn execute_input(&self, input: &str) {
        let parsed = match parser::parse(input) {
            Ok(p) => p,
            Err(e) => {
                println!("{}", formatter::format_error(&e.to_string()));
                return;
            }
        };

        match parsed {
            ParsedCommand::Empty => {}
            ParsedCommand::RpcCall {
                namespace,
                method,
                args,
            } => {
                let result = self.execute_rpc(&namespace, &method, &args).await;
                println!("{result}");
            }
            ParsedCommand::BuiltinCommand { name, args } => {
                self.execute_builtin(&name, &args);
            }
            ParsedCommand::UtilityCall { name, args } => {
                let result = execute_utility(&name, &args);
                println!("{result}");
            }
        }
    }

    async fn execute_rpc(
        &self,
        namespace: &str,
        method: &str,
        args: &[serde_json::Value],
    ) -> String {
        let cmd = match self.registry.find(namespace, method) {
            Some(c) => c,
            None => {
                return formatter::format_error(&format!("unknown command: {namespace}.{method}"))
            }
        };

        let resolved_args = match self.resolve_ens_in_args(cmd, args).await {
            Ok(a) => a,
            Err(e) => return formatter::format_error(&e),
        };

        let params = match cmd.build_params(&resolved_args) {
            Ok(p) => p,
            Err(e) => {
                return formatter::format_error(&format!(
                    "{e}\nUsage: {}",
                    formatter::command_usage(cmd)
                ))
            }
        };

        match self.client.send_request(cmd.rpc_method, params).await {
            Ok(result) => formatter::format_value(&result),
            Err(e) => formatter::format_error(&e.to_string()),
        }
    }

    /// Resolve ENS names in arguments that expect an address.
    async fn resolve_ens_in_args(
        &self,
        cmd: &CommandDef,
        args: &[serde_json::Value],
    ) -> Result<Vec<serde_json::Value>, String> {
        let mut resolved = args.to_vec();

        for (i, param_def) in cmd.params.iter().enumerate() {
            if param_def.param_type != ParamType::Address {
                continue;
            }
            let Some(value) = resolved.get(i) else {
                continue;
            };
            let Some(s) = value.as_str() else {
                continue;
            };
            if !ens::looks_like_ens_name(s) {
                continue;
            }

            let name = s.to_string();
            let address = ens::resolve(&self.client, &name).await?;
            println!("Resolved {name} -> {address}");
            resolved[i] = serde_json::Value::String(address);
        }

        Ok(resolved)
    }

    fn execute_builtin(&self, name: &str, args: &[String]) {
        match name {
            "help" => self.show_help(args),
            "exit" | "quit" => {
                println!("Bye!");
                std::process::exit(0);
            }
            "clear" => {
                print!("\x1b[2J\x1b[H");
            }
            "connect" => {
                if let Some(url) = args.first() {
                    println!("Reconnecting to {url}...");
                    println!(
                        "Note: .connect in interactive mode requires restart. Use --endpoint flag."
                    );
                } else {
                    println!("Current endpoint: {}", self.client.endpoint());
                    println!("Usage: .connect <url>");
                }
            }
            "history" => {
                println!("History file: {}", self.history_path);
            }
            _ => {
                println!(
                    "{}",
                    formatter::format_error(&format!("unknown command: .{name}"))
                );
            }
        }
    }

    fn show_help(&self, args: &[String]) {
        if args.is_empty() {
            println!("Available namespaces:");
            for ns in self.registry.namespaces() {
                let count = self.registry.methods_in_namespace(ns).len();
                println!("  {ns:<10} ({count} methods)");
            }
            println!("\nUtility functions:");
            println!("  toWei, fromWei, toHex, fromHex, keccak256, toChecksumAddress, isAddress");
            println!("\nBuilt-in commands:");
            println!("  .help [namespace|command]  Show help");
            println!("  .exit / .quit              Exit REPL");
            println!("  .clear                     Clear screen");
            println!("  .connect <url>             Show/change endpoint");
            println!("  .history                   Show history file path");
            println!("\nType .help <namespace> to list namespace methods.");
            println!("Type .help <namespace.method> for method details.");
        } else {
            let arg = &args[0];
            if let Some(dot_pos) = arg.find('.') {
                let namespace = &arg[..dot_pos];
                let method = &arg[dot_pos + 1..];
                if let Some(cmd) = self.registry.find(namespace, method) {
                    println!("{}", formatter::command_usage(cmd));
                    println!("  {}", cmd.description);
                    if !cmd.params.is_empty() {
                        println!("\nParameters:");
                        for p in cmd.params {
                            let req = if p.required { "required" } else { "optional" };
                            let def = p
                                .default_value
                                .map(|d| format!(", default: {d}"))
                                .unwrap_or_default();
                            println!(
                                "  {:<20} {:?} ({}{}) - {}",
                                p.name, p.param_type, req, def, p.description
                            );
                        }
                    }
                } else {
                    println!(
                        "{}",
                        formatter::format_error(&format!("unknown command: {arg}"))
                    );
                }
            } else {
                let methods = self.registry.methods_in_namespace(arg);
                if methods.is_empty() {
                    println!(
                        "{}",
                        formatter::format_error(&format!("unknown namespace: {arg}"))
                    );
                } else {
                    println!("{arg} namespace ({} methods):", methods.len());
                    for cmd in methods {
                        println!(
                            "  {:<45} {}",
                            formatter::command_usage(cmd),
                            cmd.description
                        );
                    }
                }
            }
        }
    }
}

fn execute_utility(name: &str, args: &[String]) -> String {
    match name {
        "toWei" => {
            if args.len() < 2 {
                return formatter::format_error(
                    "Usage: toWei <amount> <unit>\nUnits: wei, gwei, ether",
                );
            }
            let amount: f64 = match args[0].parse() {
                Ok(v) => v,
                Err(_) => {
                    return formatter::format_error(&format!("invalid number: {}", args[0]))
                }
            };
            let multiplier: f64 = match args[1].to_lowercase().as_str() {
                "wei" => 1.0,
                "gwei" => 1e9,
                "ether" | "eth" => 1e18,
                other => {
                    return formatter::format_error(&format!(
                        "unknown unit: {other}. Use: wei, gwei, ether"
                    ))
                }
            };
            let wei = (amount * multiplier) as u128;
            format!("{wei}")
        }
        "fromWei" => {
            if args.len() < 2 {
                return formatter::format_error(
                    "Usage: fromWei <amount> <unit>\nUnits: wei, gwei, ether",
                );
            }
            let wei: u128 = match args[0].parse() {
                Ok(v) => v,
                Err(_) => {
                    return formatter::format_error(&format!("invalid number: {}", args[0]))
                }
            };
            let divisor: f64 = match args[1].to_lowercase().as_str() {
                "wei" => 1.0,
                "gwei" => 1e9,
                "ether" | "eth" => 1e18,
                other => {
                    return formatter::format_error(&format!(
                        "unknown unit: {other}. Use: wei, gwei, ether"
                    ))
                }
            };
            let result = wei as f64 / divisor;
            let s = format!("{result}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
        "toHex" => {
            if args.is_empty() {
                return formatter::format_error("Usage: toHex <decimal_number>");
            }
            match args[0].parse::<u128>() {
                Ok(n) => format!("0x{n:x}"),
                Err(_) => formatter::format_error(&format!("invalid number: {}", args[0])),
            }
        }
        "fromHex" => {
            if args.is_empty() {
                return formatter::format_error("Usage: fromHex <hex_number>");
            }
            let hex = args[0].strip_prefix("0x").unwrap_or(&args[0]);
            match u128::from_str_radix(hex, 16) {
                Ok(n) => format!("{n}"),
                Err(_) => formatter::format_error(&format!("invalid hex: {}", args[0])),
            }
        }
        "keccak256" => {
            if args.is_empty() {
                return formatter::format_error("Usage: keccak256 <hex_data>");
            }
            let input = args[0].strip_prefix("0x").unwrap_or(&args[0]);
            let data = match hex::decode(input) {
                Ok(d) => d,
                Err(_) => {
                    return formatter::format_error(&format!("invalid hex data: {}", args[0]))
                }
            };
            use sha3::{Digest, Keccak256};
            let hash = Keccak256::digest(&data);
            format!("0x{}", hex::encode(hash))
        }
        "toChecksumAddress" => {
            if args.is_empty() {
                return formatter::format_error("Usage: toChecksumAddress <address>");
            }
            let raw = args[0].strip_prefix("0x").unwrap_or(&args[0]);
            if raw.len() != 40 {
                return formatter::format_error("invalid address length");
            }
            ens::to_checksum_address(&args[0])
        }
        "isAddress" => {
            if args.is_empty() {
                return formatter::format_error("Usage: isAddress <address>");
            }
            let addr = &args[0];
            let valid = addr.starts_with("0x")
                && addr.len() == 42
                && addr[2..].chars().all(|c| c.is_ascii_hexdigit());
            format!("{valid}")
        }
        _ => formatter::format_error(&format!("unknown utility: {name}")),
    }
}

fn is_balanced(s: &str) -> bool {
    let mut brace_depth: i32 = 0;
    let mut bracket_depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    let mut string_char = '"';

    for c in s.chars() {
        if escape {
            escape = false;
            continue;
        }
        if c == '\\' && in_string {
            escape = true;
            continue;
        }
        if in_string {
            if c == string_char {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                in_string = true;
                string_char = c;
            }
            '{' => brace_depth += 1,
            '}' => brace_depth -= 1,
            '[' => bracket_depth += 1,
            ']' => bracket_depth -= 1,
            _ => {}
        }
    }

    brace_depth == 0 && bracket_depth == 0
}

fn ensure_parent_dir(path: &str) {
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- execute_utility: toWei ---

    #[test]
    fn to_wei_ether() {
        let result = execute_utility("toWei", &["1".into(), "ether".into()]);
        assert_eq!(result, "1000000000000000000");
    }

    #[test]
    fn to_wei_gwei() {
        let result = execute_utility("toWei", &["1".into(), "gwei".into()]);
        assert_eq!(result, "1000000000");
    }

    #[test]
    fn to_wei_wei() {
        let result = execute_utility("toWei", &["1".into(), "wei".into()]);
        assert_eq!(result, "1");
    }

    #[test]
    fn to_wei_eth_alias() {
        let result = execute_utility("toWei", &["1".into(), "eth".into()]);
        assert_eq!(result, "1000000000000000000");
    }

    #[test]
    fn to_wei_unknown_unit() {
        let result = execute_utility("toWei", &["1".into(), "finney".into()]);
        assert!(result.contains("Error"), "expected error for unknown unit, got: {result}");
    }

    #[test]
    fn to_wei_missing_args() {
        let result = execute_utility("toWei", &["1".into()]);
        assert!(result.contains("Error"), "expected error for missing args");
    }

    #[test]
    fn to_wei_invalid_number() {
        let result = execute_utility("toWei", &["abc".into(), "ether".into()]);
        assert!(result.contains("Error"), "expected error for invalid number");
    }

    // --- execute_utility: fromWei ---

    #[test]
    fn from_wei_ether() {
        let result = execute_utility("fromWei", &["1000000000000000000".into(), "ether".into()]);
        assert_eq!(result, "1");
    }

    #[test]
    fn from_wei_gwei() {
        let result = execute_utility("fromWei", &["1000000000".into(), "gwei".into()]);
        assert_eq!(result, "1");
    }

    #[test]
    fn from_wei_missing_args() {
        let result = execute_utility("fromWei", &["1000".into()]);
        assert!(result.contains("Error"));
    }

    #[test]
    fn from_wei_invalid_number() {
        let result = execute_utility("fromWei", &["notanumber".into(), "ether".into()]);
        assert!(result.contains("Error"));
    }

    // --- execute_utility: toHex ---

    #[test]
    fn to_hex_255() {
        let result = execute_utility("toHex", &["255".into()]);
        assert_eq!(result, "0xff");
    }

    #[test]
    fn to_hex_zero() {
        let result = execute_utility("toHex", &["0".into()]);
        assert_eq!(result, "0x0");
    }

    #[test]
    fn to_hex_invalid() {
        let result = execute_utility("toHex", &["xyz".into()]);
        assert!(result.contains("Error"));
    }

    #[test]
    fn to_hex_missing_arg() {
        let result = execute_utility("toHex", &[]);
        assert!(result.contains("Error"));
    }

    // --- execute_utility: fromHex ---

    #[test]
    fn from_hex_0xff() {
        let result = execute_utility("fromHex", &["0xff".into()]);
        assert_eq!(result, "255");
    }

    #[test]
    fn from_hex_without_0x() {
        let result = execute_utility("fromHex", &["ff".into()]);
        assert_eq!(result, "255");
    }

    #[test]
    fn from_hex_invalid() {
        let result = execute_utility("fromHex", &["zzz".into()]);
        assert!(result.contains("Error"));
    }

    #[test]
    fn from_hex_missing_arg() {
        let result = execute_utility("fromHex", &[]);
        assert!(result.contains("Error"));
    }

    // --- execute_utility: keccak256 ---

    #[test]
    fn keccak256_empty_input() {
        // keccak256 of empty data (0 bytes)
        let result = execute_utility("keccak256", &["0x".into()]);
        assert_eq!(
            result,
            "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
    }

    #[test]
    fn keccak256_invalid_hex() {
        let result = execute_utility("keccak256", &["0xZZZZ".into()]);
        assert!(result.contains("Error"));
    }

    #[test]
    fn keccak256_missing_arg() {
        let result = execute_utility("keccak256", &[]);
        assert!(result.contains("Error"));
    }

    // --- execute_utility: toChecksumAddress ---

    #[test]
    fn to_checksum_address_well_known() {
        let result = execute_utility(
            "toChecksumAddress",
            &["0xd8da6bf26964af9d7eed9e03e53415d37aa96045".into()],
        );
        assert_eq!(result, "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045");
    }

    #[test]
    fn to_checksum_address_invalid_length() {
        let result = execute_utility("toChecksumAddress", &["0xabcdef".into()]);
        assert!(result.contains("Error"), "expected error for invalid length");
    }

    #[test]
    fn to_checksum_address_missing_arg() {
        let result = execute_utility("toChecksumAddress", &[]);
        assert!(result.contains("Error"));
    }

    // --- execute_utility: isAddress ---

    #[test]
    fn is_address_valid() {
        let result = execute_utility(
            "isAddress",
            &["0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045".into()],
        );
        assert_eq!(result, "true");
    }

    #[test]
    fn is_address_wrong_length() {
        let result = execute_utility("isAddress", &["0xabcdef".into()]);
        assert_eq!(result, "false");
    }

    #[test]
    fn is_address_no_0x_prefix() {
        let result = execute_utility(
            "isAddress",
            &["d8da6bf26964af9d7eed9e03e53415d37aa96045".into()],
        );
        assert_eq!(result, "false");
    }

    #[test]
    fn is_address_missing_arg() {
        let result = execute_utility("isAddress", &[]);
        assert!(result.contains("Error"));
    }

    // --- execute_utility: unknown ---

    #[test]
    fn unknown_utility() {
        let result = execute_utility("nonexistent", &[]);
        assert!(result.contains("Error"));
    }

    // --- is_balanced ---

    #[test]
    fn balanced_braces() {
        assert!(is_balanced("{}"));
    }

    #[test]
    fn balanced_brackets() {
        assert!(is_balanced("[]"));
    }

    #[test]
    fn balanced_nested() {
        assert!(is_balanced("{ [ ] }"));
    }

    #[test]
    fn balanced_empty_string() {
        assert!(is_balanced(""));
    }

    #[test]
    fn balanced_json_object() {
        assert!(is_balanced(r#"{"a": "b"}"#));
    }

    #[test]
    fn unbalanced_open_brace() {
        assert!(!is_balanced("{"));
    }

    #[test]
    fn unbalanced_open_bracket_brace() {
        assert!(!is_balanced("[{"));
    }

    #[test]
    fn unbalanced_close_brace() {
        assert!(!is_balanced("}"));
    }

    #[test]
    fn balanced_brace_inside_string() {
        // The "}" inside the string value should not break balance
        assert!(is_balanced(r#"{"a": "}"}"#));
    }

    #[test]
    fn balanced_escaped_quote_in_string() {
        // Escaped quote inside string should not end the string early
        assert!(is_balanced(r#"{"a": "he said \"hi\""}"#));
    }

    #[test]
    fn balanced_no_delimiters() {
        assert!(is_balanced("eth.getBalance 0xabc"));
    }
}
