use serde::Serialize;

use crate::report::Report;

#[derive(Serialize)]
pub struct SlackWebHookRequest {
    pub blocks: Vec<SlackWebHookBlock>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlackWebHookBlock {
    Header {
        text: Box<SlackWebHookBlock>,
    },
    Section {
        text: Box<SlackWebHookBlock>,
    },
    Actions {
        elements: Vec<SlackWebHookActionElement>,
    },
    PlainText {
        text: String,
        emoji: bool,
    },
    #[serde(rename = "mrkdwn")]
    Markdown {
        text: String,
    },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlackWebHookActionElement {
    Button {
        text: SlackWebHookBlock,
        url: String,
    },
}

pub async fn try_send_report_to_slack(
    report: &Report,
    slack_webhook_url: Option<reqwest::Url>,
) -> Result<(), reqwest::Error> {
    let Some(webhook_url) = slack_webhook_url else {
        return Ok(());
    };

    let client = reqwest::Client::new();

    let payload = report.to_slack_message();

    client.post(webhook_url).json(&payload).send().await?;

    Ok(())
}
