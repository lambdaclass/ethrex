//! JSON-RPC shape of geth's State Override Set.
//!
//! Spec: <https://geth.ethereum.org/docs/interacting-with-geth/rpc/objects#state-override-set>
//!
//! `state` and `stateDiff` are mutually exclusive per address; supplying both is
//! rejected at parse time with a descriptive error. Mixed-case hex addresses are
//! accepted (handled by `ethereum_types::Address`'s deserialize).

use std::collections::BTreeMap;

use bytes::Bytes;
use ethrex_blockchain::vm::{StateOverride, StorageMode, synthetic_code};
use ethrex_common::{Address, H256, U256};
use serde::{
    Deserialize, Deserializer,
    de::{Error as DeError, MapAccess, Visitor},
};
use std::fmt;

/// `StateOverrideSet` — keyed by address, each value is an [`AccountOverride`].
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(transparent)]
pub struct StateOverrideSet(pub BTreeMap<Address, AccountOverride>);

impl StateOverrideSet {
    /// Convert into the semantic per-address overrides consumed by
    /// `OverlaidVmDatabase`. Computes synthetic code hashes once during conversion.
    pub fn into_overrides(self) -> BTreeMap<Address, StateOverride> {
        self.0
            .into_iter()
            .map(|(addr, ov)| (addr, ov.into_state_override()))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Per-address overlay fields. Custom deserialize because `state` and `stateDiff`
/// are mutually exclusive — co-presence must error at parse time, not be silently
/// merged.
#[derive(Debug, Default, Clone)]
pub struct AccountOverride {
    pub balance: Option<U256>,
    pub nonce: Option<u64>,
    pub code: Option<Bytes>,
    pub storage_mode: StorageMode,
    pub move_precompile_to: Option<Address>,
}

impl AccountOverride {
    pub fn into_state_override(self) -> StateOverride {
        StateOverride {
            balance: self.balance,
            nonce: self.nonce,
            code: self.code.map(synthetic_code),
            storage_mode: self.storage_mode,
            move_precompile_to: self.move_precompile_to,
        }
    }
}

impl<'de> Deserialize<'de> for AccountOverride {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(AccountOverrideVisitor)
    }
}

struct AccountOverrideVisitor;

impl<'de> Visitor<'de> for AccountOverrideVisitor {
    type Value = AccountOverride;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a state override object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut balance: Option<U256> = None;
        let mut nonce: Option<u64> = None;
        let mut code: Option<Bytes> = None;
        let mut state: Option<BTreeMap<H256, U256>> = None;
        let mut state_diff: Option<BTreeMap<H256, U256>> = None;
        let mut move_precompile_to: Option<Address> = None;

        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "balance" => {
                    let v: String = map.next_value()?;
                    balance = Some(parse_u256(&v).map_err(A::Error::custom)?);
                }
                "nonce" => {
                    let v: String = map.next_value()?;
                    nonce = Some(parse_u64(&v).map_err(A::Error::custom)?);
                }
                "code" => {
                    let v: String = map.next_value()?;
                    code = Some(parse_bytes(&v).map_err(A::Error::custom)?);
                }
                "state" => {
                    state = Some(map.next_value()?);
                }
                "stateDiff" => {
                    state_diff = Some(map.next_value()?);
                }
                "movePrecompileToAddress" => {
                    move_precompile_to = Some(map.next_value()?);
                }
                other => {
                    return Err(A::Error::custom(format!(
                        "unknown field `{other}` in state override; expected one of \
                         balance, nonce, code, state, stateDiff, movePrecompileToAddress"
                    )));
                }
            }
        }

        let storage_mode = match (state, state_diff) {
            (Some(_), Some(_)) => {
                return Err(A::Error::custom(
                    "state and stateDiff cannot both be set for the same address",
                ));
            }
            (Some(m), None) => StorageMode::Replace(m),
            (None, Some(m)) => StorageMode::Diff(m),
            (None, None) => StorageMode::None,
        };

        Ok(AccountOverride {
            balance,
            nonce,
            code,
            storage_mode,
            move_precompile_to,
        })
    }
}

fn parse_u256(s: &str) -> Result<U256, String> {
    let s = s.trim_start_matches("0x");
    U256::from_str_radix(s, 16).map_err(|e| format!("invalid u256: {e}"))
}

fn parse_u64(s: &str) -> Result<u64, String> {
    let s = s.trim_start_matches("0x");
    u64::from_str_radix(s, 16).map_err(|e| format!("invalid u64: {e}"))
}

