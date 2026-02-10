mod admin;
mod debug;
mod eth;
mod net;
mod txpool;
mod web3;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamType {
    Address,
    BlockId,
    Hash,
    HexData,
    Uint,
    Bool,
    Object,
    Array,
    StringParam,
}

#[derive(Debug, Clone)]
pub struct ParamDef {
    pub name: &'static str,
    pub param_type: ParamType,
    pub required: bool,
    pub default_value: Option<&'static str>,
    pub description: &'static str,
}

#[derive(Debug, Clone)]
pub struct CommandDef {
    pub namespace: &'static str,
    pub name: &'static str,
    pub rpc_method: &'static str,
    pub params: &'static [ParamDef],
    pub description: &'static str,
}

impl CommandDef {
    pub fn full_name(&self) -> String {
        format!("{}.{}", self.namespace, self.name)
    }

    pub fn usage(&self) -> String {
        let params: Vec<String> = self
            .params
            .iter()
            .map(|p| {
                if p.required {
                    format!("<{}>", p.name)
                } else if let Some(def) = p.default_value {
                    format!("[{}={}]", p.name, def)
                } else {
                    format!("[{}]", p.name)
                }
            })
            .collect();
        format!("{}.{} {}", self.namespace, self.name, params.join(" "))
    }

    pub fn build_params(&self, args: &[Value]) -> Result<Vec<Value>, String> {
        let required_count = self.params.iter().filter(|p| p.required).count();

        if args.len() < required_count {
            return Err(format!(
                "{} requires at least {} argument(s), got {}",
                self.rpc_method,
                required_count,
                args.len()
            ));
        }

        if args.len() > self.params.len() {
            return Err(format!(
                "{} accepts at most {} argument(s), got {}",
                self.rpc_method,
                self.params.len(),
                args.len()
            ));
        }

        let mut result = Vec::with_capacity(self.params.len());

        for (i, param_def) in self.params.iter().enumerate() {
            let value = if let Some(arg) = args.get(i) {
                validate_and_convert(arg, param_def)?
            } else if let Some(default) = param_def.default_value {
                Value::String(default.to_string())
            } else {
                // Optional param with no default and no value provided — stop here
                break;
            };
            result.push(value);
        }

        Ok(result)
    }
}

fn validate_and_convert(value: &Value, param_def: &ParamDef) -> Result<Value, String> {
    match param_def.param_type {
        ParamType::Address => {
            let s = value_as_str(value)?;
            if !is_valid_address(&s) {
                return Err(format!(
                    "'{}': expected a 0x-prefixed 20-byte hex address",
                    param_def.name
                ));
            }
            Ok(Value::String(s))
        }
        ParamType::Hash => {
            let s = value_as_str(value)?;
            if !is_valid_hash(&s) {
                return Err(format!(
                    "'{}': expected a 0x-prefixed 32-byte hex hash",
                    param_def.name
                ));
            }
            Ok(Value::String(s))
        }
        ParamType::BlockId => {
            let s = value_as_str(value)?;
            Ok(Value::String(normalize_block_id(&s)))
        }
        ParamType::HexData => {
            let s = value_as_str(value)?;
            if !s.starts_with("0x") {
                return Err(format!(
                    "'{}': expected 0x-prefixed hex data",
                    param_def.name
                ));
            }
            Ok(Value::String(s))
        }
        ParamType::Uint => {
            let s = value_as_str(value)?;
            Ok(Value::String(normalize_uint(&s)?))
        }
        ParamType::Bool => match value {
            Value::Bool(b) => Ok(Value::Bool(*b)),
            Value::String(s) => match s.as_str() {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                _ => Err(format!("'{}': expected true or false", param_def.name)),
            },
            _ => Err(format!("'{}': expected a boolean", param_def.name)),
        },
        ParamType::Object => match value {
            Value::Object(_) => Ok(value.clone()),
            Value::String(s) => serde_json::from_str(s)
                .map_err(|e| format!("'{}': invalid JSON object: {}", param_def.name, e)),
            _ => Err(format!("'{}': expected a JSON object", param_def.name)),
        },
        ParamType::Array => match value {
            Value::Array(_) => Ok(value.clone()),
            Value::String(s) => serde_json::from_str(s)
                .map_err(|e| format!("'{}': invalid JSON array: {}", param_def.name, e)),
            _ => Err(format!("'{}': expected a JSON array", param_def.name)),
        },
        ParamType::StringParam => {
            let s = value_as_str(value)?;
            Ok(Value::String(s))
        }
    }
}

