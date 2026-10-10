//! The block warmer's results handed to execution give the same block as execution alone.
//!
//! Every case builds a block from signed transactions on a genesis of its own, then imports
//! it twice from that genesis: once on the path that collects a witness, which runs no warmer
//! and executes every transaction itself, and once on the pipeline, where execution takes the
//! warmer's result for each transaction whose reads still hold. The payload builder fixes the
//! header's state root, so both imports succeeding means both reached that root; the receipts
//! are compared directly, and the pipeline import must have taken results over.

use crate::Blockchain;
use crate::payload::{BuildPayloadArgs, create_payload};
use bytes::Bytes;
use ethrex_common::types::{
    AuthorizationTuple, Block, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, EIP2930Transaction,
    EIP7702Transaction, ELASTICITY_MULTIPLIER, Genesis, GenesisAccount, LegacyTransaction, Receipt,
    Transaction, TxKind,
};
use ethrex_common::{Address, H160, H256, U256};
use ethrex_crypto::keccak::keccak_hash;
use ethrex_rlp::structs::Encoder;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::backends::WarmedResultCounts;
use secp256k1::{Message, PublicKey, SECP256K1, SecretKey};
use std::collections::BTreeMap;
use std::path::Path;

const CHAIN_ID: u64 = 3_503_995_874_084_926;
const GWEI: u64 = 1_000_000_000;
const FUNDS: u128 = 1_000_000_000_000_000_000_000;

// Contract code, assembled by hand.
const PUSH1: u8 = 0x60;
const PUSH20: u8 = 0x73;
const SLOAD: u8 = 0x54;
const SSTORE: u8 = 0x55;
const TLOAD: u8 = 0x5c;
const TSTORE: u8 = 0x5d;
const ADD: u8 = 0x01;
const CALLER: u8 = 0x33;
const BALANCE: u8 = 0x31;
const GAS: u8 = 0x5a;
const CALL: u8 = 0xf1;
const POP: u8 = 0x50;
const STOP: u8 = 0x00;
const REVERT: u8 = 0xfd;
const JUMPDEST: u8 = 0x5b;
const JUMPI: u8 = 0x57;
const DUP1: u8 = 0x80;
const PUSH2: u8 = 0x61;
const GT: u8 = 0x11;
const SELFDESTRUCT: u8 = 0xff;
const CALLVALUE: u8 = 0x34;
const COINBASE: u8 = 0x41;
const PUSH3: u8 = 0x62;
const SWAP1: u8 = 0x90;
const SUB: u8 = 0x03;

/// The fee recipient of every block the cases build.
const BUILDER: u8 = 0xfe;

/// Increments storage slot 0.
fn counter_code() -> Bytes {
    Bytes::from(vec![PUSH1, 0, SLOAD, PUSH1, 1, ADD, PUSH1, 0, SSTORE, STOP])
}

/// Sends 1 wei to the caller and ignores whether that succeeded.
fn payer_code() -> Bytes {
    Bytes::from(vec![
        PUSH1, 0, PUSH1, 0, PUSH1, 0, PUSH1, 0, PUSH1, 1, CALLER, GAS, CALL, POP, STOP,
    ])
}

/// Stores `watched`'s balance in slot 1.
fn balance_reader_code(watched: Address) -> Bytes {
    let mut code = vec![PUSH20];
    code.extend_from_slice(watched.as_bytes());
    code.extend_from_slice(&[BALANCE, PUSH1, 1, SSTORE, STOP]);
    Bytes::from(code)
}

/// Writes slot 2 and reverts, so the write must be undone.
fn reverting_writer_code() -> Bytes {
    Bytes::from(vec![PUSH1, 7, PUSH1, 2, SSTORE, PUSH1, 0, PUSH1, 0, REVERT])
}

/// Calls `inner` (whose write reverts) and then writes slot 3.
fn outer_code(inner: Address) -> Bytes {
    let mut code = vec![PUSH1, 0, PUSH1, 0, PUSH1, 0, PUSH1, 0, PUSH1, 0, PUSH20];
    code.extend_from_slice(inner.as_bytes());
    code.extend_from_slice(&[GAS, CALL, POP, PUSH1, 1, PUSH1, 3, SSTORE, STOP]);
    Bytes::from(code)
}

