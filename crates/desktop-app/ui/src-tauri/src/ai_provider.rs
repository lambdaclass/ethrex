use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Duration;

const KEYRING_SERVICE: &str = "tokamak-appchain";
const KEYRING_API_KEY: &str = "ai-api-key";
const KEYRING_AI_CONFIG: &str = "ai-config";
const KEYRING_DEVICE_ID: &str = "device-id";
const KEYRING_AI_MODE: &str = "ai-mode";

const TOKAMAK_AI_BASE_URL: &str = "https://api.ai.tokamak.network";
const DAILY_TOKEN_LIMIT: u32 = 50_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AiMode {
    Tokamak,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    pub provider: String,
    #[serde(skip_serializing, skip_deserializing)]
    pub api_key: String,
    pub model: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: "tokamak".to_string(),
            api_key: String::new(),
            model: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub date: String,
    pub used: u32,
    pub limit: u32,
}

pub struct AiProvider {
    config: Mutex<AiConfig>,
    mode: Mutex<AiMode>,
    device_id: String,
    daily_tokens: Mutex<TokenUsage>,
    client: Client,
}

impl AiProvider {
    pub fn new() -> Self {
        let mut config = Self::load_config_meta().unwrap_or_default();
        config.api_key = Self::load_api_key().unwrap_or_default();
        let mode = Self::load_mode().unwrap_or(AiMode::Tokamak);
        let device_id = Self::load_or_create_device_id();
        let daily_tokens = Self::load_token_usage();

        Self {
            config: Mutex::new(config),
            mode: Mutex::new(mode),
            device_id,
            daily_tokens: Mutex::new(daily_tokens),
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    // ---- Device ID ----

    fn load_or_create_device_id() -> String {
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_DEVICE_ID) {
            if let Ok(id) = entry.get_password() {
                return id;
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_DEVICE_ID) {
            let _ = entry.set_password(&id);
        }
        id
    }

    pub fn get_device_id(&self) -> &str {
        &self.device_id
    }

    // ---- AI Mode ----

    fn load_mode() -> Option<AiMode> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_AI_MODE).ok()?;
        let data = entry.get_password().ok()?;
        serde_json::from_str(&data).ok()
    }

