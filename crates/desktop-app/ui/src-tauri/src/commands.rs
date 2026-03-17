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
    pub stack_type: Option<String>,
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
        stack_type: req.stack_type.unwrap_or_else(|| "ethrex".to_string()),
        l1_rpc_url: req.l1_rpc_url,
        l2_rpc_port: req.l2_rpc_port,
        sequencer_mode: req.sequencer_mode,
        native_token: req.native_token,
        prover_type: req.prover_type,
        bridge_address: None,
        on_chain_proposer_address: None,
        is_public: req.is_public,
        platform_deployment_id: None,
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
    am.init_setup_progress(&id, &config.network_mode, has_prover, &config.stack_type);
    am.update_status(&id, AppchainStatus::SettingUp);

    // Step 1: Config - mark done immediately
    am.update_step_status(&id, "config", StepStatus::Done);
    am.add_log(&id, format!("Config saved for '{}'", config.name));
    am.advance_step(&id);

    if config.stack_type == "thanos" {
        // Thanos stack: all modes use local-server Docker pipeline
        am.update_step_status(&id, "pulling", StepStatus::InProgress);
        am.add_log(&id, "Thanos (OP Stack) deployment — delegating to local-server Docker pipeline...".to_string());
        // The actual provisioning is triggered from the deployment manager UI
        // via local-server POST /api/deployments/:id/provision
    } else {
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
    platform_deployment_id: Option<String>,
    am: State<Arc<AppchainManager>>,
) -> Result<(), String> {
    am.get_appchain(&id)
        .ok_or(format!("Appchain not found: {id}"))?;
    am.update_public(&id, is_public, platform_deployment_id);
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
    server.resume_watchdog();
    server.start().await?;
    Ok(server.url())
}

#[tauri::command]
pub async fn stop_local_server(
    server: State<'_, Arc<LocalServer>>,
) -> Result<(), String> {
    server.pause_watchdog();
    server.stop().await
}

#[tauri::command]
pub async fn get_local_server_status(
    server: State<'_, Arc<LocalServer>>,
) -> Result<LocalServerStatus, String> {
    let has_process = server.is_running().await;
    let healthy = server.health_check().await;
    // Server is "running" if we have a child process OR if health check passes
    // (the server might have been started externally or before Tauri spawned it)
    let running = has_process || healthy;
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

/// Internal helper: writes telegram config to disk.
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

// ============================================================================
// Generic Keychain value storage — cross-platform.
// macOS: uses `security` CLI for Keychain (compatible with Node.js keychain.js)
// Windows/Linux: uses `keyring` crate (Windows Credential Manager / Secret Service)
// ============================================================================

const KEYRING_SERVICE: &str = "tokamak-appchain";

/// Allowed key prefixes for frontend access (security boundary)
const ALLOWED_KEY_PREFIXES: &[&str] = &["pinata_", "deployer_pk_", "ai-"];

fn validate_keychain_key(key: &str) -> Result<(), String> {
    if ALLOWED_KEY_PREFIXES.iter().any(|prefix| key.starts_with(prefix)) {
        Ok(())
    } else {
        Err(format!(
            "Key '{}' is not allowed. Must start with one of: {}",
            key,
            ALLOWED_KEY_PREFIXES.join(", ")
        ))
    }
}

// --- macOS: `security` CLI ---
#[cfg(target_os = "macos")]
fn keychain_get(account: &str) -> Result<Option<String>, String> {
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-a", account, "-s", KEYRING_SERVICE, "-w"])
        .output()
        .map_err(|e| format!("Failed to run security CLI: {e}"))?;
    if output.status.success() {
        Ok(Some(String::from_utf8_lossy(&output.stdout).trim().to_string()))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("could not be found") || stderr.contains("SecKeychainSearchCopyNext") {
            Ok(None)
        } else {
            Err(format!("Keychain error: {}", stderr.trim()))
        }
    }
}

#[cfg(target_os = "macos")]
fn keychain_set(account: &str, secret: &str) -> Result<(), String> {
    let _ = std::process::Command::new("security")
        .args(["delete-generic-password", "-a", account, "-s", KEYRING_SERVICE])
        .output();
    let output = std::process::Command::new("security")
        .args(["add-generic-password", "-a", account, "-s", KEYRING_SERVICE, "-w", secret])
        .output()
        .map_err(|e| format!("Failed to run security CLI: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("Failed to save keychain value: {}", String::from_utf8_lossy(&output.stderr).trim()))
    }
}

#[cfg(target_os = "macos")]
fn keychain_delete(account: &str) -> Result<(), String> {
    let output = std::process::Command::new("security")
        .args(["delete-generic-password", "-a", account, "-s", KEYRING_SERVICE])
        .output()
        .map_err(|e| format!("Failed to run security CLI: {e}"))?;
    if output.status.success() || String::from_utf8_lossy(&output.stderr).contains("could not be found") {
        Ok(())
    } else {
        Err(format!("Failed to delete keychain value: {}", String::from_utf8_lossy(&output.stderr).trim()))
    }
}

// --- Windows/Linux: `keyring` crate ---
#[cfg(not(target_os = "macos"))]
fn keychain_get(account: &str) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account)
        .map_err(|e| format!("Keyring error: {e}"))?;
    match entry.get_password() {
        Ok(pw) => Ok(Some(pw)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Keyring error: {e}")),
    }
}

#[cfg(not(target_os = "macos"))]
fn keychain_set(account: &str, secret: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account)
        .map_err(|e| format!("Keyring error: {e}"))?;
    entry.set_password(secret).map_err(|e| format!("Keyring error: {e}"))
}

