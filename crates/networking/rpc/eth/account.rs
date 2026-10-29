use std::collections::{BTreeMap, HashMap};

use serde_json::{Value, value::RawValue};
use tracing::debug;

use crate::rpc::{RpcApiContext, RpcHandler};
use crate::types::account_proof::{AccountProof, StorageProof};
use crate::types::block_identifier::{BlockIdentifierOrHash, BlockTag};
use crate::utils::RpcErr;
use ethrex_common::types::AccountState;
use ethrex_common::{Address, BigEndianHash, H256, U256, serde_utils};

pub struct GetBalanceRequest {
    pub address: Address,
    pub block: BlockIdentifierOrHash,
}

pub struct GetCodeRequest {
    pub address: Address,
    pub block: BlockIdentifierOrHash,
}

pub struct GetStorageAtRequest {
    pub address: Address,
    pub storage_slot: H256,
    pub block: BlockIdentifierOrHash,
}

pub struct GetTransactionCountRequest {
    pub address: Address,
    pub block: BlockIdentifierOrHash,
}

pub struct GetProofRequest {
    pub address: Address,
    pub storage_keys: Vec<H256>,
    /// The form of each requested key that the response echoes, by index.
    pub storage_key_echoes: Vec<String>,
    pub block: BlockIdentifierOrHash,
}

/// Parses an `eth_getProof` storage key into its slot and its echo form.
/// A key with 63 or 64 hex digits echoes as 32 bytes; any other key echoes
/// as a minimal quantity.
fn parse_storage_key(key: &str) -> Result<(H256, String), RpcErr> {
    let invalid = || RpcErr::InvalidParams(format!("invalid storage key: {key}"));
    let digits = key.strip_prefix("0x").ok_or_else(invalid)?;
    if digits.len() > 64 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    let value = if digits.is_empty() {
        U256::zero()
    } else {
        U256::from_str_radix(digits, 16).map_err(|_| invalid())?
    };
    let slot = H256::from_uint(&value);
    let echo = if digits.len() >= 63 {
        format!("{slot:#x}")
    } else {
        format!("{value:#x}")
    };
    Ok((slot, echo))
}

/// A missing state root means the node does not keep that state only once it is
/// synced, holds the state of its head, and the block is not above that head;
/// before that, the state may still arrive.
fn state_is_caught_up(context: &RpcApiContext, block_number: u64) -> Result<bool, RpcErr> {
    if !context.blockchain.is_synced() {
        return Ok(false);
    }
    let latest = context.storage.get_latest_block_number()?;
    // A block above the head has not been executed yet, so its state is not
    // pruned: a node that fell behind its consensus client can hold such blocks.
    if block_number > latest {
        return Ok(false);
    }
    match context.storage.get_block_header(latest)? {
        Some(header) => Ok(context.storage.has_state_root(header.state_root)?),
        None => Ok(false),
    }
}