/// Writes transient slot 0, reads it back and stores it in slot 4.
fn transient_code() -> Bytes {
    Bytes::from(vec![
        PUSH1, 5, PUSH1, 0, TSTORE, PUSH1, 0, TLOAD, PUSH1, 4, SSTORE, STOP,
    ])
}

/// Init code that destroys the contract being created, paying the caller.
fn create_and_destroy_init() -> Bytes {
    Bytes::from(vec![CALLER, SELFDESTRUCT])
}

/// Writes slots 0..300 (each to its own index): a transaction heavy enough for the warmer to
/// finish the block's later ones before execution reaches them.
fn heavy_loop_code() -> Bytes {
    Bytes::from(vec![
        PUSH1, 0,        // i
        JUMPDEST, // loop (offset 2)
        DUP1, DUP1, SSTORE, // slot i = i
        PUSH1, 1, ADD, // i + 1
        DUP1, PUSH2, 0x01, 0x2c, GT, // 300 > i + 1
        PUSH1, 2, JUMPI, STOP,
    ])
}

/// Counts down from `iterations` and stops: compute only, no state, 26 gas per iteration.
fn slow_loop_code(iterations: u32) -> Bytes {
    let [_, high, middle, low] = iterations.to_be_bytes();
    Bytes::from(vec![
        PUSH3, high, middle, low,      // counter
        JUMPDEST, // loop (offset 4)
        PUSH1, 1, SWAP1, SUB, // counter - 1
        DUP1, PUSH1, 4, JUMPI, // again while it is not zero
        STOP,
    ])
}

/// Forwards the call's value to the block's coinbase.
fn builder_tip_code() -> Bytes {
    Bytes::from(vec![
        PUSH1, 0, PUSH1, 0, PUSH1, 0, PUSH1, 0, CALLVALUE, COINBASE, GAS, CALL, POP, STOP,
    ])
}

/// Writes slots 0..300 like `heavy_loop_code`, then forwards the call's value to the coinbase.
fn heavy_then_tip_code() -> Bytes {
    let mut code = heavy_loop_code().to_vec();
    code.pop(); // its STOP
    code.extend_from_slice(&builder_tip_code());
    Bytes::from(code)
}

fn contract(code: Bytes, balance: u128) -> GenesisAccount {
    GenesisAccount {
        code,
        storage: BTreeMap::new(),
        balance: U256::from(balance),
        nonce: 1,
    }
}

fn funded(balance: u128) -> GenesisAccount {
    GenesisAccount {
        code: Bytes::new(),
        storage: BTreeMap::new(),
        balance: U256::from(balance),
        nonce: 0,
    }
}

fn addr(byte: u8) -> Address {
    H160::repeat_byte(byte)
}

/// A funded account that signs transactions, keeping its nonce.
struct Signer {
    key: SecretKey,
    address: Address,
    nonce: u64,
}

impl Signer {
    fn new(seed: u8) -> Self {
        let mut bytes = [seed; 32];
        bytes[31] = 1;
        let key = SecretKey::from_slice(&bytes).expect("a valid key");
        let public = PublicKey::from_secret_key(SECP256K1, &key).serialize_uncompressed();
        let address = Address::from_slice(&keccak_hash(&public[1..])[12..]);
        Self {
            key,
            address,
            nonce: 0,
        }
    }

    fn signature(&self, payload: &[u8]) -> (bool, U256, U256) {
        let message = Message::from_digest(keccak_hash(payload));
        let (recovery, bytes) = SECP256K1
            .sign_ecdsa_recoverable(&message, &self.key)
            .serialize_compact();
        (
            i32::from(recovery) == 1,
            U256::from_big_endian(&bytes[..32]),
            U256::from_big_endian(&bytes[32..]),
        )
    }

    fn sign(&self, mut tx: Transaction) -> Transaction {
        let (payload, _) = tx
            .signing_payload()
            .expect("a signable transaction")
            .expect("a transaction with a signature");
        let (parity, r, s) = self.signature(&payload);
        match &mut tx {
            Transaction::LegacyTransaction(tx) => {
                tx.v = U256::from(35 + CHAIN_ID * 2 + u64::from(parity));
                tx.r = r;
                tx.s = s;
            }
            Transaction::EIP2930Transaction(tx) => {
                tx.signature_y_parity = parity;
                tx.signature_r = r;
                tx.signature_s = s;
            }
            Transaction::EIP1559Transaction(tx) => {
                tx.signature_y_parity = parity;
                tx.signature_r = r;
                tx.signature_s = s;
            }
            Transaction::EIP7702Transaction(tx) => {
                tx.signature_y_parity = parity;
                tx.signature_r = r;
                tx.signature_s = s;
            }
            _ => unreachable!("not built here"),
        }
        tx
    }

