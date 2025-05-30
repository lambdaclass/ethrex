use crate::config::EthrexL2Config;
use bytes::Bytes;
use clap::Subcommand;
use ethereum_types::{Address, H256, U256};
use ethrex_l2_sdk::calldata::{encode_calldata, Value};
use ethrex_l2_sdk::{COMMON_BRIDGE_L2_ADDRESS, L2_WITHDRAW_SIGNATURE};
use ethrex_rpc::clients::eth::BlockByNumber;
use ethrex_rpc::clients::eth::{eth_sender::Overrides, EthClient};
use hex::FromHexError;

#[derive(Subcommand)]
pub(crate) enum Command {
    #[command(about = "Get the balance of the wallet.")]
    Balance {
        #[arg(long = "token")]
        token_address: Option<Address>,
        #[arg(long = "l2", required = false)]
        l2: bool,
        #[arg(long = "l1", required = false)]
        l1: bool,
        #[arg(long = "wei", required = false, default_value_t = false)]
        wei: bool,
    },
    #[command(about = "Deposit funds into some wallet.")]
    Deposit {
        // TODO: Parse ether instead.
        #[arg(long = "amount", value_parser = U256::from_dec_str)]
        amount: U256,
        #[arg(
            long = "token",
            help = "Specify the token address, the base token is used as default."
        )]
        token_address: Option<Address>,
        #[arg(
            long = "to",
            help = "Specify the wallet in which you want to deposit your funds."
        )]
        to: Option<Address>,
        #[arg(short = 'w', required = false)]
        wait_for_receipt: bool,
        #[arg(long, short = 'e', required = false)]
        explorer_url: bool,
    },
    #[command(about = "Finalize a pending withdrawal.")]
    ClaimWithdraw {
        l2_withdrawal_tx_hash: H256,
        #[arg(short = 'w', required = false)]
        wait_for_receipt: bool,
    },
    #[command(about = "Transfer funds to another wallet.")]
    Transfer {
        // TODO: Parse ether instead.
        #[arg(long = "amount", value_parser = U256::from_dec_str)]
        amount: U256,
        #[arg(long = "token")]
        token_address: Option<Address>,
        #[arg(long = "to")]
        to: Address,
        #[arg(long = "nonce")]
        nonce: Option<u64>,
        #[arg(short = 'w', required = false)]
        wait_for_receipt: bool,
        #[arg(
            long = "l1",
            required = false,
            help = "If set it will do an L1 transfer, defaults to an L2 transfer"
        )]
        l1: bool,
        #[arg(long, short = 'e', required = false)]
        explorer_url: bool,
    },
    #[command(about = "Withdraw funds from the wallet.")]
    Withdraw {
        // TODO: Parse ether instead.
        #[arg(long = "amount", value_parser = U256::from_dec_str)]
        amount: U256,
        #[arg(long = "to")]
        to: Option<Address>,
        #[arg(long = "nonce")]
        nonce: Option<u64>,
        #[arg(
            long = "token",
            help = "Specify the token address, the base token is used as default."
        )]
        token_address: Option<Address>,
        #[arg(short = 'w', required = false)]
        wait_for_receipt: bool,
        #[arg(long, short = 'e', required = false)]
        explorer_url: bool,
    },
    #[command(about = "Get the withdrawal merkle proof of a transaction.")]
    WithdrawalProof {
        #[arg(long = "hash")]
        tx_hash: H256,
    },
    #[command(about = "Get the wallet address.")]
    Address,
    #[command(about = "Get the wallet private key.")]
    PrivateKey,
    #[command(about = "Send a transaction")]
    Send {
        #[arg(long = "to")]
        to: Address,
        #[arg(
            long = "value",
            value_parser = U256::from_dec_str,
            default_value = "0",
            required = false,
            help = "Value to send in wei"
        )]
        value: U256,
        #[arg(long = "calldata", value_parser = decode_hex, required = false, default_value = "")]
        calldata: Bytes,
        #[arg(
            long = "l1",
            required = false,
            help = "If set it will do an L1 transfer, defaults to an L2 transfer"
        )]
        l1: bool,
        #[arg(long = "chain-id", required = false)]
        chain_id: Option<u64>,
        #[arg(long = "nonce", required = false)]
        nonce: Option<u64>,
        #[arg(long = "gas-limit", required = false)]
        gas_limit: Option<u64>,
        #[arg(long = "gas-price", required = false)]
        max_fee_per_gas: Option<u64>,
        #[arg(long = "priority-gas-price", required = false)]
        max_priority_fee_per_gas: Option<u64>,
        #[arg(short = 'w', required = false)]
        wait_for_receipt: bool,
    },
    #[command(about = "Make a call to a contract")]
    Call {
        #[arg(long = "to")]
        to: Address,
        #[arg(long = "calldata", value_parser = decode_hex, required = false, default_value = "")]
        calldata: Bytes,
        #[arg(
            long = "l1",
            required = false,
            help = "If set it will do an L1 transfer, defaults to an L2 transfer"
        )]
        l1: bool,
        #[arg(
            long = "value",
            value_parser = U256::from_dec_str,
            default_value = "0",
            required = false,
            help = "Value to send in wei"
        )]
        value: U256,
        #[arg(long = "from", required = false)]
        from: Option<Address>,
        #[arg(long = "gas-limit", required = false)]
        gas_limit: Option<u64>,
        #[arg(long = "gas-price", required = false)]
        max_fee_per_gas: Option<u64>,
    },
    #[command(about = "Deploy a contract")]
    Deploy {
        #[arg(long = "bytecode", value_parser = decode_hex)]
        bytecode: Bytes,
        #[arg(
            long = "l1",
            required = false,
            help = "If set it will do an L1 transfer, defaults to an L2 transfer"
        )]
        l1: bool,
        #[arg(
            long = "value",
            value_parser = U256::from_dec_str,
            default_value = "0",
            required = false,
            help = "Value to send in wei"
        )]
        value: U256,
        #[arg(long = "chain-id", required = false)]
        chain_id: Option<u64>,
        #[arg(long = "nonce", required = false)]
        nonce: Option<u64>,
        #[arg(long = "gas-limit", required = false)]
        gas_limit: Option<u64>,
        #[arg(long = "gas-price", required = false)]
        max_fee_per_gas: Option<u64>,
        #[arg(long = "priority-gas-price", required = false)]
        max_priority_fee_per_gas: Option<u64>,
        #[arg(short = 'w', required = false)]
        wait_for_receipt: bool,
    },
}

