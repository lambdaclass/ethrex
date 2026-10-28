//! An in-process chain with real tries, real block access lists and real receipts, built
//! with ethrex's own payload builder on the Amsterdam test genesis. Tests use it as the
//! "world" a partial node proves facts against.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]

use std::collections::HashMap;

use bytes::Bytes;
use ethrex_blockchain::fork_choice::apply_fork_choice;
use ethrex_blockchain::payload::{BuildPayloadArgs, create_payload};
use ethrex_blockchain::{Blockchain, BlockchainOptions};
use ethrex_common::types::block_access_list::BlockAccessList;
use ethrex_common::types::{
    BlockBody, BlockHeader, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, ELASTICITY_MULTIPLIER,
    Genesis, Receipt, Transaction, TxKind,
};
use ethrex_common::{Address, H256, U256};
use ethrex_p2p::rlpx::snap::{GetAccountRange, GetStorageRanges};
use ethrex_p2p::snap::constants::MAX_RESPONSE_BYTES;
use ethrex_p2p::snap::{process_account_range_request, process_storage_ranges_request};
use ethrex_rlp::structs::Encoder;
use ethrex_storage::{EngineType, Store};
use secp256k1::{Message, PublicKey, SECP256K1, SecretKey};

use crate::state::Key;
use crate::verify::{
    self, AccountQuery, AccountRangeRequest, AccountReply, Proven, SlotProof, StorageRangeRequest,
    StorageReply, TrustedHash, VerifiedHeader, keccak,
};

const GENESIS_JSON: &str = include_str!("../../../fixtures/genesis/l1-bal.json");
const PRIVATE_KEYS: &str = include_str!("../../../fixtures/keys/private_keys_tests.txt");

/// Fee recipient of every mined block.
pub const COINBASE: Address = Address::repeat_byte(0xc0);

/// `1 ether`.
pub fn eth(n: u64) -> U256 {
    U256::from(n) * U256::exp10(18)
}

/// A funded test signer.
#[derive(Clone, Debug)]
pub struct Signer {
    key: SecretKey,
    /// The signer's address.
    pub address: Address,
}

/// The in-process chain.
pub struct World {
    store: Store,
    blockchain: Blockchain,
    head: H256,
    slot: u64,
    signers: Vec<Signer>,
    chain_id: u64,
    /// Transactions that failed admission to the mempool while mining the last block.
    pub rejected: Vec<String>,
}

impl World {
    /// A chain at the Amsterdam test genesis with its funded accounts as signers.
    pub async fn new() -> Self {
        let genesis: Genesis = serde_json::from_str(GENESIS_JSON).expect("test genesis parses");
        let chain_id = genesis.config.chain_id;
        let mut store = Store::new("world", EngineType::InMemory).expect("in-memory store");
        store
            .add_initial_state(genesis)
            .await
            .expect("genesis state");
        let blockchain = Blockchain::new(
            store.clone(),
            BlockchainOptions {
                min_tip_wei: 0,
                mempool_prewarm_enabled: false,
                ..Default::default()
            },
        );
        let head = store
            .get_block_header(0)
            .expect("genesis header read")
            .expect("genesis header exists")
            .hash();
        let signers = PRIVATE_KEYS
            .lines()
            .filter(|line| !line.trim().is_empty())
            .take(8)
            .map(|line| {
                let hex = line.trim().trim_start_matches("0x");
                let bytes = hex::decode(hex).expect("test key hex");
                let key = SecretKey::from_slice(&bytes).expect("test key");
                Signer {
                    key,
                    address: address_of(&key),
                }
            })
            .collect();
        Self {
            store,
            blockchain,
            head,
            slot: 0,
            signers,
            chain_id,
            rejected: Vec::new(),
        }
    }

    /// The backing store.
    pub const fn store(&self) -> &Store {
        &self.store
    }

    /// The chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// The head block hash.
    pub const fn head(&self) -> H256 {
        self.head
    }

