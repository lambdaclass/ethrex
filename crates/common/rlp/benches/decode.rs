use criterion::{BatchSize, Bencher, BenchmarkId, Criterion, criterion_group, criterion_main};
use ethereum_types::{H256, U256};
use ethrex_common::{H32, types::ForkId};
use ethrex_p2p::{
    rlpx::{p2p::Capability, snap::StorageSlot},
    types::Endpoint,
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use ethrex_trie::{Nibbles, NodeHash};
use rand::{
    Rng,
    distr::{Alphanumeric, Distribution, SampleString, StandardUniform},
};
use std::{
    hint::black_box,
    iter::repeat_with,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

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
    // group.bench_function("u128", impl_bench_for::<u128>);

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

    // group.bench_function("u128", impl_bench_for::<u128>);

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

    // TODO: AccountInfo
    // TODO: AccountState
    // TODO: BlobsBundle
    // TODO: Block
    // TODO: BlockHeader
    // TODO: Withdrawal

    group.bench_function(BenchmarkId::new("ForkId", 1000), |b| {
        b.iter_batched_ref(
            || {
                let mut data = Vec::with_capacity(1000);
                for _ in 0..1000 {
                    data.push(
                        ForkId {
                            fork_hash: H32(rand::rng().random()),
                            fork_next: rand::rng().random(),
                        }
                        .encode_to_vec(),
                    );
                }
                data
            },
            |data| {
                for data in data {
                    black_box(ForkId::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });

    // TODO: Receipt
    // TODO: ReceiptWithBloom
    // TODO: Log
    // TODO: EncodedRequests

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
    // TODO: Benchmark NodeHash::Inline (len in {8, 16, 24}).

    // TODO: Benchmark BranchNode (empty, full, random; hash, node).
    // TODO: Benchmark ExtensionNode (short path, long path; hash, node).
    // TODO: Benchmark LeafNode (short path, long path; short value, long value).

    group.finish();
}

fn bench_decode_transactions(c: &mut Criterion) {
    // TODO: P2PTransaction
    // TODO: WrappedEIP4844Transaction
    // TODO: LegacyTransaction
    // TODO: EIP2930Transaction
    // TODO: EIP1559Transaction
    // TODO: EIP4844Transaction
    // TODO: EIP7702Transaction
    // TODO: PrivilegedL2Transaction
    // TODO: FeeTokenTransaction
    // TODO: MempoolTransaction

    // todo!()
}

fn bench_decode_p2p(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_p2p");

    group.bench_function(BenchmarkId::new("Endpoint", "ipv4/1000"), |b| {
        b.iter_batched_ref(
            || {
                let mut data = Vec::with_capacity(1000);
                let mut rng = rand::rng();
                for _ in 0..1000 {
                    data.push(
                        Endpoint {
                            ip: IpAddr::V4(Ipv4Addr::from_bits(rng.random())),
                            udp_port: rng.random(),
                            tcp_port: rng.random(),
                        }
                        .encode_to_vec(),
                    );
                }
                data
            },
            |data| {
                for data in data.iter() {
                    black_box(Endpoint::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function(BenchmarkId::new("Endpoint", "ipv6/1000"), |b| {
        b.iter_batched_ref(
            || {
                let mut data = Vec::with_capacity(1000);
                let mut rng = rand::rng();
                for _ in 0..1000 {
                    data.push(
                        Endpoint {
                            ip: IpAddr::V6(Ipv6Addr::from_bits(rng.random())),
                            udp_port: rng.random(),
                            tcp_port: rng.random(),
                        }
                        .encode_to_vec(),
                    );
                }
                data
            },
            |data| {
                for data in data.iter() {
                    black_box(Endpoint::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });

    // TODO: NodeRecord
    // TODO: Node
    // TODO: PingMessage
    // TODO: FindNodeMessage
    // TODO: NeighborsMessage
    // TODO: ENRRequestMessage
    // TODO: ENRRequestMessage (?)

    {
        fn make_bench(f: impl Fn(u8) -> Capability) -> impl Fn(&mut Bencher) {
            move |b| {
                b.iter_batched_ref(
                    || {
                        let mut data = Vec::with_capacity(1000);
                        for _ in 0..1000 {
                            data.push(f(rand::rng().random()).encode_to_vec());
                        }
                        data
                    },
                    |data| {
                        for data in data {
                            black_box(Capability::decode(data).unwrap());
                        }
                    },
                    BatchSize::SmallInput,
                )
            }
        }

        group.bench_function(
            BenchmarkId::new("Capability", "eth/1000"),
            make_bench(Capability::eth),
        );
        group.bench_function(
            BenchmarkId::new("Capability", "snap/1000"),
            make_bench(Capability::snap),
        );
        group.bench_function(
            BenchmarkId::new("Capability", "based/1000"),
            make_bench(Capability::based),
        );
    }

    // TODO: AccountRangeUnit
    // TODO: AccountStateSlim

    group.bench_function(BenchmarkId::new("StorageSlot", 1000), |b| {
        b.iter_batched_ref(
            || {
                let mut data = Vec::new();
                for _ in 0..1000 {
                    data.push(
                        StorageSlot {
                            hash: H256(rand::rng().random()),
                            data: U256(rand::rng().random()),
                        }
                        .encode_to_vec(),
                    );
                }
                data
            },
            |data| {
                for data in data {
                    black_box(StorageSlot::decode(data).unwrap());
                }
            },
            BatchSize::SmallInput,
        );
    });

    // TODO: AuthMessage (unavailable?)
    // TODO: AckMessage (unavailable?)
    // TODO: HashOrNumber (unavailable?)

    group.finish();
}

criterion_group!(
    benches,
    bench_decode_bytes,
    bench_decode_common_types,
    bench_decode_lists,
    bench_decode_nibbles,
    bench_decode_p2p,
    bench_decode_scalars,
    bench_decode_strings,
    bench_decode_transactions,
    bench_decode_trie,
);
criterion_main!(benches);
