use std::sync::Arc;

use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::{Config, Editor};

use crate::client::RpcClient;
use crate::commands::CommandRegistry;
use crate::completer::ReplHelper;
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

        let params = match cmd.build_params(args) {
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
            let addr = args[0]
                .strip_prefix("0x")
                .unwrap_or(&args[0])
                .to_lowercase();
            if addr.len() != 40 {
                return formatter::format_error("invalid address length");
            }
            use sha3::{Digest, Keccak256};
            let hash = Keccak256::digest(addr.as_bytes());
            let hash_hex = hex::encode(hash);
            let mut checksummed = String::from("0x");
            for (i, c) in addr.chars().enumerate() {
                if c.is_ascii_alphabetic() {
                    let hash_nibble =
                        u8::from_str_radix(&hash_hex[i..i + 1], 16).unwrap_or(0);
                    if hash_nibble >= 8 {
                        checksummed.push(c.to_ascii_uppercase());
                    } else {
                        checksummed.push(c);
                    }
                } else {
                    checksummed.push(c);
                }
            }
            checksummed
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
