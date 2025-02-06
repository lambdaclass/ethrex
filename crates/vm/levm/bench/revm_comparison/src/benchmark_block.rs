use clap::Parser;
use ethrex_core::types::{
    code_hash, Account, AccountInfo, Block, BlockNumber, Genesis, GenesisAccount,
};
use ethrex_core::{Address, H256, U256};
use ethrex_levm::db::Db;
use ethrex_rpc_client::constants::CANCUN_CONFIG;
use ethrex_rpc_client::db::RpcDB;
use ethrex_rpc_client::{get_block, get_latest_block_number};
use ethrex_storage::{AccountUpdate, EngineType, Store};
use ethrex_vm::db::StoreWrapper;
use ethrex_vm::execution_db::{ExecutionDB, ToExecDB};
use ethrex_vm::{evm_state, execute_block, spec_id, EvmState};
use revm::db::CacheDB;
use revm::primitives::hex;
use revm::{
    db::{states::bundle_state::BundleRetention, AccountState, AccountStatus},
    precompile::{PrecompileSpecId, Precompiles},
    primitives::{BlobExcessGasAndPrice, BlockEnv, TxEnv, B256},
    Database, DatabaseCommit, Evm,
};
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::{fs::File, io::Write};

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long)]
    rpc_url: String,
    #[arg(short, long)]
    block_number: Option<usize>,
}

#[tokio::main]
async fn main() {
    let Args {
        rpc_url,
        block_number,
    } = Args::parse();

    //let block_number = get_latest_block_number(&rpc_url).await.unwrap();
    let block_number = 21782918;

    println!("fetching block {block_number} and its parent header");
    let block = get_block(&rpc_url, block_number)
        .await
        .expect("failed to fetch block");

    let exec_db = if let Ok(file) = File::open("db.bin") {
        println!("db file found");
        bincode::deserialize_from(file).expect("failed to deserialize db from file")
    } else {
        println!("db file not found");

        println!("populating rpc db cache");
        let rpc_db = RpcDB::with_cache(&rpc_url, block_number - 1, &block)
            .await
            .expect("failed to create rpc db");

        println!("pre-executing to build execution db");
        let db = rpc_db
            .to_exec_db(&block)
            .expect("failed to build execution db");

        println!("writing db to file db.bin");
        let mut file = File::create("db.bin").expect("failed to create db file");
        file.write_all(
            bincode::serialize(&db)
                .expect("failed to serialize db")
                .as_slice(),
        )
        .expect("failed to write db to file");

        db
    };
    dbg!(&exec_db.accounts);

    let mut evm_state = EvmState::from(exec_db);
    let before = std::time::Instant::now();
    let res = execute_block(&block, &mut evm_state).unwrap();
    let after = std::time::Instant::now();
    dbg!(&res);
    println!("Execution time: {:?}", after - before);
}

fn create_genesis(db: &RpcDB) -> Genesis {
    let alloc: HashMap<Address, GenesisAccount> = db
        .cache
        .borrow()
        .iter()
        .filter_map(|(addr, opt_acc)| {
            opt_acc.as_ref().map(|acc| {
                println!("account: {}, balance: {}", addr, acc.account_state.balance);
                println!("code: {:?}", addr.to_fixed_bytes());
                let acc_c = acc.clone();
                (
                    addr.clone(),
                    GenesisAccount {
                        code: acc_c.code.unwrap_or_default(),
                        storage: acc_c.storage,
                        balance: U256::from(60915557166745715562596i128),
                        nonce: acc_c.account_state.nonce,
                    },
                )
            })
        })
        .collect();
    Genesis {
        config: CANCUN_CONFIG,
        alloc,
        coinbase: Default::default(),
        difficulty: Default::default(),
        extra_data: Default::default(),
        gas_limit: u64::MAX - 1,
        nonce: 0,
        mix_hash: Default::default(),
        timestamp: 0,
        base_fee_per_gas: None,
        blob_gas_used: None,
        excess_blob_gas: None,
    }
}

fn to_account_updates(db: &RpcDB) -> Vec<AccountUpdate> {
    let updates: Vec<AccountUpdate> = db
        .cache
        .borrow()
        .iter()
        .filter_map(|(addr, maybe_acc)| {
            let maybe_acc = maybe_acc.as_ref();
            if let Some(acc) = maybe_acc {
                let code_hash = if let Some(code) = &acc.code {
                    code_hash(code)
                } else {
                    H256::default()
                };
                Some(AccountUpdate {
                    address: *addr,
                    removed: false,
                    info: Some(AccountInfo {
                        code_hash,
                        balance: acc.account_state.balance,
                        nonce: acc.account_state.nonce,
                    }),
                    code: acc.code.clone(),
                    added_storage: Default::default(),
                })
            } else {
                None
            }
        })
        .collect();
    updates
}

/*fn repopulate_accounts(
    db: Arc<dyn LevmDatabase>,
    accounts: Vec<(Address, ethrex_levm::account::Account)>,
) {
    db.add_accounts(accounts)
}*/