#[cfg(not(target_os = "macos"))]
fn keychain_delete(account: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account)
        .map_err(|e| format!("Keyring error: {e}"))?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("Keyring error: {e}")),
    }
}

#[tauri::command]
pub fn get_keychain_value(key: String) -> Result<Option<String>, String> {
    validate_keychain_key(&key)?;
    keychain_get(&key)
}

#[tauri::command]
pub fn save_keychain_value(key: String, value: String) -> Result<(), String> {
    validate_keychain_key(&key)?;
    keychain_set(&key, &value)
}

#[tauri::command]
pub fn delete_keychain_value(key: String) -> Result<(), String> {
    validate_keychain_key(&key)?;
    keychain_delete(&key)
}

// ============================================================================
// L1 On-chain Metadata (Phase 2)
// ============================================================================

/// Prepare setMetadataURI calldata for the OnChainProposer L1 contract.
/// Returns JSON with `to`, `data`, and `chainId` for the frontend to sign via browser wallet.
/// Full on-chain signing (ethers-rs) is planned for a future iteration.
#[tauri::command]
pub async fn set_metadata_uri(
    l1_rpc_url: String,
    proposer_address: String,
    metadata_uri: String,
    keychain_key: String,
) -> Result<String, String> {
    // Validate inputs
    if !proposer_address.starts_with("0x") || proposer_address.len() != 42 {
        return Err("Invalid proposer address: must be 0x-prefixed 40-char hex".to_string());
    }
    if l1_rpc_url.is_empty() {
        return Err("L1 RPC URL is required".to_string());
    }
    if metadata_uri.is_empty() {
        return Err("Metadata URI is required".to_string());
    }

    // Verify deployer key exists in keychain
    keychain_get(&keychain_key)?
        .ok_or_else(|| format!("No deployer key found for '{keychain_key}'"))?;

    // Build setMetadataURI calldata
    // Function selector: keccak256("setMetadataURI(string)")[:4] = 0x750c5d86
    let func_selector: [u8; 4] = [0x75, 0x0c, 0x5d, 0x86];

    // ABI encode: offset (32 bytes) + length (32 bytes) + data (padded to 32 bytes)
    let uri_bytes = metadata_uri.as_bytes();
    let uri_len = uri_bytes.len();
    let padded_len = ((uri_len + 31) / 32) * 32;

    let mut calldata = Vec::with_capacity(4 + 32 + 32 + padded_len);
    calldata.extend_from_slice(&func_selector);
    // offset: 0x20 (32) — points to start of string data
    let mut offset = [0u8; 32];
    offset[31] = 32;
    calldata.extend_from_slice(&offset);
    // length of string
    let mut length = [0u8; 32];
    length[24..32].copy_from_slice(&(uri_len as u64).to_be_bytes());
    calldata.extend_from_slice(&length);
    // string data (padded to 32-byte boundary)
    calldata.extend_from_slice(uri_bytes);
    calldata.resize(calldata.len() + padded_len - uri_len, 0);

    // Fetch chain ID from L1 RPC for the transaction metadata
    let client = reqwest::Client::new();
    let chain_id_resp = client
        .post(&l1_rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "eth_chainId", "params": []
        }))
        .send()
        .await
        .map_err(|e| format!("RPC error: {e}"))?;
    let chain_id_data: serde_json::Value = chain_id_resp
        .json()
        .await
        .map_err(|e| format!("Parse error: {e}"))?;
    let chain_id_hex = chain_id_data["result"]
        .as_str()
        .ok_or("No chain_id result")?;
    let _chain_id = u64::from_str_radix(chain_id_hex.trim_start_matches("0x"), 16)
        .map_err(|e| format!("Bad chain_id: {e}"))?;

    // For now, return the calldata hex — full signing requires ethers-rs integration
    // which is a larger dependency. The UI can use this with a browser wallet instead.
    let calldata_hex = format!("0x{}", hex::encode(&calldata));

    Ok(serde_json::json!({
        "to": proposer_address,
        "data": calldata_hex,
        "chainId": chain_id_hex,
        "note": "Transaction calldata prepared. Use wallet to sign and send."
    })
    .to_string())
}