    /// The `i`th funded signer.
    pub fn signer(&self, i: usize) -> &Signer {
        self.signers.get(i).expect("enough test signers")
    }

    /// A header by hash.
    pub fn header(&self, hash: H256) -> BlockHeader {
        self.store
            .get_block_header_by_hash(hash)
            .expect("header read")
            .expect("header exists")
    }

    /// A header the light client would have vouched for.
    pub fn verified(&self, hash: H256) -> VerifiedHeader {
        verify::header(TrustedHash::attested(hash), self.header(hash)).expect("hash matches")
    }

    /// The headers strictly between `from` and `to`, newest first, as a falling
    /// `GetBlockHeaders` returns them.
    pub fn headers_between(&self, from: H256, to: H256) -> Vec<BlockHeader> {
        let mut out = Vec::new();
        let mut cursor = self.header(to).parent_hash;
        while cursor != from {
            let header = self.header(cursor);
            cursor = header.parent_hash;
            out.push(header);
            if out.len() > 10_000 {
                panic!("headers_between: `from` is not an ancestor of `to`");
            }
        }
        out
    }

    /// The block access list of a mined block, as a peer would serve it.
    pub fn bal(&self, hash: H256) -> Option<BlockAccessList> {
        self.store.get_block_access_list(hash).expect("bal read")
    }

    /// The body of a mined block.
    pub async fn body(&self, hash: H256) -> BlockBody {
        self.store
            .get_block_body_by_hash(hash)
            .await
            .expect("body read")
            .expect("body exists")
    }

    /// The receipts of a mined block.
    pub async fn receipts(&self, hash: H256) -> Vec<Receipt> {
        self.store
            .get_receipts_for_block(&hash)
            .await
            .expect("receipts read")
    }

    /// Code by hash, as a peer would serve it.
    pub fn code(&self, hash: H256) -> Option<Bytes> {
        self.store
            .get_account_code(hash)
            .expect("code read")
            .map(|code| code.code_bytes())
    }

    /// Mines one block holding `txs`, in order, and makes it the head.
    pub async fn mine(&mut self, txs: Vec<Transaction>) -> H256 {
        self.rejected.clear();
        for tx in txs {
            if let Err(err) = self.blockchain.add_transaction_to_pool(tx).await {
                self.rejected.push(err.to_string());
            }
        }
        let parent = self.header(self.head);
        self.slot += 1;
        let args = BuildPayloadArgs {
            parent: self.head,
            timestamp: parent.timestamp + 12,
            fee_recipient: COINBASE,
            random: H256::zero(),
            withdrawals: Some(Vec::new()),
            beacon_root: Some(H256::zero()),
            slot_number: Some(self.slot),
            version: 1,
            elasticity_multiplier: ELASTICITY_MULTIPLIER,
            gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
        };
        let payload = create_payload(&args, &self.store, Bytes::new()).expect("payload");
        let result = self
            .blockchain
            .build_payload(payload)
            .expect("build payload");
        let block = result.payload;
        let hash = block.hash();
        self.blockchain
            .remove_block_transactions_from_pool(&block)
            .expect("mempool cleanup");
        self.blockchain.add_block(block).expect("import block");
        apply_fork_choice(&self.store, hash, hash, hash, None)
            .await
            .expect("fork choice");
        if let Some(bal) = result.block_access_list {
            self.store
                .store_block_access_list(hash, &bal)
                .expect("store bal");
        }
        self.head = hash;
        hash
    }

    /// A signed EIP-1559 transfer from signer `from`. The gas limit covers an EIP-8037
    /// account creation, so transfers to fresh addresses succeed.
    pub fn transfer(&self, from: usize, nonce: u64, to: Address, value: U256) -> Transaction {
        self.eip1559(from, nonce, TxKind::Call(to), value, Bytes::new(), 400_000)
    }

