//! Telegram Bot integration for Tokamak Desktop App.
//! Uses long-polling to receive messages (no webhook needed).
//!
//! Env:
//!   TELEGRAM_BOT_TOKEN — Bot token from @BotFather
//!   TELEGRAM_ALLOWED_CHAT_IDS — Comma-separated allowed chat IDs (empty = deny all)
use crate::ai_provider::{AiProvider, ChatMessage};
use crate::appchain_manager::AppchainManager;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{watch, Mutex};

const MAX_HISTORY: usize = 20;
const POLL_TIMEOUT_SECS: u64 = 30;
const TELEGRAM_API: &str = "https://api.telegram.org";

pub struct TelegramBot {
    token: String,
    allowed_chat_ids: Vec<i64>,
    client: Client,
    ai: Arc<AiProvider>,
    appchain_manager: Arc<AppchainManager>,
    chat_history: Mutex<HashMap<i64, Vec<ChatMessage>>>,
}

#[derive(Debug, Deserialize)]
struct TelegramResponse<T> {
    ok: bool,
    result: Option<T>,
}

#[derive(Debug, Deserialize)]
struct Update {
    update_id: i64,
    message: Option<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    chat: Chat,
    text: Option<String>,
    from: Option<User>,
}

#[derive(Debug, Deserialize)]
struct Chat {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct User {
    first_name: String,
}

#[derive(Debug, Clone, Serialize)]
struct SendMessageRequest {
    chat_id: i64,
    text: String,
}

#[derive(Debug, Serialize)]
struct SendActionRequest {
    chat_id: i64,
    action: String,
}

impl TelegramBot {
    /// Load config from OS Keychain first, then fall back to env vars.
    pub fn new(ai: Arc<AiProvider>, appchain_manager: Arc<AppchainManager>) -> Option<Self> {
        let (token, allowed_ids_str, enabled) = Self::load_from_file()
            .unwrap_or_else(|| Self::load_from_env());

        if token.is_empty() || !enabled {
            return None;
        }

        let allowed_chat_ids: Vec<i64> = allowed_ids_str
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();

        Some(Self {
            token,
            allowed_chat_ids,
            client: Client::new(),
            ai,
            appchain_manager,
            chat_history: Mutex::new(HashMap::new()),
        })
    }

    fn load_from_file() -> Option<(String, String, bool)> {
        let path = dirs::data_dir()?.join("tokamak-appchain").join("telegram.json");
        let json = std::fs::read_to_string(path).ok()?;
        let config: crate::commands::TelegramConfig = serde_json::from_str(&json).ok()?;
        if config.bot_token.is_empty() {
            return None;
        }
        Some((config.bot_token, config.allowed_chat_ids, config.enabled))
    }

    fn load_from_env() -> (String, String, bool) {
        let token = std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
        let ids = std::env::var("TELEGRAM_ALLOWED_CHAT_IDS").unwrap_or_default();
        let enabled = !token.is_empty();
        (token, ids, enabled)
    }

