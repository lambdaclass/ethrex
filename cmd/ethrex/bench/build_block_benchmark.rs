use std::{
    collections::HashMap,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::Bytes;
use criterion::{
    Criterion, Throughput, criterion_group, criterion_main,
    measurement::{Measurement, ValueFormatter},
};
use ethrex_blockchain::{
    Blockchain, BlockchainType,
    payload::{BuildPayloadArgs, PayloadBuildResult, create_payload},
};
use ethrex_common::{
    Address, H160,
    types::{
        Block, DEFAULT_BUILDER_GAS_CEIL, EIP1559Transaction, Genesis, GenesisAccount, Transaction,
        TxKind,
    },
};
use ethrex_l2_rpc::signer::{LocalSigner, Signable, Signer};
use ethrex_storage::{EngineType, Store};
use secp256k1::SecretKey;

pub struct GasMeasurement;
impl Measurement for GasMeasurement {
    type Intermediate = (Instant, u64);
    type Value = (Duration, u64);

    fn start(&self) -> Self::Intermediate {
        (Instant::now(), 0)
    }

    fn end(&self, i: Self::Intermediate) -> Self::Value {
        let (start_time, gas_used) = i;
        (start_time.elapsed(), gas_used)
    }

    fn add(&self, v1: &Self::Value, v2: &Self::Value) -> Self::Value {
        (v1.0 + v2.0, v1.1 + v2.1)
    }

    fn zero(&self) -> Self::Value {
        (Duration::from_secs(0), 0)
    }

    fn to_f64(&self, val: &Self::Value) -> f64 {
        let duration = val.0.as_millis() as f64;
        let gas_used_ggigas = val.1 as f64 / 10_f64.powf(9_f64);

        if duration == 0f64 {
            return f64::INFINITY;
        }

        gas_used_ggigas / duration * 1000_f64
    }

    fn formatter(&self) -> &dyn ValueFormatter {
        &GasMeasurementFormatter
    }
}

struct GasMeasurementFormatter;
impl ValueFormatter for GasMeasurementFormatter {
    fn format_value(&self, value: f64) -> String {
        format!("{value:.2} Ggas/s")
    }

    fn format_throughput(&self, throughput: &Throughput, _value: f64) -> String {
        match throughput {
            Throughput::Elements(e) => format!("{} elements/s", *e),
            Throughput::Bytes(b) => format!("{:.2} GiB/s", *b as f64 / 1_073_741_824.0),
            _ => String::from("unknown"),
        }
    }

    fn scale_values(&self, _throughput: f64, _values: &mut [f64]) -> &'static str {
        ""
    }

    fn scale_for_machines(&self, _values: &mut [f64]) -> &'static str {
        ""
    }

    fn scale_throughputs(
        &self,
        _times: f64,
        throughput: &Throughput,
        throughputs: &mut [f64],
    ) -> &'static str {
        for t in throughputs.iter_mut() {
            match *throughput {
                Throughput::Elements(_) => *t /= _times,
                Throughput::Bytes(_) => *t /= _times,
                _ => {}
            }
        }
        ""
    }
}

fn read_private_keys() -> Vec<SecretKey> {
    let file = include_str!("../../../fixtures/keys/private_keys_l1.txt");
    file.lines()
        .map(|line| {
            let line = line.trim().strip_prefix("0x").unwrap();
            SecretKey::from_str(line).unwrap()
        })
        .collect()
}

fn recover_address_for_sk(sk: &SecretKey) -> Address {
    let signer = Signer::Local(LocalSigner::new(*sk));
    signer.address()
}

async fn setup_genesis(accounts: &Vec<Address>) -> (Store, Genesis) {
    let storage_path = tempfile::TempDir::new().unwrap();
    if std::fs::exists(&storage_path).unwrap_or(false) {
        std::fs::remove_dir_all(&storage_path).unwrap();
    }
    let genesis_file = include_bytes!("../../../fixtures/genesis/l1-dev.json");
    let mut genesis: Genesis = serde_json::from_slice(genesis_file).unwrap();
    let store = Store::new(storage_path, EngineType::Libmdbx).unwrap();
    for address in accounts {
        let account_info = GenesisAccount {
            code: Bytes::new(),
            storage: HashMap::new(),
            balance: u64::MAX.into(),
            nonce: 0,
        };
        genesis.alloc.insert(*address, account_info);
    }
    store.add_initial_state(genesis.clone()).await.unwrap();
    (store, genesis)
}

