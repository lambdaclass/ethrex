use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, Write},
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use ethrex_blockchain::{
    payload::{create_payload, BuildPayloadArgs},
    Blockchain,
};
use ethrex_common::{
    types::{Block, EIP1559Transaction, Signable, Transaction},
    Address, H160, H256, U256,
};
use ethrex_rlp::encode::RLPEncode;
use ethrex_storage::{EngineType, Store};
use ethrex_vm::backends::{BlockExecutionResult, EvmEngine};

use keccak_hash::keccak;
use rand::Rng;
use secp256k1::{PublicKey, SecretKey};
use serde_json::{json, Value};
use tracing::info;

use crate::utils::read_genesis_file;

fn address_from_pub_key(public_key: PublicKey) -> H160 {
    let bytes = public_key.serialize_uncompressed();
    let hash = keccak(&bytes[1..]);
    let address_bytes: [u8; 20] = hash.as_ref().get(12..32).unwrap().try_into().unwrap();
    let address = Address::from(address_bytes);

    address
}

fn get_tx_cost(tx_type: ChainGeneratorTxs) -> u64 {
    match tx_type {
        ChainGeneratorTxs::RawTransfers => 21000,
    }
}

#[derive(Clone, Copy)]
pub enum ChainGeneratorTxs {
    RawTransfers,
}

pub const GIGAGAS: u64 = 1_000_000_000;

pub struct ChainGeneratorConfig {
    pub genesis_path: String,
    pub private_keys_path: String,
    pub num_of_blocks: u32,
    pub txs_to_generate: ChainGeneratorTxs,
    pub blocks_gas_target: u64,
}

pub fn gen_chain(config: ChainGeneratorConfig) {
    let store = Store::new("store", EngineType::InMemory).expect("Failed to create Store");
    let genesis = read_genesis_file(&config.genesis_path);
    store
        .add_initial_state(genesis)
        .expect("Failed to create genesis block");
    let chain_config = store.get_chain_config().unwrap();

    let evm_engine = EvmEngine::REVM;
    let blockchain = Blockchain::new(evm_engine, store.clone());
    let mut rng = rand::thread_rng();

    let private_keys_file = File::open(config.private_keys_path).expect("Open private keys file");
    let private_keys_file_reader = BufReader::new(private_keys_file);

    info!("Loading private keys...");
    // (private_key, nonce)
    let mut wallets: Vec<(SecretKey, PublicKey, u64)> = private_keys_file_reader
        .lines()
        .map(|r| {
            let key = H256::from_str(&r.unwrap()).unwrap();
            let sk = SecretKey::from_slice(key.as_bytes()).unwrap();
            let pk = sk.public_key(secp256k1::SECP256K1);

            (sk, pk, 0)
        })
        .collect();

    let mut blocks: Vec<Block> = vec![];

    let tx_cost = get_tx_cost(config.txs_to_generate);
    let txs_to_create = (config.blocks_gas_target / tx_cost) + 1;

    info!("Starting block production with {} txs each", txs_to_create);

    for i in 0..config.num_of_blocks {
        let tx_cost = get_tx_cost(config.txs_to_generate);
        let txs_to_create = (config.blocks_gas_target / tx_cost) + 1;

        for _ in 0..txs_to_create {
            let sender_idx = rng.gen_range(0..wallets.len());
            let receiver_idx = rng.gen_range(0..wallets.len());

            let sender = wallets[sender_idx].clone();
            let receiver = wallets[receiver_idx].clone();
            let value = U256::from(rng.gen_range(0..1000));

            let tx = match config.txs_to_generate {
                ChainGeneratorTxs::RawTransfers => {
                    Transaction::EIP1559Transaction(EIP1559Transaction {
                        to: ethrex_common::types::TxKind::Call(address_from_pub_key(receiver.1)),
                        nonce: sender.2,
                        chain_id: chain_config.chain_id,
                        max_fee_per_gas: 3121115334,
                        max_priority_fee_per_gas: 3000000000,
                        value,
                        gas_limit: tx_cost * 100,
                        ..Default::default()
                    })
                }
            };
            let tx = tx.sign(&sender.0);

            // increment nonce
            wallets[sender_idx].2 += 1;

            blockchain.add_transaction_to_pool(tx).unwrap();
        }

        let head_header = {
            let current_block_number = store.get_latest_block_number().unwrap();
            store
                .get_block_header(current_block_number)
                .unwrap()
                .unwrap()
        };
        let payload_args = BuildPayloadArgs {
            parent: head_header.compute_block_hash(),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 1 * i as u64,
            fee_recipient: H160::random(),
            random: H256::zero(),
            withdrawals: Default::default(),
            beacon_root: Some(H256::zero()),
            version: 3,
        };
        let mut block = create_payload(&payload_args, &store).unwrap();
        block.header.gas_limit = GIGAGAS;
        let payload_build_result = blockchain.build_payload(&mut block).unwrap();
        let execution_result = BlockExecutionResult {
            account_updates: payload_build_result.account_updates,
            receipts: payload_build_result.receipts,
            requests: Vec::new(),
        };
        store
            .apply_account_updates(block.header.parent_hash, &execution_result.account_updates)
            .unwrap();
        blockchain.store_block(&block, execution_result).unwrap();
        store
            .update_latest_block_number(block.header.number)
            .unwrap();
        store
            .set_canonical_block(block.header.number, block.hash())
            .unwrap();
        blocks.push(block);
        info!("Finished building block number {}", i);
    }

    // encode blocks and write rlp to output file
    let mut output_file = File::create("chain.rlp").unwrap();

    for block in blocks {
        let encoded = block.encode_to_vec();
        output_file.write(&encoded).unwrap();
    }
}