    /// An EIP-1559 transaction; a higher `tip` puts it earlier in the block.
    fn send(
        &mut self,
        to: TxKind,
        value: u128,
        data: Bytes,
        gas_limit: u64,
        tip: u64,
    ) -> Transaction {
        let tx = EIP1559Transaction {
            chain_id: CHAIN_ID,
            nonce: self.nonce,
            max_priority_fee_per_gas: tip * GWEI,
            max_fee_per_gas: 10 * GWEI,
            gas_limit,
            to,
            value: U256::from(value),
            data,
            ..Default::default()
        };
        self.nonce += 1;
        self.sign(Transaction::EIP1559Transaction(tx))
    }

    fn call(&mut self, to: Address, data: Bytes, tip: u64) -> Transaction {
        self.send(TxKind::Call(to), 0, data, 200_000, tip)
    }

    fn legacy(&mut self, to: Address, value: u128) -> Transaction {
        let tx = LegacyTransaction {
            nonce: self.nonce,
            gas_price: U256::from(10 * GWEI),
            gas: 100_000,
            to: TxKind::Call(to),
            value: U256::from(value),
            data: Bytes::new(),
            v: U256::from(35 + CHAIN_ID * 2),
            ..Default::default()
        };
        self.nonce += 1;
        self.sign(Transaction::LegacyTransaction(tx))
    }

    fn access_list(&mut self, to: Address, slots: Vec<H256>) -> Transaction {
        let tx = EIP2930Transaction {
            chain_id: CHAIN_ID,
            nonce: self.nonce,
            gas_price: U256::from(10 * GWEI),
            gas_limit: 200_000,
            to: TxKind::Call(to),
            value: U256::zero(),
            data: Bytes::new(),
            access_list: vec![(to, slots)],
            ..Default::default()
        };
        self.nonce += 1;
        self.sign(Transaction::EIP2930Transaction(tx))
    }

    /// Signs an EIP-7702 authorization delegating this account to `code_address`.
    fn authorization(&mut self, code_address: Address) -> AuthorizationTuple {
        let mut payload = vec![0x05];
        Encoder::new(&mut payload)
            .encode_field(&CHAIN_ID)
            .encode_field(&code_address)
            .encode_field(&self.nonce)
            .finish();
        let (parity, r, s) = self.signature(&payload);
        self.nonce += 1;
        AuthorizationTuple {
            chain_id: U256::from(CHAIN_ID),
            address: code_address,
            nonce: self.nonce - 1,
            y_parity: U256::from(u8::from(parity)),
            r_signature: r,
            s_signature: s,
        }
    }

    fn set_code(&mut self, to: Address, authorizations: Vec<AuthorizationTuple>) -> Transaction {
        let tx = EIP7702Transaction {
            chain_id: CHAIN_ID,
            nonce: self.nonce,
            max_priority_fee_per_gas: GWEI,
            max_fee_per_gas: 10 * GWEI,
            gas_limit: 300_000,
            to,
            value: U256::zero(),
            data: Bytes::new(),
            authorization_list: authorizations,
            ..Default::default()
        };
        self.nonce += 1;
        self.sign(Transaction::EIP7702Transaction(tx))
    }
}

fn genesis(accounts: impl IntoIterator<Item = (Address, GenesisAccount)>) -> Genesis {
    let path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/genesis/execution-api.json"
    ));
    let mut genesis = Genesis::try_from(path).expect("the execution-api genesis");
    genesis.alloc.extend(accounts);
    genesis
}

async fn fresh(genesis: &Genesis) -> (Store, Blockchain) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let mut store = Store::new(dir.path(), EngineType::InMemory).expect("an in-memory store");
    store
        .add_initial_state(genesis.clone())
        .await
        .expect("the genesis state");
    let blockchain = Blockchain::for_test_harness(store.clone());
    (store, blockchain)
}

