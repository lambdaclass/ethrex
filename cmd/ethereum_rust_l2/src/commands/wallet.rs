use crate::{commands::utils::encode_calldata, config::EthereumRustL2Config};
use bytes::Bytes;
use clap::Subcommand;
use ethereum_rust_core::types::{
    EIP1559Transaction, PrivilegedL2Transaction, PrivilegedTxType, Transaction, TxKind,
};
use ethereum_rust_l2::utils::{
    eth_client::{eth_sender::Overrides, EthClient},
    merkle_tree::merkle_proof,
};
use ethereum_types::{Address, H256, U256};
use eyre::OptionExt;
use hex::FromHexError;
use itertools::Itertools;

const CLAIM_WITHDRAWAL_SIGNATURE: &str =
    "claimWithdrawal(bytes32,uint256,uint256,uint256,bytes32[])";

#[derive(Subcommand)]
pub(crate) enum Command {
    #[clap(about = "Get the balance of the wallet.")]
    Balance {
        #[clap(long = "token")]
        token_address: Option<Address>,
        #[arg(long = "l2", required = false)]
        l2: bool,
        #[arg(long = "l1", required = false)]
        l1: bool,
    },
    #[clap(about = "Deposit funds into some wallet.")]
    Deposit {
        // TODO: Parse ether instead.
        #[clap(long = "amount", value_parser = U256::from_dec_str)]
        amount: U256,
        #[clap(
            long = "token",
            help = "Specify the token address, the base token is used as default."
        )]
        token_address: Option<Address>,
        #[clap(
            long = "to",
            help = "Specify the wallet in which you want to deposit your funds."
        )]
        to: Option<Address>,
        #[clap(long, short = 'e', required = false)]
        explorer_url: bool,
    },
    #[clap(about = "Finalize a pending withdrawal.")]
    ClaimWithdraw { l2_withdrawal_tx_hash: H256 },
    #[clap(about = "Transfer funds to another wallet.")]
    Transfer {
        // TODO: Parse ether instead.
        #[clap(long = "amount", value_parser = U256::from_dec_str)]
        amount: U256,
        #[clap(long = "token")]
        token_address: Option<Address>,
        #[clap(long = "to")]
        to: Address,
        #[clap(
            long = "l1",
            required = false,
            help = "If set it will do an L1 transfer, defaults to an L2 transfer"
        )]
        l1: bool,
        #[clap(long, short = 'e', required = false)]
        explorer_url: bool,
    },
    #[clap(about = "Withdraw funds from the wallet.")]
    Withdraw {
        // TODO: Parse ether instead.
        #[clap(long = "amount", value_parser = U256::from_dec_str)]
        amount: U256,
        #[clap(long = "to")]
        to: Option<Address>,
        #[clap(long = "nonce")]
        nonce: Option<u64>,
        #[clap(
            long = "token",
            help = "Specify the token address, the base token is used as default."
        )]
        token_address: Option<Address>,
        #[clap(long, short = 'e', required = false)]
        explorer_url: bool,
    },
    #[clap(about = "Get the withdrawal merkle proof of a transaction.")]
    WithdrawalProof {
        #[clap(long = "hash")]
        tx_hash: H256,
    },
    #[clap(about = "Get the wallet address.")]
    Address,
    #[clap(about = "Get the wallet private key.")]
    PrivateKey,
    #[clap(about = "Send a transaction")]
    Send {
        #[clap(long = "to")]
        to: Address,
        #[clap(
            long = "value",
            value_parser = U256::from_dec_str,
            default_value = "0",
            required = false,
            help = "Value to send in wei"
        )]
        value: U256,
        #[clap(long = "calldata", value_parser = decode_hex, required = false, default_value = "")]
        calldata: Bytes,
        #[clap(
            long = "l1",
            required = false,
            help = "If set it will do an L1 transfer, defaults to an L2 transfer"
        )]
        l1: bool,
        #[clap(long = "chain-id", required = false)]
        chain_id: Option<u64>,
        #[clap(long = "nonce", required = false)]
        nonce: Option<u64>,
        #[clap(long = "gas-limit", required = false)]
        gas_limit: Option<u64>,
        #[clap(long = "gas-price", required = false)]
        gas_price: Option<u64>,
        #[clap(long = "priority-gas-price", required = false)]
        priority_gas_price: Option<u64>,
    },
    #[clap(about = "Make a call to a contract")]
    Call {
        #[clap(long = "to")]
        to: Address,
        #[clap(long = "calldata", value_parser = decode_hex, required = false, default_value = "")]
        calldata: Bytes,
        #[clap(
            long = "l1",
            required = false,
            help = "If set it will do an L1 transfer, defaults to an L2 transfer"
        )]
        l1: bool,
        #[clap(
            long = "value",
            value_parser = U256::from_dec_str,
            default_value = "0",
            required = false,
            help = "Value to send in wei"
        )]
        value: U256,
        #[clap(long = "from", required = false)]
        from: Option<Address>,
        #[clap(long = "gas-limit", required = false)]
        gas_limit: Option<u64>,
        #[clap(long = "gas-price", required = false)]
        gas_price: Option<u64>,
    },
    #[clap(about = "Deploy a contract")]
    Deploy {
        #[clap(long = "bytecode", value_parser = decode_hex)]
        bytecode: Bytes,
        #[clap(
            long = "l1",
            required = false,
            help = "If set it will do an L1 transfer, defaults to an L2 transfer"
        )]
        l1: bool,
        #[clap(
            long = "value",
            value_parser = U256::from_dec_str,
            default_value = "0",
            required = false,
            help = "Value to send in wei"
        )]
        value: U256,
        #[clap(long = "chain-id", required = false)]
        chain_id: Option<u64>,
        #[clap(long = "nonce", required = false)]
        nonce: Option<u64>,
        #[clap(long = "gas-limit", required = false)]
        gas_limit: Option<u64>,
        #[clap(long = "gas-price", required = false)]
        gas_price: Option<u64>,
        #[clap(long = "priority-gas-price", required = false)]
        priority_gas_price: Option<u64>,
    },
}

