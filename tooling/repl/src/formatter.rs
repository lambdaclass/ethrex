use colored::Colorize;
use serde_json::Value;

use crate::commands::CommandDef;

const MAX_VALUE_DISPLAY_LEN: usize = 72;

pub fn format_value(value: &Value) -> String {
    match value {
        Value::Null => "null".dimmed().to_string(),
        Value::Bool(b) => b.to_string().yellow().to_string(),
        Value::Number(n) => n.to_string().green().to_string(),
        Value::String(s) => format_string_value(s),
        Value::Array(arr) => format_array(arr),
        Value::Object(map) => format_object_box(map, ""),
    }
}

fn format_string_value(s: &str) -> String {
    if s.starts_with("0x") {
        if s.len() == 42 {
            // Ethereum address
            s.cyan().to_string()
        } else if s.len() == 66 {
            // Transaction/block hash (32 bytes)
            s.yellow().to_string()
        } else if let Some(decimal) = hex_to_decimal(s) {
            // Hex quantity → show as decimal
            decimal.green().to_string()
        } else {
            // Other hex data (bytecode, etc.)
            truncate_middle(s, MAX_VALUE_DISPLAY_LEN)
                .magenta()
                .to_string()
        }
    } else {
        s.white().to_string()
    }
}

fn format_array(arr: &[Value]) -> String {
    if arr.is_empty() {
        return "[]".to_string();
    }

    if arr.iter().all(|v| v.is_object()) {
        let mut out = String::new();
        for (i, v) in arr.iter().enumerate() {
            if let Value::Object(map) = v {
                out.push_str(&format_object_box(map, &format!(" [{}] ", i)));
            }
            if i < arr.len() - 1 {
                out.push('\n');
            }
        }
        out
    } else {
        let items: Vec<String> = arr
            .iter()
            .map(|v| format!("  {}", format_value(v)))
            .collect();
        format!("[\n{}\n]", items.join(",\n"))
    }
}

fn format_object_box(map: &serde_json::Map<String, Value>, title: &str) -> String {
    if map.is_empty() {
        return "{}".to_string();
    }

    let rows = flatten_object(map, "");

    let key_w = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    let val_w = rows
        .iter()
        .map(|(_, v)| v.len())
        .max()
        .unwrap_or(0)
        .min(MAX_VALUE_DISPLAY_LEN);
    let content_w = key_w + 3 + val_w;
    let box_w = content_w + 4; // "│ " + content + " │"

    let mut out = String::new();

    // Top border
    if title.is_empty() {
        out.push_str(&format!("┌{}┐\n", "─".repeat(box_w - 2)));
    } else {
        let fill = (box_w - 2).saturating_sub(title.len() + 1);
        out.push_str(&format!("┌─{}{}┐\n", title.bold(), "─".repeat(fill)));
    }

    // Rows
    for (key, value) in &rows {
        let display_val = truncate_middle(value, val_w);
        let key_pad = " ".repeat(key_w.saturating_sub(key.len()));
        let val_pad = " ".repeat(val_w.saturating_sub(display_val.len()));
        out.push_str(&format!(
            "│ {}{}   {}{} │\n",
            key_pad,
            key.cyan(),
            colorize_inline(&display_val),
            val_pad,
        ));
    }

    // Bottom border
    out.push_str(&format!("└{}┘", "─".repeat(box_w - 2)));

    out
}

/// Flatten a JSON object into (key, plain-text-value) pairs.
/// Nested objects are expanded with dot-separated keys.
fn flatten_object(map: &serde_json::Map<String, Value>, prefix: &str) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    for (key, value) in map {
        let full_key = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            Value::Object(nested) if !nested.is_empty() => {
                rows.extend(flatten_object(nested, &full_key));
            }
            Value::Array(arr) => {
                let items: Vec<String> = arr.iter().map(inline_value).collect();
                rows.push((full_key, items.join(", ")));
            }
            _ => {
                rows.push((full_key, inline_value(value)));
            }
        }
    }
    rows
}

/// Convert a Value to a plain-text string for table cells.
fn inline_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            // Convert hex quantities to decimal
            if s.starts_with("0x")
                && s.len() != 42
                && s.len() != 66
                && let Some(decimal) = hex_to_decimal(s)
            {
                return decimal;
            }
            s.clone()
        }
        Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(inline_value).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(map) => {
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{k}: {}", inline_value(v)))
                .collect();
            format!("{{{}}}", items.join(", "))
        }
    }
}

