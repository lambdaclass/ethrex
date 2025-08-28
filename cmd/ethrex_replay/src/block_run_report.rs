use std::{fmt::Display, time::Duration};

use ethrex_common::types::Block;
use ethrex_config::networks::{Network, PublicNetwork};

use crate::slack::{SlackWebHookActionElement, SlackWebHookBlock, SlackWebHookRequest};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ReplayerMode {
    Execute,
    ExecuteSP1,
    ExecuteRISC0,
    ProveSP1,
    ProveRISC0,
}

impl ReplayerMode {
    pub fn is_proving_mode(&self) -> bool {
        matches!(self, ReplayerMode::ProveSP1 | ReplayerMode::ProveRISC0)
    }

    pub fn is_execution_mode(&self) -> bool {
        matches!(
            self,
            ReplayerMode::Execute | ReplayerMode::ExecuteSP1 | ReplayerMode::ExecuteRISC0
        )
    }
}

impl Display for ReplayerMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayerMode::Execute => write!(f, "execute"),
            ReplayerMode::ExecuteSP1 => write!(f, "execute_sp1"),
            ReplayerMode::ExecuteRISC0 => write!(f, "execute_risc0"),
            ReplayerMode::ProveSP1 => write!(f, "prove_sp1"),
            ReplayerMode::ProveRISC0 => write!(f, "prove_risc0"),
        }
    }
}

pub struct BlockRunReport {
    pub network: Network,
    pub number: u64,
    pub gas: u64,
    pub txs: u64,
    pub run_result: Result<(), eyre::Report>,
    pub replayer_mode: ReplayerMode,
    pub time_taken: Duration,
}

impl BlockRunReport {
    pub fn new_for(
        block: Block,
        network: Network,
        run_result: Result<(), eyre::Report>,
        replayer_mode: ReplayerMode,
        time_taken: Duration,
    ) -> Self {
        Self {
            network,
            number: block.header.number,
            gas: block.header.gas_used,
            txs: block.body.transactions.len() as u64,
            run_result,
            replayer_mode,
            time_taken,
        }
    }

    pub fn to_csv(&self) -> String {
        let execution_result = if let Err(e) = &self.run_result {
            format!("Error: {e}")
        } else {
            "Success".to_string()
        };

        format!(
            "{},{},{},{},{},{}",
            self.number,
            self.gas,
            self.txs,
            format_duration(self.time_taken),
            self.replayer_mode,
            execution_result,
        )
    }

    pub fn to_slack_message(&self) -> SlackWebHookRequest {
        SlackWebHookRequest {
            blocks: vec![
                SlackWebHookBlock::Header {
                    text: Box::new(SlackWebHookBlock::PlainText {
                        text: match (&self.run_result, &self.replayer_mode) {
                            (Ok(_), ReplayerMode::Execute) => {
                                String::from("✅ Successfully Executed Block")
                            }
                            (Ok(_), ReplayerMode::ExecuteSP1) => {
                                String::from("✅ Successfully Executed Block with SP1")
                            }
                            (Ok(_), ReplayerMode::ExecuteRISC0) => {
                                String::from("✅ Successfully Executed Block with RISC0")
                            }
                            (Ok(_), ReplayerMode::ProveSP1) => {
                                String::from("✅ Successfully Proved Block with SP1")
                            }
                            (Ok(_), ReplayerMode::ProveRISC0) => {
                                String::from("✅ Successfully Proved Block with RISC0")
                            }
                            (Err(_), ReplayerMode::Execute) => {
                                String::from("⚠️ Failed to Execute Block")
                            }
                            (Err(_), ReplayerMode::ExecuteSP1) => {
                                String::from("⚠️ Failed to Execute Block with SP1")
                            }
                            (Err(_), ReplayerMode::ExecuteRISC0) => {
                                String::from("⚠️ Failed to Execute Block with RISC0")
                            }
                            (Err(_), ReplayerMode::ProveSP1) => {
                                String::from("⚠️ Failed to Prove Block with SP1")
                            }
                            (Err(_), ReplayerMode::ProveRISC0) => {
                                String::from("⚠️ Failed to Prove Block with RISC0")
                            }
                        },
                        emoji: true,
                    }),
                },
                SlackWebHookBlock::Section {
                    text: Box::new(SlackWebHookBlock::Markdown {
                        text: format!(
                            "*Network:* `{network}`\n*Block:* {number}\n*Gas:* {gas}\n*#Txs:* {txs}\n*Execution Result:* {execution_result}\n*Time Taken:* {time_taken}",
                            network = self.network,
                            number = self.number,
                            gas = self.gas,
                            txs = self.txs,
                            execution_result = if self.run_result.is_err() {
                                "Error: ".to_string()
                                    + &self.run_result.as_ref().err().unwrap().to_string()
                            } else {
                                "Success".to_string()
                            },
                            time_taken = format_duration(self.time_taken),
                        ),
                    }),
                },
                SlackWebHookBlock::Actions {
                    elements: vec![SlackWebHookActionElement::Button {
                        text: SlackWebHookBlock::PlainText {
                            text: String::from("View on Etherscan"),
                            emoji: false,
                        },
                        url: if let Network::PublicNetwork(PublicNetwork::Mainnet) = self.network {
                            format!("https://etherscan.io/block/{}", self.number)
                        } else {
                            format!(
                                "https://{}.etherscan.io/block/{}",
                                self.network, self.number
                            )
                        },
                    }],
                },
            ],
        }
    }
}

impl Display for BlockRunReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let execution_result = if let Err(e) = &self.run_result {
            format!("Error: {e}")
        } else {
            "Success".to_string()
        };
        if let Network::PublicNetwork(_) = self.network {
            write!(
                f,
                "[{network}] Block #{number}, Gas Used: {gas}, Tx Count: {txs}, {replayer_mode} Result: {execution_result}, Time Taken: {time_taken} | {block_url}",
                network = self.network,
                number = self.number,
                gas = self.gas,
                txs = self.txs,
                replayer_mode = self.replayer_mode,
                execution_result = execution_result,
                time_taken = format_duration(self.time_taken),
                block_url = if let Network::PublicNetwork(PublicNetwork::Mainnet) = self.network {
                    format!("https://etherscan.io/block/{}", self.number)
                } else {
                    format!(
                        "https://{}.etherscan.io/block/{}",
                        self.network, self.number
                    )
                },
            )
        } else {
            write!(
                f,
                "[{network}] Block #{number}, Gas Used: {gas}, Tx Count: {txs}, {replayer_mode} Result: {execution_result}, Time Taken: {time_taken}",
                network = self.network,
                number = self.number,
                gas = self.gas,
                txs = self.txs,
                replayer_mode = self.replayer_mode,
                execution_result = execution_result,
                time_taken = format_duration(self.time_taken),
            )
        }
    }
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    let milliseconds = duration.subsec_millis();

    if hours > 0 {
        return format!("{hours:02}h {minutes:02}m {seconds:02}s {milliseconds:03}ms");
    }

    if minutes == 0 {
        return format!("{seconds:02}s {milliseconds:03}ms");
    }

    format!("{minutes:02}m {seconds:02}s")
}
