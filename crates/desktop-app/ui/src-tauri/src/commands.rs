use crate::process_manager::{NodeInfo, ProcessManager, ProcessStatus};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,  // "user" or "assistant"
    pub content: String,
}

#[tauri::command]
pub fn get_all_status(pm: State<ProcessManager>) -> Vec<NodeInfo> {
    pm.get_all()
}

#[tauri::command]
pub fn start_node(name: String, pm: State<ProcessManager>) -> Result<String, String> {
    let info = pm.get_status(&name).ok_or(format!("Unknown node: {name}"))?;
    if matches!(info.status, ProcessStatus::Running) {
        return Err(format!("{name} is already running"));
    }
    // TODO: Actually spawn the process
    pm.set_status(&name, ProcessStatus::Running, Some(0));
    Ok(format!("{name} started"))
}

#[tauri::command]
pub fn stop_node(name: String, pm: State<ProcessManager>) -> Result<String, String> {
    let info = pm.get_status(&name).ok_or(format!("Unknown node: {name}"))?;
    if matches!(info.status, ProcessStatus::Stopped) {
        return Err(format!("{name} is already stopped"));
    }
    // TODO: Actually kill the process
    pm.set_status(&name, ProcessStatus::Stopped, None);
    Ok(format!("{name} stopped"))
}

#[tauri::command]
pub fn get_node_status(name: String, pm: State<ProcessManager>) -> Result<NodeInfo, String> {
    pm.get_status(&name).ok_or(format!("Unknown node: {name}"))
}

#[tauri::command]
pub fn get_logs(name: String, _lines: Option<usize>) -> Result<Vec<String>, String> {
    // TODO: Read actual log files
    Ok(vec![format!("[{name}] No logs available yet - process management coming in Phase 1")])
}

#[tauri::command]
pub async fn send_chat_message(message: String) -> Result<ChatMessage, String> {
    // TODO: Connect to AI providers (Phase 3)
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
