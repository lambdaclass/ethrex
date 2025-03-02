use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ethrex_blockchain::{
    payload::{create_payload, BuildPayloadArgs},
    Blockchain,
};
use ethrex_common::{
    types::{AccessList, Block, BlockHeader, Genesis, Signable, Transaction, TxKind},
    Address, Bytes, H160, H256, U256,
};
use ethrex_storage::{EngineType, Store};
use rand::{rngs::StdRng, Rng, SeedableRng};
use secp256k1::SecretKey;
use std::str::FromStr;
use std::{fs::File, io::BufReader, time::Duration};

struct Signers {
    secret_key: SecretKey,
    address: Address,
}

// Helper function to create a list of signers for rich L1 Wallets.
fn test_signers() -> Vec<Signers> {
    let signers = vec![
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("bcdf20249abf0ed6d944c0288fad489e33f66b3960d9e6229c1cd214ed3bbe31")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0x8943545177806ED17B9F23F0a21ee5948eCaa776").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0xE25583099BA105D9ec0A67f5Ae86D90e50036425").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("53321db7c1e331d93a11a41d16f004d7ff63972ec8ec7c25db329728ceeb1710")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0x614561D2d143621E126e87831AEF287678B442b8").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("ab63b23eb7941c1251757e24b3d2350d2bc05c3c388d06f8fe6feafefb1e8c70")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0xf93Ee4Cf8c6c40b329b0c0626F28333c132CF241").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("5d2344259f42259f82d2c140aa66102ba89b57b4883ee441a8b312622bd42491")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0x802dCbE1B1A97554B4F50DB5119E37E8e7336417").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("27515f805127bebad2fb9b183508bdacb8c763da16f54e0678b16e8f28ef3fff")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0xAe95d8DA9244C37CaC0a3e16BA966a8e852Bb6D6").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("7ff1a4c1d57e5e784d327c4c7651e952350bc271f156afb3d00d20f5ef924856")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0x2c57d1CFC6d5f8E4182a56b4cf75421472eBAEa4").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("3a91003acaf4c21b3953d94fa4a6db694fa69e5242b2e37be05dd82761058899")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0x741bFE4802cE1C4b5b00F9Df2F5f179A1C89171A").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("bb1d0f125b4fb2bb173c318cdead45468474ca71474e2247776b2b4c0fa2d3f5")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0xc3913d4D8bAb4914328651C2EAE817C8b78E1f4c").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("850643a0224065ecce3882673c21f56bcf6eef86274cc21cadff15930b59fc8c")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0x65D08a056c17Ae13370565B04cF77D2AfA1cB9FA").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("94eb3102993b41ec55c241060f47daa0f6372e2e3ad7e91612ae36c364042e44")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0x3e95dFbBaF6B348396E6674C7871546dCC568e56").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("daf15504c22a352648a71ef2926334fe040ac1d5005019e09f6c979808024dc7")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0x5918b2e647464d4743601a865753e64C8059Dc4F").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("eaba42282ad33c8ef2524f07277c03a776d98ae19f581990ce75becb7cfa1c23")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0x589A698b7b7dA0Bec545177D3963A2741105C7C9").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("3fd98b5187bf6526734efaa644ffbb4e3670d66f5d0268ce0323ec09124bff61")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0x4d1CB4eB7969f8806E2CaAc0cbbB71f88C8ec413").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("5288e2f440c7f0cb61a9be8afdeb4295f786383f96f5e35eb0c94ef103996b64")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0xF5504cE2BcC52614F121aff9b93b2001d92715CA").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("f296c7802555da2a5a662be70e078cbd38b44f96f8615ae529da41122ce8db05")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0xF61E98E7D47aB884C244E39E031978E33162ff4b").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("bf3beef3bd999ba9f2451e06936f0423cd62b815c9233dd3bc90f7e02a1e8673")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0xf1424826861ffbbD25405F5145B5E50d0F1bFc90").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("6ecadc396415970e91293726c3f5775225440ea0844ae5616135fd10d66b5954")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0xfDCe42116f541fc8f7b0776e2B30832bD5621C85").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("a492823c3e193d6c595f37a18e3c06650cf4c74558cc818b16130b293716106f")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0xD9211042f35968820A3407ac3d80C725f8F75c14").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("c5114526e042343c6d1899cad05e1c00ba588314de9b96929914ee0df18d46b2")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0xD8F3183DEF51A987222D845be228e0Bbb932C222").unwrap(),
        },
        Signers {
            secret_key: SecretKey::from_slice(
                &hex::decode("04b9f63ecf84210c5366c66d68fa1f5da1fa4f634fad6dfc86178e4d79ff9e59")
                    .unwrap(),
            )
            .unwrap(),
            address: Address::from_str("0xafF0CA253b97e54440965855cec0A8a2E2399896").unwrap(),
        },
    ];
    signers
}