    /// Create from explicit config values (used in tests).
    #[cfg(test)]
    fn new_with_token(
        token: &str,
        allowed_chat_ids: Vec<i64>,
        ai: Arc<AiProvider>,
        appchain_manager: Arc<AppchainManager>,
    ) -> Self {
        Self {
            token: token.to_string(),
            allowed_chat_ids,
            client: Client::new(),
            ai,
            appchain_manager,
            chat_history: Mutex::new(HashMap::new()),
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!("{}/bot{}/{}", TELEGRAM_API, self.token, method)
    }

    fn is_chat_allowed(&self, chat_id: i64) -> bool {
        self.allowed_chat_ids.contains(&chat_id)
    }

    async fn send_message(&self, chat_id: i64, text: &str) {
        let body = SendMessageRequest {
            chat_id,
            text: text.to_string(),
        };
        if let Err(e) = self
            .client
            .post(self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await
        {
            log::warn!("[TG] Failed to send message to {}: {}", chat_id, e);
        }
    }

    async fn send_typing(&self, chat_id: i64) {
        let body = SendActionRequest {
            chat_id,
            action: "typing".to_string(),
        };
        if let Err(e) = self
            .client
            .post(self.api_url("sendChatAction"))
            .json(&body)
            .send()
            .await
        {
            log::warn!("[TG] Failed to send typing to {}: {}", chat_id, e);
        }
    }

    /// Start long-polling loop. Runs until shutdown signal is received.
    pub async fn run(self: Arc<Self>, mut shutdown: watch::Receiver<bool>) {
        log::info!("Telegram bot started (polling mode)");
        let mut offset: i64 = 0;

        loop {
            if *shutdown.borrow() {
                log::info!("Telegram bot shutting down");
                return;
            }
            let url = format!(
                "{}?offset={}&timeout={}&allowed_updates=[\"message\"]",
                self.api_url("getUpdates"),
                offset,
                POLL_TIMEOUT_SECS
            );

            let resp = match self.client.get(&url).send().await {
                Ok(r) => r,
                Err(_e) => {
                    log::warn!("[TG] poll error (token masked)");
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            let updates: TelegramResponse<Vec<Update>> = match resp.json().await {
                Ok(r) => r,
                Err(_e) => {
                    log::warn!("[TG] parse error (details masked)");
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            if !updates.ok {
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                continue;
            }

            if let Some(results) = updates.result {
                for update in results {
                    offset = update.update_id + 1;
                    if let Some(message) = update.message {
                        let bot = self.clone();
                        tokio::spawn(async move {
                            bot.handle_message(message).await;
                        });
                    }
                }
            }
        }
    }

    async fn handle_message(&self, message: Message) {
        let chat_id = message.chat.id;
        let text = match message.text {
            Some(t) => t.trim().to_string(),
            None => return,
        };

        if !self.is_chat_allowed(chat_id) {
            self.send_message(chat_id, "Access denied. Your chat ID is not allowed.")
                .await;
            return;
        }

        if text == "/start" {
            self.cmd_start(chat_id, &message.from).await;
        } else if text == "/status" {
            self.cmd_status(chat_id).await;
        } else if text.starts_with("/start_chain") {
            self.cmd_start_chain(chat_id, &text).await;
        } else if text.starts_with("/stop_chain") {
            self.cmd_stop_chain(chat_id, &text).await;
        } else if text == "/clear" {
            self.chat_history.lock().await.remove(&chat_id);
            self.send_message(chat_id, "Conversation cleared.").await;
        } else if text.starts_with('/') {
            self.send_message(
                chat_id,
                "Commands:\n\
                 /status — Appchain status\n\
                 /start_chain <name> — Start appchain\n\
                 /stop_chain <name> — Stop appchain\n\
                 /clear — Clear chat history\n\n\
                 Or just send a message to chat with AI.",
            )
            .await;
        } else {
            self.cmd_ai_chat(chat_id, &text).await;
        }
    }

    async fn cmd_start(&self, chat_id: i64, from: &Option<User>) {
        let name = from
            .as_ref()
            .map(|u| u.first_name.as_str())
            .unwrap_or("there");
        self.send_message(
            chat_id,
            &format!(
                "Hi {name}! Welcome to Tokamak Appchain Bot.\n\n\
                 Commands:\n\
                 /status — View appchain status\n\
                 /start_chain <name> — Start appchain\n\
                 /stop_chain <name> — Stop appchain\n\
                 /clear — Clear chat history\n\n\
                 Or just send any message to chat with Tokamak AI."
            ),
        )
        .await;
    }

    async fn cmd_status(&self, chat_id: i64) {
        let chains = self.appchain_manager.list_appchains();

        if chains.is_empty() {
            self.send_message(chat_id, "No appchains found.").await;
            return;
        }

        let mut msg = String::from("Appchains:\n\n");
        for chain in &chains {
            let emoji = match format!("{:?}", chain.status).as_str() {
                "Running" => "🟢",
                "Stopped" => "🔴",
                "Created" => "⚪",
                "SettingUp" => "🟡",
                "Error" => "🔴",
                _ => "⚪",
            };
            msg.push_str(&format!(
                "{} {}\n   Chain ID: {}\n   Mode: {:?}\n   Status: {:?}\n\n",
                emoji, chain.name, chain.chain_id, chain.network_mode, chain.status
            ));
        }

        self.send_message(chat_id, &msg).await;
    }

    async fn cmd_start_chain(&self, chat_id: i64, text: &str) {
        let name = text.replace("/start_chain", "").trim().to_string();
        if name.is_empty() {
            self.send_message(chat_id, "Usage: /start_chain <appchain name>")
                .await;
            return;
        }

        let chains = self.appchain_manager.list_appchains();
        let chain = chains
            .iter()
            .find(|c| c.name.to_lowercase() == name.to_lowercase());

        match chain {
            Some(c) => {
                let status = format!("{:?}", c.status);
                if status == "Running" {
                    self.send_message(chat_id, &format!("{} is already running.", c.name))
                        .await;
                } else {
                    self.appchain_manager
                        .update_status(&c.id, crate::appchain_manager::AppchainStatus::Running);
                    self.send_message(chat_id, &format!("⚠️ {} marked as Running (status only — use desktop app for actual process control)", c.name))
                        .await;
                }
            }
            None => {
                self.send_message(
                    chat_id,
                    &format!("Appchain \"{name}\" not found. Use /status to see available."),
                )
                .await;
            }
        }
    }

    async fn cmd_stop_chain(&self, chat_id: i64, text: &str) {
        let name = text.replace("/stop_chain", "").trim().to_string();
        if name.is_empty() {
            self.send_message(chat_id, "Usage: /stop_chain <appchain name>")
                .await;
            return;
        }

        let chains = self.appchain_manager.list_appchains();
        let chain = chains
            .iter()
            .find(|c| c.name.to_lowercase() == name.to_lowercase());

        match chain {
            Some(c) => {
                let status = format!("{:?}", c.status);
                if status == "Stopped" {
                    self.send_message(chat_id, &format!("{} is already stopped.", c.name))
                        .await;
                } else {
                    self.appchain_manager
                        .update_status(&c.id, crate::appchain_manager::AppchainStatus::Stopped);
                    self.send_message(chat_id, &format!("⚠️ {} marked as Stopped (status only — use desktop app for actual process control)", c.name))
                        .await;
                }
            }
            None => {
                self.send_message(
                    chat_id,
                    &format!("Appchain \"{name}\" not found. Use /status to see available."),
                )
                .await;
            }
        }
    }

    /// Parse command name from message text (for testing)
    fn parse_command(text: &str) -> (&str, &str) {
        let text = text.trim();
        if !text.starts_with('/') {
            return ("", text);
        }
        match text.find(' ') {
            Some(idx) => (&text[..idx], text[idx..].trim()),
            None => (text, ""),
        }
    }

    async fn cmd_ai_chat(&self, chat_id: i64, text: &str) {
        self.send_typing(chat_id).await;

        let mut history_lock = self.chat_history.lock().await;
        let history = history_lock.entry(chat_id).or_insert_with(Vec::new);
        history.push(ChatMessage {
            role: "user".to_string(),
            content: text.to_string(),
        });

        // Trim history
        if history.len() > MAX_HISTORY {
            history.drain(..history.len() - MAX_HISTORY);
        }

        let messages = history.clone();
        drop(history_lock);

        match self.ai.chat(messages, None).await {
            Ok(response) => {
                // Save assistant response to history
                let mut history_lock = self.chat_history.lock().await;
                if let Some(history) = history_lock.get_mut(&chat_id) {
                    history.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: response.clone(),
                    });
                }
                drop(history_lock);

                // Telegram message limit is 4096 chars
                if response.len() > 4000 {
                    let truncated = format!("{}...\n\n(truncated)", &response[..4000]);
                    self.send_message(chat_id, &truncated).await;
                } else {
                    self.send_message(chat_id, &response).await;
                }
            }
            Err(e) => {
                log::error!("Telegram AI chat error: {e}");
                self.send_message(chat_id, "AI is temporarily unavailable. Please try again.")
                    .await;
            }
        }
    }
}

/// Manages the lifecycle of the Telegram bot (start/stop at runtime).
pub struct TelegramBotManager {
    shutdown_tx: std::sync::Mutex<Option<watch::Sender<bool>>>,
    ai: Arc<AiProvider>,
    appchain_manager: Arc<AppchainManager>,
    notify_config: std::sync::Mutex<Option<NotifyConfig>>,
}

struct NotifyConfig {
    token: String,
    chat_ids: Vec<i64>,
    client: Client,
}

impl TelegramBotManager {
    pub fn new(ai: Arc<AiProvider>, appchain_manager: Arc<AppchainManager>) -> Self {
        Self {
            shutdown_tx: std::sync::Mutex::new(None),
            ai,
            appchain_manager,
            notify_config: std::sync::Mutex::new(None),
        }
    }

    pub fn is_running(&self) -> bool {
        self.shutdown_tx.lock().unwrap().is_some()
    }

    pub fn start(&self) -> Result<(), String> {
        if self.is_running() {
            self.stop();
        }

        let bot = TelegramBot::new(self.ai.clone(), self.appchain_manager.clone())
            .ok_or("Telegram bot config not found or disabled")?;

        // Cache config for notify()
        let chat_ids = bot.allowed_chat_ids.clone();
        let token = bot.token.clone();
        *self.notify_config.lock().unwrap() = Some(NotifyConfig {
            token,
            chat_ids,
            client: Client::new(),
        });

        let bot = Arc::new(bot);
        let (tx, rx) = watch::channel(false);
        *self.shutdown_tx.lock().unwrap() = Some(tx);

        tauri::async_runtime::spawn(bot.run(rx));
        log::info!("Telegram bot started via manager");
        Ok(())
    }

    pub fn stop(&self) {
        if let Some(tx) = self.shutdown_tx.lock().unwrap().take() {
            let _ = tx.send(true);
            log::info!("Telegram bot stopped via manager");
        }
        *self.notify_config.lock().unwrap() = None;
    }

    /// Send a notification to all allowed chat IDs.
    pub fn notify(&self, message: &str) {
        if !self.is_running() {
            return;
        }

        let config = self.notify_config.lock().unwrap();
        let config = match config.as_ref() {
            Some(c) => c,
            None => return,
        };

        if config.chat_ids.is_empty() {
            return;
        }

        let token = config.token.clone();
        let chat_ids = config.chat_ids.clone();
        let message = message.to_string();
        let client = config.client.clone();
        drop(config);

        tauri::async_runtime::spawn(async move {
            for chat_id in chat_ids {
                let body = SendMessageRequest {
                    chat_id,
                    text: message.clone(),
                };
                let _ = client
                    .post(format!("{}/bot{}/sendMessage", TELEGRAM_API, token))
                    .json(&body)
                    .send()
                    .await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bot(allowed: Vec<i64>) -> TelegramBot {
        let ai = Arc::new(AiProvider::new());
        let am = Arc::new(AppchainManager::new());
        TelegramBot::new_with_token("test:fake_token", allowed, ai, am)
    }

    #[test]
    fn test_is_chat_allowed_empty_denies_all() {
        let bot = make_bot(vec![]);
        assert!(!bot.is_chat_allowed(12345));
        assert!(!bot.is_chat_allowed(-99999));
    }

    #[test]
    fn test_is_chat_allowed_restricts() {
        let bot = make_bot(vec![111, 222, 333]);
        assert!(bot.is_chat_allowed(111));
        assert!(bot.is_chat_allowed(222));
        assert!(!bot.is_chat_allowed(999));
    }

    #[test]
    fn test_parse_command() {
        assert_eq!(TelegramBot::parse_command("/start"), ("/start", ""));
        assert_eq!(
            TelegramBot::parse_command("/start_chain my-chain"),
            ("/start_chain", "my-chain")
        );
        assert_eq!(
            TelegramBot::parse_command("/stop_chain  test chain "),
            ("/stop_chain", "test chain")
        );
        assert_eq!(TelegramBot::parse_command("hello world"), ("", "hello world"));
        assert_eq!(TelegramBot::parse_command(""), ("", ""));
    }
}