    /// A signed EIP-1559 call from signer `from`.
    pub fn call(&self, from: usize, nonce: u64, to: Address, data: Bytes) -> Transaction {
        self.eip1559(from, nonce, TxKind::Call(to), U256::zero(), data, 1_000_000)
    }

    /// A signed EIP-1559 create from signer `from`.
    pub fn deploy(&self, from: usize, nonce: u64, initcode: Bytes) -> Transaction {
        self.eip1559(
            from,
            nonce,
            TxKind::Create,
            U256::zero(),
            initcode,
            2_000_000,
        )
    }

    fn eip1559(
        &self,
        from: usize,
        nonce: u64,
        to: TxKind,
        value: U256,
        data: Bytes,
        gas_limit: u64,
    ) -> Transaction {
        let signer = self.signer(from);
        let mut tx = EIP1559Transaction {
            chain_id: self.chain_id,
            nonce,
            max_priority_fee_per_gas: 1_000_000_000,
            max_fee_per_gas: 100_000_000_000,
            gas_limit,
            to,
            value,
            data,
            ..Default::default()
        };
        let mut buf = vec![0x02u8];
        Encoder::new(&mut buf)
            .encode_field(&tx.chain_id)
            .encode_field(&tx.nonce)
            .encode_field(&tx.max_priority_fee_per_gas)
            .encode_field(&tx.max_fee_per_gas)
            .encode_field(&tx.gas_limit)
            .encode_field(&tx.to)
            .encode_field(&tx.value)
            .encode_field(&tx.data)
            .encode_field(&tx.access_list)
            .finish();
        let digest = keccak(&buf);
        let message = Message::from_digest(digest.0);
        let (recovery_id, compact) = SECP256K1
            .sign_ecdsa_recoverable(&message, &signer.key)
            .serialize_compact();
        tx.signature_r = U256::from_big_endian(compact.get(..32).expect("r"));
        tx.signature_s = U256::from_big_endian(compact.get(32..).expect("s"));
        tx.signature_y_parity = i32::from(recovery_id) != 0;
        Transaction::EIP1559Transaction(tx)
    }

    /// Answers a single-key `GetAccountRange` from the store, as a snap server would.
    pub async fn account_reply(&self, request: &AccountRangeRequest) -> AccountReply {
        let response = process_account_range_request(
            GetAccountRange {
                id: 0,
                root_hash: request.root,
                starting_hash: request.key,
                limit_hash: request.key,
                response_bytes: MAX_RESPONSE_BYTES,
            },
            self.store.clone(),
        )
        .await;
        match response {
            Ok(range) => AccountReply {
                accounts: range
                    .accounts
                    .into_iter()
                    .map(|unit| (unit.hash, unit.account))
                    .collect(),
                proof: range.proof.into_iter().map(|node| node.to_vec()).collect(),
            },
            Err(_) => AccountReply::default(),
        }
    }

