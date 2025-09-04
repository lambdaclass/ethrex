use serde::Serialize;

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