fn value_as_str(value: &Value) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        _ => Ok(value.to_string()),
    }
}

fn is_valid_address(s: &str) -> bool {
    s.starts_with("0x") && s.len() == 42 && s[2..].chars().all(|c| c.is_ascii_hexdigit())
}

fn is_valid_hash(s: &str) -> bool {
    s.starts_with("0x") && s.len() == 66 && s[2..].chars().all(|c| c.is_ascii_hexdigit())
}

fn normalize_block_id(s: &str) -> String {
    match s {
        "latest" | "earliest" | "pending" | "finalized" | "safe" => s.to_string(),
        _ if s.starts_with("0x") => s.to_string(),
        _ => {
            // Try parsing as decimal and converting to hex
            if let Ok(n) = s.parse::<u64>() {
                format!("0x{n:x}")
            } else {
                s.to_string()
            }
        }
    }
}

fn normalize_uint(s: &str) -> Result<String, String> {
    if s.starts_with("0x") {
        // Already hex
        Ok(s.to_string())
    } else if let Ok(n) = s.parse::<u64>() {
        Ok(format!("0x{n:x}"))
    } else {
        Err(format!("invalid uint: {s}"))
    }
}

pub struct CommandRegistry {
    commands: Vec<CommandDef>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        let mut commands = Vec::new();
        commands.extend(eth::commands());
        commands.extend(debug::commands());
        commands.extend(admin::commands());
        commands.extend(net::commands());
        commands.extend(web3::commands());
        commands.extend(txpool::commands());
        Self { commands }
    }

    pub fn find(&self, namespace: &str, method: &str) -> Option<&CommandDef> {
        self.commands
            .iter()
            .find(|c| c.namespace == namespace && c.name == method)
    }

    pub fn namespaces(&self) -> Vec<&str> {
        let mut ns: Vec<&str> = self.commands.iter().map(|c| c.namespace).collect();
        ns.sort();
        ns.dedup();
        ns
    }

    pub fn methods_in_namespace(&self, namespace: &str) -> Vec<&CommandDef> {
        self.commands
            .iter()
            .filter(|c| c.namespace == namespace)
            .collect()
    }

    pub fn all_commands(&self) -> &[CommandDef] {
        &self.commands
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_params(defs: Vec<ParamDef>) -> &'static [ParamDef] {
        Box::leak(defs.into_boxed_slice())
    }

    fn make_cmd(params: &'static [ParamDef]) -> CommandDef {
        CommandDef {
            namespace: "test",
            name: "test",
            rpc_method: "test_test",
            params,
            description: "test command",
        }
    }

    /// Create a required ParamDef with the given name and type. Reduces repetition
    /// in tests that only vary these two fields.
    fn required_param(name: &'static str, param_type: ParamType) -> ParamDef {
        ParamDef {
            name,
            param_type,
            required: true,
            default_value: None,
            description: name,
        }
    }

    /// Shorthand: create a CommandDef with a single required param.
    fn cmd_with_param(name: &'static str, param_type: ParamType) -> CommandDef {
        make_cmd(make_params(vec![required_param(name, param_type)]))
    }

    // --- build_params ---

    #[test]
    fn test_build_params_correct_number() {
        let cmd = cmd_with_param("addr", ParamType::Address);
        let result = cmd
            .build_params(&[json!("0x1234567890abcdef1234567890abcdef12345678")])
            .unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_build_params_too_few_required() {
        let cmd = cmd_with_param("addr", ParamType::Address);
        let err = cmd.build_params(&[]).unwrap_err();
        assert!(err.contains("requires at least 1"));
    }

    #[test]
    fn test_build_params_too_many_args() {
        let cmd = cmd_with_param("addr", ParamType::Address);
        let err = cmd
            .build_params(&[
                json!("0x1234567890abcdef1234567890abcdef12345678"),
                json!("extra"),
            ])
            .unwrap_err();
        assert!(err.contains("accepts at most 1"));
    }

    #[test]
    fn test_build_params_optional_with_default() {
        let cmd = make_cmd(make_params(vec![
            required_param("addr", ParamType::Address),
            ParamDef {
                name: "block",
                param_type: ParamType::BlockId,
                required: false,
                default_value: Some("latest"),
                description: "block id",
            },
        ]));
        let result = cmd
            .build_params(&[json!("0x1234567890abcdef1234567890abcdef12345678")])
            .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[1], json!("latest"));
    }

    #[test]
    fn test_build_params_optional_without_default_stops() {
        let cmd = make_cmd(make_params(vec![
            required_param("addr", ParamType::Address),
            ParamDef {
                name: "extra",
                param_type: ParamType::StringParam,
                required: false,
                default_value: None,
                description: "extra",
            },
        ]));
        let result = cmd
            .build_params(&[json!("0x1234567890abcdef1234567890abcdef12345678")])
            .unwrap();
        assert_eq!(result.len(), 1);
    }

    // --- validate_and_convert via build_params ---

    #[test]
    fn test_address_valid() {
        let cmd = cmd_with_param("addr", ParamType::Address);
        let result = cmd
            .build_params(&[json!("0x1234567890abcdef1234567890abcdef12345678")])
            .unwrap();
        assert_eq!(
            result[0],
            json!("0x1234567890abcdef1234567890abcdef12345678")
        );
    }

    #[test]
    fn test_address_invalid_length() {
        let cmd = cmd_with_param("addr", ParamType::Address);
        let err = cmd.build_params(&[json!("0x1234")]).unwrap_err();
        assert!(err.contains("20-byte hex address"));
    }

    #[test]
    fn test_address_no_0x_prefix() {
        let cmd = cmd_with_param("addr", ParamType::Address);
        let err = cmd
            .build_params(&[json!("1234567890abcdef1234567890abcdef12345678")])
            .unwrap_err();
        assert!(err.contains("20-byte hex address"));
    }

    #[test]
    fn test_hash_valid() {
        let cmd = cmd_with_param("hash", ParamType::Hash);
        let valid_hash = "0x".to_string() + &"ab".repeat(32);
        let result = cmd.build_params(&[json!(valid_hash)]).unwrap();
        assert_eq!(result[0], json!(valid_hash));
    }

    #[test]
    fn test_hash_invalid() {
        let cmd = cmd_with_param("hash", ParamType::Hash);
        let err = cmd.build_params(&[json!("0x1234")]).unwrap_err();
        assert!(err.contains("32-byte hex hash"));
    }

    #[test]
    fn test_block_id_named_tags() {
        let cmd = cmd_with_param("block", ParamType::BlockId);
        for tag in &["latest", "earliest", "pending", "finalized", "safe"] {
            let result = cmd.build_params(&[json!(tag)]).unwrap();
            assert_eq!(result[0], json!(tag));
        }
    }

    #[test]
    fn test_block_id_decimal_to_hex() {
        let cmd = cmd_with_param("block", ParamType::BlockId);
        let result = cmd.build_params(&[json!("100")]).unwrap();
        assert_eq!(result[0], json!("0x64"));
    }

    #[test]
    fn test_block_id_hex_passthrough() {
        let cmd = cmd_with_param("block", ParamType::BlockId);
        let result = cmd.build_params(&[json!("0x64")]).unwrap();
        assert_eq!(result[0], json!("0x64"));
    }

    #[test]
    fn test_hex_data_valid() {
        let cmd = cmd_with_param("data", ParamType::HexData);
        let result = cmd.build_params(&[json!("0xdeadbeef")]).unwrap();
        assert_eq!(result[0], json!("0xdeadbeef"));
    }

    #[test]
    fn test_hex_data_missing_prefix() {
        let cmd = cmd_with_param("data", ParamType::HexData);
        let err = cmd.build_params(&[json!("deadbeef")]).unwrap_err();
        assert!(err.contains("0x-prefixed hex data"));
    }

    #[test]
    fn test_uint_decimal_to_hex() {
        let cmd = cmd_with_param("val", ParamType::Uint);
        let result = cmd.build_params(&[json!("255")]).unwrap();
        assert_eq!(result[0], json!("0xff"));
    }

    #[test]
    fn test_uint_hex_passthrough() {
        let cmd = cmd_with_param("val", ParamType::Uint);
        let result = cmd.build_params(&[json!("0xff")]).unwrap();
        assert_eq!(result[0], json!("0xff"));
    }

    #[test]
    fn test_uint_invalid() {
        let cmd = cmd_with_param("val", ParamType::Uint);
        let err = cmd.build_params(&[json!("abc")]).unwrap_err();
        assert!(err.contains("invalid uint"));
    }

    #[test]
    fn test_bool_true_string() {
        let cmd = cmd_with_param("flag", ParamType::Bool);
        let result = cmd.build_params(&[json!("true")]).unwrap();
        assert_eq!(result[0], json!(true));
    }

    #[test]
    fn test_bool_false_string() {
        let cmd = cmd_with_param("flag", ParamType::Bool);
        let result = cmd.build_params(&[json!("false")]).unwrap();
        assert_eq!(result[0], json!(false));
    }

    #[test]
    fn test_bool_actual_bool() {
        let cmd = cmd_with_param("flag", ParamType::Bool);
        let result = cmd.build_params(&[json!(true)]).unwrap();
        assert_eq!(result[0], json!(true));
    }

    #[test]
    fn test_bool_invalid() {
        let cmd = cmd_with_param("flag", ParamType::Bool);
        let err = cmd.build_params(&[json!("maybe")]).unwrap_err();
        assert!(err.contains("expected true or false"));
    }

    #[test]
    fn test_object_from_json_string() {
        let cmd = cmd_with_param("obj", ParamType::Object);
        let result = cmd
            .build_params(&[json!(r#"{"to":"0xabc"}"#)])
            .unwrap();
        assert!(result[0].is_object());
        assert_eq!(result[0]["to"], json!("0xabc"));
    }

    #[test]
    fn test_object_passthrough() {
        let cmd = cmd_with_param("obj", ParamType::Object);
        let obj = json!({"to": "0xabc"});
        let result = cmd.build_params(&[obj.clone()]).unwrap();
        assert_eq!(result[0], obj);
    }

    #[test]
    fn test_object_invalid_json() {
        let cmd = cmd_with_param("obj", ParamType::Object);
        let err = cmd.build_params(&[json!("not json")]).unwrap_err();
        assert!(err.contains("invalid JSON object"));
    }

    #[test]
    fn test_array_from_json_string() {
        let cmd = cmd_with_param("arr", ParamType::Array);
        let result = cmd.build_params(&[json!(r#"[1, 2, 3]"#)]).unwrap();
        assert!(result[0].is_array());
    }

    #[test]
    fn test_array_passthrough() {
        let cmd = cmd_with_param("arr", ParamType::Array);
        let arr = json!([1, 2, 3]);
        let result = cmd.build_params(&[arr.clone()]).unwrap();
        assert_eq!(result[0], arr);
    }

    #[test]
    fn test_string_param_any_value() {
        let cmd = cmd_with_param("s", ParamType::StringParam);
        let result = cmd.build_params(&[json!("anything")]).unwrap();
        assert_eq!(result[0], json!("anything"));
    }

    #[test]
    fn test_string_param_number_converted() {
        let cmd = cmd_with_param("s", ParamType::StringParam);
        let result = cmd.build_params(&[json!(42)]).unwrap();
        assert_eq!(result[0], json!("42"));
    }

    // --- CommandRegistry ---

    #[test]
    fn test_registry_find_known_command() {
        let registry = CommandRegistry::new();
        let cmd = registry.find("eth", "blockNumber");
        assert!(cmd.is_some());
        assert_eq!(cmd.unwrap().rpc_method, "eth_blockNumber");
    }

    #[test]
    fn test_registry_find_unknown_command() {
        let registry = CommandRegistry::new();
        assert!(registry.find("eth", "nonExistentMethod").is_none());
    }

    #[test]
    fn test_registry_namespaces() {
        let registry = CommandRegistry::new();
        let ns = registry.namespaces();
        assert!(ns.contains(&"eth"));
        assert!(ns.contains(&"net"));
        assert!(ns.contains(&"web3"));
    }

    #[test]
    fn test_registry_methods_in_namespace() {
        let registry = CommandRegistry::new();
        let methods = registry.methods_in_namespace("eth");
        assert!(methods.len() > 1);
        assert!(methods.iter().all(|c| c.namespace == "eth"));
    }

    #[test]
    fn test_registry_all_commands_non_empty() {
        let registry = CommandRegistry::new();
        assert!(!registry.all_commands().is_empty());
    }

    // --- CommandDef helpers ---

    #[test]
    fn test_full_name() {
        let cmd = make_cmd(&[]);
        assert_eq!(cmd.full_name(), "test.test");
    }

    #[test]
    fn test_usage_format() {
        let cmd = make_cmd(make_params(vec![
            required_param("addr", ParamType::Address),
            ParamDef {
                name: "block",
                param_type: ParamType::BlockId,
                required: false,
                default_value: Some("latest"),
                description: "block id",
            },
        ]));
        let usage = cmd.usage();
        assert!(usage.contains("<addr>"));
        assert!(usage.contains("[block=latest]"));
    }

    #[test]
    fn test_usage_optional_no_default() {
        let cmd = make_cmd(make_params(vec![ParamDef {
            name: "extra",
            param_type: ParamType::StringParam,
            required: false,
            default_value: None,
            description: "extra",
        }]));
        let usage = cmd.usage();
        assert!(usage.contains("[extra]"));
        assert!(!usage.contains("="));
    }
}