/// Apply color to a plain-text value based on its content.
fn colorize_inline(s: &str) -> String {
    if s == "true" || s == "false" {
        s.yellow().to_string()
    } else if s == "null" {
        s.dimmed().to_string()
    } else if s.starts_with("0x") && s.len() == 42 {
        s.cyan().to_string()
    } else if s.starts_with("0x") {
        s.yellow().to_string()
    } else if !s.is_empty() && (s.chars().all(|c| c.is_ascii_digit()) || is_decimal_float(s)) {
        s.green().to_string()
    } else {
        s.to_string()
    }
}

fn is_decimal_float(s: &str) -> bool {
    let mut has_dot = false;
    for c in s.chars() {
        if c == '.' {
            if has_dot {
                return false;
            }
            has_dot = true;
        } else if !c.is_ascii_digit() {
            return false;
        }
    }
    has_dot && s.len() > 1
}

/// Try to parse a 0x-prefixed hex string as a decimal number.
fn hex_to_decimal(s: &str) -> Option<String> {
    let hex = s.strip_prefix("0x")?;
    if hex.is_empty() || hex.len() > 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let n = u128::from_str_radix(hex, 16).ok()?;
    Some(n.to_string())
}

fn truncate_middle(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len || max_len < 7 {
        return s.to_string();
    }
    let keep = (max_len - 3) / 2;
    let start: String = s.chars().take(keep).collect();
    let end: String = s
        .chars()
        .rev()
        .take(keep)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{start}...{end}")
}

pub fn format_error(msg: &str) -> String {
    format!("{} {}", "Error:".red().bold(), msg.red())
}

