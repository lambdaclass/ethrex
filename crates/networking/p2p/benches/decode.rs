#![allow(clippy::unwrap_used)]
use criterion::{BatchSize, Bencher, BenchmarkId, Criterion, criterion_group, criterion_main};
use ethereum_types::{H256, U256};
use ethrex_p2p::{
    rlpx::{p2p::Capability, snap::StorageSlot},
    types::Endpoint,
};
use ethrex_rlp::{decode::RLPDecode, encode::RLPEncode};
use rand::Rng;
use std::{
    hint::black_box,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

fn bench_decode_p2p(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_p2p");

    group.bench_function(BenchmarkId::new("Endpoint", "ipv4/1000"), |b| {
        b.iter_batched_ref(
            || {
                let mut data = Vec::with_capacity(1000);
                let mut rng = rand::thread_rng();
                for _ in 0..1000 {
                    data.push(
                        Endpoint {
                            ip: IpAddr::V4(Ipv4Addr::from_bits(rng.r#gen())),
                            udp_port: rng.r#gen(),
                            tcp_port: rng.r#gen(),
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
                let mut rng = rand::thread_rng();
                for _ in 0..1000 {
                    data.push(
                        Endpoint {
                            ip: IpAddr::V6(Ipv6Addr::from_bits(rng.r#gen())),
                            udp_port: rng.r#gen(),
                            tcp_port: rng.r#gen(),
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

    {
        fn make_bench(f: impl Fn(u8) -> Capability) -> impl Fn(&mut Bencher) {
            move |b| {
                b.iter_batched_ref(
                    || {
                        let mut data = Vec::with_capacity(1000);
                        for _ in 0..1000 {
                            data.push(f(rand::thread_rng().r#gen()).encode_to_vec());
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

    group.bench_function(BenchmarkId::new("StorageSlot", 1000), |b| {
        b.iter_batched_ref(
            || {
                let mut data = Vec::new();
                for _ in 0..1000 {
                    data.push(
                        StorageSlot {
                            hash: H256(rand::thread_rng().r#gen()),
                            data: U256(rand::thread_rng().r#gen()),
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

    group.finish();
}

criterion_group!(benches, bench_decode_p2p);
criterion_main!(benches);
