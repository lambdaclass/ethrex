use bytes::Bytes;
use criterion::{BatchSize, Bencher, BenchmarkId, Criterion, criterion_group, criterion_main};
use ethereum_types::{Address, Bloom, H256, U256};
use ethrex_common::{
    H32,
    types::{
        AccountInfo, AccountState, BlockHeader, EIP1559Transaction, ForkId, LegacyTransaction, Log,
        Receipt, TxKind, TxType, Withdrawal,
    },
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_trie::{
    Nibbles, Node, NodeHash,
    node::{BranchNode, ExtensionNode, LeafNode, NodeRef},
};
use once_cell::sync::OnceCell;
use rand::{
    Rng,
    distr::{Alphanumeric, Distribution, SampleString, StandardUniform},
};
use std::{hint::black_box, iter::repeat_with};

fn random_log(rng: &mut impl Rng) -> Log {
    Log {
        address: Address::from(rng.random::<[u8; 20]>()),
        topics: (0..3).map(|_| H256(rng.random())).collect(),
        data: Bytes::from((0..64).map(|_| rng.random::<u8>()).collect::<Vec<_>>()),
    }
}

fn random_block_header(rng: &mut impl Rng) -> BlockHeader {
    BlockHeader {
        hash: OnceCell::new(),
        parent_hash: H256(rng.random()),
        ommers_hash: H256(rng.random()),
        coinbase: Address::from(rng.random::<[u8; 20]>()),
        state_root: H256(rng.random()),
        transactions_root: H256(rng.random()),
        receipts_root: H256(rng.random()),
        logs_bloom: Bloom::default(),
        difficulty: U256::from(rng.random::<u64>()),
        number: rng.random(),
        gas_limit: rng.random(),
        gas_used: rng.random(),
        timestamp: rng.random(),
        extra_data: Bytes::from(vec![0u8; 32]),
        prev_randao: H256(rng.random()),
        nonce: rng.random(),
        base_fee_per_gas: Some(rng.random()),
        withdrawals_root: Some(H256(rng.random())),
        blob_gas_used: Some(rng.random()),
        excess_blob_gas: Some(rng.random()),
        parent_beacon_block_root: Some(H256(rng.random())),
        requests_hash: Some(H256(rng.random())),
        block_access_list_hash: None,
        slot_number: None,
    }
}

fn random_legacy_tx(rng: &mut impl Rng) -> LegacyTransaction {
    LegacyTransaction {
        nonce: rng.random(),
        gas_price: U256(rng.random()),
        gas: rng.random(),
        to: TxKind::Call(Address::from(rng.random::<[u8; 20]>())),
        value: U256(rng.random()),
        data: Bytes::from((0..32).map(|_| rng.random::<u8>()).collect::<Vec<_>>()),
        v: U256::from(rng.random::<u64>()),
        r: U256(rng.random()),
        s: U256(rng.random()),
        inner_hash: OnceCell::new(),
        sender_cache: OnceCell::new(),
    }
}

fn random_eip1559_tx(rng: &mut impl Rng) -> EIP1559Transaction {
    EIP1559Transaction {
        chain_id: 1,
        nonce: rng.random(),
        max_priority_fee_per_gas: rng.random(),
        max_fee_per_gas: rng.random(),
        gas_limit: rng.random(),
        to: TxKind::Call(Address::from(rng.random::<[u8; 20]>())),
        value: U256(rng.random()),
        data: Bytes::from((0..32).map(|_| rng.random::<u8>()).collect::<Vec<_>>()),
        access_list: vec![],
        signature_y_parity: rng.random(),
        signature_r: U256(rng.random()),
        signature_s: U256(rng.random()),
        inner_hash: OnceCell::new(),
        sender_cache: OnceCell::new(),
    }
}

fn bench_decode_scalars(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_scalars");

    fn impl_bench_for<T, const N: usize>(b: &mut Bencher)
    where
        StandardUniform: Distribution<T>,
        T: RLPDecode + RLPEncode,
    {
        b.iter_batched_ref(
            || {
                rand::rng()
                    .random_iter::<T>()
                    .take(N)
                    .map(|x| x.encode_to_vec())
                    .collect::<Vec<_>>()
            },
            |data| {
                for data in data.iter() {
                    black_box(T::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    }

    group.bench_function(BenchmarkId::new("u8", 1000), impl_bench_for::<u8, 1000>);
    group.bench_function(BenchmarkId::new("u16", 1000), impl_bench_for::<u16, 1000>);
    group.bench_function(BenchmarkId::new("u32", 1000), impl_bench_for::<u32, 1000>);
    group.bench_function(BenchmarkId::new("u64", 1000), impl_bench_for::<u64, 1000>);

    group.finish();
}

fn bench_decode_bytes(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_bytes");

    fn impl_bench<const N: usize, const L: usize>(b: &mut Bencher) {
        b.iter_batched_ref(
            || {
                let mut data = Vec::with_capacity(N);
                for _ in 0..N {
                    data.push(
                        rand::rng()
                            .random_iter::<u8>()
                            .take(L)
                            .collect::<Vec<_>>()
                            .encode_to_vec(),
                    );
                }
                data
            },
            |data| {
                for data in data.iter() {
                    black_box(Vec::<u8>::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    }

    group.bench_function(
        BenchmarkId::new("[u8]", "len=5/1000"),
        impl_bench::<1000, 5>,
    );
    group.bench_function(
        BenchmarkId::new("[u8]", "len=60/1000"),
        impl_bench::<1000, 60>,
    );
    group.bench_function(
        BenchmarkId::new("[u8]", "len=500/1000"),
        impl_bench::<1000, 500>,
    );

    group.finish();
}

fn bench_decode_strings(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_strings");

    fn impl_bench<const N: usize, const L: usize>(b: &mut Bencher) {
        b.iter_batched_ref(
            || {
                let mut data = Vec::with_capacity(N);
                for _ in 0..N {
                    data.push(
                        Alphanumeric
                            .sample_string(&mut rand::rng(), L)
                            .encode_to_vec(),
                    );
                }
                data
            },
            |data| {
                for data in data.iter() {
                    black_box(String::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    }

    group.bench_function(BenchmarkId::new("str", "len=5/1000"), impl_bench::<1000, 5>);
    group.bench_function(
        BenchmarkId::new("str", "len=60/1000"),
        impl_bench::<1000, 60>,
    );
    group.bench_function(
        BenchmarkId::new("str", "len=500/1000"),
        impl_bench::<1000, 500>,
    );

    group.finish();
}

fn bench_decode_lists(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_lists");

    fn impl_bench_for<T, const N: usize, const L: usize>(b: &mut Bencher)
    where
        StandardUniform: Distribution<T>,
        T: RLPDecode + RLPEncode,
    {
        b.iter_batched_ref(
            || {
                let mut data = Vec::with_capacity(N);
                for _ in 0..N {
                    data.push(
                        rand::rng()
                            .random_iter::<T>()
                            .take(L)
                            .collect::<Vec<_>>()
                            .encode_to_vec(),
                    );
                }
                data
            },
            |data| {
                for data in data.iter() {
                    black_box(Vec::<T>::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    }

    fn impl_bench_str<const N: usize, const L: usize>(b: &mut Bencher) {
        b.iter_batched_ref(
            || {
                let mut data = Vec::with_capacity(N);
                for _ in 0..N {
                    data.push(
                        repeat_with(|| Alphanumeric.sample_string(&mut rand::rng(), 10))
                            .take(L)
                            .collect::<Vec<_>>()
                            .encode_to_vec(),
                    );
                }
                data
            },
            |data| {
                for data in data.iter() {
                    black_box(Vec::<String>::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    }

    group.bench_function(
        BenchmarkId::new("u8", "len=10/1000"),
        impl_bench_for::<u8, 1000, 10>,
    );
    group.bench_function(
        BenchmarkId::new("u8", "len=100/1000"),
        impl_bench_for::<u8, 1000, 100>,
    );
    group.bench_function(
        BenchmarkId::new("u8", "len=1000/1000"),
        impl_bench_for::<u8, 1000, 1000>,
    );

    group.bench_function(
        BenchmarkId::new("u16", "len=10/1000"),
        impl_bench_for::<u16, 1000, 10>,
    );
    group.bench_function(
        BenchmarkId::new("u16", "len=100/1000"),
        impl_bench_for::<u16, 1000, 100>,
    );
    group.bench_function(
        BenchmarkId::new("u16", "len=1000/1000"),
        impl_bench_for::<u16, 1000, 1000>,
    );

    group.bench_function(
        BenchmarkId::new("u32", "len=10/1000"),
        impl_bench_for::<u32, 1000, 10>,
    );
    group.bench_function(
        BenchmarkId::new("u32", "len=100/1000"),
        impl_bench_for::<u32, 1000, 100>,
    );
    group.bench_function(
        BenchmarkId::new("u32", "len=1000/1000"),
        impl_bench_for::<u32, 1000, 1000>,
    );

    group.bench_function(
        BenchmarkId::new("u64", "len=10/1000"),
        impl_bench_for::<u64, 1000, 10>,
    );
    group.bench_function(
        BenchmarkId::new("u64", "len=100/1000"),
        impl_bench_for::<u64, 1000, 100>,
    );
    group.bench_function(
        BenchmarkId::new("u64", "len=1000/1000"),
        impl_bench_for::<u64, 1000, 1000>,
    );

    group.bench_function(
        BenchmarkId::new("str[10]", "len=10/1000"),
        impl_bench_str::<1000, 10>,
    );
    group.bench_function(
        BenchmarkId::new("str[10]", "len=100/1000"),
        impl_bench_str::<1000, 100>,
    );
    group.bench_function(
        BenchmarkId::new("str[10]", "len=1000/1000"),
        impl_bench_str::<1000, 1000>,
    );

    group.finish();
}

fn bench_decode_common_types(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_common_types");

    group.bench_function(BenchmarkId::new("ForkId", 1000), |b| {
        b.iter_batched_ref(
            || {
                let mut rng = rand::rng();
                (0..1000)
                    .map(|_| {
                        ForkId {
                            fork_hash: H32(rng.random()),
                            fork_next: rng.random(),
                        }
                        .encode_to_vec()
                    })
                    .collect::<Vec<_>>()
            },
            |data| {
                for data in data {
                    black_box(ForkId::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function(BenchmarkId::new("AccountInfo", 1000), |b| {
        b.iter_batched_ref(
            || {
                let mut rng = rand::rng();
                (0..1000)
                    .map(|_| {
                        AccountInfo {
                            code_hash: H256(rng.random()),
                            balance: U256(rng.random()),
                            nonce: rng.random(),
                        }
                        .encode_to_vec()
                    })
                    .collect::<Vec<_>>()
            },
            |data| {
                for data in data {
                    black_box(AccountInfo::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function(BenchmarkId::new("AccountState", 1000), |b| {
        b.iter_batched_ref(
            || {
                let mut rng = rand::rng();
                (0..1000)
                    .map(|_| {
                        AccountState {
                            nonce: rng.random(),
                            balance: U256(rng.random()),
                            storage_root: H256(rng.random()),
                            code_hash: H256(rng.random()),
                        }
                        .encode_to_vec()
                    })
                    .collect::<Vec<_>>()
            },
            |data| {
                for data in data {
                    black_box(AccountState::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function(BenchmarkId::new("Withdrawal", 1000), |b| {
        b.iter_batched_ref(
            || {
                let mut rng = rand::rng();
                (0..1000)
                    .map(|_| {
                        Withdrawal {
                            index: rng.random(),
                            validator_index: rng.random(),
                            address: Address::from(rng.random::<[u8; 20]>()),
                            amount: rng.random(),
                        }
                        .encode_to_vec()
                    })
                    .collect::<Vec<_>>()
            },
            |data| {
                for data in data {
                    black_box(Withdrawal::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function(BenchmarkId::new("Log", 1000), |b| {
        b.iter_batched_ref(
            || {
                let mut rng = rand::rng();
                (0..1000)
                    .map(|_| random_log(&mut rng).encode_to_vec())
                    .collect::<Vec<_>>()
            },
            |data| {
                for data in data {
                    black_box(Log::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function(BenchmarkId::new("Receipt", 1000), |b| {
        b.iter_batched_ref(
            || {
                let mut rng = rand::rng();
                (0..1000)
                    .map(|_| {
                        Receipt {
                            tx_type: TxType::Legacy,
                            succeeded: rng.random(),
                            cumulative_gas_used: rng.random(),
                            logs: (0..2).map(|_| random_log(&mut rng)).collect(),
                        }
                        .encode_to_vec()
                    })
                    .collect::<Vec<_>>()
            },
            |data| {
                for data in data {
                    black_box(Receipt::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function(BenchmarkId::new("BlockHeader", 100), |b| {
        b.iter_batched_ref(
            || {
                let mut rng = rand::rng();
                (0..100)
                    .map(|_| random_block_header(&mut rng).encode_to_vec())
                    .collect::<Vec<_>>()
            },
            |data| {
                for data in data {
                    black_box(BlockHeader::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn bench_decode_nibbles(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_nibbles");

    fn impl_bench<const N: usize, const L: usize, const IS_LEAF: bool>(b: &mut Bencher) {
        b.iter_batched_ref(
            || {
                let mut data = Vec::with_capacity(N);
                for _ in 0..N {
                    data.push(
                        Nibbles::from_raw(
                            &rand::distr::Uniform::<u8>::new(0, 16)
                                .unwrap()
                                .sample_iter(rand::rng())
                                .take(L)
                                .collect::<Vec<_>>(),
                            IS_LEAF,
                        )
                        .encode_to_vec(),
                    );
                }
                data
            },
            |data| {
                for data in data.iter() {
                    black_box(Nibbles::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    }

    group.bench_function(
        BenchmarkId::new("nibbles", "len=65/1000"),
        impl_bench::<1000, 65, false>,
    );
    group.bench_function(
        BenchmarkId::new("nibbles", "leaf/len=65/1000"),
        impl_bench::<1000, 65, true>,
    );
    group.bench_function(
        BenchmarkId::new("nibbles", "len=129/1000"),
        impl_bench::<1000, 129, false>,
    );
    group.bench_function(
        BenchmarkId::new("nibbles", "leaf/len=129/1000"),
        impl_bench::<1000, 129, true>,
    );
    group.bench_function(
        BenchmarkId::new("nibbles", "len=130/1000"),
        impl_bench::<1000, 130, false>,
    );
    group.bench_function(
        BenchmarkId::new("nibbles", "leaf/len=130/1000"),
        impl_bench::<1000, 130, true>,
    );
    group.bench_function(
        BenchmarkId::new("nibbles", "len=500/1000"),
        impl_bench::<1000, 500, false>,
    );
    group.bench_function(
        BenchmarkId::new("nibbles", "leaf/len=500/1000"),
        impl_bench::<1000, 500, true>,
    );

    group.finish();
}

fn bench_decode_trie(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_trie");

    group.bench_function(BenchmarkId::new("NodeHash", "hashed/1000"), |b| {
        b.iter_batched_ref(
            || {
                rand::rng()
                    .random_iter::<[u8; 32]>()
                    .take(1000)
                    .map(H256)
                    .map(NodeHash::Hashed)
                    .map(|x| x.encode_to_vec())
                    .collect::<Vec<_>>()
            },
            |data| {
                for data in data.iter() {
                    black_box(NodeHash::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });

    fn bench_inline_hash<const LEN: u8>(b: &mut Bencher) {
        b.iter_batched_ref(
            || {
                let mut rng = rand::rng();
                (0..1000)
                    .map(|_| {
                        let mut buf = [0u8; 31];
                        rng.fill(&mut buf[..LEN as usize]);
                        NodeHash::Inline((buf, LEN)).encode_to_vec()
                    })
                    .collect::<Vec<_>>()
            },
            |data| {
                for data in data.iter() {
                    black_box(NodeHash::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    }

    group.bench_function(
        BenchmarkId::new("NodeHash", "inline/len=8/1000"),
        bench_inline_hash::<8>,
    );
    group.bench_function(
        BenchmarkId::new("NodeHash", "inline/len=16/1000"),
        bench_inline_hash::<16>,
    );
    group.bench_function(
        BenchmarkId::new("NodeHash", "inline/len=24/1000"),
        bench_inline_hash::<24>,
    );

    group.bench_function(BenchmarkId::new("Node::Leaf", 1000), |b| {
        b.iter_batched_ref(
            || {
                let mut rng = rand::rng();
                (0..1000)
                    .map(|_| {
                        let nibbles = Nibbles::from_raw(
                            &rand::distr::Uniform::<u8>::new(0, 16)
                                .unwrap()
                                .sample_iter(&mut rng)
                                .take(10)
                                .collect::<Vec<_>>(),
                            true,
                        );
                        let value: Vec<u8> = (0..32).map(|_| rng.random::<u8>()).collect();
                        Node::Leaf(LeafNode::new(nibbles, value)).encode_to_vec()
                    })
                    .collect::<Vec<_>>()
            },
            |data| {
                for data in data.iter() {
                    black_box(Node::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function(BenchmarkId::new("Node::Extension", 1000), |b| {
        b.iter_batched_ref(
            || {
                let mut rng = rand::rng();
                (0..1000)
                    .map(|_| {
                        let nibbles = Nibbles::from_raw(
                            &rand::distr::Uniform::<u8>::new(0, 16)
                                .unwrap()
                                .sample_iter(&mut rng)
                                .take(6)
                                .collect::<Vec<_>>(),
                            false,
                        );
                        let child = NodeRef::Hash(NodeHash::Hashed(H256(rng.random())));
                        Node::Extension(ExtensionNode::new(nibbles, child)).encode_to_vec()
                    })
                    .collect::<Vec<_>>()
            },
            |data| {
                for data in data.iter() {
                    black_box(Node::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function(BenchmarkId::new("Node::Branch", 1000), |b| {
        b.iter_batched_ref(
            || {
                let mut rng = rand::rng();
                (0..1000)
                    .map(|_| {
                        let mut choices = BranchNode::EMPTY_CHOICES;
                        for i in [0, 3, 7, 15] {
                            choices[i] =
                                NodeRef::Hash(NodeHash::Hashed(H256(rng.random())));
                        }
                        Node::Branch(Box::new(BranchNode::new(choices))).encode_to_vec()
                    })
                    .collect::<Vec<_>>()
            },
            |data| {
                for data in data.iter() {
                    black_box(Node::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn bench_decode_transactions(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_transactions");

    group.bench_function(BenchmarkId::new("LegacyTransaction", 100), |b| {
        b.iter_batched_ref(
            || {
                let mut rng = rand::rng();
                (0..100)
                    .map(|_| random_legacy_tx(&mut rng).encode_to_vec())
                    .collect::<Vec<_>>()
            },
            |data| {
                for data in data {
                    black_box(LegacyTransaction::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function(BenchmarkId::new("EIP1559Transaction", 100), |b| {
        b.iter_batched_ref(
            || {
                let mut rng = rand::rng();
                (0..100)
                    .map(|_| random_eip1559_tx(&mut rng).encode_to_vec())
                    .collect::<Vec<_>>()
            },
            |data| {
                for data in data {
                    black_box(EIP1559Transaction::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_decode_bytes,
    bench_decode_common_types,
    bench_decode_lists,
    bench_decode_nibbles,
    bench_decode_scalars,
    bench_decode_strings,
    bench_decode_transactions,
    bench_decode_trie,
);
criterion_main!(benches);
