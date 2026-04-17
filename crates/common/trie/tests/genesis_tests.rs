// Integration tests for genesis-related trie functionality.
// These tests were originally in ethrex-common but moved here because they require
// genesis_block from ethrex-trie. Keeping them in ethrex-common caused
// a circular dev-dependency issue.

use std::{fs::File, io::BufReader, str::FromStr};

use ethereum_types::Bloom;
use ethereum_types::H32;
use ethrex_common::{
    Address, H256,
    constants::DEFAULT_OMMERS_HASH,
    types::{BlockHeader, ChainConfig, ForkId, Genesis, INITIAL_BASE_FEE},
};
use ethrex_crypto::NativeCrypto;
use ethrex_trie::{
    compute_receipts_root, compute_transactions_root, compute_withdrawals_root, genesis_block,
};

// ---- Genesis block tests (moved from ethrex-common genesis.rs tests) ----

#[test]
fn test_genesis_block_fields() {
    let file =
        File::open("../../../fixtures/genesis/kurtosis.json").expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let genesis: Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
    let genesis_block = genesis_block(&genesis);
    let header = genesis_block.header;
    let body = genesis_block.body;
    assert_eq!(header.parent_hash, H256::from([0; 32]));
    assert_eq!(header.ommers_hash, *DEFAULT_OMMERS_HASH);
    assert_eq!(header.coinbase, Address::default());
    assert_eq!(
        header.state_root,
        H256::from_str("0x2dab6a1d6d638955507777aecea699e6728825524facbd446bd4e86d44fa5ecd")
            .unwrap()
    );
    assert_eq!(
        header.transactions_root,
        compute_transactions_root(&[], &NativeCrypto)
    );
    assert_eq!(
        header.receipts_root,
        compute_receipts_root(&[], &NativeCrypto)
    );
    assert_eq!(header.logs_bloom, Bloom::default());
    assert_eq!(header.difficulty, ethrex_common::U256::from(1));
    assert_eq!(header.gas_limit, 25_000_000);
    assert_eq!(header.gas_used, 0);
    assert_eq!(header.timestamp, 1_718_040_081);
    assert_eq!(header.extra_data, bytes::Bytes::default());
    assert_eq!(header.prev_randao, H256::from([0; 32]));
    assert_eq!(header.nonce, 4660);
    assert_eq!(
        header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE),
        INITIAL_BASE_FEE
    );
    assert_eq!(
        header.withdrawals_root,
        Some(compute_withdrawals_root(&[], &NativeCrypto))
    );
    assert_eq!(header.blob_gas_used, Some(0));
    assert_eq!(header.excess_blob_gas, Some(0));
    assert_eq!(header.parent_beacon_block_root, Some(H256::zero()));
    assert!(body.transactions.is_empty());
    assert!(body.ommers.is_empty());
    assert!(body.withdrawals.is_some_and(|w| w.is_empty()));
}

#[test]
fn read_and_compute_kurtosis_hash() {
    let file =
        File::open("../../../fixtures/genesis/kurtosis.json").expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let genesis: Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
    let genesis_block_hash = genesis_block(&genesis).hash();
    assert_eq!(
        genesis_block_hash,
        H256::from_str("0xcb5306dd861d0f2c1f9952fbfbc75a46d0b6ce4f37bea370c3471fe8410bf40b")
            .unwrap()
    )
}

#[test]
fn read_and_compute_hive_hash() {
    let file =
        File::open("../../../fixtures/genesis/hive.json").expect("Failed to open genesis file");
    let reader = BufReader::new(file);
    let genesis: Genesis =
        serde_json::from_reader(reader).expect("Failed to deserialize genesis file");
    let computed_block_hash = genesis_block(&genesis).hash();
    let genesis_block_hash =
        H256::from_str("0x30f516e34fc173bb5fc4daddcc7532c4aca10b702c7228f3c806b4df2646fb7e")
            .unwrap();
    assert_eq!(genesis_block_hash, computed_block_hash)
}