fn decode_hex(s: &str) -> Result<Bytes, FromHexError> {
    match s.strip_prefix("0x") {
        Some(s) => hex::decode(s).map(Into::into),
        None => hex::decode(s).map(Into::into),
    }
}

async fn get_withdraw_merkle_proof(
    client: &EthClient,
    tx_hash: H256,
) -> Result<(u64, Vec<H256>), eyre::Error> {
    let tx_receipt = client
        .get_transaction_receipt(tx_hash)
        .await?
        .ok_or_eyre("Transaction receipt not found")?;

    let transactions = client
        .get_block_by_hash(tx_receipt.block_info.block_hash)
        .await?
        .transactions;

    let (index, tx_withdrawal_hash) = transactions
        .iter()
        .filter(|tx| match tx {
            Transaction::PrivilegedL2Transaction(tx) => tx.tx_type == PrivilegedTxType::Withdrawal,
            _ => false,
        })
        .find_position(|tx| tx.compute_hash() == tx_hash)
        .map(|(i, tx)| match tx {
            Transaction::PrivilegedL2Transaction(tx) => {
                (i as u64, tx.get_withdrawal_hash().unwrap())
            }
            _ => unreachable!(),
        })
        .ok_or_eyre("Transaction is not a Withdrawal")?;

    let path = merkle_proof(
        transactions
            .iter()
            .filter_map(|tx| match tx {
                Transaction::PrivilegedL2Transaction(tx) => tx.get_withdrawal_hash(),
                _ => None,
            })
            .collect(),
        tx_withdrawal_hash,
    )
    .ok_or_eyre("Transaction's WithdrawalData is not in block's WithdrawalDataMerkleRoot")?;

    Ok((index, path))
}