async fn create_payload_block(genesis_block: &Block, store: &Store) -> (Block, u64) {
    let payload_args = BuildPayloadArgs {
        parent: genesis_block.hash(),
        timestamp: genesis_block.header.timestamp + 1,
        fee_recipient: H160::random(),
        random: genesis_block.header.prev_randao,
        withdrawals: None,
        beacon_root: genesis_block.header.parent_beacon_block_root,
        version: 3,
        elasticity_multiplier: 1,
        gas_ceil: DEFAULT_BUILDER_GAS_CEIL,
    };
    let id = payload_args.id();
    let block = create_payload(&payload_args, store).unwrap();
    (block, id.unwrap())
}

async fn fill_mempool(b: &Blockchain, accounts: Vec<SecretKey>) {
    let mut txs = vec![];
    for sk in accounts {
        let signer = Signer::Local(LocalSigner::new(sk));
        for n in 0..1000 {
            let mut tx = Transaction::EIP1559Transaction(EIP1559Transaction {
                nonce: n,
                value: 1_u64.into(),
                gas_limit: 250000_u64,
                max_fee_per_gas: u64::MAX,
                max_priority_fee_per_gas: 10_u64,
                chain_id: 9,
                to: TxKind::Call(H160::random()),
                ..Default::default()
            });
            let _ = tx.sign_inplace(&signer).await;
            txs.push(tx);
        }
    }
    for tx in txs {
        b.add_transaction_to_pool(tx).await.unwrap();
    }
}

pub async fn bench_payload(input: &(Arc<Blockchain>, Block, &Store)) -> (Duration, u64) {
    let (blockchain, genesis_block, store) = input;
    // 1. engine_forkChoiceUpdated is called, which ends up calling fork_choice::build_payload,
    // which finally calls payload::create_payload(), this mimics this step without
    // the RPC handling. The payload is then sent to `Blockchain` to initiate the payload building
    // Blockchain::initiate_payload_build eventually calls 'fill_transactions'
    // which should take transactions from the previously filled mempool
    let (payload_block, payload_id) = create_payload_block(genesis_block, store).await;
    blockchain
        .clone()
        .initiate_payload_build(payload_block, payload_id)
        .await;
    // 2. engine_getPayload is called, this code path ends up calling Blockchain::get_payload(id),
    // so we also mimic that here without the RPC part.
    // This method will wait for the building process to finish and retrieve the finalized block
    let block = {
        let PayloadBuildResult { payload, .. } = blockchain.get_payload(payload_id).await.unwrap();
        payload
    };
    // 3. engine_newPayload is called, this eventually calls Blockchain::add_block
    // which takes transactions from the mempool and fills the block with them.
    let since = Instant::now();
    blockchain.add_block(&block).await.unwrap();
    let executed = Instant::now();
    // EXTRA: Sanity check to not benchmark n empty block.
    let hash = &block.hash();
    assert!(
        !store
            .get_block_body_by_hash(*hash)
            .await
            .unwrap()
            .unwrap()
            .transactions
            .is_empty()
    );
    let header = store.get_block_header_by_hash(*hash).unwrap().unwrap();
    let duration = executed.duration_since(since);
    (duration, header.gas_used)
}

pub fn build_block_benchmark(c: &mut Criterion<GasMeasurement>) {
    c.bench_function("block payload building bench", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter_custom(|_iters| async move {
                let mut total_duration = Duration::from_secs(0);
                let mut total_gas_used = 0;
                let (blockchain, genesis_block, store) = {
                    let accounts = read_private_keys();
                    let addresses = accounts
                        .clone()
                        .into_iter()
                        .map(|sk| recover_address_for_sk(&sk))
                        .collect();

                    let (store_with_genesis, genesis) = setup_genesis(&addresses).await;
                    let block_chain = Blockchain::new(
                        store_with_genesis.clone(),
                        BlockchainType::L1, // TODO: Should we support L2?
                        false,
                    );
                    fill_mempool(&block_chain, accounts).await;

                    (block_chain, genesis.get_block(), store_with_genesis)
                };
                let input = (Arc::new(blockchain), genesis_block, &store);
                let (duration, gas_used) = bench_payload(&input).await;
                total_duration += duration;
                total_gas_used += gas_used;
                (total_duration, total_gas_used)
            });
    });
}

fn gas_throughput_measurement() -> Criterion<GasMeasurement> {
    Criterion::default()
        .with_measurement(GasMeasurement)
        .sample_size(10)
}

criterion_group!(
    name = block_bench;
    config = gas_throughput_measurement();
    targets = build_block_benchmark
);
criterion_main!(block_bench);