// ---- Compute roots tests (moved from ethrex-common block.rs and transaction.rs tests) ----

#[test]
fn test_compute_withdrawals_root() {
    use ethrex_common::types::Withdrawal;
    // Source: https://github.com/ethereum/tests/blob/9760400e667eba241265016b02644ef62ab55de2/BlockchainTests/EIPTests/bc4895-withdrawals/amountIs0.json
    let withdrawals = vec![Withdrawal {
        index: 0x00,
        validator_index: 0x00,
        address: ethereum_types::H160::from_slice(&hex_literal::hex!(
            "c94f5374fce5edbc8e2a8697c15331677e6ebf0b"
        )),
        amount: 0x00_u64,
    }];
    let expected_root = H256::from_slice(&hex_literal::hex!(
        "48a703da164234812273ea083e4ec3d09d028300cd325b46a6a75402e5a7ab95"
    ));
    let root = compute_withdrawals_root(&withdrawals, &NativeCrypto);
    assert_eq!(root, expected_root);
}

#[test]
fn test_compute_transactions_root_block() {
    use ethrex_common::types::Transaction;
    let encoded_transactions = [
        "0x01f8d68330182404842daf517a830186a08080b880c1597f3c842558e64df52c3e0f0973067577c030c0c6578dbb2eef63155a21106fd4426057527f296b2ecdfabc81e34ffc82e89dec20f6b7c41fa1969d3c3bc44262c86f08b5b76077527fb7ece918787c50c878052c30a8b1d4abc07331e6d14b8ded52bbc58a6e9992b76097527f0110937c38cc13b914f201fc09dc6f7a80c001a09930cb92b4a27dce971c697a8c47fa34c98d076abc7b36e1239d6abcfc7c8403a041b35118447fe77c38c0b3a92a2dd3ecba4a9e4b35cc6534cd787f56c0cf2e21",
        "0xf86e81fa843127403882f61894db8d964741c53e55df9c2d4e9414c6c96482874e870aa87bee538000808360306ca03aa421df67a101c45ff9cb06ce28f518a5d8d8dbb76a79361280071909650a27a05a447ff053c4ae601cfe81859b58d5603f2d0a73481c50f348089032feb0b073",
        "0x02f8ef83301824048413f157f8842daf517a830186a094000000000000000000000000000000000000000080b8807a0a600060a0553db8600060c855c77fb29ecd7661d8aefe101a0db652a728af0fded622ff55d019b545d03a7532932a60ad52604260cd5360bf60ce53609460cf53603e60d05360f560d153bc596000609e55600060c6556000601f556000609155535660556057536055605853606e60595360e7605a5360d0605b5360eb60c080a03acb03b1fc20507bc66210f7e18ff5af65038fb22c626ae488ad9513d9b6debca05d38459e9d2a221eb345b0c2761b719b313d062ff1ea3d10cf5b8762c44385a6",
        "0x01f8ea8330182402842daf517a830186a094000000000000000000000000000000000000000080b880bdb30d976000604e557145600060a155d67fe7e473caf6e33cba341136268fc1189ba07837ef8a266570289ff53afc43436260c7527f333dfe837f4838f6053e5e46e4151aeec28f356ec39a2db9769f36ec92e3e3f660e7527f0b261608674300d4621eff679096a6ed786591aca69f2b22a3ea6949621daade610107527f3cc080a01f3f906540fb56b0576c51b3ffa86df213fd1f407378c9441cfdd9d5f3c1df3da035691b16c053b68ec74683ae020293cbc6a47ac773dc8defb96cb680c576e5a3",
    ];
    let transactions: Vec<Transaction> = encoded_transactions
        .iter()
        .map(|hex| {
            Transaction::decode_canonical(&hex::decode(hex.trim_start_matches("0x")).unwrap())
                .unwrap()
        })
        .collect();
    let transactions_root = compute_transactions_root(&transactions, &NativeCrypto);
    let expected_root = H256::from_slice(
        &hex::decode("adf0387d2303fe80aeca23bf6828c979b44d8a8fe4a1ba1d3511bc1567ca80de").unwrap(),
    );
    assert_eq!(transactions_root, expected_root);
}