/// Builds one block holding `txs` on a fresh copy of `genesis`.
async fn build_block(genesis: &Genesis, txs: &[Transaction]) -> Block {
    let (store, blockchain) = fresh(genesis).await;
    for tx in txs {
        blockchain
            .add_transaction_to_pool(tx.clone())
            .await
            .expect("the transaction is accepted by the pool");
    }
    let parent = store.get_block_header(0).unwrap().unwrap();
    let args = BuildPayloadArgs {
        parent: parent.hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient: addr(BUILDER),
        random: H256::zero(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::zero()),
        slot_number: None,
        version: 3,
        elasticity_multiplier: ELASTICITY_MULTIPLIER,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let template = create_payload(&args, &store, Bytes::new()).expect("a payload template");
    let built = blockchain.build_payload(template).expect("a built payload");
    assert_eq!(
        built.payload.body.transactions.len(),
        txs.len(),
        "every transaction must make it into the block"
    );
    built.payload
}

async fn receipts_of(store: &Store, block: &Block) -> Vec<Receipt> {
    // One read per index: once the in-memory store flushes its buffer, a read from index 0
    // returns only the receipts whose key starts with that index's full key, so a whole-block
    // read would depend on whether the flush happened yet.
    let mut receipts = Vec::with_capacity(block.body.transactions.len());
    for index in 0..block.body.transactions.len() as u64 {
        let mut receipt = store
            .get_receipts_for_block_from_index(&block.hash(), index, Some(1))
            .await
            .expect("the block's receipts");
        assert_eq!(receipt.len(), 1, "no receipt stored at index {index}");
        receipts.append(&mut receipt);
    }
    receipts
}

/// Imports `block` by execution alone and with the handoff; returns the handoff's counts.
async fn import_both_ways(genesis: &Genesis, block: &Block) -> WarmedResultCounts {
    let (store, blockchain) = fresh(genesis).await;
    blockchain
        .add_block_pipeline_with_witness(block.clone(), None)
        .expect("import by execution alone");
    let by_execution = receipts_of(&store, block).await;

    let (store, blockchain) = fresh(genesis).await;
    blockchain
        .add_block_pipeline(block.clone(), None)
        .expect("import with the warmer's results handed to execution");
    let with_handoff = receipts_of(&store, block).await;

    assert_eq!(by_execution.len(), block.body.transactions.len());
    assert_eq!(
        by_execution, with_handoff,
        "receipts differ between the two imports"
    );
    let counts = blockchain.last_warmed_results();
    assert_eq!(
        counts.reused + counts.rejected + counts.missing,
        block.body.transactions.len(),
        "every transaction is accounted for: {counts:?}"
    );
    counts
}

async fn check(genesis: &Genesis, txs: &[Transaction]) -> WarmedResultCounts {
    let block = build_block(genesis, txs).await;
    import_both_ways(genesis, &block).await
}

#[tokio::test]
async fn conflicting_transactions_and_sender_chains() {
    let counter = addr(0xc0);
    let mut a = Signer::new(1);
    let mut b = Signer::new(2);
    let mut c = Signer::new(3);
    let genesis = genesis([
        (counter, contract(counter_code(), 0)),
        (a.address, funded(FUNDS)),
        (b.address, funded(FUNDS)),
        (c.address, funded(FUNDS)),
    ]);
    // Three senders increment the same slot; the second and third results read a value the
    // first changed. One sender also sends a chain of three.
    let txs = vec![
        a.call(counter, Bytes::new(), 3),
        b.call(counter, Bytes::new(), 2),
        c.call(counter, Bytes::new(), 1),
        a.call(counter, Bytes::new(), 1),
        a.call(counter, Bytes::new(), 1),
    ];
    // The block is tiny, so how many results were ready in time is up to timing; the
    // equal receipts and roots are the contract. `independent_heavy_transactions_are_taken_over`
    // checks the handoff itself.
    check(&genesis, &txs).await;
}

/// Iterations of the block's first transaction in the takeover cases: about 8.6 Mgas of
/// computation, which keeps execution busy far longer than the warmer needs for the rest.
const SLOW_ITERATIONS: u32 = 330_000;

/// The slow first transaction of the takeover cases: computation only, so nothing after it
/// depends on it, and every later result the warmer finishes while execution runs it holds.
fn slow_first(signer: &mut Signer, slow: Address) -> Transaction {
    signer.send(TxKind::Call(slow), 0, Bytes::new(), 9_000_000, 2)
}

#[tokio::test]
async fn independent_heavy_transactions_are_taken_over() {
    let slow = addr(0xdf);
    let mut first = Signer::new(19);
    let mut signers: Vec<Signer> = (20..23).map(Signer::new).collect();
    let contracts: Vec<Address> = (0xe0..0xe3).map(addr).collect();
    let mut accounts: Vec<(Address, GenesisAccount)> = contracts
        .iter()
        .map(|address| (*address, contract(heavy_loop_code(), 0)))
        .collect();
    accounts.extend(signers.iter().map(|signer| (signer.address, funded(FUNDS))));
    accounts.push((slow, contract(slow_loop_code(SLOW_ITERATIONS), 0)));
    accounts.push((first.address, funded(FUNDS)));
    let genesis = genesis(accounts);
    // Execution starts on a long computation; meanwhile the warmer runs the three senders
    // behind it, each writing 300 slots of its own contract (~6.6 Mgas, no shared state), so
    // all three results are ready and still hold when execution reaches them.
    let mut txs = vec![slow_first(&mut first, slow)];
    txs.extend(signers.iter_mut().zip(&contracts).map(|(signer, address)| {
        signer.send(TxKind::Call(*address), 0, Bytes::new(), 7_000_000, 1)
    }));
    let counts = check(&genesis, &txs).await;
    assert_eq!(
        counts.reused,
        txs.len() - 1,
        "every result behind the slow transaction is handed over: {counts:?}"
    );
}

#[tokio::test]
async fn heavy_sender_warmed_apart() {
    let counter = addr(0xc1);
    let mut a = Signer::new(4);
    let genesis = genesis([
        (counter, contract(counter_code(), 0)),
        (a.address, funded(FUNDS)),
    ]);
    // Gas limits over an eighth of the block split the sender's group into units that run
    // against the parent state with the nonce check off.
    let txs: Vec<_> = (0..3)
        .map(|_| a.send(TxKind::Call(counter), 0, Bytes::new(), 6_000_000, 1))
        .collect();
    check(&genesis, &txs).await;
}

#[tokio::test]
async fn balance_reads_against_payments() {
    let reader = addr(0xc2);
    let watched = addr(0xd0);
    let mut a = Signer::new(5);
    let mut b = Signer::new(6);
    let genesis = genesis([
        (reader, contract(balance_reader_code(watched), 0)),
        (watched, funded(1)),
        (a.address, funded(FUNDS)),
        (b.address, funded(FUNDS)),
    ]);
    // The payment runs first; the reader's warmed result read the old balance exactly.
    let txs = vec![
        a.send(TxKind::Call(watched), 100, Bytes::new(), 21_000, 3),
        b.call(reader, Bytes::new(), 1),
    ];
    check(&genesis, &txs).await;
}

#[tokio::test]
async fn contract_that_cannot_pay_every_caller() {
    let payer = addr(0xc3);
    let mut a = Signer::new(7);
    let mut b = Signer::new(8);
    let genesis = genesis([
        (payer, contract(payer_code(), 1)),
        (a.address, funded(FUNDS)),
        (b.address, funded(FUNDS)),
    ]);
    // One wei pays the first caller; the second caller's transfer fails inside the call.
    let txs = vec![
        a.call(payer, Bytes::new(), 2),
        b.call(payer, Bytes::new(), 1),
    ];
    check(&genesis, &txs).await;
}

#[tokio::test]
async fn value_to_an_account_the_block_brought_to_life() {
    let fresh_account = addr(0xd1);
    let mut a = Signer::new(9);
    let mut b = Signer::new(10);
    let genesis = genesis([(a.address, funded(FUNDS)), (b.address, funded(FUNDS))]);
    let txs = vec![
        a.send(TxKind::Call(fresh_account), 1, Bytes::new(), 21_000, 2),
        b.send(TxKind::Call(fresh_account), 1, Bytes::new(), 21_000, 1),
    ];
    check(&genesis, &txs).await;
}

#[tokio::test]
async fn write_undone_by_a_reverted_inner_call() {
    let inner = addr(0xc4);
    let outer = addr(0xc5);
    let mut a = Signer::new(11);
    let genesis = genesis([
        (inner, contract(reverting_writer_code(), 0)),
        (outer, contract(outer_code(inner), 0)),
        (a.address, funded(FUNDS)),
    ]);
    let txs = vec![
        a.call(outer, Bytes::new(), 1),
        a.call(inner, Bytes::new(), 1),
    ];
    check(&genesis, &txs).await;
}

#[tokio::test]
async fn contract_created_and_destroyed_within_its_transaction() {
    let mut a = Signer::new(12);
    let counter = addr(0xc6);
    let genesis = genesis([
        (counter, contract(counter_code(), 0)),
        (a.address, funded(FUNDS)),
    ]);
    let txs = vec![
        a.send(TxKind::Create, 5, create_and_destroy_init(), 200_000, 2),
        a.call(counter, Bytes::new(), 1),
    ];
    check(&genesis, &txs).await;
}

#[tokio::test]
async fn every_transaction_type_and_a_precompile_transfer() {
    let counter = addr(0xc7);
    let transient = addr(0xc8);
    let identity = addr(0x04);
    let mut a = Signer::new(13);
    let mut b = Signer::new(14);
    let mut authority = Signer::new(15);
    let genesis = genesis([
        (counter, contract(counter_code(), 0)),
        (transient, contract(transient_code(), 0)),
        (a.address, funded(FUNDS)),
        (b.address, funded(FUNDS)),
        (authority.address, funded(FUNDS)),
    ]);
    let delegation = authority.authorization(counter);
    let txs = vec![
        a.legacy(b.address, 1),
        a.access_list(counter, vec![H256::zero()]),
        a.call(transient, Bytes::new(), 1),
        b.set_code(authority.address, vec![delegation]),
        b.send(TxKind::Call(identity), 1, Bytes::new(), 50_000, 1),
    ];
    check(&genesis, &txs).await;
}

#[tokio::test]
async fn payments_to_the_block_builder() {
    let tipper = addr(0xc8);
    let reader = addr(0xc9);
    let mut a = Signer::new(30);
    let mut b = Signer::new(31);
    let mut c = Signer::new(32);
    let mut d = Signer::new(33);
    let genesis = genesis([
        (tipper, contract(builder_tip_code(), 0)),
        (reader, contract(balance_reader_code(addr(BUILDER)), 0)),
        (a.address, funded(FUNDS)),
        (b.address, funded(FUNDS)),
        (c.address, funded(FUNDS)),
        (d.address, funded(FUNDS)),
    ]);
    // Two senders tip the builder through a contract and one pays it directly: each result
    // credits a coinbase whose balance every earlier transaction's fee has moved. The last
    // reads the coinbase's exact balance, which no warmed result may carry.
    let txs = vec![
        a.send(TxKind::Call(tipper), 1_000, Bytes::new(), 200_000, 4),
        b.send(TxKind::Call(addr(BUILDER)), 2_000, Bytes::new(), 21_000, 3),
        c.send(TxKind::Call(tipper), 3_000, Bytes::new(), 200_000, 2),
        d.call(reader, Bytes::new(), 1),
    ];
    check(&genesis, &txs).await;
}

#[tokio::test]
async fn heavy_transactions_tipping_the_builder_are_taken_over() {
    let slow = addr(0xaf);
    let mut first = Signer::new(39);
    let mut signers: Vec<Signer> = (40..43).map(Signer::new).collect();
    let contracts: Vec<Address> = (0xb0..0xb3).map(addr).collect();
    let mut accounts: Vec<(Address, GenesisAccount)> = contracts
        .iter()
        .map(|address| (*address, contract(heavy_then_tip_code(), 0)))
        .collect();
    accounts.extend(signers.iter().map(|signer| (signer.address, funded(FUNDS))));
    accounts.push((slow, contract(slow_loop_code(SLOW_ITERATIONS), 0)));
    accounts.push((first.address, funded(FUNDS)));
    // The builder already exists, as on mainnet: paying an account that does not exist yet
    // costs more gas once an earlier fee has created it, so such a result is never kept.
    accounts.push((addr(BUILDER), funded(1)));
    let genesis = genesis(accounts);
    // As `independent_heavy_transactions_are_taken_over`, but each transaction behind the
    // slow one ends by paying the builder: those results are taken over too.
    let mut txs = vec![slow_first(&mut first, slow)];
    txs.extend(signers.iter_mut().zip(&contracts).map(|(signer, address)| {
        signer.send(TxKind::Call(*address), 1_000, Bytes::new(), 7_000_000, 1)
    }));
    let counts = check(&genesis, &txs).await;
    assert_eq!(
        counts.reused,
        txs.len() - 1,
        "every result behind the slow transaction paid the builder and is handed over: {counts:?}"
    );
}
