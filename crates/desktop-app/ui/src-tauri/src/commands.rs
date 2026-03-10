use crate::ai_provider::{AiConfig, AiMode, AiProvider, ChatMessage, TokenUsage};
use crate::appchain_manager::{
    AppchainConfig, AppchainManager, AppchainStatus, NetworkMode, SetupProgress, StepStatus,
};
use crate::deployment_db::{self, ContainerInfo, DeploymentProxy, DeploymentRow};
use crate::local_server::LocalServer;
use crate::process_manager::{NodeInfo, ProcessManager, ProcessStatus};
use crate::runner::ProcessRunner;
use crate::telegram_bot::TelegramBotManager;
use crate::unified_state::{L2Info, UnifiedL2State};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;

// ============================================================================
// AI Config
// ============================================================================

#[tauri::command]
pub fn get_ai_config(ai: State<Arc<AiProvider>>) -> AiConfig {
    ai.get_config_masked()
}

#[tauri::command]
pub fn has_ai_key(ai: State<Arc<AiProvider>>) -> bool {
    ai.has_api_key()
}

#[tauri::command]
pub fn save_ai_config(
    provider: String,
    api_key: String,
    model: String,
    ai: State<Arc<AiProvider>>,
) -> Result<(), String> {
    ai.save_config(AiConfig {
        provider,
        api_key,
        model,
    })
}

#[tauri::command]
pub async fn fetch_ai_models(
    provider: String,
    api_key: String,
    ai: State<'_, Arc<AiProvider>>,
) -> Result<Vec<String>, String> {
    let ai = ai.inner().clone();
    ai.fetch_models(&provider, &api_key).await
}

#[tauri::command]
pub fn disconnect_ai(ai: State<Arc<AiProvider>>) -> Result<(), String> {
    ai.clear_config()
}

#[tauri::command]
pub fn get_ai_mode(ai: State<Arc<AiProvider>>) -> AiMode {
    ai.get_mode()
}

#[tauri::command]
pub fn set_ai_mode(mode: AiMode, ai: State<Arc<AiProvider>>) -> Result<(), String> {
    ai.set_mode(mode)
}

#[tauri::command]
pub async fn get_token_usage(ai: State<'_, Arc<AiProvider>>) -> Result<TokenUsage, String> {
    let ai = ai.inner().clone();
    match ai.fetch_token_usage().await {
        Ok(usage) => Ok(usage),
        Err(e) if e == "login_required" => Err(e),
        Err(e) => {
            log::warn!("[AI] fetch_token_usage failed: {e}, using cache");
            Ok(ai.get_token_usage())
        }
    }
}

#[tauri::command]
pub async fn test_ai_connection(ai: State<'_, Arc<AiProvider>>) -> Result<String, String> {
    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: "Hi! Please respond with just 'Connected!' to confirm the connection works."
            .to_string(),
    }];
    let ai = ai.inner().clone();
    ai.chat(messages, None).await
}

// ============================================================================
// Chat
// ============================================================================

#[tauri::command]
pub async fn send_chat_message(
    messages: Vec<ChatMessage>,
    context: Option<String>,
    ai: State<'_, Arc<AiProvider>>,
) -> Result<ChatMessage, String> {
    let ai = ai.inner().clone();
    let content = ai.chat(messages, context).await?;
    Ok(ChatMessage {
        role: "assistant".to_string(),
        content,
    })
}

// ============================================================================
// Legacy Node Control
// ============================================================================

#[tauri::command]
pub fn get_all_status(pm: State<ProcessManager>) -> Vec<NodeInfo> {
    pm.get_all()
}

#[tauri::command]
pub fn start_node(name: String, pm: State<ProcessManager>) -> Result<String, String> {
    let info = pm
        .get_status(&name)
        .ok_or(format!("Unknown node: {name}"))?;
    if matches!(info.status, ProcessStatus::Running) {
        return Err(format!("{name} is already running"));
    }
    pm.set_status(&name, ProcessStatus::Running, Some(0));
    Ok(format!("{name} started"))
}