fn parse_bytes(s: &str) -> Result<Bytes, String> {
    let s = s.trim_start_matches("0x");
    let v = hex::decode(s).map_err(|e| format!("invalid hex bytes: {e}"))?;
    Ok(Bytes::from(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_balance_only() {
        let v = json!({"0x000000000000000000000000000000000000beef": {"balance": "0x100"}});
        let set: StateOverrideSet = serde_json::from_value(v).unwrap();
        assert_eq!(set.0.len(), 1);
        let addr: Address = "0x000000000000000000000000000000000000beef"
            .parse()
            .unwrap();
        let ov = &set.0[&addr];
        assert_eq!(ov.balance, Some(U256::from(0x100)));
        assert!(ov.nonce.is_none());
        assert!(ov.code.is_none());
        assert!(matches!(ov.storage_mode, StorageMode::None));
    }

    #[test]
    fn parse_nonce_and_code() {
        let v = json!({
            "0x00000000000000000000000000000000000000aa": {
                "nonce": "0x5",
                "code": "0x6001600152"
            }
        });
        let set: StateOverrideSet = serde_json::from_value(v).unwrap();
        let addr: Address = "0x00000000000000000000000000000000000000aa"
            .parse()
            .unwrap();
        let ov = &set.0[&addr];
        assert_eq!(ov.nonce, Some(5));
        assert_eq!(
            ov.code.as_ref().map(|b| b.as_ref().to_vec()),
            Some(vec![0x60, 0x01, 0x60, 0x01, 0x52])
        );
    }

    #[test]
    fn parse_state_replace_mode() {
        let v = json!({
            "0x00000000000000000000000000000000000000cc": {
                "state": {
                    "0x0000000000000000000000000000000000000000000000000000000000000001": "0x00000000000000000000000000000000000000000000000000000000000000aa"
                }
            }
        });
        let set: StateOverrideSet = serde_json::from_value(v).unwrap();
        let addr: Address = "0x00000000000000000000000000000000000000cc"
            .parse()
            .unwrap();
        let ov = &set.0[&addr];
        assert!(matches!(ov.storage_mode, StorageMode::Replace(_)));
    }

    #[test]
    fn parse_state_diff_mode() {
        let v = json!({
            "0x00000000000000000000000000000000000000dd": {
                "stateDiff": {
                    "0x0000000000000000000000000000000000000000000000000000000000000001": "0x00000000000000000000000000000000000000000000000000000000000000aa"
                }
            }
        });
        let set: StateOverrideSet = serde_json::from_value(v).unwrap();
        let addr: Address = "0x00000000000000000000000000000000000000dd"
            .parse()
            .unwrap();
        let ov = &set.0[&addr];
        assert!(matches!(ov.storage_mode, StorageMode::Diff(_)));
    }

    #[test]
    fn reject_state_and_state_diff_together() {
        let v = json!({
            "0x00000000000000000000000000000000000000ee": {
                "state": {},
                "stateDiff": {}
            }
        });
        let err = serde_json::from_value::<StateOverrideSet>(v).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("state") && msg.contains("stateDiff"),
            "expected combined-error message, got: {msg}"
        );
    }

    #[test]
    fn empty_account_override_is_noop() {
        let v = json!({"0x00000000000000000000000000000000000000ff": {}});
        let set: StateOverrideSet = serde_json::from_value(v).unwrap();
        let addr: Address = "0x00000000000000000000000000000000000000ff"
            .parse()
            .unwrap();
        let ov = &set.0[&addr];
        let semantic = ov.clone().into_state_override();
        assert!(semantic.is_noop());
    }

    #[test]
    fn mixed_case_address_resolves_to_canonical() {
        let v = json!({"0xAbCdEf0000000000000000000000000000000000": {"balance": "0x1"}});
        let set: StateOverrideSet = serde_json::from_value(v).unwrap();
        let canonical: Address = "0xabcdef0000000000000000000000000000000000"
            .parse()
            .unwrap();
        assert!(set.0.contains_key(&canonical));
    }

    #[test]
    fn move_precompile_field() {
        let v = json!({
            "0x0000000000000000000000000000000000000001": {
                "movePrecompileToAddress": "0x0000000000000000000000000000000000000aaa"
            }
        });
        let set: StateOverrideSet = serde_json::from_value(v).unwrap();
        let addr: Address = "0x0000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let ov = &set.0[&addr];
        let target: Address = "0x0000000000000000000000000000000000000aaa"
            .parse()
            .unwrap();
        assert_eq!(ov.move_precompile_to, Some(target));
    }

    #[test]
    fn malformed_hex_balance_rejected() {
        let v = json!({"0x0000000000000000000000000000000000000001": {"balance": "0xZZ"}});
        let err = serde_json::from_value::<StateOverrideSet>(v).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("invalid"));
    }

    #[test]
    fn unknown_field_rejected() {
        let v = json!({
            "0x0000000000000000000000000000000000000001": {"bogus": "0x1"}
        });
        let err = serde_json::from_value::<StateOverrideSet>(v).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }
}
