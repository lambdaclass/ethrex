use crate::{commands::wallet::balance_in_wei, config::EthrexL2Config};
use clap::Subcommand;
use colored::{self, Colorize};
use ethrex_common::Address;
use ethrex_rpc::clients::eth::{BlockByNumber, EthClient};
use keccak_hash::H256;
use std::str::FromStr;

#[derive(Subcommand)]
pub(crate) enum Command {
    #[command(
        about = "Get latestCommittedBatch and latestVerifiedBatch from the OnChainProposer.",
        short_flag = 'l'
    )]
    LatestBatches,
    #[command(about = "Get the current block_number.", alias = "bl")]
    BlockNumber {
        #[arg(long = "l2", required = false)]
        l2: bool,
        #[arg(long = "l1", required = false)]
        l1: bool,
    },
    #[command(about = "Get the transaction's info.", short_flag = 't')]
    Transaction {
        #[arg(long = "l2", required = false)]
        l2: bool,
        #[arg(long = "l1", required = false)]
        l1: bool,
        #[arg(short = 'h', required = true)]
        tx_hash: String,
    },
    #[command(about = "Get the account's balance info.", short_flag = 'b')]
    Balance {
        #[arg(long = "l2", required = false)]
        l2: bool,
        #[arg(long = "l1", required = false)]
        l1: bool,
        #[arg(short = 'a', required = true)]
        account: Address,
        #[arg(long = "wei", required = false, default_value_t = false)]
        wei: bool,
    },
}

impl Command {
    pub async fn run(self, cfg: EthrexL2Config) -> eyre::Result<()> {
        let eth_client = EthClient::new(&cfg.network.l1_rpc_url)?;
        let rollup_client = EthClient::new(&cfg.network.l2_rpc_url)?;
        let on_chain_proposer_address = cfg.contracts.on_chain_proposer;
        match self {
            Command::LatestBatches => {
                let last_committed_batch = eth_client
                    .get_last_committed_batch(on_chain_proposer_address)
                    .await?;

                let last_verified_batch = eth_client
                    .get_last_verified_batch(on_chain_proposer_address)
                    .await?;

                println!(
                    "latestCommittedBatch: {}",
                    format!("{last_committed_batch}").bright_cyan()
                );

                println!(
                    "latestVerifiedBatch:  {}",
                    format!("{last_verified_batch}").bright_cyan()
                );
            }
            Command::BlockNumber { l2, l1 } => {
                if !l1 || l2 {
                    let block_number = rollup_client.get_block_number().await?;
                    println!(
                        "[L2] BlockNumber: {}",
                        format!("{block_number}").bright_cyan()
                    );
                }
                if l1 {
                    let block_number = eth_client.get_block_number().await?;
                    println!(
                        "[L1] BlockNumber: {}",
                        format!("{block_number}").bright_cyan()
                    );
                }
            }
            Command::Transaction { l2, l1, tx_hash } => {
                let hash = H256::from_str(&tx_hash)?;

                if !l1 || l2 {
                    let tx = rollup_client
                        .get_transaction_by_hash(hash)
                        .await?
                        .ok_or(eyre::Error::msg("Not found"))?;
                    println!("[L2]:\n{tx}");
                }
                if l1 {
                    let tx = eth_client
                        .get_transaction_by_hash(hash)
                        .await?
                        .ok_or(eyre::Error::msg("Not found"))?;
                    println!("[L1]:\n{tx}");
                }
            }

            Command::Balance {
                l2,
                l1,
                wei,
                account,
            } => {
                if !l1 || l2 {
                    let account_balance = rollup_client
                        .get_balance(account, BlockByNumber::Latest)
                        .await?;
                    println!("{}", balance_in_wei(wei, account_balance));
                }
                if l1 {
                    let account_balance = eth_client
                        .get_balance(account, BlockByNumber::Latest)
                        .await?;
                    println!("{}", balance_in_wei(wei, account_balance));
                }
            }
        }
        Ok(())
    }
}
