use bytes::Bytes;
use criterion::{Criterion, criterion_group, criterion_main};
use ethereum_types::U256;
use ethrex_common::{
    Address, Bloom, H32, H160, H256,
    constants::EMPTY_KECCACK_HASH,
    types::{
        AccountInfo, AccountState, BYTES_PER_BLOB, BlobsBundle, Block, BlockBody, BlockHeader,
        ForkId, Log, Receipt, ReceiptWithBloom, Transaction, TxType, Withdrawal,
        requests::EncodedRequests,
    },
};
use ethrex_rlp::encode::RLPEncode;
use std::{hint::black_box, str::FromStr};

fn make_string_list(count: usize) -> Vec<String> {
    let entry = "abcdefghij".to_string();
    vec![entry; count]
}

fn make_u256_with_len(len: usize) -> U256 {
    assert!((1..=32).contains(&len));
    let shift = len.saturating_mul(8).saturating_sub(1);
    U256::from(1u64) << shift
}

static HASH: H256 = H256::repeat_byte(0xab);

fn bench_encode_integer(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_integer");

    group.bench_function("u8", |b| {
        let mut buf = Vec::new();
        let value: u8 = 42;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u8::MAX", |b| {
        let mut buf = Vec::new();
        let value: u8 = u8::MAX;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u16", |b| {
        let mut buf = Vec::new();
        let value: u16 = 42;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u16::MAX", |b| {
        let mut buf = Vec::new();
        let value: u16 = u16::MAX;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u32", |b| {
        let mut buf = Vec::new();
        let value: u32 = 42;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u32 > 0x7f", |b| {
        let mut buf = Vec::new();
        let value: u32 = u32::MAX;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u64", |b| {
        let mut buf = Vec::new();
        let value: u64 = 42;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u64::MAX", |b| {
        let mut buf = Vec::new();
        let value: u64 = u64::MAX;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u128", |b| {
        let mut buf = Vec::new();
        let value: u128 = 42;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u128::MAX", |b| {
        let mut buf = Vec::new();
        let value: u128 = u128::MAX;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u256", |b| {
        let mut buf = Vec::new();
        let value: U256 = U256::from(42u64);
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.bench_function("u256::MAX", |b| {
        let mut buf = Vec::new();
        let value: U256 = U256::MAX;
        b.iter(|| {
            buf.clear();
            value.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_integer_lengths(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_integer_lengths");
    for &len in &[1usize, 2, 4, 8, 16, 32] {
        let label = format!("bytes_{len}");
        let value = make_u256_with_len(len);
        group.bench_function(label, move |b| {
            let mut buf = Vec::new();
            b.iter(|| {
                buf.clear();
                value.encode(&mut buf);
                black_box(&buf);
            });
        });
    }
    group.finish();
}

fn bench_encode_strings(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_strings");
    for &len in &[5usize, 60, 500] {
        let label = format!("len_{len}");
        let value = "a".repeat(len);
        group.bench_function(label, move |b| {
            let mut buf = Vec::new();
            b.iter(|| {
                buf.clear();
                value.encode(&mut buf);
                black_box(&buf);
            });
        });
    }
    group.finish();
}

fn bench_encode_int_lists(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_int_lists");
    for &count in &[10usize, 100, 1000] {
        let label = format!("len_{count}");
        let value: Vec<_> = (0..count as u64).collect();
        group.bench_function(label, move |b| {
            let mut buf = Vec::new();
            b.iter(|| {
                buf.clear();
                value.encode(&mut buf);
                black_box(&buf);
            });
        });
    }
    group.finish();
}

fn bench_encode_string_lists(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_string_lists");
    for &count in &[10usize, 100, 1000] {
        let label = format!("len_{count}");
        let value = make_string_list(count);
        group.bench_function(label, move |b| {
            let mut buf = Vec::new();
            b.iter(|| {
                buf.clear();
                value.encode(&mut buf);
                black_box(&buf);
            });
        });
    }
    group.finish();
}

fn bench_encode_account_info(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_account_info");

    let account_info = AccountInfo {
        code_hash: HASH,
        balance: U256::from(0xf34ab23u64),
        nonce: 1,
    };

    group.bench_function("account_info", move |b| {
        let mut buf = Vec::new();
        b.iter(|| {
            buf.clear();
            account_info.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_account_state(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_account_state");

    let account_state = AccountState {
        nonce: 1,
        balance: U256::from(0xf34ab23u64),
        storage_root: HASH,
        code_hash: HASH,
    };

    group.bench_function("account_state", move |b| {
        let mut buf = Vec::new();
        b.iter(|| {
            buf.clear();
            account_state.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_fork_id(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_fork_id");

    let fork_id = ForkId {
        fork_hash: H32::from_slice(&[0xde, 0xad, 0xbe, 0xef]),
        fork_next: 17_000_000,
    };

    group.bench_function("fork_id", move |b| {
        let mut buf = Vec::new();
        b.iter(|| {
            buf.clear();
            fork_id.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_log(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_log");

    let log_entry = Log {
        address: Address::from_str("0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba").unwrap(),
        topics: vec![HASH, H256::repeat_byte(0x11), H256::repeat_byte(0x22)],
        data: Bytes::from(vec![0x55; 128]),
    };

    group.bench_function("log", move |b| {
        let mut buf = Vec::new();
        b.iter(|| {
            buf.clear();
            log_entry.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_receipt(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_receipt");

    let receipt_logs = vec![
        Log {
            address: Address::from_str("0x1000000000000000000000000000000000000001").unwrap(),
            topics: vec![HASH, H256::repeat_byte(0x33)],
            data: Bytes::from(vec![0xaa; 96]),
        },
        Log {
            address: Address::from_str("0x2000000000000000000000000000000000000002").unwrap(),
            topics: vec![H256::repeat_byte(0xbb)],
            data: Bytes::from(vec![0xbb; 64]),
        },
    ];

    let receipt = Receipt {
        tx_type: TxType::EIP1559,
        succeeded: true,
        cumulative_gas_used: 120_000,
        logs: receipt_logs,
    };

    group.bench_function("receipt", move |b| {
        let mut buf = Vec::new();
        b.iter(|| {
            buf.clear();
            receipt.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_receipt_with_bloom(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_receipt_with_bloom");

    let logs_with_bloom = vec![
        Log {
            address: Address::from_str("0x3000000000000000000000000000000000000003").unwrap(),
            topics: vec![H256::repeat_byte(0x44), H256::repeat_byte(0x55)],
            data: Bytes::from(vec![0xcc; 80]),
        },
        Log {
            address: Address::from_str("0x4000000000000000000000000000000000000004").unwrap(),
            topics: vec![HASH],
            data: Bytes::from(vec![0xdd; 48]),
        },
    ];

    let receipt = ReceiptWithBloom::new(TxType::EIP4844, true, 240_000, logs_with_bloom);

    group.bench_function("receipt_with_bloom", move |b| {
        let mut buf = Vec::new();
        b.iter(|| {
            buf.clear();
            receipt.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_encoded_requests(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_encoded_requests");

    let mut request_bytes: Vec<u8> = (0..192).map(|i| i as u8).collect();
    request_bytes.insert(0, 0x00);
    let encoded_requests = EncodedRequests(Bytes::from(request_bytes));

    group.bench_function("encoded_requests", move |b| {
        let mut buf = Vec::new();
        b.iter(|| {
            buf.clear();
            encoded_requests.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_blobs_bundle(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_blobs_bundle");

    let blobs_bundle = BlobsBundle {
        blobs: vec![[6u8; BYTES_PER_BLOB]; 4],
        commitments: vec![[0x78u8; 48]],
        proofs: vec![[0x78u8; 48]],
        version: 1,
    };

    group.bench_function("blobs_bundle", move |b| {
        let mut buf = Vec::new();
        b.iter(|| {
            buf.clear();
            blobs_bundle.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_block_header(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_block_header");

    let block_header = BlockHeader {
        parent_hash: H256::from_str(
            "0x48e29e7357408113a4166e04e9f1aeff0680daa2b97ba93df6512a73ddf7a154",
        )
        .unwrap(),
        ommers_hash: H256::from_str(
            "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
        )
        .unwrap(),
        coinbase: Address::from_str("0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba").unwrap(),
        state_root: H256::from_str(
            "0x9de6f95cb4ff4ef22a73705d6ba38c4b927c7bca9887ef5d24a734bb863218d9",
        )
        .unwrap(),
        transactions_root: H256::from_str(
            "0x578602b2b7e3a3291c3eefca3a08bc13c0d194f9845a39b6f3bcf843d9fed79d",
        )
        .unwrap(),
        receipts_root: H256::from_str(
            "0x035d56bac3f47246c5eed0e6642ca40dc262f9144b582f058bc23ded72aa72fa",
        )
        .unwrap(),
        logs_bloom: Bloom::from([0; 256]),
        difficulty: U256::zero(),
        number: 1,
        gas_limit: 0x016345785d8a0000,
        gas_used: 0xa8de,
        timestamp: 0x03e8,
        extra_data: Bytes::new(),
        prev_randao: H256::zero(),
        nonce: 0x0000000000000000,
        base_fee_per_gas: Some(0x07),
        withdrawals_root: Some(
            H256::from_str("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421")
                .unwrap(),
        ),
        blob_gas_used: Some(0x00),
        excess_blob_gas: Some(0x00),
        parent_beacon_block_root: Some(H256::zero()),
        requests_hash: Some(*EMPTY_KECCACK_HASH),
        ..Default::default()
    };

    group.bench_function("block_header", move |b| {
        let mut buf = Vec::new();
        b.iter(|| {
            buf.clear();
            block_header.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_block(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_block");

    let block_header = BlockHeader {
        parent_hash: H256::from_str(
            "0x48e29e7357408113a4166e04e9f1aeff0680daa2b97ba93df6512a73ddf7a154",
        )
        .unwrap(),
        ommers_hash: H256::from_str(
            "0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347",
        )
        .unwrap(),
        coinbase: Address::from_str("0x2adc25665018aa1fe0e6bc666dac8fc2697ff9ba").unwrap(),
        state_root: H256::from_str(
            "0x9de6f95cb4ff4ef22a73705d6ba38c4b927c7bca9887ef5d24a734bb863218d9",
        )
        .unwrap(),
        transactions_root: H256::from_str(
            "0x578602b2b7e3a3291c3eefca3a08bc13c0d194f9845a39b6f3bcf843d9fed79d",
        )
        .unwrap(),
        receipts_root: H256::from_str(
            "0x035d56bac3f47246c5eed0e6642ca40dc262f9144b582f058bc23ded72aa72fa",
        )
        .unwrap(),
        logs_bloom: Bloom::from([0; 256]),
        difficulty: U256::zero(),
        number: 1,
        gas_limit: 0x016345785d8a0000,
        gas_used: 0xa8de,
        timestamp: 0x03e8,
        extra_data: Bytes::new(),
        prev_randao: H256::zero(),
        nonce: 0x0000000000000000,
        base_fee_per_gas: Some(0x07),
        withdrawals_root: Some(
            H256::from_str("0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421")
                .unwrap(),
        ),
        blob_gas_used: Some(0x00),
        excess_blob_gas: Some(0x00),
        parent_beacon_block_root: Some(H256::zero()),
        requests_hash: Some(*EMPTY_KECCACK_HASH),
        ..Default::default()
    };

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

    let block = Block {
        header: block_header.clone(),
        body: BlockBody {
            transactions: transactions,
            ommers: vec![block_header],
            withdrawals: Some(vec![
                Withdrawal {
                    index: 0x00,
                    validator_index: 0x00,
                    address: H160::repeat_byte(0xf9),
                    amount: 0x00_u64,
                };
                4
            ]),
        },
    };

    group.bench_function("block", move |b| {
        let mut buf = Vec::new();
        b.iter(|| {
            buf.clear();
            block.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

fn bench_encode_withdrawals(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_withdrawals");

    let withdrawal = Withdrawal {
        index: 0x00,
        validator_index: 0x00,
        address: H160::repeat_byte(0xf9),
        amount: 0x80_u64,
    };

    group.bench_function("withdrawals", move |b| {
        let mut buf = Vec::new();
        b.iter(|| {
            buf.clear();
            withdrawal.encode(&mut buf);
            black_box(&buf);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_encode_integer,
    bench_encode_integer_lengths,
    bench_encode_strings,
    bench_encode_int_lists,
    bench_encode_string_lists,
    bench_encode_account_info,
    bench_encode_account_state,
    bench_encode_fork_id,
    bench_encode_log,
    bench_encode_receipt,
    bench_encode_receipt_with_bloom,
    bench_encode_encoded_requests,
    bench_encode_blobs_bundle,
    bench_encode_block_header,
    bench_encode_block,
    bench_encode_withdrawals
);
criterion_main!(benches);