pub fn gen_big_genesis() {
    let addresses_file = File::open("addresses.txt").unwrap();
    let addresses_reader = BufReader::new(addresses_file);

    let mut alloc: HashMap<String, Value> = HashMap::with_capacity(1000000);

    for address in addresses_reader.lines() {
        let address = address.unwrap();
        let account = json!({
            "balance": "0xc097ce7bc90715b34b9f1000000000",
            "nonce": "0"
        });

        alloc.insert(address, account);
    }

    let genesis = json!({
        "config": {
            "chainId": 1729,
            "homesteadBlock": 0,
            "eip150Block": 0,
            "eip150Hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "eip155Block": 0,
            "eip158Block": 0,
            "daoForkBlock": 0,
            "frontierBlock": 0,
            "byzantiumBlock": 0,
            "constantinopleBlock": 0,
            "petersburgBlock": 0,
            "muirGlacierBlock": 0,
            "istanbulBlock": 0,
            "berlinBlock": 0,
            "londonBlock": 0,
            "terminalTotalDifficulty": 0,
            "terminalTotalDifficultyPassed": true,
            "mergeNetsplitBlock": 0,
            "shanghaiTime": 0,
            "cancunTime": 0,
            "clique": {
              "period": 0,
              "epoch": 30000
            },
            "depositContractAddress": "0x4242424242424242424242424242424242424242"
          },
          "nonce": "0x0",
          "timestamp": "0x5ca9158b",
          "gasLimit": "0x8f0d180",
          "difficulty": "0x0",
          "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
          "coinbase": "0x0000000000000000000000000000000000000000",
          "alloc": alloc,
          "number": "0x0",
          "gasUsed": "0x0",
          "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
          "baseFeePerGas": "0x1",
          "excessBlobGas": "0x0",
          "blobGasUsed": "0x0"
    });

    let output_file = File::create("genesis_out.json").unwrap();

    // Serialize to pretty JSON format and write to file
    serde_json::to_writer_pretty(output_file, &genesis).unwrap();
}

pub struct BackupStateConfig {
    store: Store,
    genesis_path: String,
    output_store: String,
    store_engine: EngineType,
    to_block: u64,
}

/// Reconstructs the state from the given blocks and stores it in output_store for the desired engine
pub fn gen_state_libmdbx_file(config: BackupStateConfig) {
    let mut blocks = vec![];
    let mut block_hashes = vec![];
    let mut block_headers = vec![];

    // get the blocks
    for i in 1..config.to_block {
        let header = config
            .store
            .get_block_header(i)
            .unwrap()
            .expect("block header exists");
        let body = config
            .store
            .get_block_body(i)
            .unwrap()
            .expect("block body exists");

        block_headers.push(header.clone());
        block_hashes.push(header.compute_block_hash());
        blocks.push(Block::new(header, body));
    }

    let store = Store::new(&config.output_store, config.store_engine).expect("init output db");
    let genesis = read_genesis_file(&config.genesis_path);
    store.add_initial_state(genesis).unwrap();
    store
        .add_block_headers(block_hashes, block_headers)
        .unwrap();
    let blockchain = Blockchain::new(EvmEngine::REVM, store.clone());

    // TODO when #2174 is merged, replace this with blockchain.add_blocks_in_batch(&blocks) and mark_chain_as_canonical
    for (number, block) in blocks.into_iter().enumerate() {
        store
            .set_canonical_block(number as u64, block.hash())
            .unwrap();
        blockchain.add_block(&block).expect("block is added");
    }
}

pub fn download_blocks_to_chain_rlp(from_block: u64, to_block: u64, store: Store) {
    let mut blocks = vec![];

    // get the blocks
    for i in from_block..to_block {
        let header = store
            .get_block_header(i)
            .unwrap()
            .expect("block header exists");
        let body = store.get_block_body(i).unwrap().expect("block body exists");

        blocks.push(Block::new(header, body));
    }

    let mut output_file = File::create("chain.rlp").unwrap();

    for block in blocks {
        let encoded = block.encode_to_vec();
        output_file.write(&encoded).unwrap();
    }
}
