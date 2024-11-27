use crate::config::EthrexL2Config;
use clap::Subcommand;
use colored::{self, Colorize};
use ethrex_l2::utils::eth_client::EthClient;
use keccak_hash::H256;
use std::str::FromStr;

#[derive(Subcommand)]
pub(crate) enum Command {
    #[clap(
        about = "Get latestCommittedBlock and latestVerifiedBlock from the OnChainProposer.",
        short_flag = 'l'
    )]
    LatestBlocks,
    #[clap(about = "Get the current block_number.", short_flag = 'b')]
    BlockNumber {
        #[arg(long = "l2", required = false)]
        l2: bool,
        #[arg(long = "l1", required = false)]
        l1: bool,
    },
    #[clap(about = "Get the transaction's info.", short_flag = 't')]
    Transaction {
        #[arg(long = "l2", required = false)]
        l2: bool,
        #[arg(long = "l1", required = false)]
        l1: bool,
        #[arg(short = 'h', required = true)]
        tx_hash: String,
    },
}

impl Command {
    pub async fn run(self, cfg: EthrexL2Config) -> eyre::Result<()> {
        let eth_client = EthClient::new(&cfg.network.l1_rpc_url);
        let rollup_client = EthClient::new(&cfg.network.l2_rpc_url);
        let on_chain_proposer_address = cfg.contracts.on_chain_proposer;
        match self {
            Command::LatestBlocks => {
                let last_committed_block =
                    EthClient::get_last_committed_block(&eth_client, on_chain_proposer_address)
                        .await?;

                let last_verified_block =
                    EthClient::get_last_verified_block(&eth_client, on_chain_proposer_address)
                        .await?;

                println!(
                    "latestCommittedBlock: {}",
                    format!("{last_committed_block}").bright_cyan()
                );

                println!(
                    "latestVerifiedBlock:  {}",
                    format!("{last_verified_block}").bright_cyan()
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
        }
        Ok(())
    }
}
