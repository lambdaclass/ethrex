use crate::appchain_manager::{
    AppchainConfig, AppchainManager, AppchainStatus, NetworkMode, SetupProgress, StepStatus,
};
use crate::process_manager::{NodeInfo, ProcessManager, ProcessStatus};
use serde::{Deserialize, Serialize};
use tauri::State;

// ============================================================================
// Chat
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[tauri::command]
pub async fn send_chat_message(message: String) -> Result<ChatMessage, String> {
    Ok(ChatMessage {
        role: "assistant".to_string(),
        content: format!(
            "AI 연결이 아직 구현되지 않았습니다. (Phase 3에서 구현 예정)\n\n\
             받은 메시지: \"{message}\"\n\n\
             현재 사용 가능한 기능:\n\
             - 노드 제어 패널에서 L1/L2 노드를 시작/중지할 수 있습니다.\n\
             - 대시보드 탭에서 각 레이어의 상태를 확인할 수 있습니다."
        ),
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
    am: State<AppchainManager>,
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
pub fn list_appchains(am: State<AppchainManager>) -> Vec<AppchainConfig> {
    am.list_appchains()
}

#[tauri::command]
pub fn get_appchain(id: String, am: State<AppchainManager>) -> Result<AppchainConfig, String> {
    am.get_appchain(&id)
        .ok_or(format!("Appchain not found: {id}"))
}

#[tauri::command]
pub fn delete_appchain(id: String, am: State<AppchainManager>) -> Result<(), String> {
    am.delete_appchain(&id)
}

#[tauri::command]
pub fn start_appchain_setup(id: String, am: State<AppchainManager>) -> Result<(), String> {
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

    // TODO: Phase 1A - Actually spawn ethrex processes here
    // For now, simulate progress
    match config.network_mode {
        NetworkMode::Local => {
            am.update_step_status(&id, "dev", StepStatus::InProgress);
            am.add_log(
                &id,
                "Starting ethrex l2 --dev ... (not yet implemented)".to_string(),
            );
        }
        _ => {
            am.update_step_status(&id, "l1_check", StepStatus::InProgress);
            am.add_log(
                &id,
                format!("Checking L1 connection to {} ...", config.l1_rpc_url),
            );
        }
    }

    Ok(())
}

#[tauri::command]
pub fn get_setup_progress(
    id: String,
    am: State<AppchainManager>,
) -> Result<SetupProgress, String> {
    am.get_setup_progress(&id)
        .ok_or(format!("No setup in progress for: {id}"))
}