#[tauri::command]
pub fn stop_node(name: String, pm: State<ProcessManager>) -> Result<String, String> {
    let info = pm
        .get_status(&name)
        .ok_or(format!("Unknown node: {name}"))?;
    if matches!(info.status, ProcessStatus::Stopped) {
        return Err(format!("{name} is already stopped"));
    }
    pm.set_status(&name, ProcessStatus::Stopped, None);
    Ok(format!("{name} stopped"))
}

#[tauri::command]
pub fn get_node_status(name: String, pm: State<ProcessManager>) -> Result<NodeInfo, String> {
    pm.get_status(&name).ok_or(format!("Unknown node: {name}"))
}

#[tauri::command]
pub fn get_logs(name: String, _lines: Option<usize>) -> Result<Vec<String>, String> {
    Ok(vec![format!(
        "[{name}] No logs available yet - process management coming in Phase 1"
    )])
}

// ============================================================================
// Appchain Management
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateAppchainRequest {
    pub name: String,
    pub icon: String,
    pub chain_id: u64,
    pub description: String,
    pub network_mode: String,
    pub l1_rpc_url: String,
    pub l2_rpc_port: u16,
    pub sequencer_mode: String,
    pub native_token: String,
    pub prover_type: String,
    pub is_public: bool,
    pub hashtags: String,
}

