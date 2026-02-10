use serde_json::Value;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("unexpected end of input")]
    UnexpectedEof,
    #[error("unexpected character: '{0}'")]
    UnexpectedChar(char),
    #[error("unterminated string")]
    UnterminatedString,
    #[error("unterminated JSON")]
    UnterminatedJson,
    #[error("invalid JSON: {0}")]
    InvalidJson(String),
    #[error("expected method name after '.'")]
    ExpectedMethod,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Ident(String),
    Dot,
    LParen,
    RParen,
    Comma,
    String(String),
    Number(String),
    Bool(bool),
    JsonObject(String),
    JsonArray(String),
    Colon,
}

const UTILITY_NAMES: &[&str] = &[
    "toWei",
    "fromWei",
    "toHex",
    "fromHex",
    "keccak256",
    "toChecksumAddress",
    "isAddress",
];

#[derive(Debug, Clone)]
pub enum ParsedCommand {
    RpcCall {
        namespace: String,
        method: String,
        args: Vec<Value>,
    },
    BuiltinCommand {
        name: String,
        args: Vec<String>,
    },
    UtilityCall {
        name: String,
        args: Vec<String>,
    },
    Empty,
}

struct Tokenizer<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let ch = self.input.get(self.pos).copied()?;
        self.pos += 1;
        Some(ch)
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == b' ' || ch == b'\t' || ch == b'\r' || ch == b'\n' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn read_string(&mut self, quote: u8) -> Result<String, ParseError> {
        let mut s = Vec::new();
        loop {
            match self.advance() {
                None => return Err(ParseError::UnterminatedString),
                Some(ch) if ch == quote => return Ok(String::from_utf8_lossy(&s).into_owned()),
                Some(b'\\') => match self.advance() {
                    None => return Err(ParseError::UnterminatedString),
                    Some(b'n') => s.push(b'\n'),
                    Some(b't') => s.push(b'\t'),
                    Some(b'\\') => s.push(b'\\'),
                    Some(ch) if ch == quote => s.push(quote),
                    Some(ch) => {
                        s.push(b'\\');
                        s.push(ch);
                    }
                },
                Some(ch) => s.push(ch),
            }
        }
    }

    fn read_json_block(&mut self, open: u8, close: u8) -> Result<String, ParseError> {
        let start = self.pos - 1; // include the opening brace/bracket
        let mut depth = 1u32;
        while depth > 0 {
            match self.advance() {
                None => return Err(ParseError::UnterminatedJson),
                Some(b'"') => {
                    // skip string contents inside JSON
                    loop {
                        match self.advance() {
                            None => return Err(ParseError::UnterminatedJson),
                            Some(b'\\') => {
                                self.advance(); // skip escaped char
                            }
                            Some(b'"') => break,
                            _ => {}
                        }
                    }
                }
                Some(ch) if ch == open => depth += 1,
                Some(ch) if ch == close => depth -= 1,
                _ => {}
            }
        }
        let json_str = String::from_utf8_lossy(&self.input[start..self.pos]).into_owned();
        // Validate JSON
        serde_json::from_str::<Value>(&json_str)
            .map_err(|e| ParseError::InvalidJson(e.to_string()))?;
        Ok(json_str)
    }

    fn read_ident(&mut self) -> String {
        let start = self.pos - 1;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphanumeric() || ch == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        String::from_utf8_lossy(&self.input[start..self.pos]).into_owned()
    }

    fn read_number_or_hex(&mut self, first: u8) -> String {
        let start = self.pos - 1;
        // Check for 0x prefix
        if first == b'0' && self.peek() == Some(b'x') {
            self.pos += 1; // consume 'x'
            while let Some(ch) = self.peek() {
                if ch.is_ascii_hexdigit() {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        } else {
            while let Some(ch) = self.peek() {
                if ch.is_ascii_digit() || ch == b'.' {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&self.input[start..self.pos]).into_owned()
    }

    fn tokenize(&mut self) -> Result<Vec<Token>, ParseError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace();
            let ch = match self.advance() {
                None => break,
                Some(ch) => ch,
            };
            let tok = match ch {
                b'.' => Token::Dot,
                b'(' => Token::LParen,
                b')' => Token::RParen,
                b',' => Token::Comma,
                b':' => Token::Colon,
                b'"' | b'\'' => Token::String(self.read_string(ch)?),
                b'{' => Token::JsonObject(self.read_json_block(b'{', b'}')?),
                b'[' => Token::JsonArray(self.read_json_block(b'[', b']')?),
                ch if ch.is_ascii_digit() => Token::Number(self.read_number_or_hex(ch)),
                ch if ch.is_ascii_alphabetic() || ch == b'_' => {
                    let ident = self.read_ident();
                    match ident.as_str() {
                        "true" => Token::Bool(true),
                        "false" => Token::Bool(false),
                        _ => Token::Ident(ident),
                    }
                }
                ch => return Err(ParseError::UnexpectedChar(ch as char)),
            };
            tokens.push(tok);
        }
        Ok(tokens)
    }
}

fn token_to_value(token: &Token) -> Value {
    match token {
        Token::String(s) => Value::String(s.clone()),
        Token::Number(n) => Value::String(n.clone()),
        Token::Bool(b) => Value::Bool(*b),
        Token::JsonObject(s) | Token::JsonArray(s) => {
            serde_json::from_str(s).unwrap_or(Value::String(s.clone()))
        }
        Token::Ident(s) => Value::String(s.clone()),
        _ => Value::Null,
    }
}

fn token_to_string(token: &Token) -> String {
    match token {
        Token::String(s) | Token::Number(s) | Token::Ident(s) => s.clone(),
        Token::Bool(b) => b.to_string(),
        Token::JsonObject(s) | Token::JsonArray(s) => s.clone(),
        Token::Dot => ".".to_string(),
        Token::LParen => "(".to_string(),
        Token::RParen => ")".to_string(),
        Token::Comma => ",".to_string(),
        Token::Colon => ":".to_string(),
    }
}

pub fn parse(input: &str) -> Result<ParsedCommand, ParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(ParsedCommand::Empty);
    }

    // Builtin commands start with "."
    if let Some(rest) = trimmed.strip_prefix('.') {
        let parts: Vec<&str> = rest.split_whitespace().collect();
        let name = parts.first().unwrap_or(&"").to_string();
        let args = parts[1..].iter().map(|s| s.to_string()).collect();
        return Ok(ParsedCommand::BuiltinCommand { name, args });
    }

    let mut tokenizer = Tokenizer::new(trimmed);
    let tokens = tokenizer.tokenize()?;

    if tokens.is_empty() {
        return Ok(ParsedCommand::Empty);
    }

    // Check for namespace.method pattern (RPC call)
    if tokens.len() >= 3
        && let (Token::Ident(ns), Token::Dot, Token::Ident(method)) =
            (&tokens[0], &tokens[1], &tokens[2])
    {
        let args = parse_rpc_args(&tokens[3..])?;
        return Ok(ParsedCommand::RpcCall {
            namespace: ns.clone(),
            method: method.clone(),
            args,
        });
    }

    // Check for utility call
    if let Token::Ident(name) = &tokens[0]
        && UTILITY_NAMES.contains(&name.as_str())
    {
        let args = tokens[1..]
            .iter()
            .filter(|t| !matches!(t, Token::LParen | Token::RParen | Token::Comma))
            .map(token_to_string)
            .collect();
        return Ok(ParsedCommand::UtilityCall {
            name: name.clone(),
            args,
        });
    }

    // Fallback: treat as utility call with first ident
    if let Token::Ident(name) = &tokens[0] {
        let args = tokens[1..]
            .iter()
            .filter(|t| !matches!(t, Token::LParen | Token::RParen | Token::Comma))
            .map(token_to_string)
            .collect();
        return Ok(ParsedCommand::UtilityCall {
            name: name.clone(),
            args,
        });
    }

    Err(ParseError::UnexpectedChar(trimmed.chars().next().unwrap()))
}

