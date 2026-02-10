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
            if s.starts_with("0x") && s.len() != 42 && s.len() != 66 {
                if let Some(decimal) = hex_to_decimal(s) {
                    return decimal;
                }
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
    if s.len() <= max_len || max_len < 7 {
        return s.to_string();
    }
    let keep = (max_len - 3) / 2;
    format!("{}...{}", &s[..keep], &s[s.len() - keep..])
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