#[tauri::command]
pub fn create_appchain(
    req: CreateAppchainRequest,
    am: State<Arc<AppchainManager>>,
) -> Result<AppchainConfig, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let network_mode = match req.network_mode.as_str() {
        "local" => NetworkMode::Local,
        "testnet" => NetworkMode::Testnet,
        "mainnet" => NetworkMode::Mainnet,
        _ => return Err(format!("Unknown network mode: {}", req.network_mode)),
    };

    let hashtags: Vec<String> = req
        .hashtags
        .split_whitespace()
        .map(|s| s.trim_start_matches('#').to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let config = AppchainConfig {
        id: id.clone(),
        name: req.name,
        icon: req.icon,
        chain_id: req.chain_id,
        description: req.description,
        network_mode,
        l1_rpc_url: req.l1_rpc_url,
        l2_rpc_port: req.l2_rpc_port,
        sequencer_mode: req.sequencer_mode,
        native_token: req.native_token,
        prover_type: req.prover_type,
        bridge_address: None,
        on_chain_proposer_address: None,
        is_public: req.is_public,
        hashtags,
        status: AppchainStatus::Created,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    am.create_appchain(config.clone())?;
    Ok(config)
}

#[tauri::command]
pub fn list_appchains(am: State<Arc<AppchainManager>>) -> Vec<AppchainConfig> {
    am.list_appchains()
}

#[tauri::command]
pub fn get_appchain(
    id: String,
    am: State<Arc<AppchainManager>>,
) -> Result<AppchainConfig, String> {
    am.get_appchain(&id)
        .ok_or(format!("Appchain not found: {id}"))
}

#[tauri::command]
pub fn delete_appchain(id: String, am: State<Arc<AppchainManager>>) -> Result<(), String> {
    am.delete_appchain(&id)
}

#[tauri::command]
pub async fn start_appchain_setup(
    id: String,
    am: State<'_, Arc<AppchainManager>>,
    runner: State<'_, Arc<ProcessRunner>>,
    tg_manager: State<'_, Arc<TelegramBotManager>>,
) -> Result<(), String> {
    let config = am
        .get_appchain(&id)
        .ok_or(format!("Appchain not found: {id}"))?;
    tg_manager.notify(&format!("🟡 앱체인 '{}' 생성을 시작합니다.", config.name));

    let has_prover = config.prover_type != "none";
    am.init_setup_progress(&id, &config.network_mode, has_prover);
    am.update_status(&id, AppchainStatus::SettingUp);

    // Step 1: Config - mark done immediately
    am.update_step_status(&id, "config", StepStatus::Done);
    am.add_log(&id, format!("Config saved for '{}'", config.name));
    am.advance_step(&id);

    match config.network_mode {
        NetworkMode::Local => {
            am.update_step_status(&id, "dev", StepStatus::InProgress);
            am.add_log(&id, "Starting ethrex l2 --dev ...".to_string());

            // Clone Arc handles for the background task
            let am_clone = am.inner().clone();
            let runner_clone = runner.inner().clone();
            let tg_clone = tg_manager.inner().clone();
            let chain_id = id.clone();
            let chain_name = config.name.clone();

            // Spawn the actual process in background
            tokio::spawn(async move {
                ProcessRunner::start_local_dev(runner_clone, am_clone.clone(), chain_id.clone()).await;
                // Notify after setup completes
                let status = am_clone.get_appchain(&chain_id)
                    .map(|c| format!("{:?}", c.status))
                    .unwrap_or_default();
                match status.as_str() {
                    "Running" => tg_clone.notify(&format!("🟢 앱체인 '{chain_name}' 이(가) 시작되었습니다.")),
                    "Error" => tg_clone.notify(&format!("❌ 앱체인 '{chain_name}' 생성 중 오류가 발생했습니다.")),
                    _ => {}
                }
            });
        }
        _ => {
            // Testnet/Mainnet - not yet implemented
            am.update_step_status(&id, "l1_check", StepStatus::InProgress);
            am.add_log(
                &id,
                format!(
                    "Checking L1 connection to {} ... (not yet implemented)",
                    config.l1_rpc_url
                ),
            );
        }
    }

    Ok(())
}

#[tauri::command]
pub fn get_setup_progress(
    id: String,
    am: State<Arc<AppchainManager>>,
) -> Result<SetupProgress, String> {
    am.get_setup_progress(&id)
        .ok_or(format!("No setup in progress for: {id}"))
}

#[tauri::command]
pub async fn stop_appchain(
    id: String,
    am: State<'_, Arc<AppchainManager>>,
    runner: State<'_, Arc<ProcessRunner>>,
    tg_manager: State<'_, Arc<TelegramBotManager>>,
) -> Result<(), String> {
    let config = am.get_appchain(&id)
        .ok_or_else(|| format!("Appchain with id '{id}' not found"))?;
    runner.stop_chain(&id).await?;
    am.update_status(&id, AppchainStatus::Stopped);
    am.add_log(&id, "Appchain stopped by user.".to_string());
    tg_manager.notify(&format!("🔴 앱체인 '{}' 이(가) 중지되었습니다.", config.name));
    Ok(())
}

#[tauri::command]
pub fn update_appchain_public(
    id: String,
    is_public: bool,
    am: State<Arc<AppchainManager>>,
) -> Result<(), String> {
    am.get_appchain(&id)
        .ok_or(format!("Appchain not found: {id}"))?;
    am.update_public(&id, is_public);
    Ok(())
}

/// Returns current app state as context for AI chat (unified: appchains + deployments)
#[tauri::command]
pub fn get_chat_context(state: State<Arc<UnifiedL2State>>) -> serde_json::Value {
    state.to_context_json()
}

/// Returns all L2 instances (appchains + deployments) as unified list
#[tauri::command]
pub fn get_all_l2(state: State<Arc<UnifiedL2State>>) -> Vec<L2Info> {
    state.get_all()
}

// ============================================================================
// Local Server (Deployment Engine)
// ============================================================================

#[derive(Debug, Serialize)]
pub struct LocalServerStatus {
    pub running: bool,
    pub healthy: bool,
    pub url: String,
    pub port: u16,
}

#[tauri::command]
pub async fn start_local_server(
    server: State<'_, Arc<LocalServer>>,
) -> Result<String, String> {
    server.start().await?;
    Ok(server.url())
}

#[tauri::command]
pub async fn stop_local_server(
    server: State<'_, Arc<LocalServer>>,
) -> Result<(), String> {
    server.stop().await
}

#[tauri::command]
pub async fn get_local_server_status(
    server: State<'_, Arc<LocalServer>>,
) -> Result<LocalServerStatus, String> {
    let running = server.is_running().await;
    let healthy = if running {
        server.health_check().await
    } else {
        false
    };
    Ok(LocalServerStatus {
        running,
        healthy,
        url: server.url(),
        port: server.port(),
    })
}

#[tauri::command]
pub async fn open_deployment_ui(
    server: State<'_, Arc<LocalServer>>,
) -> Result<String, String> {
    let url = format!("http://127.0.0.1:{}", server.port());

    // If port already has a healthy server (e.g. started separately), skip start
    if server.health_check().await {
        return Ok(url);
    }

    // Try to start if not running
    if !server.is_running().await {
        match server.start().await {
            Ok(_) => {
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            }
            Err(e) => {
                log::warn!("Failed to start local-server: {e}");
                // Even if start fails, port might already be in use by another server
                if server.health_check().await {
                    return Ok(url);
                }
                return Err(e);
            }
        }
    }

    Ok(url)
}

// ============================================================================
// Platform Auth (token stored in file — keychain unreliable in dev mode)
// ============================================================================

fn platform_token_path() -> Result<std::path::PathBuf, String> {
    let dir = dirs::data_dir()
        .ok_or_else(|| "Cannot find data directory".to_string())?
        .join("tokamak-appchain");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {e}"))?;
    Ok(dir.join("platform-token.json"))
}

#[tauri::command]
pub fn save_platform_token(token: String) -> Result<(), String> {
    let path = platform_token_path()?;
    let data = serde_json::json!({ "token": token });
    let content = serde_json::to_string_pretty(&data)
        .map_err(|e| format!("Failed to serialize token: {e}"))?;
    std::fs::write(&path, content)
        .map_err(|e| format!("Failed to save token: {e}"))
}

#[tauri::command]
pub fn get_platform_token(
    ai: State<'_, Arc<crate::ai_provider::AiProvider>>,
) -> Result<Option<String>, String> {
    Ok(ai.get_platform_token_value())
}

#[tauri::command]
pub fn delete_platform_token(
    ai: State<'_, Arc<crate::ai_provider::AiProvider>>,
) -> Result<(), String> {
    ai.clear_platform_token();
    let path = platform_token_path()?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("Failed to delete token: {e}"))?;
    }
    Ok(())
}