fn parse_rpc_args(tokens: &[Token]) -> Result<Vec<Value>, ParseError> {
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    let mut args = Vec::new();

    // Parenthesized syntax: (arg1, arg2, ...)
    if tokens.first() == Some(&Token::LParen) {
        for token in &tokens[1..] {
            match token {
                Token::RParen => break,
                Token::Comma => continue,
                t => args.push(token_to_value(t)),
            }
        }
        return Ok(args);
    }

    // Space-separated syntax: arg1 arg2 ...
    for token in tokens {
        match token {
            Token::Comma => continue,
            t => args.push(token_to_value(t)),
        }
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_input() {
        let result = parse("").unwrap();
        assert!(matches!(result, ParsedCommand::Empty));
    }

    #[test]
    fn test_builtin_command() {
        let result = parse(".help").unwrap();
        match result {
            ParsedCommand::BuiltinCommand { name, args } => {
                assert_eq!(name, "help");
                assert!(args.is_empty());
            }
            _ => panic!("expected BuiltinCommand"),
        }
    }

    #[test]
    fn test_rpc_call_no_args() {
        let result = parse("eth.blockNumber").unwrap();
        match result {
            ParsedCommand::RpcCall {
                namespace,
                method,
                args,
            } => {
                assert_eq!(namespace, "eth");
                assert_eq!(method, "blockNumber");
                assert!(args.is_empty());
            }
            _ => panic!("expected RpcCall"),
        }
    }

    #[test]
    fn test_rpc_call_with_parens() {
        let result =
            parse(r#"eth.getBalance("0x1234567890abcdef1234567890abcdef12345678", "latest")"#)
                .unwrap();
        match result {
            ParsedCommand::RpcCall {
                namespace,
                method,
                args,
            } => {
                assert_eq!(namespace, "eth");
                assert_eq!(method, "getBalance");
                assert_eq!(args.len(), 2);
            }
            _ => panic!("expected RpcCall"),
        }
    }

    #[test]
    fn test_rpc_call_space_separated() {
        let result = parse("eth.getBalance 0x1234 latest").unwrap();
        match result {
            ParsedCommand::RpcCall {
                namespace,
                method,
                args,
            } => {
                assert_eq!(namespace, "eth");
                assert_eq!(method, "getBalance");
                assert_eq!(args.len(), 2);
            }
            _ => panic!("expected RpcCall"),
        }
    }

    #[test]
    fn test_utility_call() {
        let result = parse("toWei 1.5 ether").unwrap();
        match result {
            ParsedCommand::UtilityCall { name, args } => {
                assert_eq!(name, "toWei");
                assert_eq!(args, vec!["1.5", "ether"]);
            }
            _ => panic!("expected UtilityCall"),
        }
    }

    #[test]
    fn test_json_object_arg() {
        let result =
            parse(r#"eth.call({"to": "0xabc", "data": "0x1234"}, "latest")"#).unwrap();
        match result {
            ParsedCommand::RpcCall { args, .. } => {
                assert_eq!(args.len(), 2);
                assert!(args[0].is_object());
            }
            _ => panic!("expected RpcCall"),
        }
    }

    #[test]
    fn test_bool_arg() {
        let result = parse("eth.getBlockByNumber 0x1 true").unwrap();
        match result {
            ParsedCommand::RpcCall { args, .. } => {
                assert_eq!(args.len(), 2);
                assert_eq!(args[1], Value::Bool(true));
            }
            _ => panic!("expected RpcCall"),
        }
    }
}
