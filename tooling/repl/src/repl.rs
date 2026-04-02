use std::sync::Arc;

use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::{Config, Editor};
use serde_json::Value;

use crate::client::RpcClient;
use crate::commands::{CommandDef, CommandRegistry, ParamType};
use crate::completer::ReplHelper;
use crate::ens;
use crate::formatter;
use crate::parser::{self, ParsedCommand, RpcArg};
use crate::variables::VariableStore;

pub struct Repl {
    client: RpcClient,
    authrpc_client: Option<RpcClient>,
    registry: Arc<CommandRegistry>,
    history_path: String,
    variables: VariableStore,
    proof_callback_port: u16,
    proof_callback_timeout: u64,
}

impl Repl {
    pub fn new(
        client: RpcClient,
        authrpc_client: Option<RpcClient>,
        history_path: String,
        proof_callback_port: u16,
        proof_callback_timeout: u64,
    ) -> Self {
        Self {
            client,
            authrpc_client,
            registry: Arc::new(CommandRegistry::new()),
            history_path,
            variables: VariableStore::new(),
            proof_callback_port,
            proof_callback_timeout,
        }
    }

    pub async fn run(&mut self) {
        let config = Config::builder().auto_add_history(true).build();

        let helper = ReplHelper::new(Arc::clone(&self.registry), self.variables.clone());
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
                    let should_exit = if !multiline_buffer.is_empty() {
                        multiline_buffer.push(' ');
                        multiline_buffer.push_str(line);
                        if !is_balanced(&multiline_buffer) {
                            continue;
                        }
                        let full_input = std::mem::take(&mut multiline_buffer);
                        self.execute_input(&full_input).await
                    } else if !is_balanced(line) {
                        multiline_buffer = line.to_string();
                        continue;
                    } else {
                        self.execute_input(line).await
                    };
                    if should_exit {
                        break;
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
    /// Does not store variables (one-shot execution).
    pub async fn execute_command(&self, input: &str) -> String {
        match parser::parse(input) {
            Ok(cmd) => match cmd {
                ParsedCommand::RpcCall {
                    namespace,
                    method,
                    args,
                } => match self.resolve_var_args(&args) {
                    Ok(resolved) => self.execute_rpc(&namespace, &method, &resolved).await,
                    Err(e) => formatter::format_error(&e),
                },
                ParsedCommand::Assignment { .. } | ParsedCommand::PrintVar { .. } => {
                    formatter::format_error("variables not available in one-shot mode (-x)")
                }
                ParsedCommand::UtilityCall { name, args } => execute_utility(&name, &args),
                ParsedCommand::BuiltinCommand { name, .. } => {
                    format!("Built-in command .{name} not available in non-interactive mode")
                }
                ParsedCommand::Empty => String::new(),
            },
            Err(e) => formatter::format_error(&e.to_string()),
        }
    }

    /// Execute a single input line. Returns `true` if the REPL should exit.
    async fn execute_input(&mut self, input: &str) -> bool {
        let parsed = match parser::parse(input) {
            Ok(p) => p,
            Err(e) => {
                println!("{}", formatter::format_error(&e.to_string()));
                return false;
            }
        };

        match parsed {
            ParsedCommand::Empty => {}
            ParsedCommand::RpcCall {
                namespace,
                method,
                args,
            } => {
                let resolved = match self.resolve_var_args(&args) {
                    Ok(r) => r,
                    Err(e) => {
                        println!("{}", formatter::format_error(&e));
                        return false;
                    }
                };
                let result = self.execute_rpc(&namespace, &method, &resolved).await;
                println!("{result}");
            }
            ParsedCommand::Assignment { var_name, command } => {
                let result = self.execute_assignment(&var_name, &command).await;
                println!("{result}");
            }
            ParsedCommand::PrintVar { name, path, offset } => {
                match self.resolve_var_ref(&name, &path) {
                    Ok(value) => {
                        let result = match offset {
                            Some(off) => apply_offset(&value, off),
                            None => Ok(value),
                        };
                        match result {
                            Ok(v) => println!("{}", formatter::format_value(&v)),
                            Err(e) => println!("{}", formatter::format_error(&e)),
                        }
                    }
                    Err(e) => println!("{}", formatter::format_error(&e)),
                }
            }
            ParsedCommand::BuiltinCommand { name, args } => {
                if self.execute_builtin(&name, &args) {
                    return true;
                }
            }
            ParsedCommand::UtilityCall { name, args } => {
                let result = execute_utility(&name, &args);
                println!("{result}");
            }
        }
        false
    }

    /// Resolve variable references in RPC arguments to concrete JSON values.
    /// Handles both top-level `$var.path` tokens and `$var.path` strings
    /// embedded inside JSON objects/arrays.
    fn resolve_var_args(&self, args: &[RpcArg]) -> Result<Vec<Value>, String> {
        args.iter()
            .map(|arg| match arg {
                RpcArg::Literal(v) => self.resolve_vars_in_value(v),
                RpcArg::VarRef { name, path, offset } => {
                    let value = self.resolve_var_ref(name, path)?;
                    match offset {
                        Some(off) => apply_offset(&value, *off),
                        None => Ok(value),
                    }
                }
            })
            .collect()
    }

    /// Resolve a single `$name.path` variable reference.
    fn resolve_var_ref(&self, name: &str, path: &[String]) -> Result<Value, String> {
        let value = self
            .variables
            .get(name)
            .ok_or_else(|| format!("undefined variable: {name}"))?;
        let mut current = value;
        for field in path {
            current = current
                .get(field)
                .ok_or_else(|| format!("field '{field}' not found in variable '{name}'"))?
                .clone();
        }
        Ok(current)
    }

    /// Walk a JSON value tree and resolve any string values that look like
    /// `$var` or `$var.field.nested`.
    fn resolve_vars_in_value(&self, value: &Value) -> Result<Value, String> {
        match value {
            Value::String(s) if s.starts_with('$') => {
                let ref_str = &s[1..]; // strip '$'
                let parts: Vec<&str> = ref_str.split('.').collect();
                let name = parts[0];
                let path: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
                self.resolve_var_ref(name, &path)
            }
            Value::Object(map) => {
                let mut resolved = serde_json::Map::new();
                for (k, v) in map {
                    resolved.insert(k.clone(), self.resolve_vars_in_value(v)?);
                }
                Ok(Value::Object(resolved))
            }
            Value::Array(arr) => {
                let resolved: Result<Vec<Value>, String> =
                    arr.iter().map(|v| self.resolve_vars_in_value(v)).collect();
                Ok(Value::Array(resolved?))
            }
            other => Ok(other.clone()),
        }
    }

    /// Execute an assignment: run the inner command and store the result.
    async fn execute_assignment(&mut self, var_name: &str, command: &ParsedCommand) -> String {
        match command {
            ParsedCommand::RpcCall {
                namespace,
                method,
                args,
            } => {
                let resolved = match self.resolve_var_args(args) {
                    Ok(r) => r,
                    Err(e) => return formatter::format_error(&e),
                };
                match self.call_rpc_raw(namespace, method, &resolved).await {
                    Ok((result, cmd)) => {
                        let formatted = formatter::format_value(&result);
                        let is_request_proofs = cmd.rpc_method == "engine_requestProofsV1";
                        self.variables.insert(var_name.to_string(), result);

                        // After a successful requestProofsV1, spawn a one-shot
                        // HTTP listener to receive the GeneratedProof callback.
                        if is_request_proofs {
                            let port = self.proof_callback_port;
                            let timeout = self.proof_callback_timeout;
                            crate::proof_callback::spawn_listener(
                                port,
                                timeout,
                                self.variables.clone(),
                            );
                            format!(
                                "{formatted}\nListening for proof callback on port {port} (timeout: {timeout}s)..."
                            )
                        } else {
                            formatted
                        }
                    }
                    Err(e) => formatter::format_error(&e),
                }
            }
            ParsedCommand::UtilityCall { name, args } => {
                let result_str = execute_utility(name, args);
                // Try to parse as JSON value for storage; fall back to string
                let value: Value = serde_json::from_str(&result_str)
                    .unwrap_or_else(|_| Value::String(result_str.clone()));
                self.variables.insert(var_name.to_string(), value);
                result_str
            }
            ParsedCommand::PrintVar { name, path, offset } => {
                match self.resolve_var_ref(name, path) {
                    Ok(value) => {
                        let result = match offset {
                            Some(off) => apply_offset(&value, *off),
                            None => Ok(value),
                        };
                        match result {
                            Ok(v) => {
                                let formatted = formatter::format_value(&v);
                                self.variables.insert(var_name.to_string(), v);
                                formatted
                            }
                            Err(e) => formatter::format_error(&e),
                        }
                    }
                    Err(e) => formatter::format_error(&e),
                }
            }
            _ => formatter::format_error(
                "can only assign from RPC calls, utility functions, or variable expressions",
            ),
        }
    }

    /// Select the appropriate RPC client based on namespace.
    fn client_for_namespace(&self, namespace: &str) -> Result<&RpcClient, String> {
        if namespace == "engine" {
            self.authrpc_client
                .as_ref()
                .ok_or_else(|| "engine namespace requires --authrpc.jwtsecret".to_string())
        } else {
            Ok(&self.client)
        }
    }

    /// Send an RPC request and return the raw result together with the resolved
    /// `CommandDef`.  Both `execute_rpc` and `execute_assignment` use this so
    /// that the shared plumbing (client selection, registry lookup, ENS
    /// resolution, param building, network call) lives in one place.
    async fn call_rpc_raw<'a>(
        &'a self,
        namespace: &str,
        method: &str,
        args: &[Value],
    ) -> Result<(Value, &'a CommandDef), String> {
        let client = self.client_for_namespace(namespace)?;

        let cmd = self
            .registry
            .find(namespace, method)
            .ok_or_else(|| format!("unknown command: {namespace}.{method}"))?;

        let resolved_args = self.resolve_ens_in_args(cmd, args).await?;

        let params = cmd
            .build_params(&resolved_args)
            .map_err(|e| format!("{e}\nUsage: {}", formatter::command_usage(cmd)))?;

        let result = client
            .send_request(cmd.rpc_method, params)
            .await
            .map_err(|e| e.to_string())?;

        Ok((result, cmd))
    }

    async fn execute_rpc(&self, namespace: &str, method: &str, args: &[Value]) -> String {
        match self.call_rpc_raw(namespace, method, args).await {
            Ok((result, _cmd)) => formatter::format_value(&result),
            Err(e) => formatter::format_error(&e),
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

    /// Execute a built-in command. Returns `true` if the REPL should exit.
    fn execute_builtin(&self, name: &str, args: &[String]) -> bool {
        match name {
            "help" => self.show_help(args),
            "exit" | "quit" => {
                println!("Bye!");
                return true;
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
            "vars" => {
                let entries = self.variables.entries();
                if entries.is_empty() {
                    println!("No variables stored");
                } else {
                    for (name, value) in &entries {
                        let s = value.to_string();
                        let preview = if s.len() > 80 {
                            format!("{}...", &s[..77])
                        } else {
                            s
                        };
                        println!("  {name} = {preview}");
                    }
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
        false
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
            println!("  .vars                      List stored variables");
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

/// Apply an arithmetic offset to a JSON value.
///
/// Handles:
/// - Hex strings (`"0x1a"` + 1 → `"0x1b"`)
/// - Decimal strings (`"100"` + 1 → `"101"`)
/// - JSON numbers (`100` + 1 → `101`)
fn apply_offset(value: &Value, offset: i64) -> Result<Value, String> {
    let n: i64 = match value {
        Value::String(s) => {
            if let Some(hex) = s.strip_prefix("0x") {
                i64::from_str_radix(hex, 16)
                    .map_err(|_| format!("cannot apply arithmetic to non-numeric value: {s}"))?
            } else {
                s.parse()
                    .map_err(|_| format!("cannot apply arithmetic to non-numeric value: {s}"))?
            }
        }
        Value::Number(n) => n
            .as_i64()
            .ok_or_else(|| "cannot apply arithmetic to non-integer number".to_string())?,
        other => {
            return Err(format!(
                "cannot apply arithmetic to {}",
                match other {
                    Value::Object(_) => "object",
                    Value::Array(_) => "array",
                    Value::Bool(_) => "boolean",
                    Value::Null => "null",
                    _ => "value",
                }
            ));
        }
    };

    let result = n
        .checked_add(offset)
        .ok_or_else(|| "arithmetic overflow".to_string())?;

    // Preserve the original format: hex in → hex out, decimal in → decimal out
    match value {
        Value::String(s) if s.starts_with("0x") => Ok(Value::String(format!("0x{result:x}"))),
        Value::Number(_) => Ok(Value::Number(result.into())),
        _ => Ok(Value::String(result.to_string())),
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
            let decimals = match unit_decimals(&args[1]) {
                Ok(d) => d,
                Err(e) => return formatter::format_error(&e),
            };
            match parse_wei_amount(&args[0], decimals) {
                Ok(wei) => format!("{wei}"),
                Err(e) => formatter::format_error(&e),
            }
        }
        "fromWei" => {
            if args.len() < 2 {
                return formatter::format_error(
                    "Usage: fromWei <amount> <unit>\nUnits: wei, gwei, ether",
                );
            }
            let wei: u128 = match args[0].parse() {
                Ok(v) => v,
                Err(_) => return formatter::format_error(&format!("invalid number: {}", args[0])),
            };
            let decimals = match unit_decimals(&args[1]) {
                Ok(d) => d,
                Err(e) => return formatter::format_error(&e),
            };
            format_from_wei(wei, decimals)
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
                    return formatter::format_error(&format!("invalid hex data: {}", args[0]));
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

fn unit_decimals(unit: &str) -> Result<u32, String> {
    match unit.to_lowercase().as_str() {
        "wei" => Ok(0),
        "gwei" => Ok(9),
        "ether" | "eth" => Ok(18),
        other => Err(format!("unknown unit: {other}. Use: wei, gwei, ether")),
    }
}

/// Parse an amount string (possibly with decimals) and a unit's decimal count into wei.
/// Uses pure integer arithmetic to avoid f64 precision loss.
fn parse_wei_amount(amount: &str, decimals: u32) -> Result<u128, String> {
    let parts: Vec<&str> = amount.split('.').collect();
    if parts.len() > 2 {
        return Err(format!("invalid number: {amount}"));
    }

    let whole: u128 = if parts[0].is_empty() {
        0
    } else {
        parts[0]
            .parse()
            .map_err(|_| format!("invalid number: {amount}"))?
    };

    let multiplier = 10u128.pow(decimals);
    let whole_wei = whole
        .checked_mul(multiplier)
        .ok_or_else(|| format!("value too large: {amount}"))?;

    if parts.len() == 1 || parts[1].is_empty() {
        return Ok(whole_wei);
    }

    let frac_str = parts[1];
    if frac_str.len() > decimals as usize {
        return Err(format!("too many decimal places for unit (max {decimals})"));
    }

    let padded = format!("{frac_str:0<width$}", width = decimals as usize);
    let frac_wei: u128 = padded
        .parse()
        .map_err(|_| format!("invalid number: {amount}"))?;

    whole_wei
        .checked_add(frac_wei)
        .ok_or_else(|| format!("value too large: {amount}"))
}

/// Format a wei amount into a human-readable string with the given unit's decimal count.
/// Uses pure integer arithmetic to avoid f64 precision loss.
fn format_from_wei(wei: u128, decimals: u32) -> String {
    if decimals == 0 {
        return wei.to_string();
    }
    let divisor = 10u128.pow(decimals);
    let whole = wei / divisor;
    let frac = wei % divisor;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{frac:0>width$}", width = decimals as usize);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{whole}.{trimmed}")
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
            '}' => {
                brace_depth -= 1;
                if brace_depth < 0 {
                    return false;
                }
            }
            '[' => bracket_depth += 1,
            ']' => {
                bracket_depth -= 1;
                if bracket_depth < 0 {
                    return false;
                }
            }
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
    fn to_wei_fractional_ether() {
        let result = execute_utility("toWei", &["1.1".into(), "ether".into()]);
        assert_eq!(result, "1100000000000000000");
    }

    #[test]
    fn to_wei_fractional_gwei() {
        let result = execute_utility("toWei", &["0.5".into(), "gwei".into()]);
        assert_eq!(result, "500000000");
    }

    #[test]
    fn from_wei_fractional_ether() {
        let result = execute_utility("fromWei", &["1100000000000000000".into(), "ether".into()]);
        assert_eq!(result, "1.1");
    }

    #[test]
    fn to_wei_unknown_unit() {
        let result = execute_utility("toWei", &["1".into(), "finney".into()]);
        assert!(
            result.contains("Error"),
            "expected error for unknown unit, got: {result}"
        );
    }

    #[test]
    fn to_wei_missing_args() {
        let result = execute_utility("toWei", &["1".into()]);
        assert!(result.contains("Error"), "expected error for missing args");
    }

    #[test]
    fn to_wei_invalid_number() {
        let result = execute_utility("toWei", &["abc".into(), "ether".into()]);
        assert!(
            result.contains("Error"),
            "expected error for invalid number"
        );
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
        assert!(
            result.contains("Error"),
            "expected error for invalid length"
        );
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