// Desktop login flow constants
const POLL_INTERVAL_SECS: u64 = 2;
const POLL_MAX_ATTEMPTS: u32 = 150; // 2s × 150 = 5 minutes

/// PKCE: generate code_verifier and code_challenge (SHA-256 hex)
fn generate_pkce() -> (String, String) {
    use sha2::{Digest, Sha256};
    let verifier: String = (0..64)
        .map(|_| format!("{:02x}", rand::random::<u8>()))
        .collect();
    let challenge = hex::encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

#[derive(Deserialize)]
struct DesktopCodeResponse {
    code: String,
}

#[derive(Deserialize)]
struct DesktopTokenResponse {
    status: Option<String>,
    token: Option<String>,
    error: Option<String>,
}

#[derive(Serialize)]
pub struct LoginStartResult {
    login_url: String,
    code: String,
    code_verifier: String,
}

/// Step 1: Generate PKCE code and return login URL (also tries to open browser)
#[tauri::command]
pub async fn start_platform_login(app: tauri::AppHandle) -> Result<LoginStartResult, String> {
    use tauri_plugin_shell::ShellExt;

    let client = reqwest::Client::new();
    let base_url = crate::ai_provider::PLATFORM_BASE_URL;

    let (code_verifier, code_challenge) = generate_pkce();

    let resp = client
        .post(format!("{base_url}/api/auth/desktop-code"))
        .json(&serde_json::json!({ "code_challenge": code_challenge }))
        .send()
        .await
        .map_err(|e| format!("Failed to request code: {e}"))?;

    let result: DesktopCodeResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {e}"))?;

    let code = result.code;
    let login_url = format!("{base_url}/login?desktop_code={code}");

    // Try to open browser (best-effort)
    #[allow(deprecated)]
    let _ = app.shell().open(&login_url, None);

    Ok(LoginStartResult {
        login_url,
        code,
        code_verifier,
    })
}

/// Step 2: Poll for token after user logs in via browser
#[tauri::command]
pub async fn poll_platform_login(
    code: String,
    code_verifier: String,
    ai: State<'_, Arc<AiProvider>>,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let base_url = crate::ai_provider::PLATFORM_BASE_URL;

    for _ in 0..POLL_MAX_ATTEMPTS {
        tokio::time::sleep(tokio::time::Duration::from_secs(POLL_INTERVAL_SECS)).await;

        let url = format!("{base_url}/api/auth/desktop-token?code={code}&code_verifier={code_verifier}");
        let poll_resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(_) => continue,
        };

        let body = match poll_resp.text().await {
            Ok(b) => b,
            Err(_) => continue,
        };

        let poll_result: DesktopTokenResponse = match serde_json::from_str(&body) {
            Ok(r) => r,
            Err(_) => continue,
        };

        if poll_result.status.as_deref() == Some("ready") {
            if let Some(token) = poll_result.token {
                save_platform_token(token.clone())?;
                ai.set_platform_token(token.clone());
                return Ok(token);
            }
        }

        if poll_result
            .error
            .as_deref()
            .is_some_and(|e| e == "code_expired" || e == "invalid_code")
        {
            return Err("login_timeout".to_string());
        }
    }

    Err("login_timeout".to_string())
}

