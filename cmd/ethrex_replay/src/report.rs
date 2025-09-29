use std::{fmt::Display, process::Command, time::Duration};

use ethrex_common::types::Block;
use ethrex_config::networks::{Network, PublicNetwork};
use tracing::{error, info};

use crate::{
    cli::{Action, Resource, ZKVM},
    slack::{SlackWebHookActionElement, SlackWebHookBlock, SlackWebHookRequest},
};

pub struct Report {
    pub zkvm: Option<ZKVM>,
    pub resource: Resource,
    pub action: Action,
    pub block: Block,
    pub network: Network,
    pub execution_result: Result<Duration, eyre::Report>,
    pub proving_result: Option<Result<Duration, eyre::Report>>,
}

impl Report {
    pub fn new_for(
        zkvm: Option<ZKVM>,
        resource: Resource,
        action: Action,
        block: Block,
        network: Network,
        execution_result: Result<Duration, eyre::Report>,
        proving_result: Option<Result<Duration, eyre::Report>>,
    ) -> Self {
        Self {
            zkvm,
            resource,
            action,
            block,
            network,
            execution_result,
            proving_result,
        }
    }

    pub fn to_slack_message(&self) -> SlackWebHookRequest {
        let eth_proofs_button = SlackWebHookActionElement::Button {
            text: SlackWebHookBlock::PlainText {
                text: String::from("View on EthProofs"),
                emoji: false,
            },
            url: format!("https://ethproofs.org/blocks/{}", self.block.header.number),
        };

        let mut slack_webhook_actions = vec![SlackWebHookActionElement::Button {
            text: SlackWebHookBlock::PlainText {
                text: String::from("View on Etherscan"),
                emoji: false,
            },
            url: if let Network::PublicNetwork(PublicNetwork::Mainnet) = self.network {
                format!("https://etherscan.io/block/{}", self.block.header.number)
            } else {
                format!(
                    "https://{}.etherscan.io/block/{}",
                    self.network, self.block.header.number
                )
            },
        }];

        if let Network::PublicNetwork(PublicNetwork::Mainnet) = self.network {
            // EthProofs only prove block numbers multiples of 100.
            if self.block.header.number % 100 == 0 && matches!(self.action, Action::Prove) {
                slack_webhook_actions.push(eth_proofs_button);
            }
        }

        let maybe_zkvm = if let Some(zkvm) = &self.zkvm {
            format!(" with {zkvm}")
        } else {
            "".to_string()
        };

        SlackWebHookRequest {
            blocks: vec![
                SlackWebHookBlock::Header {
                    text: Box::new(SlackWebHookBlock::PlainText {
                        text: match (&self.execution_result, &self.proving_result) {
                            (Ok(_), Some(Ok(_))) | (Ok(_), None) => format!(
                                "✅ Succeeded to {} Block{} on {}",
                                self.action, maybe_zkvm, self.resource
                            ),
                            (Ok(_), Some(Err(_))) | (Err(_), _) => format!(
                                "⚠️ Failed to {} Block{} on {}",
                                self.action, maybe_zkvm, self.resource
                            ),
                        },
                        emoji: true,
                    }),
                },
                SlackWebHookBlock::Section {
                    text: Box::new(SlackWebHookBlock::Markdown {
                        text: format!(
                            "*Network:* `{network}`\n*Block:* {number}\n*Gas:* {gas}\n*#Txs:* {txs}{maybe_execution_result}{maybe_proving_result}{maybe_gpu}{maybe_cpu}{maybe_ram}{maybe_git_info}{maybe_execution_time}{maybe_proving_time}",
                            network = self.network,
                            number = self.block.header.number,
                            gas = self.block.header.gas_used,
                            txs = self.block.body.transactions.len(),
                            maybe_proving_result = if let Some(Err(err)) = &self.proving_result {
                                format!("\n*Proving Error:* {err}")
                            } else {
                                "".to_string()
                            },
                            maybe_gpu = hardware_info_slack_message("GPU"),
                            maybe_cpu = hardware_info_slack_message("CPU"),
                            maybe_ram = hardware_info_slack_message("RAM"),
                            maybe_git_info = git_info_slack_message(),
                            maybe_execution_result = if self.proving_result.is_some() {
                                format!(
                                    "\n*Execution:* {}",
                                    match &self.execution_result {
                                        Ok(_) => "Succeeded".to_string(),
                                        Err(err) => format!("⚠️ Failed with {err}"),
                                    }
                                )
                            } else if let Err(err) = &self.execution_result {
                                format!("\n*Execution:* Failed with {err}")
                            } else {
                                "".to_string()
                            },
                            maybe_execution_time =
                                if let Ok(execution_duration) = &self.execution_result {
                                    format!(
                                        "\n*Execution Time:* {}",
                                        format_duration(execution_duration)
                                    )
                                } else {
                                    "".to_string()
                                },
                            maybe_proving_time = if let Some(Ok(proving_duration)) =
                                &self.proving_result
                            {
                                format!("\n*Proving Time:* {}", format_duration(proving_duration))
                            } else {
                                "".to_string()
                            },
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
        let network = &self.network;

        let block_number = &self.block.header.number;

        let gas = self.block.header.gas_used;

        let txs = self.block.body.transactions.len();

        let maybe_proving_time = if let Some(Ok(proving_duration)) = &self.proving_result {
            format!(", Proving Time: {}", format_duration(proving_duration))
        } else {
            "".to_string()
        };

        let maybe_etherscan_url =
            if let Some(url) = etherscan_url(&self.network, self.block.header.number) {
                format!(" | {url}")
            } else {
                "".to_string()
            };

        let maybe_ethproofs_url =
            if let Some(url) = ethproofs_url(&self.network, self.block.header.number) {
                format!(" | {url}")
            } else {
                "".to_string()
            };

        let maybe_proving_result = if let Some(Err(err)) = &self.proving_result {
            format!(", Proving Error: {err}")
        } else {
            "".to_string()
        };

        match (self.execution_result.as_ref(), self.proving_result.as_ref()) {
            (Ok(execution_result), Some(Ok(_))) | (Ok(execution_result), None) => {
                info!(
                    "[{network}] Block: {block_number}, Gas: {gas}, #Txs: {txs}, Execution Time: {execution_result}{maybe_proving_time}{maybe_etherscan_url}{maybe_ethproofs_url}",
                    execution_result = format_duration(execution_result)
                );
            }
            (Ok(_), Some(Err(_))) | (Err(_), _) => {
                error!(
                    "[{network}] Block: {block_number}, Gas: {gas}, #Txs: {txs}, Execution Result: {execution_result}{maybe_proving_result}{maybe_etherscan_url}{maybe_ethproofs_url}",
                    execution_result = if let Err(execution_result) = &self.execution_result {
                        format!("⚠️ Failed with {execution_result}")
                    } else {
                        "Succeeded".to_string()
                    }
                );
            }
        }
    }

    /// Convert the report to a benchmark file in JSON format.
    ///
    /// # CAUTION
    ///
    /// This function is used to create a benchmark file that is used by our CI
    /// for updating benchmarks from https://docs.ethrex.xyz/benchmarks/.
    ///
    /// Do not remove it under any circumstances, unless you are refactoring how
    /// we do benchmarks in CI.
    pub fn to_bench_file(&self) -> eyre::Result<serde_json::Value> {
        let elapsed = match (&self.execution_result, &self.proving_result) {
            (Ok(_execution_duration), Some(Ok(proving_duration))) => proving_duration.as_secs_f64(),
            (Ok(execution_duration), None) => execution_duration.as_secs_f64(),
            (Err(err), _) | (_, Some(Err(err))) => {
                return Err(eyre::Error::msg(format!(
                    "Cannot create benchmark file: {err}"
                )));
            }
        };
        let json = serde_json::json!([{
            "name": format!("{}, {}", self.zkvm.as_ref().ok_or_else(|| eyre::Error::msg("--zkvm must be set in CI mode"))?, match self.resource {
                Resource::CPU => cpu_info().unwrap_or_else(|| "CPU".to_string()),
                Resource::GPU => gpu_info().unwrap_or_else(|| "GPU".to_string()),
            }),
            "unit": "Mgas/s",
            "value": self.block.header.gas_used as f64 / 1e6 / elapsed,
        }]);

        Ok(json)
    }
}

impl Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let maybe_zkvm = if let Some(zkvm) = &self.zkvm {
            format!(" with {zkvm}")
        } else {
            "".to_string()
        };
        match (&self.execution_result, &self.proving_result) {
            (Ok(_), Some(Ok(_))) | (Ok(_), None) => writeln!(
                f,
                "✅ Succeeded to {} Block{} on {}",
                self.action, maybe_zkvm, self.resource
            )?,
            (Ok(_), Some(Err(_))) | (Err(_), _) => writeln!(
                f,
                "⚠️ Failed to {} Block{} on {}",
                self.action, maybe_zkvm, self.resource
            )?,
        };
        writeln!(f, "Network: {}", self.network)?;
        writeln!(f, "Block: {}", self.block.header.number)?;
        writeln!(f, "Gas: {}", self.block.header.gas_used)?;
        writeln!(f, "#Txs: {}", self.block.body.transactions.len())?;
        if self.proving_result.is_some() {
            writeln!(
                f,
                "Execution Result: {}",
                match &self.execution_result {
                    Ok(_) => "Succeeded".to_string(),
                    Err(err) => format!("⚠️ Failed with {err}"),
                }
            )?;
        } else if let Err(err) = &self.execution_result {
            writeln!(f, "Execution Error: {err}")?;
        }
        if let Some(Err(err)) = &self.proving_result {
            writeln!(f, "Proving Error: {err}")?;
        }

        if let Some(info) = gpu_info() {
            writeln!(f, "GPU: {info}")?;
        }
        if let Some(info) = cpu_info() {
            writeln!(f, "CPU: {info}")?;
        }
        if let Some(info) = ram_info() {
            writeln!(f, "RAM: {info}")?;
        }
        let git_info = git_info_slack_message();
        if !git_info.is_empty() {
            writeln!(
                f,
                "Branch & Commit: {}",
                git_info
                    .replace("\n*Branch & Commit:* `", "")
                    .replace("`", "")
            )?;
        }
        if let Ok(execution_duration) = &self.execution_result {
            writeln!(f, "Execution Time: {}", format_duration(execution_duration))?;
        }
        if let Some(Ok(proving_duration)) = &self.proving_result {
            writeln!(f, "Proving Time: {}", format_duration(proving_duration))?;
        }
        if let Some(url) = etherscan_url(&self.network, self.block.header.number) {
            writeln!(f, "Etherscan: {url}")?;
        }
        if let Some(url) = ethproofs_url(&self.network, self.block.header.number) {
            writeln!(f, "EthProofs: {url}",)?;
        }
        Ok(())
    }
}

fn format_duration(duration: &Duration) -> String {
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

fn git_info_slack_message() -> String {
    let branch = get_current_git_branch();

    let commit = get_current_git_commit();

    match (branch, commit) {
        (Some(b), Some(c)) => format!("\n*Branch & Commit:* `{b}` (`{c}`)"),
        _ => String::new(),
    }
}

fn get_current_git_branch() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8(output.stdout).ok()?.trim().to_string())
    } else {
        None
    }
}

fn get_current_git_commit() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8(output.stdout).ok()?.trim().to_string())
    } else {
        None
    }
}

fn etherscan_url(network: &Network, block_number: u64) -> Option<String> {
    match network {
        Network::PublicNetwork(PublicNetwork::Mainnet) => {
            Some(format!("https://etherscan.io/block/{block_number}"))
        }
        Network::PublicNetwork(_) => Some(format!(
            "https://{network}.etherscan.io/block/{block_number}"
        )),
        _ => None,
    }
}

fn ethproofs_url(network: &Network, block_number: u64) -> Option<String> {
    if block_number % 100 != 0 {
        return None;
    }

    if network != &Network::PublicNetwork(PublicNetwork::Mainnet) {
        return None;
    }

    Some(format!("https://ethproofs.org/blocks/{}", block_number))
}