impl Command {
    pub async fn run(self, cfg: EthereumRustL2Config) -> eyre::Result<()> {
        let eth_client = EthClient::new(&cfg.network.l1_rpc_url);
        let rollup_client = EthClient::new(&cfg.network.l2_rpc_url);
        let from = cfg.wallet.address;
        match self {
            Command::Balance {
                token_address,
                l2,
                l1,
            } => {
                if token_address.is_some() {
                    todo!("Handle ERC20 balances")
                }
                if !l1 || l2 {
                    let account_balance = rollup_client.get_balance(from).await?;
                    println!("[L2] Account balance: {account_balance}");
                }
                if l1 {
                    let account_balance = eth_client.get_balance(from).await?;
                    println!("[L1] Account balance: {account_balance}");
                }
            }
            Command::Deposit {
                amount,
                token_address,
                to,
                explorer_url: _,
            } => {
                if to.is_some() {
                    // There are two ways of depositing funds into the L2:
                    // 1. Directly transferring funds to the bridge.
                    // 2. Depositing through a contract call to the deposit method of the bridge.
                    // The second method is not handled in the CLI yet.
                    todo!("Handle deposits through contract")
                }
                if token_address.is_some() {
                    todo!("Handle ERC20 deposits")
                }
                Box::pin(async {
                    Self::Transfer {
                        amount,
                        token_address: None,
                        to: cfg.contracts.common_bridge,
                        l1: true,
                        explorer_url: false,
                    }
                    .run(cfg)
                    .await
                })
                .await?;
            }
            Command::ClaimWithdraw {
                l2_withdrawal_tx_hash,
            } => {
                let (withdrawal_l2_block_number, claimed_amount) = match rollup_client
                    .get_transaction_by_hash(l2_withdrawal_tx_hash)
                    .await?
                {
                    Some(l2_withdrawal_tx) => {
                        (l2_withdrawal_tx.block_number, l2_withdrawal_tx.value)
                    }
                    None => {
                        println!("Withdrawal transaction not found in L2");
                        return Ok(());
                    }
                };

                let (index, proof) =
                    get_withdraw_merkle_proof(&rollup_client, l2_withdrawal_tx_hash).await?;

                let claim_withdrawal_data = encode_calldata(
                    CLAIM_WITHDRAWAL_SIGNATURE,
                    &format!(
                        "{l2_withdrawal_tx_hash:#x} {claimed_amount} {withdrawal_l2_block_number} {index} {}",
                        proof.iter().map(hex::encode).join(",")
                    ),
                    false
                )?;
                println!(
                    "ClaimWithdrawalData: {}",
                    hex::encode(claim_withdrawal_data.clone())
                );

                let tx_hash = eth_client
                    .send(
                        claim_withdrawal_data.into(),
                        from,
                        TxKind::Call(cfg.contracts.common_bridge),
                        cfg.wallet.private_key,
                        Overrides {
                            chain_id: Some(cfg.network.l1_chain_id),
                            ..Default::default()
                        },
                    )
                    .await?;

                println!("Withdrawal claim sent: {tx_hash:#x}");
            }
            Command::Transfer {
                amount,
                token_address,
                to,
                l1,
                explorer_url: _,
            } => {
                if token_address.is_some() {
                    todo!("Handle ERC20 transfers")
                }

                let client = if l1 { eth_client } else { rollup_client };

                let mut transfer_transaction = EIP1559Transaction {
                    to: TxKind::Call(to),
                    value: amount,
                    chain_id: if l1 {
                        cfg.network.l1_chain_id
                    } else {
                        cfg.network.l2_chain_id
                    },
                    nonce: client.get_nonce(from).await?,
                    max_fee_per_gas: client.get_gas_price().await?.as_u64() * 100,
                    gas_limit: 21000 * 100,
                    ..Default::default()
                };

                // transfer_transaction.gas_limit = client
                //     .estimate_gas(transfer_transaction.clone().into())
                //     .await?;

                let tx_hash = client
                    .send_eip1559_transaction(&mut transfer_transaction, cfg.wallet.private_key)
                    .await?;

                println!(
                    "[{}] Transfer sent: {tx_hash:#x}",
                    if l1 { "L1" } else { "L2" }
                );
            }
            Command::Withdraw {
                amount,
                to,
                nonce,
                token_address: _,
                explorer_url: _,
            } => {
                let withdraw_transaction = PrivilegedL2Transaction {
                    to: TxKind::Call(to.unwrap_or(cfg.wallet.address)),
                    value: amount,
                    chain_id: cfg.network.l2_chain_id,
                    nonce: nonce.unwrap_or(rollup_client.get_nonce(from).await?),
                    max_fee_per_gas: 800000000,
                    tx_type: PrivilegedTxType::Withdrawal,
                    gas_limit: 21000 * 2,
                    ..Default::default()
                };

                let tx_hash = rollup_client
                    .send_privileged_l2_transaction(withdraw_transaction, cfg.wallet.private_key)
                    .await?;

                println!("Withdrawal sent: {tx_hash:#x}");
            }
            Command::WithdrawalProof { tx_hash } => {
                let (_index, path) = get_withdraw_merkle_proof(&rollup_client, tx_hash).await?;
                println!("{path:?}");
            }
            Command::Address => {
                todo!()
            }
            Command::PrivateKey => {
                todo!()
            }
            Command::Send {
                to,
                value,
                calldata,
                l1,
                chain_id,
                nonce,
                gas_limit,
                gas_price,
                priority_gas_price,
            } => {
                let client = match l1 {
                    true => eth_client,
                    false => rollup_client,
                };

                let tx_hash = client
                    .send(
                        calldata,
                        from,
                        TxKind::Call(to),
                        cfg.wallet.private_key,
                        Overrides {
                            value: value.into(),
                            nonce,
                            chain_id,
                            gas_limit,
                            gas_price,
                            priority_gas_price,
                            ..Default::default()
                        },
                    )
                    .await?;

                println!(
                    "[{}] Transaction sent: {tx_hash:#x}",
                    if l1 { "L1" } else { "L2" }
                );
            }
            Command::Call {
                to,
                calldata,
                l1,
                value,
                from,
                gas_limit,
                gas_price,
            } => {
                let client = match l1 {
                    true => eth_client,
                    false => rollup_client,
                };

                let result = client
                    .call(
                        to,
                        calldata,
                        Overrides {
                            from,
                            value: value.into(),
                            gas_limit,
                            gas_price,
                            ..Default::default()
                        },
                    )
                    .await?;

                println!("{result}");
            }
            Command::Deploy {
                bytecode,
                l1,
                value,
                chain_id,
                nonce,
                gas_limit,
                gas_price,
                priority_gas_price,
            } => {
                let client = match l1 {
                    true => eth_client,
                    false => rollup_client,
                };

                let (deployment_tx_hash, deployed_contract_address) = client
                    .deploy(
                        from,
                        cfg.wallet.private_key,
                        bytecode,
                        Overrides {
                            value: value.into(),
                            nonce,
                            chain_id,
                            gas_limit,
                            gas_price,
                            priority_gas_price,
                            ..Default::default()
                        },
                    )
                    .await?;

                println!("Contract deployed in tx: {deployment_tx_hash:#x}");
                println!("Contract address: {deployed_contract_address:#x}");
            }
        };
        Ok(())
    }
}