impl RpcHandler for GetBalanceRequest {
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<GetBalanceRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        // The block parameter is optional and defaults to "latest" (per execution-apis).
        if params.len() != 1 && params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 1 or 2 params".to_owned()));
        };
        Ok(GetBalanceRequest {
            address: serde_json::from_str(params[0].get())?,
            block: params
                .get(1)
                .map(|param| BlockIdentifierOrHash::parse_param(param, 1))
                .transpose()?
                .unwrap_or_default(),
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested balance of account {} at block {}",
            self.address, self.block
        );

        let Some(block_number) = self.block.resolve_block_number(&context.storage).await? else {
            return Err(RpcErr::Internal(
                "Could not resolve block number".to_owned(),
            )); // Should we return Null here?
        };

        let account = context
            .storage
            .get_account_info(block_number, self.address)
            .await?;
        let balance = account.map(|acc| acc.balance).unwrap_or_default();

        serde_json::to_value(format!("{balance:#x}"))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetCodeRequest {
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<GetCodeRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        // The block parameter is optional and defaults to "latest" (per execution-apis).
        if params.len() != 1 && params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 1 or 2 params".to_owned()));
        };
        Ok(GetCodeRequest {
            address: serde_json::from_str(params[0].get())?,
            block: params
                .get(1)
                .map(|param| BlockIdentifierOrHash::parse_param(param, 1))
                .transpose()?
                .unwrap_or_default(),
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested code of account {} at block {}",
            self.address, self.block
        );

        let Some(block_number) = self.block.resolve_block_number(&context.storage).await? else {
            return Err(RpcErr::Internal(
                "Could not resolve block number".to_owned(),
            )); // Should we return Null here?
        };

        let code = context
            .storage
            .get_code_by_account_address(block_number, self.address)
            .await?
            .map(|c| c.code_bytes())
            .unwrap_or_default();

        serde_json::to_value(format!("0x{code:x}"))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetStorageAtRequest {
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<GetStorageAtRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        // The block parameter is optional and defaults to "latest" (per execution-apis).
        if params.len() != 2 && params.len() != 3 {
            return Err(RpcErr::BadParams("Expected 2 or 3 params".to_owned()));
        };
        let storage_slot_u256 = serde_utils::u256::deser_hex_or_dec_str(&*params[1])?;
        Ok(GetStorageAtRequest {
            address: serde_json::from_str(params[0].get())?,
            storage_slot: H256::from_uint(&storage_slot_u256),
            block: params
                .get(2)
                .map(|param| BlockIdentifierOrHash::parse_param(param, 2))
                .transpose()?
                .unwrap_or_default(),
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested storage slot {} of account {} at block {}",
            self.storage_slot, self.address, self.block
        );

        let Some(block_number) = self.block.resolve_block_number(&context.storage).await? else {
            return Err(RpcErr::Internal(
                "Could not resolve block number".to_owned(),
            )); // Should we return Null here?
        };

        let storage_value = context
            .storage
            .get_storage_at(block_number, self.address, self.storage_slot)?
            .unwrap_or_default();
        let storage_value = H256::from_uint(&storage_value);
        serde_json::to_value(format!("{storage_value:#x}"))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

impl RpcHandler for GetTransactionCountRequest {
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<GetTransactionCountRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        // The block parameter is optional and defaults to "latest" (per execution-apis).
        if params.len() != 1 && params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 1 or 2 params".to_owned()));
        };
        Ok(GetTransactionCountRequest {
            address: serde_json::from_str(params[0].get())?,
            block: params
                .get(1)
                .map(|param| BlockIdentifierOrHash::parse_param(param, 1))
                .transpose()?
                .unwrap_or_default(),
        })
    }
    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested nonce of account {} at block {}",
            self.address, self.block
        );

        // Resolve the canonical nonce for the requested block first. For the
        // `Pending` tag this resolves to the latest block.
        let Some(block_number) = self.block.resolve_block_number(&context.storage).await? else {
            return serde_json::to_value("0x0")
                .map_err(|error| RpcErr::Internal(error.to_string()));
        };
        let account_nonce = context
            .storage
            .get_nonce_by_account_address(block_number, self.address)
            .await?
            .unwrap_or_default();

        // For `Pending`, the mempool may advance the nonce past the on-chain
        // value, but it must never report a value below it. Stale txs left in
        // the pool can otherwise yield a pending nonce lower than `latest`.
        let nonce = if self.block == BlockTag::Pending {
            match context.blockchain.mempool.get_nonce(&self.address)? {
                Some(mempool_nonce) => mempool_nonce.max(account_nonce),
                None => account_nonce,
            }
        } else {
            account_nonce
        };

        serde_json::to_value(format!("0x{nonce:x}"))
            .map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

/// Maximum number of storage keys accepted in a single `eth_getProof` call, the same
/// bound `eth_getStorageValues` puts on its slots. Every key costs a storage-trie
/// walk and a multi-kilobyte proof in the response, and a key can be as short as
/// `"0x"`, so without a cap one request body could ask for hundreds of thousands
/// of proofs and have them all held in memory at once.
const MAX_PROOF_STORAGE_KEYS: usize = 1024;

impl RpcHandler for GetProofRequest {
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        // The block parameter is optional and defaults to "latest" (per execution-apis).
        if params.len() != 2 && params.len() != 3 {
            return Err(RpcErr::BadParams("Expected 2 or 3 params".to_owned()));
        };
        let raw_keys: Vec<String> = serde_json::from_str(params[1].get())
            .map_err(|error| RpcErr::InvalidParams(format!("invalid storage keys: {error}")))?;
        if raw_keys.len() > MAX_PROOF_STORAGE_KEYS {
            return Err(RpcErr::LimitExceeded(format!(
                "too many storage keys (max {MAX_PROOF_STORAGE_KEYS})"
            )));
        }
        let (storage_keys, storage_key_echoes) = raw_keys
            .iter()
            .map(|key| parse_storage_key(key))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .unzip();
        Ok(GetProofRequest {
            address: serde_json::from_str(params[0].get())?,
            storage_keys,
            storage_key_echoes,
            block: params
                .get(2)
                .map(|param| BlockIdentifierOrHash::parse_param(param, 2))
                .transpose()?
                .unwrap_or_default(),
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        debug!(
            "Requested proof for account {} at block {} with storage keys: {:?}",
            self.address, self.block, self.storage_keys
        );
        // A hash must resolve to its own header. Going through the block number
        // would return the canonical header at that height for a fork block.
        let header = match &self.block {
            BlockIdentifierOrHash::Hash(hash) => storage.get_block_header_by_hash(*hash)?,
            BlockIdentifierOrHash::Identifier(id) => id.resolve_block_header(storage).await?,
        };
        let Some(header) = header else {
            return Ok(Value::Null);
        };
        if !storage.has_state_root(header.state_root)? {
            if state_is_caught_up(&context, header.number)? {
                return Err(RpcErr::PrunedHistoryUnavailable(format!(
                    "state for block {:#x} is unavailable",
                    header.hash()
                )));
            }
            return Err(RpcErr::Internal(format!(
                "state for block {:#x} is not available yet: the node is syncing",
                header.hash()
            )));
        }
        let Some(account_proof) = storage
            .get_account_proof(header.state_root, self.address, &self.storage_keys)
            .await?
        else {
            return Err(RpcErr::Internal("Could not get account proof".to_owned()));
        };
        let storage_proof = account_proof
            .storage_proof
            .into_iter()
            .zip(&self.storage_key_echoes)
            .map(|(sp, key)| StorageProof {
                key: key.clone(),
                value: sp.value,
                proof: sp.proof,
            })
            .collect();
        // The eth_getProof spec reports zero hashes for an absent account.
        let account = account_proof.account.unwrap_or(AccountState {
            nonce: 0,
            balance: U256::zero(),
            code_hash: H256::zero(),
            storage_root: H256::zero(),
        });
        let account_proof = AccountProof {
            account_proof: account_proof.proof,
            address: self.address,
            balance: account.balance,
            code_hash: account.code_hash,
            nonce: account.nonce,
            storage_hash: account.storage_root,
            storage_proof,
        };
        serde_json::to_value(account_proof).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

/// Maximum number of storage slots that can be requested across all accounts in a
/// single `eth_getStorageValues` call (matches go-ethereum's limit).
const MAX_STORAGE_VALUES_SLOTS: usize = 1024;

pub struct GetStorageValuesRequest {
    pub requests: HashMap<Address, Vec<H256>>,
    pub block: BlockIdentifierOrHash,
}

/// Parses a storage slot as the spec's `bytesMax32`: `0x` and at most 64 hex
/// digits, where `0x` alone is slot 0.
fn parse_storage_slot(slot: &str) -> Result<H256, RpcErr> {
    let invalid =
        || RpcErr::InvalidParams("storage slot must be 0x and at most 64 hex digits".to_owned());
    let digits = slot.strip_prefix("0x").ok_or_else(invalid)?;
    if digits.len() > 64 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    if digits.is_empty() {
        return Ok(H256::zero());
    }
    let slot = U256::from_str_radix(digits, 16).map_err(|_| invalid())?;
    Ok(H256::from_uint(&slot))
}

impl RpcHandler for GetStorageValuesRequest {
    fn parse(params: &Option<Vec<Box<RawValue>>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        // The block parameter is optional and defaults to "latest" (per execution-apis).
        if params.len() != 1 && params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 1 or 2 params".to_owned()));
        };
        // params[0] is an object mapping address -> array of storage slots. A
        // BTreeMap reads it in a fixed order, so when two keys spell the same
        // address the same one wins on every call.
        let raw: BTreeMap<String, Vec<String>> = serde_json::from_str(params[0].get())?;
        let mut requests = HashMap::with_capacity(raw.len());
        let mut total_slots = 0usize;
        for (address, slots) in raw {
            total_slots += slots.len();
            if total_slots > MAX_STORAGE_VALUES_SLOTS {
                return Err(RpcErr::LimitExceeded(format!(
                    "too many slots (max {MAX_STORAGE_VALUES_SLOTS})"
                )));
            }
            let address: Address = serde_json::from_value(Value::String(address))?;
            let slots = slots
                .iter()
                .map(|slot| parse_storage_slot(slot))
                .collect::<Result<_, _>>()?;
            requests.insert(address, slots);
        }
        if total_slots == 0 {
            return Err(RpcErr::InvalidParams("empty request".to_owned()));
        }
        Ok(GetStorageValuesRequest {
            requests,
            block: params
                .get(1)
                .map(|param| BlockIdentifierOrHash::parse_param(param, 1))
                .transpose()?
                .unwrap_or_default(),
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        debug!(
            "Requested storage values for {} accounts at block {}",
            self.requests.len(),
            self.block
        );
        let storage = &context.storage;
        // Resolve the block once and read every slot from its state root. Going
        // through the block number for each slot could read two blocks' states if
        // the chain reorged mid-request, read the canonical block instead of the
        // one a hash names, and answer zeros for a block above the head.
        let header = match &self.block {
            BlockIdentifierOrHash::Hash(hash) => storage.get_block_header_by_hash(*hash)?,
            BlockIdentifierOrHash::Identifier(id) => id.resolve_block_header(storage).await?,
        };
        let Some(header) = header else {
            return Err(RpcErr::Internal(
                "Could not resolve block number".to_owned(),
            ));
        };
        let mut result: HashMap<String, Vec<String>> = HashMap::with_capacity(self.requests.len());
        for (address, slots) in &self.requests {
            let mut values = Vec::with_capacity(slots.len());
            for slot in slots {
                let value = storage
                    .get_storage_at_root(header.state_root, *address, *slot)?
                    .unwrap_or_default();
                values.push(format!("{:#x}", H256::from_uint(&value)));
            }
            result.insert(format!("{address:#x}"), values);
        }
        serde_json::to_value(result).map_err(|error| RpcErr::Internal(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{TestContext, raw_params};
    use serde_json::json;

    #[test]
    fn test_get_storage_at_request_parse_hex_slot() {
        let params = Some(vec![
            json!("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"),
            // Storage slot can be provided as hex string
            json!("0x1"),
            json!("latest"),
        ]);
        let request = GetStorageAtRequest::parse(&raw_params(&params)).unwrap();

        let expected_address = "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
            .parse()
            .unwrap();
        assert_eq!(request.address, expected_address);
        assert_eq!(request.storage_slot, H256::from_uint(&U256::from(1u64)));
        assert_eq!(request.block, BlockTag::Latest);
    }

    #[test]
    fn test_get_storage_at_request_parse_number_slot() {
        let params = Some(vec![
            json!("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"),
            // Storage slot can be provided as number
            json!("1"),
            json!("latest"),
        ]);
        let request = GetStorageAtRequest::parse(&raw_params(&params)).unwrap();

        let expected_address = "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
            .parse()
            .unwrap();
        assert_eq!(request.address, expected_address);
        assert_eq!(request.storage_slot, H256::from_uint(&U256::from(1u64)));
        assert_eq!(request.block, BlockTag::Latest);
    }

    #[test]
    fn test_state_methods_default_block_to_latest_when_omitted() {
        // Per execution-apis the Block parameter is optional and defaults to
        // "latest". Each state method must parse a request with the block omitted
        // and resolve it to the latest tag.
        let addr = json!("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
        assert_eq!(
            GetBalanceRequest::parse(&raw_params(&Some(vec![addr.clone()])))
                .unwrap()
                .block,
            BlockTag::Latest
        );
        assert_eq!(
            GetCodeRequest::parse(&raw_params(&Some(vec![addr.clone()])))
                .unwrap()
                .block,
            BlockTag::Latest
        );
        assert_eq!(
            GetTransactionCountRequest::parse(&raw_params(&Some(vec![addr.clone()])))
                .unwrap()
                .block,
            BlockTag::Latest
        );
        assert_eq!(
            GetStorageAtRequest::parse(&raw_params(&Some(vec![addr.clone(), json!("0x0")])))
                .unwrap()
                .block,
            BlockTag::Latest
        );
        assert_eq!(
            GetProofRequest::parse(&raw_params(&Some(vec![addr, json!([])])))
                .unwrap()
                .block,
            BlockTag::Latest
        );
    }

    #[test]
    fn test_get_storage_values_request_parse_defaults_to_latest() {
        let params = Some(vec![json!({
            "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef": ["0x1"]
        })]);
        let request = GetStorageValuesRequest::parse(&raw_params(&params)).unwrap();
        assert_eq!(request.block, BlockTag::Latest);
        let addr: Address = "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
            .parse()
            .unwrap();
        assert_eq!(
            request.requests.get(&addr).unwrap(),
            &vec![H256::from_uint(&U256::from(1u64))]
        );
    }

    #[test]
    fn test_get_storage_values_request_rejects_empty() {
        for requests in [
            json!({}),
            json!({"0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef": []}),
        ] {
            assert!(matches!(
                GetStorageValuesRequest::parse(&raw_params(&Some(vec![requests]))),
                Err(RpcErr::InvalidParams(ref msg)) if msg == "empty request"
            ));
        }
    }

    #[test]
    fn test_get_storage_values_request_slot_format() {
        let parse = |slot: &str| {
            GetStorageValuesRequest::parse(&raw_params(&Some(vec![json!({
                "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef": [slot]
            })])))
        };
        let addr: Address = "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
            .parse()
            .unwrap();
        for (slot, expected) in [
            ("0x", U256::zero()),
            ("0x10", U256::from(16u64)),
            (
                "0x0100000000000000000000000000000000000000000000000000000000000000",
                U256::one() << 248,
            ),
        ] {
            let request = parse(slot).unwrap();
            assert_eq!(
                request.requests.get(&addr).unwrap(),
                &vec![H256::from_uint(&expected)],
                "{slot}"
            );
        }
        // Without 0x, `10` would read slot 16 where eth_getStorageAt reads slot 10.
        let too_long = format!("0x{}", "0".repeat(65));
        for slot in ["10", "0xzz", too_long.as_str()] {
            assert!(
                matches!(parse(slot), Err(RpcErr::InvalidParams(_))),
                "{slot} must be rejected"
            );
        }
    }

    #[test]
    fn test_get_storage_values_request_accepts_the_cap() {
        let slots: Vec<_> = (0..MAX_STORAGE_VALUES_SLOTS)
            .map(|slot| format!("{slot:#x}"))
            .collect();
        let params = Some(vec![json!({
            "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef": slots
        })]);
        assert!(GetStorageValuesRequest::parse(&raw_params(&params)).is_ok());
    }

    #[test]
    fn test_get_storage_values_request_rejects_single_account_over_cap() {
        let slots: Vec<_> = (0..=MAX_STORAGE_VALUES_SLOTS)
            .map(|slot| format!("{slot:#x}"))
            .collect();
        let params = Some(vec![json!({
            "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef": slots
        })]);

        assert!(matches!(
            GetStorageValuesRequest::parse(&raw_params(&params)),
            Err(RpcErr::LimitExceeded(ref msg)) if msg.contains("too many slots")
        ));
    }

    #[test]
    fn test_get_storage_values_request_rejects_multi_account_over_cap() {
        let first_slots: Vec<_> = (0..600).map(|slot| format!("{slot:#x}")).collect();
        let second_slots: Vec<_> = (600..1200).map(|slot| format!("{slot:#x}")).collect();
        let params = Some(vec![json!({
            "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef": first_slots,
            "0xfeedfeedfeedfeedfeedfeedfeedfeedfeedfeed": second_slots,
        })]);

        assert!(matches!(
            GetStorageValuesRequest::parse(&raw_params(&params)),
            Err(RpcErr::LimitExceeded(ref msg)) if msg.contains("too many slots")
        ));
    }

    /// Values come from the requested block's state, keyed by the lowercase
    /// address, with zeros for an absent account or slot. A block the node does
    /// not have is an error rather than zeros.
    #[tokio::test]
    async fn test_get_storage_values_handle() {
        use crate::test_utils::{TEST_GENESIS, default_context_with_storage};
        use crate::types::block_identifier::BlockIdentifier;
        use ethrex_common::types::{Genesis, GenesisAccount};
        use ethrex_storage::{EngineType, Store};

        let contract = Address::from_low_u64_be(0xabcd);
        let mut genesis: Genesis = serde_json::from_str(TEST_GENESIS).unwrap();
        genesis.alloc.insert(
            contract,
            GenesisAccount {
                code: Default::default(),
                storage: [(U256::from(1u64), U256::from(0x33u64))].into(),
                balance: U256::zero(),
                nonce: 1,
            },
        );
        let mut store = Store::new("", EngineType::InMemory).unwrap();
        store.add_initial_state(genesis).await.unwrap();
        let context = default_context_with_storage(store).await;
        let genesis_hash = context.storage.get_block_header(0).unwrap().unwrap().hash();

        let absent = Address::from_low_u64_be(0xdead);
        let requests: HashMap<Address, Vec<H256>> = [
            (
                contract,
                vec![H256::from_low_u64_be(1), H256::from_low_u64_be(2)],
            ),
            (absent, vec![H256::from_low_u64_be(1)]),
        ]
        .into();
        let word = |value: u64| format!("{:#x}", H256::from_low_u64_be(value));
        let expected = json!({
            format!("{contract:#x}"): [word(0x33), word(0)],
            format!("{absent:#x}"): [word(0)],
        });
        for block in [
            BlockIdentifierOrHash::default(),
            BlockIdentifierOrHash::Hash(genesis_hash),
        ] {
            let result = GetStorageValuesRequest {
                requests: requests.clone(),
                block,
            }
            .handle(context.clone())
            .await
            .unwrap();
            assert_eq!(result, expected);
        }

        let above_head = GetStorageValuesRequest {
            requests,
            block: BlockIdentifierOrHash::Identifier(BlockIdentifier::Number(5)),
        }
        .handle(context.clone())
        .await;
        assert!(matches!(above_head, Err(RpcErr::Internal(_))));
    }

    /// Builds an in-memory store whose genesis pre-sets `address`'s nonce, and a
    /// context over it. Mirrors `setup_store` but lets the test fix the on-chain
    /// nonce without executing blocks.
    async fn context_with_account_nonce(address: Address, nonce: u64) -> TestContext {
        use crate::test_utils::{TEST_GENESIS, default_context_with_storage};
        use ethrex_common::types::{Genesis, GenesisAccount};
        use ethrex_storage::{EngineType, Store};

        let mut genesis: Genesis = serde_json::from_str(TEST_GENESIS).unwrap();
        genesis.alloc.insert(
            address,
            GenesisAccount {
                code: Default::default(),
                storage: Default::default(),
                balance: U256::from(10u64).pow(U256::from(20u64)),
                nonce,
            },
        );
        let mut store = Store::new("", EngineType::InMemory).unwrap();
        store.add_initial_state(genesis).await.unwrap();
        default_context_with_storage(store).await
    }

    fn nonce_request(address: Address, tag: BlockTag) -> GetTransactionCountRequest {
        use crate::types::block_identifier::BlockIdentifier;
        GetTransactionCountRequest {
            address,
            block: BlockIdentifierOrHash::Identifier(BlockIdentifier::Tag(tag)),
        }
    }

    fn stale_mempool_tx(address: Address, nonce: u64, context: &RpcApiContext) {
        use ethrex_common::types::{LegacyTransaction, MempoolTransaction, Transaction, TxKind};
        let tx = Transaction::LegacyTransaction(LegacyTransaction {
            nonce,
            gas: 21000,
            to: TxKind::Create,
            ..Default::default()
        });
        context
            .blockchain
            .mempool
            .add_transaction(
                H256::random(),
                address,
                MempoolTransaction::new(tx, address),
                None,
                None,
            )
            .unwrap();
    }

    /// Regression: a stale tx left in the pool with a nonce below the account's
    /// on-chain nonce must not make `pending` report a value lower than `latest`.
    #[tokio::test]
    async fn pending_nonce_is_clamped_to_latest() {
        let address = Address::from_low_u64_be(0xabcd);
        let context = context_with_account_nonce(address, 0x59).await;
        stale_mempool_tx(address, 0x50, &context);

        let latest = nonce_request(address, BlockTag::Latest)
            .handle(context.clone())
            .await
            .unwrap();
        let pending = nonce_request(address, BlockTag::Pending)
            .handle(context.clone())
            .await
            .unwrap();

        assert_eq!(latest, json!("0x59"));
        assert_eq!(pending, json!("0x59"));
    }

    /// A pending tx above the on-chain nonce still advances `pending`.
    #[tokio::test]
    async fn pending_nonce_advances_past_latest() {
        let address = Address::from_low_u64_be(0xabcd);
        let context = context_with_account_nonce(address, 0x59).await;
        // Highest pending nonce is 0x59, so the next usable nonce is 0x5a.
        stale_mempool_tx(address, 0x59, &context);

        let pending = nonce_request(address, BlockTag::Pending)
            .handle(context.clone())
            .await
            .unwrap();

        assert_eq!(pending, json!("0x5a"));
    }

    fn proof_params(keys: Value) -> Option<Vec<Box<RawValue>>> {
        raw_params(&Some(vec![
            json!("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"),
            keys,
            json!("latest"),
        ]))
    }

    fn proof_request(
        address: Address,
        keys: &[&str],
        block: BlockIdentifierOrHash,
    ) -> GetProofRequest {
        let (storage_keys, storage_key_echoes) = keys
            .iter()
            .map(|key| parse_storage_key(key).unwrap())
            .unzip();
        GetProofRequest {
            address,
            storage_keys,
            storage_key_echoes,
            block,
        }
    }

    fn latest() -> BlockIdentifierOrHash {
        use crate::types::block_identifier::BlockIdentifier;
        BlockIdentifierOrHash::Identifier(BlockIdentifier::Tag(BlockTag::Latest))
    }

    #[test]
    fn get_proof_parse_echoes_storage_keys() {
        let key_64 = "0x00000000000000000000000000000000000000000000000000000000000000AB";
        let key_63 = "0x0000000000000000000000000000000000000000000000000000000000000ab";
        let key_62 = "0x00000000000000000000000000000000000000000000000000000000000ab";
        let request = GetProofRequest::parse(&proof_params(json!([
            key_64, key_63, key_62, "0x", "0x0001", key_64
        ])))
        .unwrap();

        let slot_ab = H256::from_low_u64_be(0xab);
        assert_eq!(
            request.storage_keys,
            vec![
                slot_ab,
                slot_ab,
                slot_ab,
                H256::zero(),
                H256::from_low_u64_be(1),
                slot_ab
            ]
        );
        let padded_ab = format!("{slot_ab:#x}");
        assert_eq!(
            request.storage_key_echoes,
            vec![
                padded_ab.clone(),
                padded_ab.clone(),
                "0xab".to_string(),
                "0x0".to_string(),
                "0x1".to_string(),
                padded_ab
            ]
        );
    }

    #[test]
    fn get_proof_parse_accepts_the_storage_key_cap() {
        let keys = vec!["0x"; MAX_PROOF_STORAGE_KEYS];
        let request = GetProofRequest::parse(&proof_params(json!(keys))).unwrap();
        assert_eq!(request.storage_keys.len(), MAX_PROOF_STORAGE_KEYS);
    }

    #[test]
    fn get_proof_parse_rejects_more_storage_keys_than_the_cap() {
        // `"0x"` is a valid key, so a request body this size could otherwise
        // carry hundreds of thousands of them.
        let keys = vec!["0x"; MAX_PROOF_STORAGE_KEYS + 1];
        assert!(matches!(
            GetProofRequest::parse(&proof_params(json!(keys))),
            Err(RpcErr::LimitExceeded(ref msg)) if msg.contains("too many storage keys")
        ));
    }

    #[test]
    fn get_proof_parse_rejects_invalid_storage_keys() {
        let key_66 = format!("0x{}", "0".repeat(66));
        for keys in [
            json!(["0xzz"]),
            json!([key_66]),
            json!(["01"]),
            json!([1]),
            json!("0x1"),
        ] {
            let err = GetProofRequest::parse(&proof_params(keys.clone())).err();
            assert!(
                matches!(err, Some(RpcErr::InvalidParams(_))),
                "keys {keys} gave {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn get_proof_existing_account_echoes_keys_in_order() {
        let address = Address::from_low_u64_be(0xabcd);
        let context = context_with_account_nonce(address, 0x59).await;
        let key_64 = "0x0000000000000000000000000000000000000000000000000000000000000001";

        let response = proof_request(address, &[key_64, "0x", "0x01", key_64], latest())
            .handle(context.clone())
            .await
            .unwrap();

        assert_eq!(response["nonce"], json!("0x59"));
        assert_eq!(
            response["codeHash"],
            json!(format!(
                "{:#x}",
                *ethrex_common::constants::EMPTY_KECCAK_HASH
            ))
        );
        assert_eq!(
            response["storageHash"],
            json!(format!("{:#x}", *ethrex_common::constants::EMPTY_TRIE_HASH))
        );
        let keys: Vec<&Value> = response["storageProof"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| &entry["key"])
            .collect();
        assert_eq!(keys, vec![key_64, "0x0", "0x1", key_64]);
    }

    #[tokio::test]
    async fn get_proof_absent_account_has_zero_hashes() {
        let address = Address::from_low_u64_be(0xabcd);
        let context = context_with_account_nonce(address, 1).await;
        let absent = Address::from_low_u64_be(0xdead);

        let response = proof_request(absent, &["0x5", "0x5"], latest())
            .handle(context.clone())
            .await
            .unwrap();

        assert_eq!(response["codeHash"], json!(format!("{:#x}", H256::zero())));
        assert_eq!(
            response["storageHash"],
            json!(format!("{:#x}", H256::zero()))
        );
        assert_eq!(response["balance"], json!("0x0"));
        assert_eq!(response["nonce"], json!("0x0"));
        assert!(!response["accountProof"].as_array().unwrap().is_empty());
        let entry = json!({"key": "0x5", "value": "0x0", "proof": []});
        assert_eq!(response["storageProof"], json!([entry, entry]));
    }

    #[tokio::test]
    async fn get_proof_missing_state_error_depends_on_sync() {
        use crate::test_utils::add_empty_blocks;
        let address = Address::from_low_u64_be(0xabcd);
        let context = context_with_account_nonce(address, 1).await;
        let genesis = context.storage.get_block_header(0).unwrap().unwrap();
        // Test blocks carry a state root that the store does not hold.
        add_empty_blocks(&context.storage, 1).await;
        let block_one = context.storage.get_block_header(1).unwrap().unwrap();
        let mut head = genesis.clone();
        head.number = 2;
        head.parent_hash = block_one.hash();
        let head_hash = head.hash();
        context
            .storage
            .add_block(ethrex_common::types::Block::new(
                head,
                ethrex_common::types::BlockBody::default(),
            ))
            .await
            .unwrap();
        context
            .storage
            .forkchoice_update(vec![], 2, head_hash, None, None)
            .await
            .unwrap();

        let syncing = proof_request(address, &[], BlockIdentifierOrHash::Hash(block_one.hash()))
            .handle(context.clone())
            .await
            .unwrap_err();
        assert!(matches!(syncing, RpcErr::Internal(_)));

        context.blockchain.set_synced();
        let pruned = proof_request(address, &[], BlockIdentifierOrHash::Hash(block_one.hash()))
            .handle(context.clone())
            .await
            .unwrap_err();
        assert!(matches!(pruned, RpcErr::PrunedHistoryUnavailable(_)));

        // A stored block above the head has not been executed yet, so its state
        // can still arrive, even on a synced node.
        let mut above_head = block_one.clone();
        above_head.number = 3;
        above_head.parent_hash = head_hash;
        let above_head_hash = above_head.hash();
        context
            .storage
            .add_block(ethrex_common::types::Block::new(
                above_head,
                ethrex_common::types::BlockBody::default(),
            ))
            .await
            .unwrap();
        let not_executed =
            proof_request(address, &[], BlockIdentifierOrHash::Hash(above_head_hash))
                .handle(context.clone())
                .await
                .unwrap_err();
        assert!(matches!(not_executed, RpcErr::Internal(_)));
    }

    #[tokio::test]
    async fn get_proof_by_hash_uses_that_block_not_the_canonical_one() {
        use crate::test_utils::add_empty_blocks;
        let address = Address::from_low_u64_be(0xabcd);
        let context = context_with_account_nonce(address, 0x59).await;
        let genesis = context.storage.get_block_header(0).unwrap().unwrap();
        add_empty_blocks(&context.storage, 1).await;
        let canonical = context.storage.get_block_header(1).unwrap().unwrap();
        let mut fork = genesis.clone();
        fork.number = 1;
        fork.parent_hash = genesis.hash();
        fork.extra_data = vec![0xf0].into();
        let fork_hash = fork.hash();
        assert_ne!(fork_hash, canonical.hash());
        context.storage.add_block_headers(vec![fork]).await.unwrap();

        let fork_response = proof_request(address, &[], BlockIdentifierOrHash::Hash(fork_hash))
            .handle(context.clone())
            .await
            .unwrap();
        assert_eq!(fork_response["nonce"], json!("0x59"));

        let canonical_err =
            proof_request(address, &[], BlockIdentifierOrHash::Hash(canonical.hash()))
                .handle(context.clone())
                .await
                .unwrap_err();
        assert!(matches!(canonical_err, RpcErr::Internal(_)));
    }
}