fn test_store() -> Store {
    // Get genesis
    let file = File::open("../../test_data/genesis-l1.json").expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let genesis: Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");

    // Build store with genesis
    let store =
        Store::new("store.db", EngineType::InMemory).expect("Failed to build DB for testing");


    store
        .add_initial_state(genesis)
        .expect("Failed to add genesis state");

    store
}

// Helper function to create a properly signed transaction
fn create_valid_transaction(
    store: &Store,
    header: &BlockHeader,
    seed: u64,
    signers: &Vec<Signers>,
) -> Transaction {
    let mut rng = StdRng::seed_from_u64(seed);
    // Get a random signer from the list
    let signer = &signers[rng.gen_range(0..signers.len())];
    let secret_key = signer.secret_key;

    // Create transaction data
    let nonce = store.get_nonce_by_account_address(header.number, signer.address).unwrap().unwrap();
    let gas_price = header.base_fee_per_gas.unwrap_or(0);
    let gas = header.gas_limit;
    let to = TxKind::Call(signer.address);
    let value = U256::zero();
    let data = Bytes::from(
        (0..rng.gen_range(0..128))
            .map(|_| rng.gen::<u8>())
            .collect::<Vec<_>>(),
    );

    let mut tx = Transaction::EIP2930Transaction(ethrex_common::types::EIP2930Transaction {
        chain_id: store.get_chain_config().unwrap().chain_id,
        access_list: AccessList::new(),
        signature_y_parity: false,
        signature_r: U256::zero(),
        signature_s: U256::zero(),
        to,
        value,
        data,
        gas_limit: gas,
        gas_price,
        nonce,
    });
    tx.sign_inplace(&secret_key);
    tx
}

// Helper function to create a new block with a given number of transactions 
fn new_block(
    store: &Store,
    parent: &BlockHeader,
    num_transactions: usize,
    seed: u64,
    signers: &Vec<Signers>,
) -> Block {
    let args = BuildPayloadArgs {
        parent: parent.compute_block_hash(),
        timestamp: parent.timestamp + 12,
        fee_recipient: H160::random(),
        random: H256::random(),
        withdrawals: Some(Vec::new()),
        beacon_root: Some(H256::random()),
        version: 1,
    };

    // Create blockchain
    let blockchain = Blockchain::default_with_store(store.clone());

    let mut block = create_payload(&args, store).unwrap();
    let header_no = store.get_latest_block_number().unwrap();
    let header = store.get_block_header(header_no).unwrap().unwrap();

    assert!(
        parent.number == header.number,
        "Parent number {} is not equal to header number {} for seed {}",
        parent.number,
        header.number,
        seed
    );
    (0..num_transactions)
        .map(|i| create_valid_transaction(store, &header, seed + i as u64, signers))
        .for_each(|transaction| {
            blockchain.add_transaction_to_pool(transaction).unwrap();
        });
    blockchain.build_payload(&mut block).unwrap();
    block
}

fn bench_add_blocks(c: &mut Criterion) {
    let mut group = c.benchmark_group("blockchain_add_blocks");
    group.measurement_time(Duration::from_secs(15));
    group.sample_size(10);

    // Use the same base seed for consistent results across runs

    let num_blocks = 1000; // We'll benchmark adding 1000 blocks
    let txs_per_block = 5; // Number of transactions per block

    let group_id = BenchmarkId::new("add_1000_blocks", num_blocks);
    group.bench_function(group_id, |b| {
        b.iter_with_setup(
            || {
                // Setup: Create store, blockchain, and generate all blocks before benchmarking
                let store = test_store();
                let blockchain = Blockchain::new(ethrex_vm::backends::EVM::LEVM, store.clone());
                let signers = test_signers();

                // Get genesis block header
                let genesis_header = store.get_latest_block_number().unwrap();
                let parent_header = store.get_block_header(genesis_header).unwrap().unwrap();

                (store, blockchain, signers, parent_header)
            },
            |(store,blockchain,signers,mut parent_header)| {
                // Only benchmark the actual block addition process
                    // Only measuring the addition of blocks
                    for i in 0..num_blocks {
                        let block = new_block(&store, &parent_header, txs_per_block, i as u64, &signers);
                        parent_header = block.header.clone();
                        black_box(blockchain.add_block(&block)).expect("Failed to add block");
                        black_box(store.update_latest_block_number(block.header.number)).expect("Failed to update latest block number");
                        black_box(store.set_canonical_block(block.header.number, block.hash())).expect("Failed to set canonical block");
                    }
            },
        );
    });


    group.finish();
}

criterion_group!(benches, bench_add_blocks);
criterion_main!(benches);