fn decode_hex(s: &str) -> Result<Bytes, FromHexError> {
    match s.strip_prefix("0x") {
        Some(s) => hex::decode(s).map(Into::into),
        None => hex::decode(s).map(Into::into),
    }
}

impl Command {
    pub async fn run(self, cfg: EthrexL2Config) -> eyre::Result<()> {
        let eth_client = EthClient::new(&cfg.network.l1_rpc_url)?;
        let rollup_client = EthClient::new(&cfg.network.l2_rpc_url)?;
        let from = cfg.wallet.address;
        match self {
            Command::Balance {
                token_address,
                l2,
                l1,
                wei,
            } => {
                if token_address.is_some() {
                    todo!("Handle ERC20 balances")
                }
                if !l1 || l2 {
                    let account_balance = rollup_client
                        .get_balance(from, BlockByNumber::Latest)
                        .await?;
                    println!(
                        "[L2] Account balance: {}",
                        balance_in_wei(wei, account_balance)
                    );
                }
                if l1 {
                    let account_balance =
                        eth_client.get_balance(from, BlockByNumber::Latest).await?;
                    println!(
                        "[L1] Account balance: {}",
                        balance_in_wei(wei, account_balance)
                    );
                }
            }
            Command::Deposit {
                amount,
                token_address,
                to,
                wait_for_receipt,
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
                        wait_for_receipt,
                        l1: true,
                        nonce: None,
                        explorer_url: false,
                    }
                    .run(cfg)
                    .await
                })
                .await?;
            }
            Command::ClaimWithdraw {
                l2_withdrawal_tx_hash,
                wait_for_receipt,
            } => {
                let claimed_amount = match rollup_client
                    .get_transaction_by_hash(l2_withdrawal_tx_hash)
                    .await?
                {
                    Some(l2_withdrawal_tx) => l2_withdrawal_tx.value,
                    None => {
                        println!("Withdrawal transaction not found in L2");
                        return Ok(());
                    }
                };

                let withdrawal_proof = match rollup_client
                    .get_withdrawal_proof(l2_withdrawal_tx_hash)
                    .await?
                {
                    Some(withdrawal_proof) => withdrawal_proof,
                    None => {
                        println!("Withdrawal proof not found in L2");
                        return Ok(());
                    }
                };

                let claim_tx = ethrex_l2_sdk::claim_withdraw(
                    claimed_amount,
                    l2_withdrawal_tx_hash,
                    cfg.wallet.address,
                    cfg.wallet.private_key,
                    &eth_client,
                    &withdrawal_proof,
                )
                .await?;

                if wait_for_receipt {
                    wait_for_transaction_receipt(&eth_client, claim_tx).await?;
                }
            }
            Command::Transfer {
                amount,
                token_address,
                to,
                nonce,
                wait_for_receipt,
                l1,
                explorer_url: _,
            } => {
                if token_address.is_some() {
                    todo!("Handle ERC20 transfers")
                }

                let client = if l1 { eth_client } else { rollup_client };

                let transfer_tx = client
                    .build_eip1559_transaction(
                        to,
                        cfg.wallet.address,
                        Bytes::new(),
                        Overrides {
                            value: Some(amount),
                            chain_id: if l1 {
                                Some(cfg.network.l1_chain_id)
                            } else {
                                Some(cfg.network.l2_chain_id)
                            },
                            nonce,
                            from: Some(cfg.wallet.address),
                            ..Default::default()
                        },
                    )
                    .await?;

                let tx_hash = client
                    .send_eip1559_transaction(&transfer_tx, &cfg.wallet.private_key)
                    .await?;

                println!(
                    "[{}] Transfer sent: {tx_hash:#x}",
                    if l1 { "L1" } else { "L2" }
                );

                if wait_for_receipt {
                    wait_for_transaction_receipt(&client, tx_hash).await?;
                }
            }
            Command::Withdraw {
                amount,
                to,
                nonce,
                token_address: _,
                wait_for_receipt,
                explorer_url: _,
            } => {
                let withdraw_transaction = rollup_client
                    .build_privileged_transaction(
                        to.unwrap_or(cfg.wallet.address),
                        to.unwrap_or(cfg.wallet.address),
                        COMMON_BRIDGE_L2_ADDRESS,
                        Bytes::from(encode_calldata(
                            L2_WITHDRAW_SIGNATURE,
                            &[Value::Address(from)],
                        )?),
                        Overrides {
                            nonce,
                            from: Some(cfg.wallet.address),
                            value: Some(amount),
                            ..Default::default()
                        },
                    )
                    .await?;

                let tx_hash = rollup_client
                    .send_privileged_l2_transaction(&withdraw_transaction)
                    .await?;

                println!("Withdrawal sent: {tx_hash:#x}");

                if wait_for_receipt {
                    wait_for_transaction_receipt(&rollup_client, tx_hash).await?;
                }
            }
            Command::WithdrawalProof { tx_hash } => {
                let withdrawal_proof = match rollup_client.get_withdrawal_proof(tx_hash).await? {
                    Some(withdrawal_proof) => withdrawal_proof,
                    None => {
                        println!("Withdrawal proof not found in L2");
                        return Ok(());
                    }
                };
                println!("{:?}", withdrawal_proof.merkle_proof);
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
                max_fee_per_gas,
                max_priority_fee_per_gas,
                wait_for_receipt,
            } => {
                let client = match l1 {
                    true => eth_client,
                    false => rollup_client,
                };

                let tx = client
                    .build_eip1559_transaction(
                        to,
                        cfg.wallet.address,
                        calldata,
                        Overrides {
                            value: Some(value),
                            chain_id: if let Some(chain_id) = chain_id {
                                Some(chain_id)
                            } else if l1 {
                                Some(cfg.network.l1_chain_id)
                            } else {
                                Some(cfg.network.l2_chain_id)
                            },
                            nonce,
                            gas_limit,
                            max_fee_per_gas,
                            max_priority_fee_per_gas,
                            from: Some(cfg.wallet.address),
                            ..Default::default()
                        },
                    )
                    .await?;
                let tx_hash = client
                    .send_eip1559_transaction(&tx, &cfg.wallet.private_key)
                    .await?;

                println!(
                    "[{}] Transaction sent: {tx_hash:#x}",
                    if l1 { "L1" } else { "L2" }
                );

                if wait_for_receipt {
                    wait_for_transaction_receipt(&client, tx_hash).await?;
                }
            }
            Command::Call {
                to,
                calldata,
                l1,
                value,
                from,
                gas_limit,
                max_fee_per_gas,
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
                            max_fee_per_gas,
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
                max_fee_per_gas,
                max_priority_fee_per_gas,
                wait_for_receipt,
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
                            max_fee_per_gas,
                            max_priority_fee_per_gas,
                            ..Default::default()
                        },
                    )
                    .await?;

                println!("Contract deployed in tx: {deployment_tx_hash:#x}");
                println!("Contract address: {deployed_contract_address:#x}");

                if wait_for_receipt {
                    wait_for_transaction_receipt(&client, deployment_tx_hash).await?;
                }
            }
        };
        Ok(())
    }
}