    /// Answers a single-key `GetStorageRanges` from the store, as a snap server would.
    pub async fn storage_reply(&self, request: &StorageRangeRequest) -> StorageReply {
        let response = process_storage_ranges_request(
            GetStorageRanges {
                id: 0,
                root_hash: request.root,
                account_hashes: vec![request.account],
                starting_hash: request.slot,
                limit_hash: request.slot,
                response_bytes: MAX_RESPONSE_BYTES,
            },
            self.store.clone(),
        )
        .await;
        match response {
            Ok(ranges) => StorageReply {
                slots: ranges
                    .slots
                    .into_iter()
                    .next()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|slot| (slot.hash, slot.data))
                    .collect(),
                proof: ranges.proof.into_iter().map(|node| node.to_vec()).collect(),
            },
            Err(_) => StorageReply::default(),
        }
    }

    /// Proves `key` at `at` the way the node would, through the snap replies.
    pub async fn prove(&self, at: &VerifiedHeader, key: Key) -> Proven {
        match key {
            Key::Account(address) => self.prove_account(at, address).await.proven(),
            Key::Slot(address, slot) => {
                let account = self.prove_account(at, address).await;
                match account.slot(slot) {
                    SlotProof::Known(proven) => proven,
                    SlotProof::Wanted(query) => query
                        .verify(self.storage_reply(&query.request()).await)
                        .expect("storage proof verifies"),
                }
            }
        }
    }

    /// Proves the account at `address` at `at`.
    pub async fn prove_account(
        &self,
        at: &VerifiedHeader,
        address: Address,
    ) -> verify::ProvenAccount {
        let query = AccountQuery::new(at, address);
        query
            .verify(self.account_reply(&query.request()).await)
            .expect("account proof verifies")
    }

    /// Proves code by hash.
    pub fn prove_code(&self, hash: H256) -> Proven {
        verify::CodeQuery::new(hash)
            .verify(self.code(hash))
            .expect("code verifies")
    }

    /// The account state of `address` at the head, straight from the store.
    pub fn account_at_head(&self, address: Address) -> Option<ethrex_common::types::AccountState> {
        let root = self.header(self.head).state_root;
        self.store
            .get_account_state_by_root(root, address)
            .expect("account read")
    }

    /// Balances at the head of every signer, by address.
    pub fn balances(&self) -> HashMap<Address, U256> {
        self.signers
            .iter()
            .map(|s| {
                (
                    s.address,
                    self.account_at_head(s.address)
                        .map(|a| a.balance)
                        .unwrap_or_default(),
                )
            })
            .collect()
    }
}

fn address_of(key: &SecretKey) -> Address {
    let public = PublicKey::from_secret_key(SECP256K1, key).serialize_uncompressed();
    let hash = keccak(public.get(1..).expect("65-byte key"));
    Address::from_slice(hash.0.get(12..).expect("20 bytes"))
}

/// `PUSH1 v PUSH1 slot SSTORE` for each pair, then `STOP`: runtime code that writes storage.
pub fn sstore_code(writes: &[(u8, u8)]) -> Bytes {
    let mut code = Vec::new();
    for (slot, value) in writes {
        code.extend_from_slice(&[0x60, *value, 0x60, *slot, 0x55]);
    }
    code.push(0x00);
    Bytes::from(code)
}

/// Init code that performs `writes` as `SSTORE`s and then returns `runtime` as the deployed
/// code.
pub fn deploy_code(writes: &[(u8, u8)], runtime: &[u8]) -> Bytes {
    let len = u8::try_from(runtime.len()).expect("short runtime");
    let mut code = Vec::new();
    for (slot, value) in writes {
        code.extend_from_slice(&[0x60, *value, 0x60, *slot, 0x55]);
    }
    // PUSH1 len, DUP1, PUSH1 offset, PUSH1 0, CODECOPY, PUSH1 0, RETURN, STOP
    let offset = u8::try_from(code.len() + 12).expect("short init code");
    code.extend_from_slice(&[
        0x60, len, 0x80, 0x60, offset, 0x60, 0x00, 0x39, 0x60, 0x00, 0xf3, 0x00,
    ]);
    code.extend_from_slice(runtime);
    Bytes::from(code)
}

/// Runtime code: `SSTORE(calldata[0..32], calldata[32..64])`, then `STOP`.
pub fn writer_runtime() -> Vec<u8> {
    vec![0x60, 0x20, 0x35, 0x60, 0x00, 0x35, 0x55, 0x00]
}

/// Runtime code: `return SLOAD(calldata[0..32])`.
pub fn reader_runtime() -> Vec<u8> {
    vec![
        0x60, 0x00, 0x35, 0x54, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3,
    ]
}

// ── An offline peer set over the world ────────────────────────────────────────

use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use tokio::sync::Mutex as AsyncMutex;

use crate::peers::{Census, NetError, PeerKey, Peers, Reply};
use crate::state::Store as PartialStore;