// ============================================================================
// Appchain Metadata Signing (EIP-191 personal_sign with k256)
// ============================================================================

/// Sign appchain metadata for submission to the metadata repository.
/// Uses EIP-191 personal_sign with the deployer key from OS Keychain.
#[tauri::command]
pub fn sign_appchain_metadata(
    l1_chain_id: u64,
    l2_chain_id: u64,
    stack_type: String,
    operation: String,
    identity_contract: String,
    timestamp: u64,
    keychain_key: String,
) -> Result<serde_json::Value, String> {
    use k256::ecdsa::{SigningKey, VerifyingKey};
    use sha3::{Digest, Keccak256};

    // Validate inputs
    if !identity_contract.starts_with("0x") || identity_contract.len() != 42 {
        return Err("Invalid identity contract address".to_string());
    }
    if operation != "register" && operation != "update" {
        return Err("Operation must be 'register' or 'update'".to_string());
    }

    // Load deployer private key from keychain
    let pk_hex = keychain_get(&keychain_key)?
        .ok_or_else(|| format!("No deployer key found for '{keychain_key}'"))?;

    // Parse private key (strip 0x prefix if present)
    let pk_bytes_hex = pk_hex.trim_start_matches("0x");
    let pk_bytes =
        hex::decode(pk_bytes_hex).map_err(|e| format!("Invalid private key hex: {e}"))?;
    let signing_key =
        SigningKey::from_bytes(pk_bytes.as_slice().into()).map_err(|e| format!("Invalid private key: {e}"))?;

    // Build signing message (must match signature-validator.ts format exactly)
    let contract = identity_contract.to_lowercase();
    let message = [
        "Tokamak Appchain Registry",
        &format!("L1 Chain ID: {l1_chain_id}"),
        &format!("L2 Chain ID: {l2_chain_id}"),
        &format!("Stack: {stack_type}"),
        &format!("Operation: {operation}"),
        &format!("Contract: {contract}"),
        &format!("Timestamp: {timestamp}"),
    ].join("\n");

    // EIP-191 personal_sign: keccak256("\x19Ethereum Signed Message:\n" + len + message)
    let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
    let mut hasher = Keccak256::new();
    hasher.update(prefix.as_bytes());
    hasher.update(message.as_bytes());
    let hash = hasher.finalize();

    // Sign with recoverable signature
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(&hash)
        .map_err(|e| format!("Signing failed: {e}"))?;

    // Encode as 0x{r}{s}{v} where v = recovery_id + 27
    let sig_bytes = signature.to_bytes();
    let v = recovery_id.to_byte() + 27;
    let mut full_sig = [0u8; 65];
    full_sig[..64].copy_from_slice(&sig_bytes);
    full_sig[64] = v;
    let signature_hex = format!("0x{}", hex::encode(full_sig));

    // Derive signer address from public key
    let verifying_key = VerifyingKey::from(&signing_key);
    let pubkey_bytes = verifying_key.to_encoded_point(false);
    let pubkey_uncompressed = &pubkey_bytes.as_bytes()[1..]; // skip 0x04 prefix
    let mut addr_hasher = Keccak256::new();
    addr_hasher.update(pubkey_uncompressed);
    let addr_hash = addr_hasher.finalize();
    let signer_address = format!("0x{}", hex::encode(&addr_hash[12..]));

    Ok(serde_json::json!({
        "signature": signature_hex,
        "signerAddress": signer_address,
    }))
}

