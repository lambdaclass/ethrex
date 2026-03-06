use crate::ai_provider::{AiConfig, AiProvider, ChatMessage};
use crate::appchain_manager::{
    AppchainConfig, AppchainManager, AppchainStatus, NetworkMode, SetupProgress, StepStatus,
};
use crate::local_server::LocalServer;
use crate::process_manager::{NodeInfo, ProcessManager, ProcessStatus};
use crate::runner::ProcessRunner;
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
) -> Result<(), String> {
    let config = am
        .get_appchain(&id)
        .ok_or(format!("Appchain not found: {id}"))?;

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
            let chain_id = id.clone();

            // Spawn the actual process in background
            tokio::spawn(async move {
                ProcessRunner::start_local_dev(runner_clone, am_clone, chain_id).await;
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
) -> Result<(), String> {
    runner.stop_chain(&id).await?;
    am.update_status(&id, AppchainStatus::Stopped);
    am.add_log(&id, "Appchain stopped by user.".to_string());
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

/// Returns current app state as context for AI chat
#[tauri::command]
pub fn get_chat_context(am: State<Arc<AppchainManager>>) -> serde_json::Value {
    let chains = am.list_appchains();
    let chain_summaries: Vec<serde_json::Value> = chains
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id,
                "name": c.name,
                "chain_id": c.chain_id,
                "status": format!("{:?}", c.status),
                "network_mode": format!("{:?}", c.network_mode),
                "rpc_port": c.l2_rpc_port,
                "is_public": c.is_public,
                "native_token": c.native_token,
            })
        })
        .collect();

    serde_json::json!({
        "appchains": chain_summaries,
        "total_count": chains.len(),
    })
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
) -> Result<(), String> {
    // Ensure server is running
    if !server.is_running().await {
        server.start().await?;
        // Wait briefly for server to be ready
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }

    let url = format!("http://127.0.0.1:{}", server.port());
    open::that(&url).map_err(|e| format!("Failed to open browser: {e}"))
}

// ============================================================================
// Platform Auth (token stored in OS Keychain)
// ============================================================================

const KEYRING_SERVICE_PLATFORM: &str = "tokamak-appchain";
const KEYRING_PLATFORM_TOKEN: &str = "platform-token";

#[tauri::command]
pub fn save_platform_token(token: String) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE_PLATFORM, KEYRING_PLATFORM_TOKEN)
        .map_err(|e| format!("Keyring error: {e}"))?;
    entry
        .set_password(&token)
        .map_err(|e| format!("Failed to save token: {e}"))
}

#[tauri::command]
pub fn get_platform_token() -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE_PLATFORM, KEYRING_PLATFORM_TOKEN)
        .map_err(|e| format!("Keyring error: {e}"))?;
    match entry.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Failed to get token: {e}")),
    }
}

#[tauri::command]
pub fn delete_platform_token() -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE_PLATFORM, KEYRING_PLATFORM_TOKEN)
        .map_err(|e| format!("Keyring error: {e}"))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("Failed to delete token: {e}")),
    }
}