/// Format a command definition as a usage string: `namespace.method <required> [optional]`
pub fn command_usage(cmd: &CommandDef) -> String {
    let mut usage = format!("{}.{}", cmd.namespace, cmd.name);
    for p in cmd.params {
        if p.required {
            usage.push_str(&format!(" <{}>", p.name));
        } else {
            usage.push_str(&format!(" [{}]", p.name));
        }
    }
    usage
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{ParamDef, ParamType};
    use serde_json::json;

    // --- hex_to_decimal ---

    #[test]
    fn test_hex_to_decimal_zero() {
        assert_eq!(hex_to_decimal("0x0"), Some("0".to_string()));
    }

    #[test]
    fn test_hex_to_decimal_a() {
        assert_eq!(hex_to_decimal("0xa"), Some("10".to_string()));
    }

    #[test]
    fn test_hex_to_decimal_ff() {
        assert_eq!(hex_to_decimal("0xff"), Some("255".to_string()));
    }

    #[test]
    fn test_hex_to_decimal_empty_hex() {
        assert_eq!(hex_to_decimal("0x"), None);
    }

    #[test]
    fn test_hex_to_decimal_non_hex() {
        assert_eq!(hex_to_decimal("0xzz"), None);
    }

    #[test]
    fn test_hex_to_decimal_too_long() {
        let long = format!("0x{}", "f".repeat(33));
        assert_eq!(hex_to_decimal(&long), None);
    }

    #[test]
    fn test_hex_to_decimal_no_prefix() {
        assert_eq!(hex_to_decimal("ff"), None);
    }

    // --- truncate_middle ---

    #[test]
    fn test_truncate_short_string() {
        assert_eq!(truncate_middle("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_exact_max() {
        assert_eq!(truncate_middle("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_longer_than_max() {
        let result = truncate_middle("0123456789abcdef", 10);
        assert!(result.contains("..."));
        assert!(result.len() <= 10);
    }

    #[test]
    fn test_truncate_max_lt_7_unchanged() {
        assert_eq!(truncate_middle("long string here", 6), "long string here");
    }

    // --- inline_value ---

    #[test]
    fn test_inline_null() {
        assert_eq!(inline_value(&json!(null)), "null");
    }

    #[test]
    fn test_inline_bool_true() {
        assert_eq!(inline_value(&json!(true)), "true");
    }

    #[test]
    fn test_inline_bool_false() {
        assert_eq!(inline_value(&json!(false)), "false");
    }

    #[test]
    fn test_inline_number() {
        assert_eq!(inline_value(&json!(42)), "42");
    }

    #[test]
    fn test_inline_hex_string_to_decimal() {
        assert_eq!(inline_value(&json!("0xa")), "10");
    }

    #[test]
    fn test_inline_address_unchanged() {
        let addr = "0x1234567890abcdef1234567890abcdef12345678";
        assert_eq!(inline_value(&json!(addr)), addr);
    }

    #[test]
    fn test_inline_hash_unchanged() {
        let hash = format!("0x{}", "ab".repeat(32));
        assert_eq!(inline_value(&json!(hash)), hash);
    }

    #[test]
    fn test_inline_nested_array() {
        assert_eq!(inline_value(&json!([1, 2])), "[1, 2]");
    }

    #[test]
    fn test_inline_nested_object() {
        let result = inline_value(&json!({"a": 1}));
        assert!(result.contains("a: 1"));
    }

    #[test]
    fn test_inline_plain_string() {
        assert_eq!(inline_value(&json!("hello")), "hello");
    }

    // --- flatten_object ---

    #[test]
    fn test_flatten_simple() {
        let map =
            serde_json::from_str::<serde_json::Map<String, Value>>(r#"{"a":"1","b":"2"}"#).unwrap();
        let rows = flatten_object(&map, "");
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|(k, _)| k == "a"));
        assert!(rows.iter().any(|(k, _)| k == "b"));
    }

    #[test]
    fn test_flatten_nested() {
        let map =
            serde_json::from_str::<serde_json::Map<String, Value>>(r#"{"a":{"b":"1"}}"#).unwrap();
        let rows = flatten_object(&map, "");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "a.b");
        assert_eq!(rows[0].1, "1");
    }

    #[test]
    fn test_flatten_array_values() {
        let map =
            serde_json::from_str::<serde_json::Map<String, Value>>(r#"{"tags":[1,2,3]}"#).unwrap();
        let rows = flatten_object(&map, "");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "tags");
        assert_eq!(rows[0].1, "1, 2, 3");
    }

    #[test]
    fn test_flatten_empty_nested_object() {
        let map = serde_json::from_str::<serde_json::Map<String, Value>>(r#"{"a":{}}"#).unwrap();
        let rows = flatten_object(&map, "");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "a");
        assert_eq!(rows[0].1, "{}");
    }

    #[test]
    fn test_flatten_with_prefix() {
        let map = serde_json::from_str::<serde_json::Map<String, Value>>(r#"{"x":"1"}"#).unwrap();
        let rows = flatten_object(&map, "parent");
        assert_eq!(rows[0].0, "parent.x");
    }

    // --- is_decimal_float ---

    #[test]
    fn test_is_decimal_float_valid() {
        assert!(is_decimal_float("1.5"));
    }

    #[test]
    fn test_is_decimal_float_integer() {
        assert!(!is_decimal_float("1"));
    }

    #[test]
    fn test_is_decimal_float_double_dot() {
        assert!(!is_decimal_float("1.2.3"));
    }

    #[test]
    fn test_is_decimal_float_dot_prefix() {
        assert!(is_decimal_float(".1"));
    }

    #[test]
    fn test_is_decimal_float_empty() {
        assert!(!is_decimal_float(""));
    }

    #[test]
    fn test_is_decimal_float_only_dot() {
        assert!(!is_decimal_float("."));
    }

    // --- command_usage ---

    fn make_params(defs: Vec<ParamDef>) -> &'static [ParamDef] {
        Box::leak(defs.into_boxed_slice())
    }

    #[test]
    fn test_command_usage_required_and_optional() {
        let cmd = CommandDef {
            namespace: "eth",
            name: "getBalance",
            rpc_method: "eth_getBalance",
            params: make_params(vec![
                ParamDef {
                    name: "address",
                    param_type: ParamType::Address,
                    required: true,
                    default_value: None,
                    description: "address",
                },
                ParamDef {
                    name: "block",
                    param_type: ParamType::BlockId,
                    required: false,
                    default_value: Some("latest"),
                    description: "block id",
                },
            ]),
            description: "get balance",
        };
        let usage = command_usage(&cmd);
        assert_eq!(usage, "eth.getBalance <address> [block]");
    }

    #[test]
    fn test_command_usage_no_params() {
        let cmd = CommandDef {
            namespace: "eth",
            name: "blockNumber",
            rpc_method: "eth_blockNumber",
            params: &[],
            description: "block number",
        };
        let usage = command_usage(&cmd);
        assert_eq!(usage, "eth.blockNumber");
    }
}