    fn save_mode(mode: &AiMode) -> Result<(), String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_AI_MODE)
            .map_err(|e| format!("Keyring error: {e}"))?;
        let data = serde_json::to_string(mode).map_err(|e| e.to_string())?;
        entry
            .set_password(&data)
            .map_err(|e| format!("Failed to save mode: {e}"))
    }

    pub fn get_mode(&self) -> AiMode {
        self.mode.lock().unwrap().clone()
    }

    pub fn set_mode(&self, mode: AiMode) -> Result<(), String> {
        Self::save_mode(&mode)?;
        *self.mode.lock().unwrap() = mode;
        Ok(())
    }

    // ---- Token Usage (local tracking) ----

    fn today() -> String {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    }

    fn load_token_usage() -> TokenUsage {
        let today = Self::today();
        // Simple in-memory reset per day; starts fresh each app launch per day
        TokenUsage {
            date: today,
            used: 0,
            limit: DAILY_TOKEN_LIMIT,
        }
    }

    pub fn get_token_usage(&self) -> TokenUsage {
        let mut usage = self.daily_tokens.lock().unwrap();
        let today = Self::today();
        if usage.date != today {
            usage.date = today;
            usage.used = 0;
        }
        usage.clone()
    }

    fn add_token_usage(&self, tokens: u32) {
        let mut usage = self.daily_tokens.lock().unwrap();
        let today = Self::today();
        if usage.date != today {
            usage.date = today;
            usage.used = 0;
        }
        usage.used += tokens;
    }

    fn check_token_limit(&self) -> Result<(), String> {
        let usage = self.get_token_usage();
        if usage.used >= usage.limit {
            return Err(
                "daily_limit_exceeded".to_string(),
            );
        }
        Ok(())
    }

    // ---- Config persistence (for custom mode) ----

    fn load_config_meta() -> Option<AiConfig> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_AI_CONFIG).ok()?;
        let data = entry.get_password().ok()?;
        serde_json::from_str(&data).ok()
    }

    fn save_config_meta(config: &AiConfig) -> Result<(), String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_AI_CONFIG)
            .map_err(|e| format!("Keyring error: {e}"))?;
        let data = serde_json::to_string(config).map_err(|e| e.to_string())?;
        entry
            .set_password(&data)
            .map_err(|e| format!("Failed to save config: {e}"))
    }

    fn load_api_key() -> Option<String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_API_KEY).ok()?;
        entry.get_password().ok()
    }

    fn save_api_key(key: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_API_KEY)
            .map_err(|e| format!("Keyring error: {e}"))?;
        entry
            .set_password(key)
            .map_err(|e| format!("Failed to save API key: {e}"))
    }

    pub fn save_config(&self, config: AiConfig) -> Result<(), String> {
        Self::save_api_key(&config.api_key)?;
        Self::save_config_meta(&config)?;
        *self.config.lock().unwrap() = config;
        Ok(())
    }

    pub fn get_config(&self) -> AiConfig {
        self.config.lock().unwrap().clone()
    }

    pub fn get_config_masked(&self) -> AiConfig {
        let mut config = self.get_config();
        if config.api_key.len() > 8 {
            let visible = &config.api_key[..4];
            config.api_key =
                format!("{visible}...{}", &config.api_key[config.api_key.len() - 4..]);
        } else if !config.api_key.is_empty() {
            config.api_key = "****".to_string();
        }
        config
    }

    pub fn has_api_key(&self) -> bool {
        !self.config.lock().unwrap().api_key.is_empty()
    }

    pub fn clear_config(&self) -> Result<(), String> {
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_API_KEY) {
            let _ = entry.delete_credential();
        }
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_AI_CONFIG) {
            let _ = entry.delete_credential();
        }
        *self.config.lock().unwrap() = AiConfig::default();
        Ok(())
    }

    // ---- Model fetching ----

    pub async fn fetch_models(&self, provider: &str, api_key: &str) -> Result<Vec<String>, String> {
        let url = Self::models_url(provider);
        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .send()
            .await
            .map_err(|e| format!("Failed to fetch models: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("API error ({status}): {body}"));
        }

        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        let models = result["data"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        Ok(models)
    }

    // ---- Chat ----

    pub async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        context_json: Option<String>,
    ) -> Result<String, String> {
        let mode = self.get_mode();
        let ctx_ref = context_json.as_deref();

        match mode {
            AiMode::Tokamak => {
                self.check_token_limit()?;
                let result = self.chat_tokamak(messages, ctx_ref).await?;
                // Rough token estimate: ~4 chars per token
                let estimated_tokens = (result.len() as u32) / 4;
                self.add_token_usage(estimated_tokens);
                Ok(result)
            }
            AiMode::Custom => {
                let config = self.get_config();
                if config.api_key.is_empty() {
                    return Err("API key not configured. Please enter your API key in Settings.".to_string());
                }
                match config.provider.as_str() {
                    "claude" => self.chat_claude(&config, messages, ctx_ref).await,
                    "gpt" | "gemini" => {
                        self.chat_openai_compat(&config, messages, ctx_ref).await
                    }
                    _ => Err(format!("Unsupported provider: {}", config.provider)),
                }
            }
        }
    }

    // ---- Tokamak AI (no API key, device_id auth) ----

    async fn chat_tokamak(
        &self,
        messages: Vec<ChatMessage>,
        context_json: Option<&str>,
    ) -> Result<String, String> {
        let system_prompt = Self::build_system_prompt(context_json);

        let mut api_messages = vec![serde_json::json!({
            "role": "system",
            "content": system_prompt
        })];
        for m in &messages {
            api_messages.push(serde_json::json!({
                "role": m.role,
                "content": m.content
            }));
        }

        let body = serde_json::json!({
            "model": "tokamak-default",
            "messages": api_messages,
            "max_tokens": 4096
        });

        let response = self
            .client
            .post(format!("{TOKAMAK_AI_BASE_URL}/v1/chat/completions"))
            .header("X-Device-Id", &self.device_id)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Tokamak AI request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_body = response.text().await.unwrap_or_default();
            return Err(format!("Tokamak AI error ({status}): {error_body}"));
        }

        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        // Update token usage from server response if available
        if let Some(usage) = result.get("usage") {
            if let Some(total) = usage["total_tokens"].as_u64() {
                // Server-reported tokens override our estimate; adjust by subtracting
                // the rough estimate we'll add in chat() and using real value
                let real_tokens = total as u32;
                let estimated = (result["choices"][0]["message"]["content"]
                    .as_str()
                    .map(|s| s.len())
                    .unwrap_or(0) as u32) / 4;
                if real_tokens > estimated {
                    self.add_token_usage(real_tokens - estimated);
                }
            }
        }

        result["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "No text found in response".to_string())
    }

    // ---- URL helpers ----

    fn models_url(provider: &str) -> String {
        match provider {
            "gemini" => "https://generativelanguage.googleapis.com/v1beta/openai/models".to_string(),
            _ => format!("{}/v1/models", Self::base_url(provider)),
        }
    }

    fn chat_url(provider: &str) -> String {
        match provider {
            "gemini" => "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions".to_string(),
            _ => format!("{}/v1/chat/completions", Self::base_url(provider)),
        }
    }

    fn base_url(provider: &str) -> &'static str {
        match provider {
            "gpt" => "https://api.openai.com",
            "claude" => "https://api.anthropic.com",
            _ => "https://api.openai.com",
        }
    }

    // ---- Custom provider chat methods ----

    async fn chat_openai_compat(
        &self,
        config: &AiConfig,
        messages: Vec<ChatMessage>,
        context_json: Option<&str>,
    ) -> Result<String, String> {
        let system_prompt = Self::build_system_prompt(context_json);

        let mut api_messages = vec![serde_json::json!({
            "role": "system",
            "content": system_prompt
        })];
        for m in &messages {
            api_messages.push(serde_json::json!({
                "role": m.role,
                "content": m.content
            }));
        }

        let body = if config.provider == "gpt" {
            serde_json::json!({
                "model": config.model,
                "messages": api_messages,
                "max_completion_tokens": 4096
            })
        } else {
            serde_json::json!({
                "model": config.model,
                "messages": api_messages,
                "max_tokens": 4096
            })
        };

        let chat_url = Self::chat_url(&config.provider);
        let response = self
            .client
            .post(&chat_url)
            .header("Authorization", format!("Bearer {}", config.api_key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("API request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_body = response.text().await.unwrap_or_default();
            return Err(format!("API error ({status}): {error_body}"));
        }

        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        result["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "No text found in response".to_string())
    }

    async fn chat_claude(
        &self,
        config: &AiConfig,
        messages: Vec<ChatMessage>,
        context_json: Option<&str>,
    ) -> Result<String, String> {
        let system_prompt = Self::build_system_prompt(context_json);

        let api_messages: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content
                })
            })
            .collect();

        let body = serde_json::json!({
            "model": config.model,
            "max_tokens": 4096,
            "system": system_prompt,
            "messages": api_messages
        });

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("API request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_body = response.text().await.unwrap_or_default();
            return Err(format!("Claude API error ({status}): {error_body}"));
        }

        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        result["content"][0]["text"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "No text found in response".to_string())
    }

    pub fn build_system_prompt(context_json: Option<&str>) -> String {
        let mut prompt = r#"You are "Appchain Pilot", an AI assistant built into the Tokamak Appchain Desktop App.

## Your Role
- Guide users through the Tokamak Appchain desktop application
- Help create, manage, and troubleshoot L2 appchains
- Answer questions about Tokamak Network, ethrex, and L2 operations

## App Features You Can Help With
1. **Home** - Quick start, appchain creation shortcuts
2. **My Appchains** - Create/manage L2 appchains (local, testnet, mainnet)
3. **Appchain Pilot (this chat)** - AI-powered guidance
4. **Open Appchain** - Browse and connect to public appchains
5. **Dashboard** - Monitor L1/L2 node status
6. **Tokamak Wallet** - Manage TON tokens, bridge L1<>L2
7. **Program Store** - Browse available programs
8. **Settings** - AI provider, Platform account, node config

## Appchain Creation Flow
- **Local mode**: One-click setup, runs `ethrex l2 --dev` locally
- **Testnet mode**: Connects to Sepolia L1
- **Mainnet mode**: Deploys on Ethereum mainnet
- Native token is always TON (TOKAMAK)
- Prover type is always SP1

## Technical Context
- Built on ethrex (Ethereum L2 client by Tokamak Network)
- Tauri 2.x desktop app (Rust backend + React frontend)
- Supports L1 node, L2 sequencer, prover management

## Actions
When it is appropriate to suggest an action the user can take in the app, include an action block in your response using this exact format:

[ACTION:action_name:param1=value1,param2=value2]

Available actions:
- `[ACTION:navigate:view=home]` - Navigate to a view (home, myl2, chat, nodes, dashboard, openl2, wallet, store, settings)
- `[ACTION:create_appchain:network=local]` - Start creating a new appchain (network: local, testnet, mainnet)
- `[ACTION:stop_appchain:id=CHAIN_ID]` - Stop a running appchain
- `[ACTION:open_appchain:id=CHAIN_ID]` - View appchain details

Only include actions when they directly help the user accomplish their request. Multiple actions can be included.

## Guidelines
- Respond in the same language the user uses (Korean or English)
- Be concise and practical
- If the user asks to perform an action, include the relevant ACTION block so they can execute it with one click
- If something isn't implemented yet, honestly say so and suggest alternatives"#
            .to_string();

        if let Some(ctx) = context_json {
            prompt.push_str("\n\n## Current App State\n```json\n");
            prompt.push_str(ctx);
            prompt.push_str("\n```");
        }

        prompt
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}
