use std::{fmt::Display, process::Command, time::Duration};

use ethrex_common::types::Block;
use ethrex_config::networks::{Network, PublicNetwork};
use tracing::{error, info};

use crate::slack::{SlackWebHookActionElement, SlackWebHookBlock, SlackWebHookRequest};

#[derive(Debug, Clone)]
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
        let eth_proofs_button = SlackWebHookActionElement::Button {
            text: SlackWebHookBlock::PlainText {
                text: String::from("View on EthProofs"),
                emoji: false,
            },
            url: format!("https://ethproofs.org/blocks/{}", self.number),
        };

        let mut slack_webhook_actions = vec![SlackWebHookActionElement::Button {
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
        }];

        if let Network::PublicNetwork(PublicNetwork::Mainnet) = self.network {
            // EthProofs only prove block numbers multiples of 100.
            if self.number % 100 == 0 && self.replayer_mode.is_proving_mode() {
                slack_webhook_actions.push(eth_proofs_button);
            }
        }

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
                            "*Network:* `{network}`\n*Block:* {number}\n*Gas:* {gas}\n*#Txs:* {txs}\n*Execution Result:* {execution_result}{maybe_gpu}{maybe_cpu}{maybe_ram}\n*Time Taken:* {time_taken}\n*Branch:* `{branch}`",
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
                            maybe_gpu = hardware_info_slack_message("GPU"),
                            maybe_cpu = hardware_info_slack_message("CPU"),
                            maybe_ram = hardware_info_slack_message("RAM"),
                            time_taken = format_duration(self.time_taken),
                            branch =
                                get_current_git_branch().unwrap_or_else(|| "unknown".to_string(),)
                        ),
                    }),
                },
                SlackWebHookBlock::Actions {
                    elements: slack_webhook_actions,
                },
            ],
        }
    }

    pub fn log(&self) {
        if self.run_result.is_err() {
            error!("{self}");
        } else {
            info!("{self}");
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
                "[{network}] Block #{number}, Gas Used: {gas}, Tx Count: {txs}, {replayer_mode} Result: {execution_result}, Time Taken: {time_taken}, Branch: {branch} | {block_url}",
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
                branch = get_current_git_branch().unwrap_or_else(|| "unknown".to_string()),
            )
        } else {
            write!(
                f,
                "[{network}] Block #{number}, Gas Used: {gas}, Tx Count: {txs}, {replayer_mode} Result: {execution_result}, Time Taken: {time_taken}, Branch: {branch}",
                network = self.network,
                number = self.number,
                gas = self.gas,
                txs = self.txs,
                replayer_mode = self.replayer_mode,
                execution_result = execution_result,
                time_taken = format_duration(self.time_taken),
                branch = get_current_git_branch().unwrap_or_else(|| "unknown".to_string()),
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

fn hardware_info_slack_message(hardware: &str) -> String {
    let hardware_info = match hardware {
        "GPU" => gpu_info(),
        "CPU" => cpu_info(),
        "RAM" => ram_info(),
        _ => None,
    };

    if let Some(info) = hardware_info {
        format!("\n*{hardware}:* `{info}`")
    } else {
        String::new()
    }
}

fn gpu_info() -> Option<String> {
    match std::env::consts::OS {
        // Linux: nvidia-smi --query-gpu=name --format=csv | tail -n +2
        "linux" => {
            let output = Command::new("sh")
                .arg("-c")
                .arg("nvidia-smi --query-gpu=name --format=csv | tail -n +2")
                .output()
                .ok()?;
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        // macOS: system_profiler SPDisplaysDataType | grep "Chipset Model" | awk -F': ' '{print $2}' | head -n 1
        "macos" => {
            let output = Command::new("sh")
                .arg("-c")
                .arg("system_profiler SPDisplaysDataType | grep \"Chipset Model\" | awk -F': ' '{print $2}' | head -n 1")
                .output()
                .ok()?;
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        _ => None,
    }
}

fn cpu_info() -> Option<String> {
    match std::env::consts::OS {
        // Linux: cat /proc/cpuinfo | grep "model name" | head -n 1 | awk -F': ' '{print $2}'
        "linux" => {
            let output = Command::new("sh")
                .arg("-c")
                .arg(
                    "cat /proc/cpuinfo | grep \"model name\" | head -n 1 | awk -F': ' '{print $2}'",
                )
                .output()
                .inspect_err(|e| eprintln!("Failed to get CPU info: {}", e))
                .ok()?;
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        // macOS: sysctl -n machdep.cpu.brand_string
        "macos" => {
            let output = Command::new("sysctl")
                .arg("-n")
                .arg("machdep.cpu.brand_string")
                .output()
                .inspect_err(|e| eprintln!("Failed to get CPU info: {}", e))
                .ok()?;
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        _ => None,
    }
}

fn ram_info() -> Option<String> {
    match std::env::consts::OS {
        // Linux: free --giga -h | grep "Mem:" | awk '{print $2}'
        "linux" => {
            let output = Command::new("sh")
                .arg("-c")
                .arg("free --giga -h | grep \"Mem:\" | awk '{print $2}'")
                .output()
                .inspect_err(|e| eprintln!("Failed to get RAM info: {}", e))
                .ok()?;
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        // macOS: system_profiler SPHardwareDataType | grep "Memory:" | awk -F': ' '{print $2}'
        "macos" => {
            let output = Command::new("sh")
                .arg("-c")
                .arg("system_profiler SPHardwareDataType | grep \"Memory:\" | awk -F': ' '{print $2}'")
                .output()
                .inspect_err(|e| eprintln!("Failed to get RAM info: {}", e))
                .ok()?;
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        _ => None,
    }
}

fn get_current_git_branch() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;

    if output.status.success() {
        let branch = String::from_utf8(output.stdout).ok()?.trim().to_string();
        if !branch.is_empty() {
            Some(branch)
        } else {
            None
        }
    } else {
        None
    }
}