/// Fetch current user info from Platform API using stored token
#[tauri::command]
pub async fn get_platform_user(
    ai: State<'_, Arc<crate::ai_provider::AiProvider>>,
) -> Result<serde_json::Value, String> {
    let token = ai.get_platform_token_value()
        .ok_or_else(|| "no_token".to_string())?;
    fetch_platform_me(&token).await
}

async fn fetch_platform_me(token: &str) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let base_url = crate::ai_provider::PLATFORM_BASE_URL;

    let resp = client
        .get(format!("{base_url}/api/auth/me"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch user: {e}"))?;

    if !resp.status().is_success() {
        return Err("auth_failed".to_string());
    }

    resp.json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Failed to parse user: {e}"))
}

// ============================================================================
// Telegram Bot Config (stored in data dir)
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub allowed_chat_ids: String,
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub system_alerts_enabled: bool,
}

fn default_true() -> bool {
    true
}

pub(crate) fn telegram_config_path() -> Result<std::path::PathBuf, String> {
    let dir = dirs::data_dir()
        .ok_or("Cannot find app data directory")?
        .join("tokamak-appchain");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {e}"))?;
    Ok(dir.join("telegram.json"))
}

fn mask_token(token: &str) -> String {
    if token.is_empty() { return String::new(); }
    if token.len() < 12 { return "***".to_string(); }
    format!("{}...{}", &token[..8], &token[token.len()-4..])
}

fn write_telegram_config(config: &TelegramConfig) -> Result<(), String> {
    let json = serde_json::to_string_pretty(config).map_err(|e| format!("Serialize error: {e}"))?;
    let path = telegram_config_path()?;
    std::fs::write(&path, &json).map_err(|e| format!("Failed to save: {e}"))?;
    Ok(())
}

