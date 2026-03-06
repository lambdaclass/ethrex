use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Duration;

const KEYRING_SERVICE: &str = "tokamak-appchain";
const KEYRING_API_KEY: &str = "ai-api-key";
const KEYRING_AI_CONFIG: &str = "ai-config";

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

pub struct AiProvider {
    config: Mutex<AiConfig>,
    client: Client,
}

impl AiProvider {
    pub fn new() -> Self {
        let mut config = Self::load_config_meta().unwrap_or_default();
        config.api_key = Self::load_api_key().unwrap_or_default();
        Self {
            config: Mutex::new(config),
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

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
        // Delete API key from keychain
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_API_KEY) {
            let _ = entry.delete_credential();
        }
        // Delete config from keychain
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_AI_CONFIG) {
            let _ = entry.delete_credential();
        }
        *self.config.lock().unwrap() = AiConfig::default();
        Ok(())
    }

    /// Fetch available models from provider API
    pub async fn fetch_models(&self, provider: &str, api_key: &str) -> Result<Vec<String>, String> {
        let url = Self::models_url(provider);
        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .send()
            .await
            .map_err(|e| format!("모델 목록 조회 실패: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("API 에러 ({status}): {body}"));
        }

        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("응답 파싱 실패: {e}"))?;

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

    pub async fn chat(&self, messages: Vec<ChatMessage>) -> Result<String, String> {
        let config = self.get_config();
        if config.api_key.is_empty() {
            return Err(
                "API 키가 설정되지 않았습니다. 설정에서 API 키를 입력하세요.".to_string(),
            );
        }

        match config.provider.as_str() {
            "claude" => self.chat_claude(&config, messages).await,
            // Tokamak AI, GPT, Gemini all use OpenAI-compatible format
            "tokamak" | "gpt" | "gemini" => {
                self.chat_openai_compat(&config, messages).await
            }
            _ => Err(format!("지원하지 않는 프로바이더: {}", config.provider)),
        }
    }

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
            "tokamak" => "https://api.ai.tokamak.network",
            "gpt" => "https://api.openai.com",
            "claude" => "https://api.anthropic.com",
            _ => "https://api.ai.tokamak.network",
        }
    }

    async fn chat_openai_compat(
        &self,
        config: &AiConfig,
        messages: Vec<ChatMessage>,
    ) -> Result<String, String> {
        let system_prompt = Self::build_system_prompt();

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
            .map_err(|e| format!("API 요청 실패: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_body = response.text().await.unwrap_or_default();
            return Err(format!("API 에러 ({status}): {error_body}"));
        }

        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("응답 파싱 실패: {e}"))?;

        result["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "응답에서 텍스트를 찾을 수 없습니다".to_string())
    }

    async fn chat_claude(
        &self,
        config: &AiConfig,
        messages: Vec<ChatMessage>,
    ) -> Result<String, String> {
        let system_prompt = Self::build_system_prompt();

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
            .map_err(|e| format!("API 요청 실패: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_body = response.text().await.unwrap_or_default();
            return Err(format!("Claude API 에러 ({status}): {error_body}"));
        }

        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("응답 파싱 실패: {e}"))?;

        result["content"][0]["text"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "응답에서 텍스트를 찾을 수 없습니다".to_string())
    }

    fn build_system_prompt() -> String {
        r#"You are "Appchain Pilot", an AI assistant built into the Tokamak Appchain Desktop App.

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
6. **Tokamak Wallet** - Manage TON tokens, bridge L1↔L2
7. **Settings** - AI provider, node config, theme, language

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

## Guidelines
- Respond in the same language the user uses (Korean or English)
- Be concise and practical
- If the user asks to perform an action (create appchain, start node, etc.), guide them step by step
- If something isn't implemented yet, honestly say so and suggest alternatives"#
            .to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}