/// A misbehaviour the offline peer set can be told to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Every block access list reply is `None`.
    WithholdBals,
    /// Account and storage replies for `Address` are empty, as from a peer outside its
    /// snap window.
    Unavailable(Address),
    /// The reply for `Address` reports this balance.
    LieAboutBalance(Address, U256),
    /// Header requests fail.
    RejectHeaders,
}

/// The id the lying peer answers under.
pub const LIAR: PeerKey = H256::repeat_byte(0x11);
/// The id honest replies carry.
pub const HONEST: PeerKey = H256::repeat_byte(0x22);

/// [`Peers`] answered by a shared [`World`], with optional faults.
pub struct WorldPeers {
    world: Arc<AsyncMutex<World>>,
    faults: StdMutex<Vec<Fault>>,
    /// Peers penalized, in order.
    pub penalized: StdMutex<Vec<PeerKey>>,
    /// Transactions broadcast, in order.
    pub broadcasts: StdMutex<Vec<Transaction>>,
    /// Account range requests served.
    pub account_requests: AtomicUsize,
    /// Storage range requests served.
    pub storage_requests: AtomicUsize,
    /// Block access list requests served.
    pub bal_requests: AtomicUsize,
}

use std::sync::Arc;

impl WorldPeers {
    /// Peers over `world`.
    pub fn new(world: Arc<AsyncMutex<World>>) -> Arc<Self> {
        Arc::new(Self {
            world,
            faults: StdMutex::new(Vec::new()),
            penalized: StdMutex::new(Vec::new()),
            broadcasts: StdMutex::new(Vec::new()),
            account_requests: AtomicUsize::new(0),
            storage_requests: AtomicUsize::new(0),
            bal_requests: AtomicUsize::new(0),
        })
    }

    /// Adds a fault.
    pub fn fault(&self, fault: Fault) {
        self.faults.lock().expect("faults").push(fault);
    }

    /// Removes every fault.
    pub fn heal(&self) {
        self.faults.lock().expect("faults").clear();
    }

    fn has(&self, fault: &Fault) -> bool {
        self.faults.lock().expect("faults").contains(fault)
    }

    fn faults(&self) -> Vec<Fault> {
        self.faults.lock().expect("faults").clone()
    }

    fn address_of_key(world: &World, key: H256) -> Option<Address> {
        world
            .signers
            .iter()
            .map(|s| s.address)
            .chain(std::iter::once(COINBASE))
            .find(|a| keccak(a.as_bytes()) == key)
    }
}

#[async_trait]
impl Peers for WorldPeers {
    async fn headers(&self, start: H256, count: u64) -> Result<Reply<Vec<BlockHeader>>, NetError> {
        if self.has(&Fault::RejectHeaders) {
            return Err(NetError::Rejected("headers refused".into()));
        }
        let world = self.world.lock().await;
        let mut out = Vec::new();
        let mut cursor = start;
        for _ in 0..count {
            let Some(header) = world
                .store
                .get_block_header_by_hash(cursor)
                .expect("header read")
            else {
                break;
            };
            cursor = header.parent_hash;
            let at_genesis = header.number == 0;
            out.push(header);
            if at_genesis {
                break;
            }
        }
        Ok(Reply {
            peer: HONEST,
            value: out,
        })
    }

    async fn account_range(
        &self,
        request: AccountRangeRequest,
    ) -> Result<Reply<AccountReply>, NetError> {
        self.account_requests.fetch_add(1, Ordering::SeqCst);
        let world = self.world.lock().await;
        let address = Self::address_of_key(&world, request.key);
        for fault in self.faults() {
            match fault {
                Fault::Unavailable(a) if Some(a) == address => {
                    return Ok(Reply {
                        peer: HONEST,
                        value: AccountReply::default(),
                    });
                }
                Fault::LieAboutBalance(a, balance) if Some(a) == address => {
                    let mut reply = world.account_reply(&request).await;
                    if let Some((_, account)) =
                        reply.accounts.iter_mut().find(|(h, _)| *h == request.key)
                    {
                        account.balance = balance;
                    }
                    return Ok(Reply {
                        peer: LIAR,
                        value: reply,
                    });
                }
                _ => {}
            }
        }
        Ok(Reply {
            peer: HONEST,
            value: world.account_reply(&request).await,
        })
    }

