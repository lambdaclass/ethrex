use std::{fmt::Display, time::Duration};

use ethrex_common::types::Block;
use ethrex_config::networks::{Network, PublicNetwork};

use crate::{
    ReplayerMode,
    slack::{SlackWebHookActionElement, SlackWebHookBlock, SlackWebHookRequest},
};

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

    pub fn to_slack_message(&self) -> SlackWebHookRequest {
        SlackWebHookRequest {
            blocks: vec![
                SlackWebHookBlock::Header {
                    text: Box::new(SlackWebHookBlock::PlainText {
                        text: match (&self.run_result, &self.replayer_mode) {
                            (Ok(_), ReplayerMode::Execute) => {
                                String::from("✅ Successfully Executed Block")
                            }
                            (Ok(_), ReplayerMode::Prove) => {
                                String::from("✅ Successfully Proved Block")
                            }
                            (Err(_), ReplayerMode::Execute) => {
                                String::from("⚠️ Failed to Execute Block")
                            }
                            (Err(_), ReplayerMode::Prove) => {
                                String::from("⚠️ Failed to Prove Block")
                            }
                        },
                        emoji: true,
                    }),
                },
                SlackWebHookBlock::Section {
                    text: Box::new(SlackWebHookBlock::Markdown {
                        text: format!(
                            "*Network:* `{network}`\n*Block:* {number}\n*Gas:* {gas}\n*#Txs:* {txs}\n*Execution Result:* {execution_result}",
                            network = self.network,
                            number = self.number,
                            gas = self.gas,
                            txs = self.txs,
                            execution_result = if self.run_result.is_err() {
                                "Error: ".to_string()
                                    + &self.run_result.as_ref().err().unwrap().to_string()
                            } else {
                                "Success".to_string()
                            }
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
    let milliseconds = total_seconds / 1000;

    if hours > 0 {
        return format!("{hours:02}h {minutes:02}m {seconds:02}s {milliseconds:03}ms");
    }

    if minutes == 0 {
        return format!("{seconds:02}s {milliseconds:03}ms");
    }

    format!("{minutes:02}m {seconds:02}s")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slack_message_failed_mainnet_execution() {
        let report = BlockRunReport::new_for(
            Block::default(),
            Network::PublicNetwork(PublicNetwork::Mainnet),
            Err(eyre::Report::msg("Execution failed")),
            ReplayerMode::Execute,
            Duration::from_secs(1),
        );

        let slack_message = report.to_slack_message();

        let slack_message_json_request = serde_json::to_string_pretty(&slack_message)
            .expect("Failed to serialize Slack message");

        let expected_json = r#"{
  "blocks": [
    {
      "type": "header",
      "text": {
        "type": "plain_text",
        "text": "⚠️ Failed to Execute Block",
        "emoji": true
      }
    },
    {
      "type": "section",
      "text": {
        "type": "mrkdwn",
        "text": "*Network:* `mainnet`\n*Block:* 0\n*Gas:* 0\n*#Txs:* 0\n*Execution Result:* Error: Execution failed"
      }
    },
    {
      "type": "actions",
      "elements": [
        {
          "type": "button",
          "text": {
            "type": "plain_text",
            "text": "View on Etherscan",
            "emoji": false
          },
          "url": "https://etherscan.io/block/0"
        }
      ]
    }
  ]
}"#;

        assert_eq!(slack_message_json_request, expected_json);
    }

    #[test]
    fn test_slack_message_failed_hoodi_execution() {
        let report = BlockRunReport::new_for(
            Block::default(),
            Network::PublicNetwork(PublicNetwork::Hoodi),
            Err(eyre::Report::msg("Execution failed")),
            ReplayerMode::Execute,
            Duration::from_secs(1),
        );

        let slack_message = report.to_slack_message();

        let slack_message_json_request = serde_json::to_string_pretty(&slack_message)
            .expect("Failed to serialize Slack message");

        let expected_json = r#"{
  "blocks": [
    {
      "type": "header",
      "text": {
        "type": "plain_text",
        "text": "⚠️ Failed to Execute Block",
        "emoji": true
      }
    },
    {
      "type": "section",
      "text": {
        "type": "mrkdwn",
        "text": "*Network:* `hoodi`\n*Block:* 0\n*Gas:* 0\n*#Txs:* 0\n*Execution Result:* Error: Execution failed"
      }
    },
    {
      "type": "actions",
      "elements": [
        {
          "type": "button",
          "text": {
            "type": "plain_text",
            "text": "View on Etherscan",
            "emoji": false
          },
          "url": "https://hoodi.etherscan.io/block/0"
        }
      ]
    }
  ]
}"#;

        assert_eq!(slack_message_json_request, expected_json);
    }

    #[test]
    fn test_slack_message_failed_sepolia_execution() {
        let report = BlockRunReport::new_for(
            Block::default(),
            Network::PublicNetwork(PublicNetwork::Sepolia),
            Err(eyre::Report::msg("Execution failed")),
            ReplayerMode::Execute,
            Duration::from_secs(1),
        );

        let slack_message = report.to_slack_message();

        let slack_message_json_request = serde_json::to_string_pretty(&slack_message)
            .expect("Failed to serialize Slack message");

        let expected_json = r#"{
  "blocks": [
    {
      "type": "header",
      "text": {
        "type": "plain_text",
        "text": "⚠️ Failed to Execute Block",
        "emoji": true
      }
    },
    {
      "type": "section",
      "text": {
        "type": "mrkdwn",
        "text": "*Network:* `sepolia`\n*Block:* 0\n*Gas:* 0\n*#Txs:* 0\n*Execution Result:* Error: Execution failed"
      }
    },
    {
      "type": "actions",
      "elements": [
        {
          "type": "button",
          "text": {
            "type": "plain_text",
            "text": "View on Etherscan",
            "emoji": false
          },
          "url": "https://sepolia.etherscan.io/block/0"
        }
      ]
    }
  ]
}"#;

        assert_eq!(slack_message_json_request, expected_json);
    }
}
