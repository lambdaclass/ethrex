//! JSON-RPC shape of geth's State Override Set.
//!
//! Spec: <https://geth.ethereum.org/docs/interacting-with-geth/rpc/objects#state-override-set>
//!
//! `state` and `stateDiff` are mutually exclusive per address; supplying both is
//! rejected at parse time with a descriptive error. Mixed-case hex addresses are
//! accepted (handled by `ethereum_types::Address`'s deserialize), and so are mixed-case
//! field names, because geth's `encoding/json` matches them case-insensitively.
//!
//! One deliberate deviation: geth ignores a field it does not know, and this rejects it.
//! A silently dropped override yields a plausible-looking but wrong answer, which is
//! worse for the caller than an error naming the field. Every field geth itself defines
//! is modelled, so what this rejects is a misspelling.

use std::collections::BTreeMap;

use bytes::Bytes;
use ethrex_blockchain::vm::{StateOverride, StorageMode, synthetic_code};
use ethrex_common::types::Fork;
use ethrex_common::{Address, H256, U256};
use ethrex_vm::backends::VMType;
use ethrex_vm::is_precompile;
use serde::{
    Deserialize, Deserializer,
    de::{Error as DeError, MapAccess, Visitor},
};
use std::fmt;

use crate::utils::RpcErr;

/// `StateOverrideSet` — keyed by address, each value is an [`AccountOverride`].
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(transparent)]
pub struct StateOverrideSet(pub BTreeMap<Address, AccountOverride>);

impl StateOverrideSet {
    /// Convert into the semantic per-address overrides consumed by
    /// `OverlaidVmDatabase`. Computes synthetic code hashes once during conversion.
    ///
    /// This is also where `movePrecompileToAddress` is validated, because it is the one
    /// function every override-carrying endpoint has to call to get a usable map — the
    /// same reason `Blockchain::new_overlaid_evm` is the only way to build the overlay.
    /// Both checks mirror geth's `StateOverride.Apply`, and both are caller mistakes, so
    /// they surface as bad parameters rather than as VM or internal errors:
    ///
    /// - the source must actually be a precompile at `fork` (geth: `"account %s is not a
    ///   precompile"`), which is why the fork and [`VMType`] have to be known here;
    /// - the destination must not itself be overridden (geth: `"account %s is already
    ///   overridden"`), which keeps "relocated precompile or override?" from being
    ///   silently resolved one way.
    pub fn into_overrides(
        self,
        fork: Fork,
        vm_type: VMType,
    ) -> Result<BTreeMap<Address, StateOverride>, RpcErr> {
        for (address, ov) in &self.0 {
            let Some(destination) = ov.move_precompile_to else {
                continue;
            };
            if !is_precompile(address, fork, vm_type) {
                return Err(RpcErr::BadParams(format!(
                    "account {address:#x} is not a precompile, so movePrecompileToAddress \
                     cannot relocate it"
                )));
            }
            if self.0.contains_key(&destination) {
                return Err(RpcErr::BadParams(format!(
                    "account {destination:#x} is already overridden, so a precompile \
                     cannot be moved onto it"
                )));
            }
        }
        Ok(self
            .0
            .into_iter()
            .map(|(addr, ov)| (addr, ov.into_state_override()))
            .collect())
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
            // geth's `encoding/json` matches keys case-insensitively, so `Balance` and
            // `balance` name the same field there. Match on a lowercased key so every
            // casing a client may send is accepted; the unknown-field error below still
            // echoes the key as it was written.
            match key.to_ascii_lowercase().as_str() {
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
                "statediff" => {
                    state_diff = Some(map.next_value()?);
                }
                "moveprecompiletoaddress" => {
                    move_precompile_to = Some(map.next_value()?);
                }
                _ => {
                    return Err(A::Error::custom(format!(
                        "unknown field `{key}` in state override; expected one of \
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

    /// Go matches JSON keys case-insensitively, so a client that sends geth's field
    /// names in any other casing gets an answer from geth. Rejecting those here would
    /// fail requests geth serves.
    #[test]
    fn field_names_are_case_insensitive() {
        let v = json!({
            "0x00000000000000000000000000000000000000ee": {
                "Balance": "0x1",
                "NONCE": "0x2",
                "Code": "0x00",
                "StateDiff": {
                    "0x0000000000000000000000000000000000000000000000000000000000000001": "0x2a"
                }
            }
        });
        let set: StateOverrideSet =
            serde_json::from_value(v).expect("mixed-case field names must parse");
        let addr: Address = "0x00000000000000000000000000000000000000ee"
            .parse()
            .unwrap();
        let ov = &set.0[&addr];
        assert_eq!(ov.balance, Some(U256::one()));
        assert_eq!(ov.nonce, Some(2));
        assert!(ov.code.is_some());
        assert!(matches!(ov.storage_mode, StorageMode::Diff(_)));
    }

    /// The unknown-field error must echo the key as the caller wrote it, not the
    /// lowercased form matched against.
    #[test]
    fn unknown_field_error_names_the_key_as_written() {
        let v = json!({ "0x00000000000000000000000000000000000000ee": { "BalanceOf": "0x1" } });
        let err = serde_json::from_value::<StateOverrideSet>(v)
            .expect_err("unknown field must be rejected");
        assert!(
            err.to_string().contains("BalanceOf"),
            "error should name the field as written, got: {err}"
        );
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

    /// geth's `StateOverride.Apply` refuses a `movePrecompileToAddress` whose *source* is
    /// not a precompile (`"account %s is not a precompile"`). A silent no-op would answer
    /// a request that asked for something impossible.
    #[test]
    fn moving_a_non_precompile_is_rejected() {
        let v = json!({
            "0x000000000000000000000000000000000000dead": {
                "movePrecompileToAddress": "0x0000000000000000000000000000000000000aaa"
            }
        });
        let set: StateOverrideSet = serde_json::from_value(v).unwrap();
        let err = set
            .into_overrides(Fork::Prague, VMType::L1)
            .expect_err("moving a non-precompile must be rejected");
        assert!(format!("{err}").contains("not a precompile"), "got: {err}");
    }

    /// And a destination that is itself overridden (`"account %s is already overridden"`),
    /// which would otherwise leave "does the relocated precompile or the override win?"
    /// silently resolved one way.
    #[test]
    fn moving_onto_an_overridden_destination_is_rejected() {
        let v = json!({
            "0x0000000000000000000000000000000000000004": {
                "movePrecompileToAddress": "0x0000000000000000000000000000000000000aaa"
            },
            "0x0000000000000000000000000000000000000aaa": { "balance": "0x1" }
        });
        let set: StateOverrideSet = serde_json::from_value(v).unwrap();
        let err = set
            .into_overrides(Fork::Prague, VMType::L1)
            .expect_err("an overridden destination must be rejected");
        assert!(
            format!("{err}").contains("already overridden"),
            "got: {err}"
        );
    }

    /// The legitimate case still works.
    #[test]
    fn moving_a_real_precompile_is_accepted() {
        let v = json!({
            "0x0000000000000000000000000000000000000004": {
                "movePrecompileToAddress": "0x0000000000000000000000000000000000000aaa"
            }
        });
        let set: StateOverrideSet = serde_json::from_value(v).unwrap();
        let overrides = set
            .into_overrides(Fork::Prague, VMType::L1)
            .expect("moving the identity precompile must be accepted");
        assert_eq!(overrides.len(), 1);
    }
}