/// Internal helper: reads telegram config from disk without masking.
pub(crate) fn read_telegram_config() -> Result<TelegramConfig, String> {
    let path = telegram_config_path()?;
    match std::fs::read_to_string(&path) {
        Ok(json) => {
            log::info!("[TG] loaded config from {}", path.display());
            serde_json::from_str(&json).map_err(|e| format!("Parse error: {e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::info!("[TG] no config file yet");
            Ok(TelegramConfig {
                bot_token: String::new(),
                allowed_chat_ids: String::new(),
                enabled: false,
                system_alerts_enabled: true,
            })
        }
        Err(e) => Err(format!("Failed to read config: {e}")),
    }
}

#[tauri::command]
pub fn get_telegram_config() -> Result<TelegramConfig, String> {
    let mut config = read_telegram_config()?;
    // Mask token before returning to frontend
    config.bot_token = mask_token(&config.bot_token);
    Ok(config)
}

#[tauri::command]
pub fn save_telegram_config(
    bot_token: String,
    allowed_chat_ids: String,
    tg_manager: State<Arc<TelegramBotManager>>,
) -> Result<(), String> {
    log::info!("[TG] save_telegram_config: token={}, ids={}", if bot_token == "__keep__" { "__keep__" } else { "***" }, allowed_chat_ids);
    let existing = read_telegram_config()?;

    let final_token = if bot_token == "__keep__" {
        existing.bot_token
    } else {
        bot_token
    };

    let config = TelegramConfig {
        bot_token: final_token,
        allowed_chat_ids,
        enabled: existing.enabled,
        system_alerts_enabled: existing.system_alerts_enabled,
    };
    write_telegram_config(&config)?;
    log::info!("[TG] config saved");

    // Restart bot if running (to pick up new token/chat IDs)
    if existing.enabled && tg_manager.is_running() {
        tg_manager.stop();
        if let Err(e) = tg_manager.start() {
            log::warn!("[TG] bot failed to restart with new config: {e}");
        } else {
            log::info!("[TG] bot restarted with new config");
        }
    }

    Ok(())
}

#[tauri::command]
pub fn toggle_telegram_bot(
    enabled: bool,
    tg_manager: State<Arc<TelegramBotManager>>,
) -> Result<bool, String> {
    // Update enabled in config file
    let mut config = read_telegram_config()?;
    config.enabled = enabled;
    write_telegram_config(&config)?;

    if enabled {
        match tg_manager.start() {
            Ok(()) => {
                log::info!("[TG] bot started via toggle");
                Ok(true)
            }
            Err(e) => {
                log::warn!("[TG] failed to start bot: {e}");
                // Revert enabled
                config.enabled = false;
                write_telegram_config(&config)?;
                Err(e)
            }
        }
    } else {
        tg_manager.stop();
        log::info!("[TG] bot stopped via toggle");
        Ok(false)
    }
}

#[tauri::command]
pub fn send_telegram_notification(
    message: String,
    tg_manager: State<Arc<TelegramBotManager>>,
) -> Result<(), String> {
    tg_manager.notify(&message);
    Ok(())
}

#[tauri::command]
pub fn get_telegram_bot_status(
    tg_manager: State<Arc<TelegramBotManager>>,
) -> bool {
    tg_manager.is_running()
}

#[tauri::command]
pub fn toggle_system_alerts(
    enabled: bool,
    tg_manager: State<Arc<TelegramBotManager>>,
) -> Result<bool, String> {
    let mut config = read_telegram_config()?;
    config.system_alerts_enabled = enabled;
    write_telegram_config(&config)?;

    tg_manager.set_system_alerts_enabled(enabled);
    log::info!("[TG] system alerts {}", if enabled { "enabled" } else { "disabled" });
    Ok(enabled)
}

// ============================================================================
// Deployment DB (read-only) + Docker lifecycle (proxied to local-server)
// ============================================================================

#[tauri::command]
pub fn list_docker_deployments() -> Result<Vec<DeploymentRow>, String> {
    deployment_db::list_deployments_from_db()
}

#[tauri::command]
pub async fn delete_docker_deployment(
    id: String,
    server: State<'_, Arc<LocalServer>>,
) -> Result<(), String> {
    let proxy = DeploymentProxy::new(&server.url());
    proxy.destroy_deployment(&id).await
}

#[tauri::command]
pub async fn stop_docker_deployment(
    id: String,
    server: State<'_, Arc<LocalServer>>,
) -> Result<(), String> {
    let proxy = DeploymentProxy::new(&server.url());
    proxy.stop_deployment(&id).await
}

#[tauri::command]
pub async fn start_docker_deployment(
    id: String,
    server: State<'_, Arc<LocalServer>>,
) -> Result<(), String> {
    let proxy = DeploymentProxy::new(&server.url());
    proxy.start_deployment(&id).await
}

#[tauri::command]
pub async fn get_docker_containers(
    id: String,
    server: State<'_, Arc<LocalServer>>,
) -> Result<Vec<ContainerInfo>, String> {
    let proxy = DeploymentProxy::new(&server.url());
    proxy.get_containers(&id).await
}