#[test]
fn test_compute_transactions_root_legacy() {
    use ethrex_common::types::{BlockBody, LegacyTransaction, Transaction, TxKind};
    let mut body = BlockBody::empty();
    let tx = LegacyTransaction {
        nonce: 0,
        gas_price: ethrex_common::U256::from(0x0a),
        gas: 0x05f5e100,
        to: TxKind::Call(hex_literal::hex!("1000000000000000000000000000000000000000").into()),
        value: 0.into(),
        data: Default::default(),
        v: ethrex_common::U256::from(0x1b),
        r: ethrex_common::U256::from_big_endian(&hex_literal::hex!(
            "7e09e26678ed4fac08a249ebe8ed680bf9051a5e14ad223e4b2b9d26e0208f37"
        )),
        s: ethrex_common::U256::from_big_endian(&hex_literal::hex!(
            "5f6e3f188e3e6eab7d7d3b6568f5eac7d687b08d307d3154ccd8c87b4630509b"
        )),
        ..Default::default()
    };
    body.transactions.push(Transaction::LegacyTransaction(tx));
    let expected_root =
        hex_literal::hex!("8151d548273f6683169524b66ca9fe338b9ce42bc3540046c828fd939ae23bcb");
    let result = compute_transactions_root(&body.transactions, &NativeCrypto);
    assert_eq!(result, expected_root.into());
}

#[test]
fn test_compute_receipts_root() {
    use ethrex_common::types::{Receipt, TxType};
    // example taken from
    // https://github.com/ethereum/go-ethereum/blob/f8aa62353666a6368fb3f1a378bd0a82d1542052/cmd/evm/testdata/1/exp.json#L18
    let tx_type = TxType::Legacy;
    let succeeded = true;
    let cumulative_gas_used = 0x5208;
    let logs = vec![];
    let receipt = Receipt::new(tx_type, succeeded, cumulative_gas_used, logs);

    let receipts = [receipt];
    let result = compute_receipts_root(&receipts, &NativeCrypto);
    let expected_root =
        hex_literal::hex!("056b23fbba480696b65fe5a59b8f2148a1299103c4f57df839233af2cf4ca2d2");
    assert_eq!(result, expected_root.into());
}

// ---- Fork ID tests (moved from ethrex-common fork_id.rs tests) ----

struct TestCase {
    head: u64,
    time: u64,
    fork_id: ForkId,
    is_valid: bool,
}

fn assert_test_cases(
    test_cases: Vec<TestCase>,
    chain_config: ChainConfig,
    genesis_header: BlockHeader,
) {
    for test_case in test_cases {
        let fork_id = ForkId::new(
            chain_config,
            genesis_header.clone(),
            test_case.time,
            test_case.head,
        );
        assert_eq!(
            fork_id.is_valid(
                test_case.fork_id,
                test_case.head,
                test_case.time,
                chain_config,
                genesis_header.clone()
            ),
            test_case.is_valid
        )
    }
}

