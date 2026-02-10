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
