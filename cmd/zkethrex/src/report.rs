use std::{fmt::Display, process::Command, time::Duration};

use ethrex_common::types::Block;
use ethrex_config::networks::{Network, PublicNetwork};
use zkvm_interface::{
    ProgramExecutionReport, ProgramProvingReport, Proof, PublicValues, zkVMError,
};

use crate::{
    cli::{Action, Resource, ZKVM},
    slack::{SlackWebHookActionElement, SlackWebHookBlock, SlackWebHookRequest},
};

pub struct Report {
    pub zkvm: ZKVM,
    pub resource: Resource,
    pub action: Action,
    pub block: Block,
    pub network: Network,
    pub execution_result: Result<(PublicValues, ProgramExecutionReport), zkVMError>,
    pub proving_result: Option<Result<(PublicValues, Proof, ProgramProvingReport), zkVMError>>,
}

impl Report {
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

        if let Network::PublicNetwork(_) = self.network {
            // EthProofs only prove block numbers multiples of 100.
            if self.block.header.number % 100 == 0 && matches!(self.action, Action::Prove) {
                slack_webhook_actions.push(eth_proofs_button);
            }
        }

        SlackWebHookRequest {
            blocks: vec![
                SlackWebHookBlock::Header {
                    text: Box::new(SlackWebHookBlock::PlainText {
                        text: match (&self.execution_result, &self.proving_result) {
                            (Ok(_), Some(Ok(_))) | (Ok(_), None) => format!(
                                "✅ Succeeded to {} Block with {} on {}",
                                self.action, self.zkvm, self.resource
                            ),
                            (Ok(_), Some(Err(_))) | (Err(_), _) => format!(
                                "⚠️ Failed to {} Block with {} on {}",
                                self.action, self.zkvm, self.resource
                            ),
                        },
                        emoji: true,
                    }),
                },
                SlackWebHookBlock::Section {
                    text: Box::new(SlackWebHookBlock::Markdown {
                        text: format!(
                            "*Network:* `{network}`\n*Block:* {number}\n*Gas:* {gas}\n*#Txs:* {txs}{maybe_execution_result}{maybe_execution_result}{maybe_proving_result}{maybe_gpu}{maybe_cpu}{maybe_ram}{maybe_execution_time}{maybe_proving_time}",
                            network = self.network,
                            number = self.block.header.number,
                            gas = self.block.header.gas_used,
                            txs = self.block.body.transactions.len(),
                            maybe_execution_result = if self.proving_result.is_some() {
                                format!(
                                    "\n*Execution Result:* {}",
                                    match &self.execution_result {
                                        Ok(_) => "✅ Succeeded".to_string(),
                                        Err(err) => format!("⚠️ Failed with {err}"),
                                    }
                                )
                            } else if let Err(err) = &self.execution_result {
                                format!("\n*Execution Error:* {err}")
                            } else {
                                "".to_string()
                            },
                            maybe_execution_time =
                                if let Ok((_public_values, report)) = &self.execution_result {
                                    format!(
                                        "\n*Execution Time:* {}",
                                        format_duration(report.execution_duration)
                                    )
                                } else {
                                    "".to_string()
                                },
                            maybe_proving_result = if let Some(Err(err)) = &self.proving_result {
                                format!("\n*Proving Error:* {err}")
                            } else {
                                "".to_string()
                            },
                            maybe_proving_time = if let Some(Ok((_public_values, _proof, report))) =
                                &self.proving_result
                            {
                                format!(
                                    "\n*Proving Time:* {}",
                                    format_duration(report.proving_time)
                                )
                            } else {
                                "".to_string()
                            },
                            maybe_gpu = hardware_info_slack_message("GPU"),
                            maybe_cpu = hardware_info_slack_message("CPU"),
                            maybe_ram = hardware_info_slack_message("RAM"),
                        ),
                    }),
                },
                SlackWebHookBlock::Actions {
                    elements: slack_webhook_actions,
                },
            ],
        }
    }
}

impl Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.execution_result, &self.proving_result) {
            (Ok(_), Some(Ok(_))) | (Ok(_), None) => writeln!(
                f,
                "✅ Succeeded to {} Block with {} on {}",
                self.action, self.zkvm, self.resource
            )?,
            (Ok(_), Some(Err(_))) | (Err(_), _) => writeln!(
                f,
                "⚠️ Failed to {} Block with {} on {}",
                self.action, self.zkvm, self.resource
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
                    Ok(_) => "✅ Succeeded".to_string(),
                    Err(err) => format!("⚠️ Failed with {err}"),
                }
            )?;
        } else if let Err(err) = &self.execution_result {
            writeln!(f, "Execution Error: {err}")?;
        }
        if let Ok((_public_values, report)) = &self.execution_result {
            writeln!(
                f,
                "Execution Time: {}",
                format_duration(report.execution_duration)
            )?;
        }
        if let Some(Err(err)) = &self.proving_result {
            writeln!(f, "Proving Error: {err}")?;
        }
        if let Some(Ok((_public_values, _proof, report))) = &self.proving_result {
            writeln!(f, "Proving Time: {}", format_duration(report.proving_time))?;
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
        Ok(())
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