    async fn storage_range(
        &self,
        request: StorageRangeRequest,
    ) -> Result<Reply<StorageReply>, NetError> {
        self.storage_requests.fetch_add(1, Ordering::SeqCst);
        let world = self.world.lock().await;
        let address = Self::address_of_key(&world, request.account);
        if let Some(address) = address
            && self.has(&Fault::Unavailable(address))
        {
            return Ok(Reply {
                peer: HONEST,
                value: StorageReply::default(),
            });
        }
        Ok(Reply {
            peer: HONEST,
            value: world.storage_reply(&request).await,
        })
    }

    async fn byte_codes(&self, hashes: Vec<H256>) -> Result<Reply<Vec<Option<Bytes>>>, NetError> {
        let world = self.world.lock().await;
        Ok(Reply {
            peer: HONEST,
            value: hashes.into_iter().map(|h| world.code(h)).collect(),
        })
    }

    async fn block_access_lists(
        &self,
        headers: Vec<BlockHeader>,
    ) -> Result<Reply<Vec<Option<BlockAccessList>>>, NetError> {
        self.bal_requests.fetch_add(1, Ordering::SeqCst);
        if self.has(&Fault::WithholdBals) {
            return Ok(Reply {
                peer: HONEST,
                value: headers.iter().map(|_| None).collect(),
            });
        }
        let world = self.world.lock().await;
        Ok(Reply {
            peer: HONEST,
            value: headers.iter().map(|h| world.bal(h.hash())).collect(),
        })
    }

    async fn bodies(&self, headers: Vec<BlockHeader>) -> Result<Reply<Vec<BlockBody>>, NetError> {
        let world = self.world.lock().await;
        let mut out = Vec::new();
        for header in headers {
            out.push(world.body(header.hash()).await);
        }
        Ok(Reply {
            peer: HONEST,
            value: out,
        })
    }

    async fn receipts(&self, header: BlockHeader) -> Result<Reply<Vec<Receipt>>, NetError> {
        let world = self.world.lock().await;
        Ok(Reply {
            peer: HONEST,
            value: world.receipts(header.hash()).await,
        })
    }

    async fn broadcast(&self, tx: Transaction) -> Result<usize, NetError> {
        self.broadcasts.lock().expect("broadcasts").push(tx);
        Ok(1)
    }

    fn penalize(&self, peer: PeerKey) {
        self.penalized.lock().expect("penalized").push(peer);
    }

    async fn announce(&self, _store: &PartialStore) {}

    async fn census(&self) -> Census {
        Census {
            state_peers: 1,
            eth_peers: 1,
        }
    }
}

/// Synthetic light client heads over the world's chain: beacon root `keccak(number)`,
/// parent the previous number's root, and the payload applied before each beacon block
/// being that block number's execution block (so beacon block `n` commits payload `n+1`,
/// matching Gloas's one-slot lag).
pub fn lc_head_for(world: &World, number: u64) -> crate::light_client::LcHead {
    let header = world
        .store
        .get_block_header(number)
        .expect("header read")
        .expect("canonical header exists");
    crate::light_client::LcHead {
        slot: number * 2,
        beacon_root: beacon_root(number),
        parent_root: beacon_root(number.saturating_sub(1)),
        execution_block_hash: header.hash(),
    }
}

/// The synthetic beacon root of block `number`.
pub fn beacon_root(number: u64) -> H256 {
    keccak(&number.to_be_bytes())
}