pub async fn wait_for_transaction_receipt(client: &EthClient, tx_hash: H256) -> eyre::Result<()> {
    println!("Waiting for transaction receipt...");
    while client.get_transaction_receipt(tx_hash).await?.is_none() {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    println!("Transaction confirmed");
    Ok(())
}

pub fn balance_in_wei(wei: bool, balance: U256) -> String {
    if wei {
        format!("{balance}")
    } else {
        let mut balance = format!("{balance}");
        let len = balance.len();

        balance = match len {
            18 => {
                let mut front = "0.".to_owned();
                front.push_str(&balance);
                front
            }
            0..=17 => {
                let mut front = "0.".to_owned();
                let zeros = "0".repeat(18 - len);
                front.push_str(&zeros);
                front.push_str(&balance);
                front
            }
            19.. => {
                balance.insert(len - 18, '.');
                balance
            }
        };
        balance
    }
}

#[test]
fn test_balance_in_ether() {
    // test more than 1 ether
    assert_eq!(
        "999999999.999003869993631450",
        balance_in_wei(
            false,
            U256::from_dec_str("999999999999003869993631450").unwrap()
        )
    );

    // test 0.5
    assert_eq!(
        "0.509003869993631450",
        balance_in_wei(
            false,
            U256::from_dec_str("000000000509003869993631450").unwrap()
        )
    );

    // test 0.005
    assert_eq!(
        "0.005090038699936314",
        balance_in_wei(
            false,
            U256::from_dec_str("000000000005090038699936314").unwrap()
        )
    );

    // test 0.0
    assert_eq!("0.000000000000000000", balance_in_wei(false, U256::zero()));
}
