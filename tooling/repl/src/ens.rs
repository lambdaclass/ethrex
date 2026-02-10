use serde_json::{json, Value};
use sha3::{Digest, Keccak256};

use crate::client::RpcClient;

/// ENS registry contract address (same on mainnet, Goerli, Sepolia).
const ENS_REGISTRY: &str = "0x00000000000C2E074eC69A0dFb2997BA6C7d2e1e";

/// Compute the ENS namehash for a domain name.
///
/// namehash("") = [0u8; 32]
/// namehash("vitalik.eth") = keccak256(namehash("eth") ++ keccak256("vitalik"))
fn namehash(name: &str) -> [u8; 32] {
    if name.is_empty() {
        return [0u8; 32];
    }

    let mut node = [0u8; 32];
    for label in name.rsplit('.') {
        let label_hash = Keccak256::digest(label.as_bytes());
        let mut data = [0u8; 64];
        data[..32].copy_from_slice(&node);
        data[32..].copy_from_slice(&label_hash);
        node = Keccak256::digest(&data).into();
    }
    node
}

/// Returns true if the string looks like an ENS name (contains `.` and doesn't start with `0x`).
pub fn looks_like_ens_name(s: &str) -> bool {
    !s.starts_with("0x") && s.contains('.')
}

/// Resolve an ENS name to a checksummed `0x`-prefixed address.
pub async fn resolve(client: &RpcClient, name: &str) -> Result<String, String> {
    let node = namehash(name);
    let node_hex = hex::encode(node);

    // Call resolver(bytes32) on the ENS registry — selector 0x0178b8bf
    let resolver_calldata = format!("0x0178b8bf{node_hex}");
    let resolver_result = eth_call(client, ENS_REGISTRY, &resolver_calldata).await?;

    let resolver_addr = parse_address_from_abi_word(&resolver_result)?;
    if resolver_addr == "0x0000000000000000000000000000000000000000" {
        return Err(format!("ENS name not found: {name}"));
    }

    // Call addr(bytes32) on the resolver — selector 0x3b3b57de
    let addr_calldata = format!("0x3b3b57de{node_hex}");
    let addr_result = eth_call(client, &resolver_addr, &addr_calldata).await?;

    let resolved = parse_address_from_abi_word(&addr_result)?;
    if resolved == "0x0000000000000000000000000000000000000000" {
        return Err(format!("ENS name has no address set: {name}"));
    }

    Ok(to_checksum_address(&resolved))
}

/// Execute an `eth_call` with the given `to` and `data`, returning the raw hex result.
async fn eth_call(client: &RpcClient, to: &str, data: &str) -> Result<String, String> {
    let params = vec![
        json!({"to": to, "data": data}),
        Value::String("latest".to_string()),
    ];

    client
        .send_request("eth_call", params)
        .await
        .map_err(|e| format!("ENS resolution failed: {e}"))
        .and_then(|v| {
            v.as_str()
                .map(String::from)
                .ok_or_else(|| "ENS resolution returned non-string result".to_string())
        })
}

/// Parse the last 20 bytes of a 32-byte ABI-encoded word as a `0x`-prefixed address.
fn parse_address_from_abi_word(hex_str: &str) -> Result<String, String> {
    let hex = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    if hex.len() < 40 {
        return Err(format!("unexpected response length: 0x{hex}"));
    }
    // Last 40 hex chars = 20 bytes = address
    let addr = &hex[hex.len() - 40..];
    Ok(format!("0x{addr}"))
}

/// EIP-55 checksum encoding.
fn to_checksum_address(addr: &str) -> String {
    let addr_lower = addr
        .strip_prefix("0x")
        .unwrap_or(addr)
        .to_lowercase();
    let hash = Keccak256::digest(addr_lower.as_bytes());
    let hash_hex = hex::encode(hash);

    let mut checksummed = String::from("0x");
    for (i, c) in addr_lower.chars().enumerate() {
        if c.is_ascii_alphabetic() {
            let nibble = u8::from_str_radix(&hash_hex[i..i + 1], 16).unwrap_or(0);
            if nibble >= 8 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namehash_empty() {
        assert_eq!(namehash(""), [0u8; 32]);
    }

    #[test]
    fn namehash_eth() {
        // Well-known: namehash("eth") = 0x93cdeb708b7545dc668eb9280176169d1c33cfd8ed6f04690a0bcc88a93fc4ae
        let result = hex::encode(namehash("eth"));
        assert_eq!(
            result,
            "93cdeb708b7545dc668eb9280176169d1c33cfd8ed6f04690a0bcc88a93fc4ae"
        );
    }

    #[test]
    fn namehash_vitalik_eth() {
        // namehash("vitalik.eth") is a well-known test vector
        let result = hex::encode(namehash("vitalik.eth"));
        assert_eq!(
            result,
            "ee6c4522aab0003e8d14cd40a6af439055fd2577951148c14b6cea9a53475835"
        );
    }

    #[test]
    fn ens_name_detection() {
        assert!(looks_like_ens_name("vitalik.eth"));
        assert!(looks_like_ens_name("foo.bar.eth"));
        assert!(!looks_like_ens_name("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"));
        assert!(!looks_like_ens_name("latest"));
        assert!(!looks_like_ens_name("12345"));
    }

    #[test]
    fn parse_address_from_word() {
        // 32-byte ABI word with address in last 20 bytes
        let word = "0x000000000000000000000000d8da6bf26964af9d7eed9e03e53415d37aa96045";
        let addr = parse_address_from_abi_word(word).unwrap();
        assert_eq!(addr, "0xd8da6bf26964af9d7eed9e03e53415d37aa96045");
    }

    #[test]
    fn checksum_address() {
        let addr = "0xd8da6bf26964af9d7eed9e03e53415d37aa96045";
        assert_eq!(
            to_checksum_address(addr),
            "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045"
        );
    }
}