/// Internal signing helper (no keychain dependency, for testing).
#[cfg(test)]
pub fn sign_metadata_with_key(
    pk_hex: &str,
    l1_chain_id: u64,
    l2_chain_id: u64,
    stack_type: &str,
    operation: &str,
    identity_contract: &str,
    timestamp: u64,
) -> (String, String) {
    use k256::ecdsa::{SigningKey, VerifyingKey};
    use sha3::{Digest, Keccak256};

    let pk_bytes = hex::decode(pk_hex.trim_start_matches("0x")).unwrap();
    let signing_key = SigningKey::from_bytes(pk_bytes.as_slice().into()).unwrap();

    let message = format!(
        "Tokamak Appchain Registry\n\
         L1 Chain ID: {l1_chain_id}\n\
         L2 Chain ID: {l2_chain_id}\n\
         Stack: {stack_type}\n\
         Operation: {operation}\n\
         Contract: {contract}\n\
         Timestamp: {timestamp}",
        contract = identity_contract.to_lowercase(),
    );

    let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
    let mut hasher = Keccak256::new();
    hasher.update(prefix.as_bytes());
    hasher.update(message.as_bytes());
    let hash = hasher.finalize();

    let (signature, recovery_id) = signing_key.sign_prehash_recoverable(&hash).unwrap();
    let sig_bytes = signature.to_bytes();
    let v = recovery_id.to_byte() + 27;
    let mut full_sig = [0u8; 65];
    full_sig[..64].copy_from_slice(&sig_bytes);
    full_sig[64] = v;
    let signature_hex = format!("0x{}", hex::encode(full_sig));

    let verifying_key = VerifyingKey::from(&signing_key);
    let pubkey_bytes = verifying_key.to_encoded_point(false);
    let pubkey_uncompressed = &pubkey_bytes.as_bytes()[1..];
    let mut addr_hasher = Keccak256::new();
    addr_hasher.update(pubkey_uncompressed);
    let addr_hash = addr_hasher.finalize();
    let signer_address = format!("0x{}", hex::encode(&addr_hash[12..]));

    (signature_hex, signer_address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_metadata_matches_ethers() {
        // Hardhat account #0
        let pk = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let expected_address = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";
        // ethers.js produced this signature for the same message:
        let expected_sig = "0x04661a5da29005215bc53f225d2b6e98a5edfcfba30c8c5f264e9ebd29fea77c7c036b25079e6137252f4d8af34876efc307d57871aed9d355df09cc59bce0171c";

        let (sig, addr) = sign_metadata_with_key(
            pk,
            11155111,
            12345,
            "tokamak-appchain",
            "register",
            "0x1234567890123456789012345678901234567890",
            1710000000,
        );

        assert_eq!(addr, expected_address, "Address mismatch");
        assert_eq!(sig, expected_sig, "Signature mismatch");
    }
}
