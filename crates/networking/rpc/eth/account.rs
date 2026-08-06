use serde_json::Value;
use tracing::debug;

use crate::rpc::{RpcApiContext, RpcHandler};
use crate::types::account_proof::{AccountProof, StorageProof};
use crate::types::block_identifier::{BlockIdentifierOrHash, BlockTag};
use crate::utils::RpcErr;
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
    pub block: BlockIdentifierOrHash,
}

impl RpcHandler for GetBalanceRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<GetBalanceRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 2 params".to_owned()));
        };
        Ok(GetBalanceRequest {
            address: serde_json::from_value(params[0].clone())?,
            block: BlockIdentifierOrHash::parse(params[1].clone(), 1)?,
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
    fn parse(params: &Option<Vec<Value>>) -> Result<GetCodeRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 2 params".to_owned()));
        };
        Ok(GetCodeRequest {
            address: serde_json::from_value(params[0].clone())?,
            block: BlockIdentifierOrHash::parse(params[1].clone(), 1)?,
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
    fn parse(params: &Option<Vec<Value>>) -> Result<GetStorageAtRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 3 {
            return Err(RpcErr::BadParams("Expected 3 params".to_owned()));
        };
        let storage_slot_u256 = serde_utils::u256::deser_hex_or_dec_str(params[1].clone())?;
        Ok(GetStorageAtRequest {
            address: serde_json::from_value(params[0].clone())?,
            storage_slot: H256::from_uint(&storage_slot_u256),
            block: BlockIdentifierOrHash::parse(params[2].clone(), 2)?,
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
    fn parse(params: &Option<Vec<Value>>) -> Result<GetTransactionCountRequest, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 2 {
            return Err(RpcErr::BadParams("Expected 2 params".to_owned()));
        };
        Ok(GetTransactionCountRequest {
            address: serde_json::from_value(params[0].clone())?,
            block: BlockIdentifierOrHash::parse(params[1].clone(), 1)?,
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

impl RpcHandler for GetProofRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        let params = params
            .as_ref()
            .ok_or(RpcErr::BadParams("No params provided".to_owned()))?;
        if params.len() != 3 {
            return Err(RpcErr::BadParams("Expected 3 params".to_owned()));
        };
        let storage_keys: Vec<U256> = serde_json::from_value(params[1].clone())?;
        let storage_keys = storage_keys.iter().map(H256::from_uint).collect();
        Ok(GetProofRequest {
            address: serde_json::from_value(params[0].clone())?,
            storage_keys,
            block: BlockIdentifierOrHash::parse(params[2].clone(), 2)?,
        })
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        let storage = &context.storage;
        debug!(
            "Requested proof for account {} at block {} with storage keys: {:?}",
            self.address, self.block, self.storage_keys
        );
        let Some(block_number) = self.block.resolve_block_number(storage).await? else {
            return Ok(Value::Null);
        };
        let Some(header) = storage.get_block_header(block_number)? else {
            return Ok(Value::Null);
        };
        // Past the EIP-8297 activation `header.state_root` is a binary-trie
        // root, and there is no MPT behind it to prove against. Refuse, rather
        // than serve anything.
        //
        // The other four state RPCs can follow the header into the binary trie
        // because their answers are values, and a value is the same value out of
        // either trie. A proof is not: it is a witness of a *shape*, and this
        // response type is the MPT's shape — a list of RLP-encoded MPT nodes
        // from root to leaf, plus a `storageHash` naming a per-account storage
        // subtrie. The binary trie has neither. Producing a binary-trie proof
        // format is a separate piece of work with its own wire type, and
        // inventing one here would be worse than an error: clients verify these
        // against `header.stateRoot`, so any shape we improvise would either
        // fail verification or, worse, be accepted by a client that does not
        // check what it is verifying.
        //
        // What must never happen is the third option, and it is the reason this
        // check is here rather than left to fail somewhere downstream: an
        // account object beside an empty `accountProof` array reads as a valid
        // *exclusion* proof for an account that exists. `Store::get_account_proof`
        // guards its own root for exactly that reason; this makes the refusal say
        // why instead of reporting a missing state root for state that is
        // present, just in the other trie.
        //
        // Per header, never per chain: a pre-activation block on the same chain
        // still has a real MPT proof, and still serves it.
        if storage
            .get_chain_config()
            .is_binary_tree_active(header.timestamp)
        {
            return Err(RpcErr::UnsupportedFork(format!(
                "eth_getProof is not available at block {block_number}: the chain has reached \
                 the binary-tree commitment (EIP-8297), whose state root cannot be proven in \
                 the Merkle-Patricia format this method returns"
            )));
        }
        // Create account proof
        let Some(account_proof) = storage
            .get_account_proof(header.state_root, self.address, &self.storage_keys)
            .await?
        else {
            return Err(RpcErr::Internal("Could not get account proof".to_owned()));
        };
        let storage_proof = account_proof
            .storage_proof
            .into_iter()
            .map(|sp| StorageProof {
                key: sp.key.into_uint(),
                value: sp.value,
                proof: sp.proof,
            })
            .collect();
        let account = account_proof.account;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_get_storage_at_request_parse_hex_slot() {
        let params = Some(vec![
            json!("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"),
            // Storage slot can be provided as hex string
            json!("0x1"),
            json!("latest"),
        ]);
        let request = GetStorageAtRequest::parse(&params).unwrap();

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
        let request = GetStorageAtRequest::parse(&params).unwrap();

        let expected_address = "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
            .parse()
            .unwrap();
        assert_eq!(request.address, expected_address);
        assert_eq!(request.storage_slot, H256::from_uint(&U256::from(1u64)));
        assert_eq!(request.block, BlockTag::Latest);
    }

    /// Builds an in-memory store whose genesis pre-sets `address`'s nonce, and a
    /// context over it. Mirrors `setup_store` but lets the test fix the on-chain
    /// nonce without executing blocks.
    async fn context_with_account_nonce(address: Address, nonce: u64) -> RpcApiContext {
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

    /// A state root no block on this chain ever produced. ethrex keeps one
    /// version of the state trie on disk plus a bounded chain of in-memory diff
    /// layers, so this is the shape of any block past the retention window: the
    /// header survives, the state behind it does not.
    const UNHELD_STATE_ROOT: H256 = H256::repeat_byte(0xaa);

    /// Appends a canonical block whose header claims [`UNHELD_STATE_ROOT`]
    /// without that state ever being written. Returns its number.
    async fn append_block_with_unheld_state_root(context: &RpcApiContext) -> u64 {
        use ethrex_common::types::{Block, BlockBody, BlockHeader};
        let storage = &context.storage;
        let genesis_hash = storage
            .get_canonical_block_hash(0)
            .await
            .unwrap()
            .expect("genesis is canonical");
        let header = BlockHeader {
            number: 1,
            parent_hash: genesis_hash,
            state_root: UNHELD_STATE_ROOT,
            ..Default::default()
        };
        let hash = header.hash();
        storage
            .add_block(Block::new(header, BlockBody::default()))
            .await
            .unwrap();
        storage
            .forkchoice_update(vec![(1, hash)], 1, hash, None, None)
            .await
            .unwrap();
        1
    }

    fn block_id(number: u64) -> BlockIdentifierOrHash {
        use crate::types::block_identifier::BlockIdentifier;
        BlockIdentifierOrHash::Identifier(BlockIdentifier::Number(number))
    }

    #[track_caller]
    fn assert_state_unavailable(result: Result<Value, RpcErr>, method: &str) {
        match result {
            Err(RpcErr::Internal(message)) => assert!(
                message.contains("state root missing"),
                "{method}: expected a missing-state error, got {message:?}"
            ),
            Err(other) => panic!("{method}: expected a missing-state error, got {other:?}"),
            Ok(value) => panic!(
                "{method}: answered {value} from a state this node does not hold. \
                 The store-level guard did not reach the caller."
            ),
        }
    }

    /// The store-level guard has to survive the trip out through the RPC layer.
    /// Every one of these handlers ends in an `unwrap_or_default()` over an
    /// `Option`, so a guard that only reported "no such account" would be
    /// flattened into `0x0` / `0x` and the user-visible bug would be untouched.
    /// What must happen instead is what `eth_call` has always done at the same
    /// block: fail, and say why.
    #[tokio::test]
    async fn account_reads_at_an_unheld_state_root_error() {
        let address = Address::from_low_u64_be(0xabcd);
        let context = context_with_account_nonce(address, 0x59).await;
        let number = append_block_with_unheld_state_root(&context).await;

        assert_state_unavailable(
            GetBalanceRequest {
                address,
                block: block_id(number),
            }
            .handle(context.clone())
            .await,
            "eth_getBalance",
        );
        assert_state_unavailable(
            GetCodeRequest {
                address,
                block: block_id(number),
            }
            .handle(context.clone())
            .await,
            "eth_getCode",
        );
        assert_state_unavailable(
            GetTransactionCountRequest {
                address,
                block: block_id(number),
            }
            .handle(context.clone())
            .await,
            "eth_getTransactionCount",
        );
        assert_state_unavailable(
            GetStorageAtRequest {
                address,
                storage_slot: H256::from_low_u64_be(1),
                block: block_id(number),
            }
            .handle(context.clone())
            .await,
            "eth_getStorageAt",
        );
        assert_state_unavailable(
            GetProofRequest {
                address,
                storage_keys: vec![H256::from_low_u64_be(1)],
                block: block_id(number),
            }
            .handle(context.clone())
            .await,
            "eth_getProof",
        );
    }

    /// The same handlers at a block whose state the node does hold keep working.
    #[tokio::test]
    async fn account_reads_at_a_held_state_root_still_answer() {
        let address = Address::from_low_u64_be(0xabcd);
        let context = context_with_account_nonce(address, 0x59).await;
        // Genesis is still canonical and its state is on disk.
        append_block_with_unheld_state_root(&context).await;

        let balance = GetBalanceRequest {
            address,
            block: block_id(0),
        }
        .handle(context.clone())
        .await
        .expect("genesis state is held");
        assert_eq!(balance, json!("0x56bc75e2d63100000"));

        let nonce = GetTransactionCountRequest {
            address,
            block: block_id(0),
        }
        .handle(context.clone())
        .await
        .expect("genesis state is held");
        assert_eq!(nonce, json!("0x59"));

        let proof = GetProofRequest {
            address,
            storage_keys: vec![],
            block: block_id(0),
        }
        .handle(context.clone())
        .await
        .expect("genesis state is held");
        assert!(
            !proof["accountProof"]
                .as_array()
                .expect("accountProof array")
                .is_empty(),
            "a held root must produce a non-empty account proof"
        );
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

    // =======================================================================
    // EIP-8297 — the state RPCs past the binary-tree activation.
    //
    // From `binaryTreeTime` onward a header's `state_root` is a binary-trie
    // root, which names no MPT. Every handler below therefore has to ask
    // `is_binary_tree_active(header.timestamp)` of *the header being queried*
    // and read the trie that root actually addresses. The question is never
    // chain-level: a chain that merely *schedules* the commitment still has a
    // pre-activation history whose headers commit MPT roots forever, which
    // `scheduled_but_pre_activation_blocks_still_read_the_mpt` pins.
    //
    // Activation at genesis is a test-only shape — it changes the genesis hash,
    // and hence the chain's identity (see `Genesis::compute_state_root`) — but
    // it is the cheapest way to get an *active header* without building blocks,
    // and one active header is all a per-header branch needs to be exercised.
    // The cross-boundary version, on a real chain that flips mid-history, lives
    // in `test/tests/blockchain/binary_tree_shadow_tests.rs`.
    // =======================================================================

    /// Address of the account the binary-tree tests probe.
    fn probe_address() -> Address {
        Address::from_low_u64_be(0x5eed)
    }

    const PROBE_BALANCE: u64 = 0x1234;
    const PROBE_NONCE: u64 = 7;
    const PROBE_SLOT: u64 = 1;
    const PROBE_SLOT_VALUE: u64 = 0x2a;

    /// A probe account carrying all four things the state RPCs read: a balance,
    /// a nonce, code, and one non-zero storage slot.
    fn probe_account() -> ethrex_common::types::GenesisAccount {
        use ethrex_common::types::GenesisAccount;
        GenesisAccount {
            code: bytes::Bytes::from_static(&[0x60, 0x00, 0x60, 0x00, 0xf3]),
            storage: [(U256::from(PROBE_SLOT), U256::from(PROBE_SLOT_VALUE))]
                .into_iter()
                .collect(),
            balance: U256::from(PROBE_BALANCE),
            nonce: PROBE_NONCE,
        }
    }

    /// Builds a context whose genesis carries [`probe_account`] and whose chain
    /// config schedules `binaryTreeTime` at `binary_tree_time`.
    ///
    /// Passing genesis' own timestamp makes genesis itself active, so its header
    /// commits the binary root and `add_initial_state` seeds the persistent
    /// binary trie from the alloc. Passing a far-future timestamp gives a
    /// *scheduled but not active* chain; `None` gives an ordinary MPT chain.
    async fn binary_tree_context(binary_tree_time: Option<u64>) -> RpcApiContext {
        use crate::test_utils::{TEST_GENESIS, default_context_with_storage};
        use ethrex_common::types::Genesis;
        use ethrex_storage::{EngineType, Store};

        let mut genesis: Genesis = serde_json::from_str(TEST_GENESIS).unwrap();
        genesis.alloc.insert(probe_address(), probe_account());
        genesis.config.binary_tree_time = binary_tree_time;
        let mut store = Store::new("", EngineType::InMemory).unwrap();
        store.add_initial_state(genesis).await.unwrap();
        default_context_with_storage(store).await
    }

    /// A context whose genesis header is already past the activation.
    async fn active_at_genesis_context() -> RpcApiContext {
        let context = binary_tree_context(Some(genesis_timestamp())).await;
        // Guard against a vacuous pass: if genesis were not active the whole
        // section below would be testing the MPT path under a binary name.
        assert!(
            context
                .storage
                .get_chain_config()
                .is_binary_tree_active(genesis_timestamp()),
            "genesis must be past the activation for these tests to mean anything"
        );
        context
    }

    /// The fixture genesis' timestamp, read from the fixture rather than pinned
    /// here so a change to it cannot silently un-schedule these tests.
    fn genesis_timestamp() -> u64 {
        use crate::test_utils::TEST_GENESIS;
        use ethrex_common::types::Genesis;
        let genesis: Genesis = serde_json::from_str(TEST_GENESIS).unwrap();
        genesis.timestamp
    }

    /// Appends a canonical block at `timestamp` whose header claims
    /// [`UNHELD_STATE_ROOT`], with no state of either shape behind it. On an
    /// active timestamp that means no *binary* state is recorded for it, which
    /// is the binary-side counterpart of the unheld-MPT-root case.
    async fn append_block_at(context: &RpcApiContext, timestamp: u64) -> u64 {
        use ethrex_common::types::{Block, BlockBody, BlockHeader};
        let storage = &context.storage;
        let genesis_hash = storage
            .get_canonical_block_hash(0)
            .await
            .unwrap()
            .expect("genesis is canonical");
        let header = BlockHeader {
            number: 1,
            parent_hash: genesis_hash,
            state_root: UNHELD_STATE_ROOT,
            timestamp,
            ..Default::default()
        };
        let hash = header.hash();
        storage
            .add_block(Block::new(header, BlockBody::default()))
            .await
            .unwrap();
        storage
            .forkchoice_update(vec![(1, hash)], 1, hash, None, None)
            .await
            .unwrap();
        1
    }

    /// 1. Balance, nonce, code and storage all answer at an active header, out
    ///    of the binary trie, with the values the alloc put there.
    #[tokio::test]
    async fn state_reads_answer_from_the_binary_trie_at_an_active_header() {
        let context = active_at_genesis_context().await;
        let address = probe_address();

        let balance = GetBalanceRequest {
            address,
            block: block_id(0),
        }
        .handle(context.clone())
        .await
        .expect("eth_getBalance must resolve through the binary trie");
        assert_eq!(balance, json!(format!("{:#x}", PROBE_BALANCE)));

        let nonce = GetTransactionCountRequest {
            address,
            block: block_id(0),
        }
        .handle(context.clone())
        .await
        .expect("eth_getTransactionCount must resolve through the binary trie");
        assert_eq!(nonce, json!(format!("0x{:x}", PROBE_NONCE)));

        let code = GetCodeRequest {
            address,
            block: block_id(0),
        }
        .handle(context.clone())
        .await
        .expect("eth_getCode must resolve through the binary trie");
        assert_eq!(code, json!("0x60006000f3"));

        let slot = GetStorageAtRequest {
            address,
            storage_slot: H256::from_low_u64_be(PROBE_SLOT),
            block: block_id(0),
        }
        .handle(context.clone())
        .await
        .expect("eth_getStorageAt must resolve through the binary trie");
        assert_eq!(
            slot,
            json!(format!("{:#x}", H256::from_low_u64_be(PROBE_SLOT_VALUE)))
        );

        // An unwritten slot reads as zero, not as some other slot's value.
        let empty_slot = GetStorageAtRequest {
            address,
            storage_slot: H256::from_low_u64_be(0xdead),
            block: block_id(0),
        }
        .handle(context.clone())
        .await
        .expect("an absent slot is not an error");
        assert_eq!(empty_slot, json!(format!("{:#x}", H256::zero())));

        // An account the alloc never mentioned is absent, not a stray hit.
        let absent = GetBalanceRequest {
            address: Address::from_low_u64_be(0xffff_ffff),
            block: block_id(0),
        }
        .handle(context.clone())
        .await
        .expect("an absent account is not an error");
        assert_eq!(absent, json!("0x0"));
    }

    /// 2. A chain that merely *schedules* the commitment still reads its
    ///    pre-activation headers through the MPT. This is the falsification
    ///    target: swapping the per-header `is_binary_tree_active(timestamp)` for
    ///    a chain-level `binary_tree_scheduled()` makes every read here try the
    ///    binary trie at an MPT root and fail.
    #[tokio::test]
    async fn scheduled_but_pre_activation_blocks_still_read_the_mpt() {
        let far_future = genesis_timestamp() + 1_000_000;
        let context = binary_tree_context(Some(far_future)).await;
        assert!(context.storage.get_chain_config().binary_tree_scheduled());
        assert!(
            !context
                .storage
                .get_chain_config()
                .is_binary_tree_active(genesis_timestamp())
        );
        let address = probe_address();

        let balance = GetBalanceRequest {
            address,
            block: block_id(0),
        }
        .handle(context.clone())
        .await
        .expect("a scheduled-but-inactive header still resolves against the MPT");
        assert_eq!(balance, json!(format!("{:#x}", PROBE_BALANCE)));

        let nonce = GetTransactionCountRequest {
            address,
            block: block_id(0),
        }
        .handle(context.clone())
        .await
        .expect("a scheduled-but-inactive header still resolves against the MPT");
        assert_eq!(nonce, json!(format!("0x{:x}", PROBE_NONCE)));

        let slot = GetStorageAtRequest {
            address,
            storage_slot: H256::from_low_u64_be(PROBE_SLOT),
            block: block_id(0),
        }
        .handle(context.clone())
        .await
        .expect("a scheduled-but-inactive header still resolves against the MPT");
        assert_eq!(
            slot,
            json!(format!("{:#x}", H256::from_low_u64_be(PROBE_SLOT_VALUE)))
        );

        // And `eth_getProof` is a real MPT proof here, not the binary-tree
        // refusal: the block is before the flip.
        let proof = GetProofRequest {
            address,
            storage_keys: vec![H256::from_low_u64_be(PROBE_SLOT)],
            block: block_id(0),
        }
        .handle(context.clone())
        .await
        .expect("a pre-activation header still has an MPT proof");
        assert!(
            !proof["accountProof"]
                .as_array()
                .expect("accountProof array")
                .is_empty()
        );
    }

    /// 3. The staleness guard still fires on the binary side: an active header
    ///    whose binary state this node does not hold must error rather than
    ///    answer from whatever the single-version binary trie currently holds.
    #[tokio::test]
    async fn active_header_without_binary_state_errors() {
        let context = active_at_genesis_context().await;
        let address = probe_address();
        // One second past genesis, so the header is active — and no binary root
        // was ever recorded for it.
        let number = append_block_at(&context, genesis_timestamp() + 1).await;

        assert_state_unavailable(
            GetBalanceRequest {
                address,
                block: block_id(number),
            }
            .handle(context.clone())
            .await,
            "eth_getBalance",
        );
        assert_state_unavailable(
            GetCodeRequest {
                address,
                block: block_id(number),
            }
            .handle(context.clone())
            .await,
            "eth_getCode",
        );
        assert_state_unavailable(
            GetTransactionCountRequest {
                address,
                block: block_id(number),
            }
            .handle(context.clone())
            .await,
            "eth_getTransactionCount",
        );
        assert_state_unavailable(
            GetStorageAtRequest {
                address,
                storage_slot: H256::from_low_u64_be(PROBE_SLOT),
                block: block_id(number),
            }
            .handle(context.clone())
            .await,
            "eth_getStorageAt",
        );
    }

    /// 4. `eth_getProof` on an active header refuses, clearly and permanently.
    ///
    /// The binary trie has no MPT-shaped proof to give and this branch does not
    /// invent one, so the only two honest answers are "refuse" and "serve a
    /// different format". What must never happen is the third thing — an
    /// account object beside an empty `accountProof`, which reads as a valid
    /// exclusion proof for an account that exists.
    #[tokio::test]
    async fn get_proof_refuses_on_an_active_header_and_is_never_self_inconsistent() {
        let context = active_at_genesis_context().await;
        let result = GetProofRequest {
            address: probe_address(),
            storage_keys: vec![H256::from_low_u64_be(PROBE_SLOT)],
            block: block_id(0),
        }
        .handle(context.clone())
        .await;

        match result {
            Err(RpcErr::UnsupportedFork(message)) => {
                assert!(
                    message.contains("eth_getProof"),
                    "the refusal must name the method, got {message:?}"
                );
                assert!(
                    message.to_lowercase().contains("binary"),
                    "the refusal must say why, got {message:?}"
                );
            }
            Err(other) => panic!("expected an unsupported-fork refusal, got {other:?}"),
            Ok(value) => panic!(
                "eth_getProof answered {value} at a binary-tree block. \
                 A proof over a trie whose shape it cannot express is never a valid answer."
            ),
        }
    }

    /// 5. An unscheduled chain is untouched: no branch, no behaviour change.
    #[tokio::test]
    async fn unscheduled_chains_are_unchanged() {
        let context = binary_tree_context(None).await;
        assert!(!context.storage.get_chain_config().binary_tree_scheduled());
        let address = probe_address();

        let balance = GetBalanceRequest {
            address,
            block: block_id(0),
        }
        .handle(context.clone())
        .await
        .unwrap();
        assert_eq!(balance, json!(format!("{:#x}", PROBE_BALANCE)));

        let proof = GetProofRequest {
            address,
            storage_keys: vec![H256::from_low_u64_be(PROBE_SLOT)],
            block: block_id(0),
        }
        .handle(context.clone())
        .await
        .expect("an unscheduled chain still proves");
        assert!(
            !proof["accountProof"]
                .as_array()
                .expect("accountProof array")
                .is_empty()
        );
        assert_eq!(
            proof["storageProof"][0]["value"],
            json!(format!("{:#x}", PROBE_SLOT_VALUE))
        );
    }
}