#[test]
fn hoodi_test_cases() {
    let genesis_file = std::fs::File::open("../../../cmd/ethrex/networks/hoodi/genesis.json")
        .expect("Failed to open genesis file");
    let genesis_reader = BufReader::new(genesis_file);
    let genesis: Genesis =
        serde_json::from_reader(genesis_reader).expect("Failed to read genesis file");
    let genesis_header = genesis_block(&genesis).header;
    // See https://github.com/ethereum/go-ethereum/blob/444a6d007a08bddcec0b68b60ab507ea8bc1d078/core/forkid/forkid_test.go#L100
    let test_cases: Vec<TestCase> = vec![
        TestCase {
            head: 123,
            time: 0,
            fork_id: ForkId {
                fork_hash: H32::from_str("0xbef71d30").unwrap(),
                fork_next: 1742999832,
            },
            is_valid: true,
        },
        TestCase {
            head: 123,
            time: 1742999831,
            fork_id: ForkId {
                fork_hash: H32::from_str("0xbef71d30").unwrap(),
                fork_next: 1742999832,
            },
            is_valid: true,
        },
        TestCase {
            head: 123,
            time: 1742999832,
            fork_id: ForkId {
                fork_hash: H32::from_str("0x0929e24e").unwrap(),
                fork_next: 1761677592,
            },
            is_valid: true,
        },
        TestCase {
            head: 123,
            time: 1761677591,
            fork_id: ForkId {
                fork_hash: H32::from_str("0x0929e24e").unwrap(),
                fork_next: 1761677592,
            },
            is_valid: true,
        },
        TestCase {
            head: 123,
            time: 1761677592,
            fork_id: ForkId {
                fork_hash: H32::from_str("0xe7e0e7ff").unwrap(),
                fork_next: 1762365720,
            },
            is_valid: true,
        },
        TestCase {
            head: 123,
            time: 1762365719,
            fork_id: ForkId {
                fork_hash: H32::from_str("0xe7e0e7ff").unwrap(),
                fork_next: 1762365720,
            },
            is_valid: true,
        },
        TestCase {
            head: 123,
            time: 1762365720,
            fork_id: ForkId {
                fork_hash: H32::from_str("0x3893353e").unwrap(),
                fork_next: 1762955544,
            },
            is_valid: true,
        },
        TestCase {
            head: 123,
            time: 1762955543,
            fork_id: ForkId {
                fork_hash: H32::from_str("0x3893353e").unwrap(),
                fork_next: 1762955544,
            },
            is_valid: true,
        },
        TestCase {
            head: 123,
            time: 1762955544,
            fork_id: ForkId {
                fork_hash: H32::from_str("0x23aa1351").unwrap(),
                fork_next: 0,
            },
            is_valid: true,
        },
        TestCase {
            head: 123,
            time: 2740434112,
            fork_id: ForkId {
                fork_hash: H32::from_str("0x23aa1351").unwrap(),
                fork_next: 0,
            },
            is_valid: true,
        },
    ];
    assert_test_cases(test_cases, genesis.config, genesis_header);
}

fn get_sepolia_genesis() -> (Genesis, BlockHeader) {
    let genesis_file = std::fs::File::open("../../../cmd/ethrex/networks/sepolia/genesis.json")
        .expect("Failed to open genesis file");
    let genesis_reader = BufReader::new(genesis_file);
    let genesis: Genesis =
        serde_json::from_reader(genesis_reader).expect("Failed to read genesis file");
    let genesis_header = genesis_block(&genesis).header;
    (genesis, genesis_header)
}

#[test]
fn sepolia_test_cases() {
    let (genesis, genesis_hash) = get_sepolia_genesis();
    // See https://github.com/ethereum/go-ethereum/blob/444a6d007a08bddcec0b68b60ab507ea8bc1d078/core/forkid/forkid_test.go#L83
    let test_cases: Vec<TestCase> = vec![
        TestCase {
            head: 0,
            time: 0,
            fork_id: ForkId {
                fork_hash: H32::from_str("0xfe3366e7").unwrap(),
                fork_next: 1735371,
            },
            is_valid: true,
        },
        TestCase {
            head: 1735370,
            time: 0,
            fork_id: ForkId {
                fork_hash: H32::from_str("0xfe3366e7").unwrap(),
                fork_next: 1735371,
            },
            is_valid: true,
        },
        TestCase {
            head: 1735371,
            time: 0,
            fork_id: ForkId {
                fork_hash: H32::from_str("0xb96cbd13").unwrap(),
                fork_next: 1677557088,
            },
            is_valid: true,
        },
        TestCase {
            head: 1735372,
            time: 1677557087,
            fork_id: ForkId {
                fork_hash: H32::from_str("0xb96cbd13").unwrap(),
                fork_next: 1677557088,
            },
            is_valid: true,
        },
        TestCase {
            head: 1735372,
            time: 1677557088,
            fork_id: ForkId {
                fork_hash: H32::from_str("0xf7f9bc08").unwrap(),
                fork_next: 1706655072,
            },
            is_valid: true,
        },
        TestCase {
            head: 1735372,
            time: 1706655071,
            fork_id: ForkId {
                fork_hash: H32::from_str("0xf7f9bc08").unwrap(),
                fork_next: 1706655072,
            },
            is_valid: true,
        },
        TestCase {
            head: 1735372,
            time: 1706655072,
            fork_id: ForkId {
                fork_hash: H32::from_str("0x88cf81d9").unwrap(),
                fork_next: 1741159776,
            },
            is_valid: true,
        },
        TestCase {
            head: 1735372,
            time: 1741159775,
            fork_id: ForkId {
                fork_hash: H32::from_str("0x88cf81d9").unwrap(),
                fork_next: 1741159776,
            },
            is_valid: true,
        },
        TestCase {
            head: 1735372,
            time: 1741159776,
            fork_id: ForkId {
                fork_hash: H32::from_str("0xed88b5fd").unwrap(),
                fork_next: 1760427360,
            },
            is_valid: true,
        },
        TestCase {
            head: 1735372,
            time: 1760427359,
            fork_id: ForkId {
                fork_hash: H32::from_str("0xed88b5fd").unwrap(),
                fork_next: 1760427360,
            },
            is_valid: true,
        },
        TestCase {
            head: 1735372,
            time: 1760427360,
            fork_id: ForkId {
                fork_hash: H32::from_str("0xe2ae4999").unwrap(),
                fork_next: 1761017184,
            },
            is_valid: true,
        },
        TestCase {
            head: 1735372,
            time: 1761017183,
            fork_id: ForkId {
                fork_hash: H32::from_str("0xe2ae4999").unwrap(),
                fork_next: 1761017184,
            },
            is_valid: true,
        },
        TestCase {
            head: 1735372,
            time: 1761017184,
            fork_id: ForkId {
                fork_hash: H32::from_str("0x56078a1e").unwrap(),
                fork_next: 1761607008,
            },
            is_valid: true,
        },
        TestCase {
            head: 1735372,
            time: 1761607007,
            fork_id: ForkId {
                fork_hash: H32::from_str("0x56078a1e").unwrap(),
                fork_next: 1761607008,
            },
            is_valid: true,
        },
        TestCase {
            head: 1735372,
            time: 1761607008,
            fork_id: ForkId {
                fork_hash: H32::from_str("0x268956b6").unwrap(),
                fork_next: 0,
            },
            is_valid: true,
        },
        TestCase {
            head: 1735372,
            time: 2741159776,
            fork_id: ForkId {
                fork_hash: H32::from_str("0x268956b6").unwrap(),
                fork_next: 0,
            },
            is_valid: true,
        },
    ];
    assert_test_cases(test_cases, genesis.config, genesis_hash);
}

#[test]
fn local_needs_software_update() {
    let (genesis, genesis_hash) = get_sepolia_genesis();
    let test_cases: Vec<TestCase> = vec![TestCase {
        head: 1735372,
        time: 2706655072,
        fork_id: ForkId {
            fork_hash: H32::random(),
            fork_next: 0,
        },
        is_valid: false,
    }];
    assert_test_cases(test_cases, genesis.config, genesis_hash);
}

#[test]
fn remote_needs_software_update() {
    let (genesis, genesis_hash) = get_sepolia_genesis();
    let local_time = 1706655072;
    let test_cases: Vec<TestCase> = vec![TestCase {
        head: 5443392,
        time: local_time,
        fork_id: ForkId {
            fork_hash: H32::from_str("0xf7f9bc08").unwrap(),
            fork_next: 0,
        },
        is_valid: false,
    }];
    assert_test_cases(test_cases, genesis.config, genesis_hash);
}
