//! Telegram AI Pilot for Tokamak Desktop App.
//!
//! Natural language control of appchains via Telegram.
//! All messages go through AI Pilot which interprets intent,
//! executes actions, and responds with results.
//!
//! Features:
//! - Natural language appchain control (start/stop/create/delete)
//! - Docker deployment management
//! - Persistent memory (chat history, events, AI summary)
//! - Auto-briefing after inactivity
//! - Background health monitoring

use crate::ai_provider::{AiProvider, ChatMessage};
use crate::appchain_manager::{AppchainManager, AppchainStatus, StepStatus};
use crate::deployment_db::{self, DeploymentProxy};
use crate::pilot_memory::PilotMemory;
use crate::runner::ProcessRunner;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{watch, Mutex};

const MAX_HISTORY: usize = 20;
const POLL_TIMEOUT_SECS: u64 = 30;
const TELEGRAM_API: &str = "https://api.telegram.org";
const BRIEFING_GAP_SECS: i64 = 6 * 3600; // 6 hours
const HEALTH_CHECK_INTERVAL_SECS: u64 = 300; // 5 minutes
const LOCAL_SERVER_URL: &str = "http://127.0.0.1:5002";

pub struct TelegramBot {
    token: String,
    allowed_chat_ids: Vec<i64>,
    client: Client,
    ai: Arc<AiProvider>,
    appchain_manager: Arc<AppchainManager>,
    runner: Arc<ProcessRunner>,
    memory: Arc<PilotMemory>,
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

/// Parsed ACTION from AI response
#[derive(Debug, Clone)]
struct ParsedAction {
    name: String,
    params: HashMap<String, String>,
}

impl TelegramBot {
    pub fn new(
        ai: Arc<AiProvider>,
        appchain_manager: Arc<AppchainManager>,
        runner: Arc<ProcessRunner>,
        memory: Arc<PilotMemory>,
    ) -> Option<Self> {
        let (token, allowed_ids_str, enabled) =
            Self::load_from_file().unwrap_or_else(|| Self::load_from_env());

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
            runner,
            memory,
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

    #[cfg(test)]
    fn new_with_token(
        token: &str,
        allowed_chat_ids: Vec<i64>,
        ai: Arc<AiProvider>,
        appchain_manager: Arc<AppchainManager>,
        runner: Arc<ProcessRunner>,
        memory: Arc<PilotMemory>,
    ) -> Self {
        Self {
            token: token.to_string(),
            allowed_chat_ids,
            client: Client::new(),
            ai,
            appchain_manager,
            runner,
            memory,
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

    // ── Long-polling loop ──

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

    // ── Unified message handler ──

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

        // Auto-briefing: if inactive for 6+ hours
        if let Some(last) = self.memory.last_message_time(chat_id) {
            let gap = chrono::Utc::now().signed_duration_since(last).num_seconds();
            if gap > BRIEFING_GAP_SECS {
                let briefing = self.generate_briefing(last).await;
                self.send_message(chat_id, &briefing).await;
            }
        }

        // /start and /help are handled directly, everything else goes through AI
        if text == "/start" {
            self.cmd_start(chat_id, &message.from).await;
            return;
        }
        if text == "/help" {
            self.send_help(chat_id).await;
            return;
        }
        if text == "/clear" {
            self.chat_history.lock().await.remove(&chat_id);
            self.memory.append_message(chat_id, "user", "/clear");
            self.send_message(chat_id, "Conversation cleared.").await;
            return;
        }

        // All other messages (including old slash commands) → AI Pilot
        self.handle_ai_message(chat_id, &text).await;
    }

    async fn cmd_start(&self, chat_id: i64, from: &Option<User>) {
        let name = from
            .as_ref()
            .map(|u| u.first_name.as_str())
            .unwrap_or("there");

        // Generate a brief status summary
        let chains = self.appchain_manager.list_appchains();
        let deployments = deployment_db::list_deployments_from_db().unwrap_or_default();

        let mut status_lines = Vec::new();
        for chain in &chains {
            let emoji = status_emoji(&chain.status);
            status_lines.push(format!("  {} {} — {:?}", emoji, chain.name, chain.status));
        }
        for dep in &deployments {
            let emoji = match dep.status.as_str() {
                "running" => "🟢",
                "stopped" => "🔴",
                "deploying" => "🟡",
                _ => "⚪",
            };
            status_lines.push(format!("  {} 🐳 {} — {}", emoji, dep.name, dep.status));
        }

        let status_block = if status_lines.is_empty() {
            "등록된 앱체인이 없습니다.".to_string()
        } else {
            status_lines.join("\n")
        };

        self.send_message(
            chat_id,
            &format!(
                "안녕하세요 {name}님! Tokamak Appchain Pilot입니다.\n\n\
                 📊 현재 상태:\n{status_block}\n\n\
                 자연어로 무엇이든 물어보세요:\n\
                 • \"앱체인 상태 알려줘\"\n\
                 • \"test 앱체인 중지해줘\"\n\
                 • \"새 로컬 앱체인 만들어줘\"\n\
                 • \"어제 뭐 했었지?\"\n\n\
                 /clear — 대화 초기화\n\
                 /help — 도움말",
            ),
        )
        .await;
    }

    async fn send_help(&self, chat_id: i64) {
        self.send_message(
            chat_id,
            "🤖 Tokamak Appchain Pilot\n\n\
             자연어로 앱체인을 관리할 수 있습니다:\n\n\
             📊 조회:\n\
             • \"현재 상태\" / \"뭐 돌아가고 있어?\"\n\
             • \"컨테이너 상태 보여줘\"\n\
             • \"어제 뭐 했지?\"\n\n\
             ⚡ 제어:\n\
             • \"test 앱체인 시작해줘\"\n\
             • \"앱체인 중지해줘\"\n\
             • \"새 로컬 앱체인 만들어줘\"\n\
             • \"배포 시작/중지/삭제\"\n\n\
             💬 AI 대화:\n\
             • 기술 질문, 트러블슈팅, 가이드\n\n\
             /clear — 대화 초기화",
        )
        .await;
    }

    // ── AI Pilot message processing ──

    async fn handle_ai_message(&self, chat_id: i64, text: &str) {
        self.send_typing(chat_id).await;

        // Save user message to memory
        self.memory.append_message(chat_id, "user", text);

        // Build context
        let appchain_context = build_appchain_context(&self.appchain_manager);
        let deployment_context = build_deployment_context();
        let pilot_context = self.memory.load_recent_context(chat_id, 20, 20);

        // Build chat history for AI
        let mut history_lock = self.chat_history.lock().await;
        let history = history_lock.entry(chat_id).or_insert_with(Vec::new);
        history.push(ChatMessage {
            role: "user".to_string(),
            content: text.to_string(),
        });
        if history.len() > MAX_HISTORY {
            history.drain(..history.len() - MAX_HISTORY);
        }
        let messages = history.clone();
        drop(history_lock);

        // Build telegram system prompt and call AI
        let system_prompt = AiProvider::build_telegram_prompt(
            &appchain_context,
            &deployment_context,
            &pilot_context,
        );
        let ai_response = match self.ai.chat_with_system_prompt(messages, &system_prompt).await {
            Ok(response) => response,
            Err(e) => {
                log::error!("[TG] AI error: {e}");
                let error_msg = if e.contains("login_required") {
                    "Tokamak AI 로그인이 필요합니다. 데스크톱 앱에서 로그인해주세요."
                } else if e.contains("daily_limit_exceeded") {
                    "오늘의 AI 토큰 한도를 초과했습니다."
                } else {
                    "AI가 일시적으로 사용 불가합니다. 잠시 후 다시 시도해주세요."
                };
                self.send_message(chat_id, error_msg).await;
                return;
            }
        };

        // Parse actions from AI response
        let (clean_text, actions) = parse_actions(&ai_response);

        // Execute actions
        let mut action_results = Vec::new();
        for action in &actions {
            let result = self.execute_action(chat_id, action).await;
            self.memory.append_action(
                chat_id,
                &format!("{}:{}", action.name, format_params(&action.params)),
                &result,
            );
            action_results.push(result);
        }

        // Build final response
        let mut final_text = clean_text.clone();
        if !action_results.is_empty() {
            final_text.push_str("\n\n");
            for result in &action_results {
                final_text.push_str(result);
                final_text.push('\n');
            }
        }

        // Send response (respect Telegram 4096 char limit)
        let final_text = final_text.trim().to_string();
        if final_text.len() > 4000 {
            let truncated = format!("{}...\n\n(truncated)", truncate_utf8(&final_text, 4000));
            self.send_message(chat_id, &truncated).await;
        } else if !final_text.is_empty() {
            self.send_message(chat_id, &final_text).await;
        }

        // Save to history and memory
        let mut history_lock = self.chat_history.lock().await;
        if let Some(history) = history_lock.get_mut(&chat_id) {
            history.push(ChatMessage {
                role: "assistant".to_string(),
                content: final_text.clone(),
            });
        }
        drop(history_lock);
        self.memory.append_message(chat_id, "assistant", &final_text);
    }

    // ── ACTION execution engine ──

    async fn execute_action(&self, chat_id: i64, action: &ParsedAction) -> String {
        match action.name.as_str() {
            "start_appchain" => self.action_start_appchain(chat_id, &action.params).await,
            "stop_appchain" => self.action_stop_appchain(chat_id, &action.params).await,
            "delete_appchain" => self.action_delete_appchain(&action.params).await,
            "start_deployment" => self.action_start_deployment(&action.params).await,
            "stop_deployment" => self.action_stop_deployment(&action.params).await,
            "delete_deployment" => self.action_delete_deployment(&action.params).await,
            "create_appchain" => self.action_create_appchain(chat_id, &action.params).await,
            "update_summary" => {
                if let Some(content) = action.params.get("content") {
                    self.memory.update_summary(content);
                    "📝 요약 업데이트됨".to_string()
                } else {
                    "❌ content 파라미터 필요".to_string()
                }
            }
            _ => format!("⚠️ 알 수 없는 액션: {}", action.name),
        }
    }

    async fn action_start_appchain(&self, chat_id: i64, params: &HashMap<String, String>) -> String {
        let chain_id = match self.resolve_chain_id(params) {
            Ok(id) => id,
            Err(e) => return format!("❌ {e}"),
        };

        let config = match self.appchain_manager.get_appchain(&chain_id) {
            Some(c) => c,
            None => return "❌ 앱체인을 찾을 수 없습니다.".to_string(),
        };

        if matches!(config.status, AppchainStatus::Running) {
            return format!("ℹ️ {} 은(는) 이미 실행 중입니다.", config.name);
        }

        let chain_name = config.name.clone();
        let has_prover = config.prover_type != "none";

        // Initialize setup
        self.appchain_manager.init_setup_progress(&chain_id, &config.network_mode, has_prover);
        self.appchain_manager.update_status(&chain_id, AppchainStatus::SettingUp);
        self.appchain_manager.update_step_status(&chain_id, "config", StepStatus::Done);

        self.memory.append_event("started", &chain_name, &chain_id, "", "telegram");

        // Start process in background
        let runner = self.runner.clone();
        let am = self.appchain_manager.clone();
        let cid = chain_id.clone();
        tokio::spawn(async move {
            ProcessRunner::start_local_dev(runner, am, cid).await;
        });

        // Poll progress and report
        self.poll_setup_progress(chat_id, &chain_id, &chain_name).await
    }

    async fn poll_setup_progress(&self, chat_id: i64, chain_id: &str, chain_name: &str) -> String {
        let mut last_step = String::new();

        for _ in 0..200 {
            // ~10 min max
            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

            let progress = match self.appchain_manager.get_setup_progress(chain_id) {
                Some(p) => p,
                None => continue,
            };

            let step = match progress.steps.get(progress.current_step) {
                Some(s) => s,
                None => continue,
            };
            let current_step = &step.id;

            // Report step changes
            if current_step != &last_step {
                let emoji = match current_step.as_str() {
                    "dev" => "🔧",
                    "l1_check" => "🔍",
                    "deploy" => "📜",
                    "l2" => "⚡",
                    "prover" => "🧮",
                    "done" => "✅",
                    _ => "📦",
                };
                let label = &step.label;
                self.send_message(chat_id, &format!("{} {}", emoji, label)).await;
                last_step = current_step.clone();
            }

            // Check completion
            if current_step == "done" {
                let config = self.appchain_manager.get_appchain(chain_id);
                let rpc_port = config.map(|c| c.l2_rpc_port).unwrap_or(1729);
                self.memory.append_event("running", chain_name, chain_id, "", "telegram");
                return format!(
                    "✅ {} 앱체인이 시작되었습니다!\nRPC: http://localhost:{}",
                    chain_name, rpc_port
                );
            }

            // Check error
            if let Some(err) = &progress.error {
                self.memory.append_event("error", chain_name, chain_id, err, "telegram");
                return format!("❌ {} 시작 실패: {}", chain_name, err);
            }
        }

        format!("⏰ {} 시작 타임아웃 (10분 초과)", chain_name)
    }

    async fn action_stop_appchain(&self, _chat_id: i64, params: &HashMap<String, String>) -> String {
        let chain_id = match self.resolve_chain_id(params) {
            Ok(id) => id,
            Err(e) => return format!("❌ {e}"),
        };

        let config = match self.appchain_manager.get_appchain(&chain_id) {
            Some(c) => c,
            None => return "❌ 앱체인을 찾을 수 없습니다.".to_string(),
        };

        if matches!(config.status, AppchainStatus::Stopped) {
            return format!("ℹ️ {} 은(는) 이미 중지되어 있습니다.", config.name);
        }

        let chain_name = config.name.clone();
        match self.runner.stop_chain(&chain_id).await {
            Ok(()) => {
                self.appchain_manager.update_status(&chain_id, AppchainStatus::Stopped);
                self.appchain_manager.add_log(&chain_id, "Stopped via Telegram.".to_string());
                self.memory.append_event("stopped", &chain_name, &chain_id, "", "telegram");
                format!("✅ {} 중지 완료.", chain_name)
            }
            Err(e) => {
                // If process not found, just update status
                self.appchain_manager.update_status(&chain_id, AppchainStatus::Stopped);
                self.memory.append_event("stopped", &chain_name, &chain_id, &e, "telegram");
                format!("⚠️ {} 프로세스가 이미 종료된 상태입니다. 상태를 Stopped로 변경했습니다.", chain_name)
            }
        }
    }

    async fn action_delete_appchain(&self, params: &HashMap<String, String>) -> String {
        let chain_id = match self.resolve_chain_id(params) {
            Ok(id) => id,
            Err(e) => return format!("❌ {e}"),
        };

        let config = match self.appchain_manager.get_appchain(&chain_id) {
            Some(c) => c,
            None => return "❌ 앱체인을 찾을 수 없습니다.".to_string(),
        };

        let chain_name = config.name.clone();

        // Stop if running
        let _ = self.runner.stop_chain(&chain_id).await;

        match self.appchain_manager.delete_appchain(&chain_id) {
            Ok(()) => {
                self.memory.append_event("deleted", &chain_name, &chain_id, "", "telegram");
                format!("✅ {} 삭제 완료.", chain_name)
            }
            Err(e) => format!("❌ 삭제 실패: {e}"),
        }
    }

    async fn action_create_appchain(&self, chat_id: i64, params: &HashMap<String, String>) -> String {
        use crate::appchain_manager::{AppchainConfig, NetworkMode};

        let name = params.get("name").cloned().unwrap_or_else(|| {
            format!("chain-{}", &uuid::Uuid::new_v4().to_string()[..8])
        });

        let network_mode = match params.get("network").map(|s| s.as_str()) {
            Some("testnet") => NetworkMode::Testnet,
            _ => NetworkMode::Local,
        };

        let chain_id: u64 = params
            .get("chain_id")
            .and_then(|s| s.parse().ok())
            .unwrap_or(17001);

        // Auto-assign l2_rpc_port to avoid conflicts with existing chains
        let existing_chains = self.appchain_manager.list_appchains();
        let used_ports: std::collections::HashSet<u16> = existing_chains
            .iter()
            .map(|c| c.l2_rpc_port)
            .collect();
        let mut l2_rpc_port: u16 = 1729;
        while used_ports.contains(&l2_rpc_port) {
            l2_rpc_port += 1;
        }

        let id = uuid::Uuid::new_v4().to_string();
        let config = AppchainConfig {
            id: id.clone(),
            name: name.clone(),
            icon: "🔗".to_string(),
            chain_id,
            description: "Created via Telegram Pilot".to_string(),
            network_mode: network_mode.clone(),
            l1_rpc_url: "http://localhost:8545".to_string(),
            l2_rpc_port,
            sequencer_mode: "single".to_string(),
            native_token: params.get("token").cloned().unwrap_or_else(|| "ETH".to_string()),
            prover_type: params.get("prover").cloned().unwrap_or_else(|| "none".to_string()),
            bridge_address: None,
            on_chain_proposer_address: None,
            is_public: false,
            hashtags: vec![],
            status: AppchainStatus::Created,
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        match self.appchain_manager.create_appchain(config) {
            Ok(_) => {
                self.memory.append_event("created", &name, &id, "", "telegram");

                // Auto-start if requested
                if params.get("auto_start").map(|s| s == "true").unwrap_or(true) {
                    let mut start_params = HashMap::new();
                    start_params.insert("id".to_string(), id);
                    return self.action_start_appchain(chat_id, &start_params).await;
                }

                format!("✅ {} 앱체인 생성 완료 (chain_id: {}).", name, chain_id)
            }
            Err(e) => format!("❌ 앱체인 생성 실패: {e}"),
        }
    }

    async fn action_start_deployment(&self, params: &HashMap<String, String>) -> String {
        let id = match params.get("id") {
            Some(id) => id.clone(),
            None => return "❌ 배포 ID가 필요합니다.".to_string(),
        };
        let proxy = DeploymentProxy::new(LOCAL_SERVER_URL);
        match proxy.start_deployment(&id).await {
            Ok(()) => {
                self.memory.append_event("deployment_started", &id, &id, "", "telegram");
                format!("✅ 배포 {} 시작됨.", id)
            }
            Err(e) => format!("❌ 배포 시작 실패: {e}"),
        }
    }

    async fn action_stop_deployment(&self, params: &HashMap<String, String>) -> String {
        let id = match params.get("id") {
            Some(id) => id.clone(),
            None => return "❌ 배포 ID가 필요합니다.".to_string(),
        };
        let proxy = DeploymentProxy::new(LOCAL_SERVER_URL);
        match proxy.stop_deployment(&id).await {
            Ok(()) => {
                self.memory.append_event("deployment_stopped", &id, &id, "", "telegram");
                format!("✅ 배포 {} 중지됨.", id)
            }
            Err(e) => format!("❌ 배포 중지 실패: {e}"),
        }
    }

    async fn action_delete_deployment(&self, params: &HashMap<String, String>) -> String {
        let id = match params.get("id") {
            Some(id) => id.clone(),
            None => return "❌ 배포 ID가 필요합니다.".to_string(),
        };
        let proxy = DeploymentProxy::new(LOCAL_SERVER_URL);
        match proxy.destroy_deployment(&id).await {
            Ok(()) => {
                self.memory.append_event("deployment_deleted", &id, &id, "", "telegram");
                format!("✅ 배포 {} 삭제됨.", id)
            }
            Err(e) => format!("❌ 배포 삭제 실패: {e}"),
        }
    }

    // ── Helpers ──

    fn resolve_chain_id(&self, params: &HashMap<String, String>) -> Result<String, String> {
        if let Some(id) = params.get("id") {
            return Ok(id.clone());
        }
        if let Some(name) = params.get("name") {
            let chains = self.appchain_manager.list_appchains();
            let chain = chains
                .iter()
                .find(|c| c.name.to_lowercase() == name.to_lowercase())
                .ok_or_else(|| format!("앱체인 '{name}'을(를) 찾을 수 없습니다."))?;
            return Ok(chain.id.clone());
        }
        Err("앱체인 id 또는 name이 필요합니다.".to_string())
    }

    // ── Auto-briefing ──

    async fn generate_briefing(&self, since: chrono::DateTime<chrono::Utc>) -> String {
        let events = self.memory.events_since(since);
        let chains = self.appchain_manager.list_appchains();
        let deployments = deployment_db::list_deployments_from_db().unwrap_or_default();

        let now = chrono::Utc::now();
        let gap = now.signed_duration_since(since);
        let gap_str = if gap.num_hours() >= 24 {
            format!("{}일 전", gap.num_days())
        } else {
            format!("{}시간 전", gap.num_hours())
        };

        let mut briefing = format!("🤖 Tokamak Pilot 브리핑\n\n📅 마지막 접속: {}\n", gap_str);

        // Events since last activity
        if !events.is_empty() {
            briefing.push_str("\n⚡ 그동안 일어난 일:\n");
            for event in &events {
                let emoji = match event.event.as_str() {
                    "created" => "🆕",
                    "started" | "running" => "🟢",
                    "stopped" => "🔴",
                    "deleted" => "🗑️",
                    "process_crashed" | "error" => "💥",
                    "container_exited" => "⚠️",
                    _ => "•",
                };
                let time = event.ts.format("%H:%M");
                let detail = if event.detail.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", event.detail)
                };
                briefing.push_str(&format!(
                    "  {} {} — {}{} [{}]\n",
                    emoji, event.chain_name, event.event, detail, time
                ));
            }
        }

        // Current status
        briefing.push_str("\n📊 현재 상태:\n");
        if chains.is_empty() && deployments.is_empty() {
            briefing.push_str("  등록된 앱체인 없음\n");
        }
        for chain in &chains {
            let emoji = status_emoji(&chain.status);
            briefing.push_str(&format!("  {} {} — {:?}\n", emoji, chain.name, chain.status));
        }
        for dep in &deployments {
            let emoji = match dep.status.as_str() {
                "running" => "🟢",
                "stopped" => "🔴",
                _ => "⚪",
            };
            briefing.push_str(&format!("  {} 🐳 {} — {}\n", emoji, dep.name, dep.status));
        }

        // Activity statistics
        if !events.is_empty() {
            let mut created = 0u32;
            let mut started = 0u32;
            let mut stopped = 0u32;
            let mut errors = 0u32;
            for event in &events {
                match event.event.as_str() {
                    "created" => created += 1,
                    "started" | "running" => started += 1,
                    "stopped" => stopped += 1,
                    "process_crashed" | "container_exited" | "log_error" | "rpc_unhealthy" | "error" => errors += 1,
                    _ => {}
                }
            }
            let mut stats = Vec::new();
            if created > 0 { stats.push(format!("생성 {}회", created)); }
            if started > 0 { stats.push(format!("시작 {}회", started)); }
            if stopped > 0 { stats.push(format!("중지 {}회", stopped)); }
            if errors > 0 { stats.push(format!("에러 {}건", errors)); }
            if !stats.is_empty() {
                briefing.push_str(&format!("\n📈 활동 요약: {}\n", stats.join(", ")));
            }
        }

        // Alerts
        let mut alerts = Vec::new();
        for chain in &chains {
            if matches!(chain.status, AppchainStatus::Error) {
                alerts.push(format!("  • {} 에러 상태 — \"상태 확인해줘\"", chain.name));
            }
        }
        if !alerts.is_empty() {
            briefing.push_str("\n💡 조치 필요:\n");
            for alert in &alerts {
                briefing.push_str(alert);
                briefing.push('\n');
            }
        }

        briefing.push_str("\n무엇을 도와드릴까요?");
        briefing
    }
}

// ── ACTION parsing (same format as ChatView.tsx) ──

fn parse_actions(text: &str) -> (String, Vec<ParsedAction>) {
    use std::sync::LazyLock;
    static RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\[ACTION:(\w+)(?::([^\]]*))?\]").unwrap());
    let re = &*RE;
    let mut actions = Vec::new();

    for cap in re.captures_iter(text) {
        let name = cap[1].to_string();
        let mut params = HashMap::new();
        if let Some(params_str) = cap.get(2) {
            let s = params_str.as_str();
            // Split on commas that separate key=value pairs.
            // A comma inside a value (no '=' after it) is kept as part of the value.
            let mut last_key: Option<String> = None;
            for part in s.split(',') {
                if let Some((k, v)) = part.split_once('=') {
                    let key = k.trim().to_string();
                    params.insert(key.clone(), v.trim().to_string());
                    last_key = Some(key);
                } else if let Some(ref key) = last_key {
                    // No '=' means this is a continuation of the previous value
                    if let Some(val) = params.get_mut(key) {
                        val.push(',');
                        val.push_str(part);
                    }
                }
            }
        }
        actions.push(ParsedAction { name, params });
    }

    let clean_text = re.replace_all(text, "").trim().to_string();
    (clean_text, actions)
}

fn format_params(params: &HashMap<String, String>) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Truncate a string at a UTF-8 safe boundary
fn truncate_utf8(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    let end = s
        .char_indices()
        .take_while(|(i, _)| *i < max_len)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(max_len);
    format!("{}…", &s[..end])
}

fn status_emoji(status: &AppchainStatus) -> &'static str {
    match status {
        AppchainStatus::Running => "🟢",
        AppchainStatus::Stopped => "🔴",
        AppchainStatus::Created => "⚪",
        AppchainStatus::SettingUp => "🟡",
        AppchainStatus::Error => "❌",
    }
}

// ── Shared context builders ──

pub fn build_appchain_context(am: &AppchainManager) -> serde_json::Value {
    let chains = am.list_appchains();
    let summaries: Vec<serde_json::Value> = chains
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
        "appchains": summaries,
        "total_count": chains.len(),
    })
}

pub fn build_deployment_context() -> serde_json::Value {
    let deployments = deployment_db::list_deployments_from_db().unwrap_or_default();
    let summaries: Vec<serde_json::Value> = deployments
        .iter()
        .map(|d| {
            serde_json::json!({
                "id": d.id,
                "name": d.name,
                "program": d.program_slug,
                "status": d.status,
                "chain_id": d.chain_id,
                "l1_port": d.l1_port,
                "l2_port": d.l2_port,
                "phase": d.phase,
                "error": d.error_message,
            })
        })
        .collect();

    serde_json::json!({
        "deployments": summaries,
        "total_count": deployments.len(),
    })
}

// ── TelegramBotManager ──

pub struct TelegramBotManager {
    shutdown_tx: std::sync::Mutex<Option<watch::Sender<bool>>>,
    ai: Arc<AiProvider>,
    appchain_manager: Arc<AppchainManager>,
    runner: Arc<ProcessRunner>,
    memory: Arc<PilotMemory>,
    notify_config: std::sync::Mutex<Option<NotifyConfig>>,
}

struct NotifyConfig {
    token: String,
    chat_ids: Vec<i64>,
    client: Client,
}

impl TelegramBotManager {
    pub fn new(
        ai: Arc<AiProvider>,
        appchain_manager: Arc<AppchainManager>,
        runner: Arc<ProcessRunner>,
        memory: Arc<PilotMemory>,
    ) -> Self {
        Self {
            shutdown_tx: std::sync::Mutex::new(None),
            ai,
            appchain_manager,
            runner,
            memory,
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

        let bot = TelegramBot::new(
            self.ai.clone(),
            self.appchain_manager.clone(),
            self.runner.clone(),
            self.memory.clone(),
        )
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

        // Also record event in memory
        self.memory.append_event("notification", "", "", message, "system");

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

    /// Background health monitor — checks process/container/RPC health periodically
    pub async fn health_monitor(
        am: Arc<AppchainManager>,
        runner: Arc<ProcessRunner>,
        memory: Arc<PilotMemory>,
        notify_tx: Arc<TelegramBotManager>,
    ) {
        use std::collections::HashSet;
        use std::sync::LazyLock;
        static ERROR_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(r"(?i)(panic|fatal|out of memory|OOM|segfault|SIGSEGV|SIGKILL|killed|error.*exited|exited with code [^0])")
                .unwrap()
        });

        log::info!("Health monitor started");
        let proxy = DeploymentProxy::new(LOCAL_SERVER_URL);
        // Track already-alerted issues to avoid notification spam.
        // Key format: "type:entity_id:detail" — cleared when issue resolves.
        let mut alerted: HashSet<String> = HashSet::new();

        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(HEALTH_CHECK_INTERVAL_SECS)).await;

            if !notify_tx.is_running() {
                continue;
            }

            // Track current issues this cycle to detect resolved ones
            let mut current_issues: HashSet<String> = HashSet::new();

            // ── 1. Check appchain processes ──
            let chains = am.list_appchains();
            for chain in &chains {
                if matches!(chain.status, AppchainStatus::Running)
                    && !runner.is_running(&chain.id).await
                {
                    let key = format!("process_crashed:{}", chain.id);
                    current_issues.insert(key.clone());
                    // update_status prevents re-detection (Running→Error),
                    // but we still deduplicate for safety
                    if !alerted.contains(&key) {
                        am.update_status(&chain.id, AppchainStatus::Error);
                        memory.append_event(
                            "process_crashed",
                            &chain.name,
                            &chain.id,
                            "Process not found",
                            "system",
                        );
                        notify_tx.notify(&format!(
                            "⚠️ {} 프로세스가 비정상 종료되었습니다.",
                            chain.name
                        ));
                        alerted.insert(key);
                    }
                }
            }

            // ── 2. Check Docker deployments ──
            let deployments = deployment_db::list_deployments_from_db().unwrap_or_default();
            for dep in &deployments {
                if dep.phase != "running" {
                    continue;
                }

                // 2a. Container health — check for exited/restarting containers
                if let Ok(containers) = proxy.get_containers(&dep.id).await {
                    for c in &containers {
                        let is_down = c.state == "exited" || c.state == "dead";
                        let is_restarting = c.state == "restarting";
                        if is_down || is_restarting {
                            let key = format!("container:{}:{}", dep.id, c.service);
                            current_issues.insert(key.clone());
                            if !alerted.contains(&key) {
                                let detail = format!(
                                    "service={} state={} status={}",
                                    c.service, c.state, c.status
                                );
                                memory.append_event(
                                    "container_exited",
                                    &dep.name,
                                    &dep.id,
                                    &detail,
                                    "system",
                                );
                                let emoji = if is_restarting { "🔄" } else { "💀" };
                                notify_tx.notify(&format!(
                                    "{} 🐳 {} — 컨테이너 {} {}({})",
                                    emoji, dep.name, c.service, c.state, c.status
                                ));
                                alerted.insert(key);
                            }
                        }
                    }
                }

                // 2b. RPC health — check L1/L2 node responsiveness
                if let Ok(mon) = proxy.get_monitoring(&dep.id).await {
                    if let Some(l1) = &mon.l1 {
                        let key = format!("rpc:{}:l1", dep.id);
                        if !l1.healthy {
                            current_issues.insert(key.clone());
                            if !alerted.contains(&key) {
                                memory.append_event(
                                    "rpc_unhealthy",
                                    &dep.name,
                                    &dep.id,
                                    "L1 RPC not responding",
                                    "system",
                                );
                                notify_tx.notify(&format!(
                                    "🔴 🐳 {} — L1 RPC가 응답하지 않습니다.",
                                    dep.name
                                ));
                                alerted.insert(key);
                            }
                        }
                    }
                    if let Some(l2) = &mon.l2 {
                        let key = format!("rpc:{}:l2", dep.id);
                        if !l2.healthy {
                            current_issues.insert(key.clone());
                            if !alerted.contains(&key) {
                                memory.append_event(
                                    "rpc_unhealthy",
                                    &dep.name,
                                    &dep.id,
                                    "L2 RPC not responding",
                                    "system",
                                );
                                notify_tx.notify(&format!(
                                    "🔴 🐳 {} — L2 RPC가 응답하지 않습니다.",
                                    dep.name
                                ));
                                alerted.insert(key);
                            }
                        }
                    }
                }

                // 2c. Log error detection — scan recent logs for critical errors
                if let Ok(logs) = proxy.get_logs(&dep.id, None, 50).await {
                    let errors: Vec<&str> = logs
                        .lines()
                        .filter(|line| ERROR_RE.is_match(line))
                        .collect();
                    if !errors.is_empty() {
                        // Use hash of error content to detect new vs stale errors
                        let sample = errors.last().unwrap_or(&"unknown error");
                        let key = format!("log_error:{}:{}", dep.id, &sample[..sample.len().min(80)]);
                        current_issues.insert(key.clone());
                        if !alerted.contains(&key) {
                            let truncated = truncate_utf8(sample, 200);
                            memory.append_event(
                                "log_error",
                                &dep.name,
                                &dep.id,
                                &truncated,
                                "system",
                            );
                            notify_tx.notify(&format!(
                                "⚠️ 🐳 {} — 로그에서 에러 감지 ({}건):\n{}",
                                dep.name,
                                errors.len(),
                                truncated
                            ));
                            alerted.insert(key);
                        }
                    }
                }
            }

            // Clear alerts for resolved issues so they can re-fire if they recur
            alerted.retain(|key| current_issues.contains(key));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bot(allowed: Vec<i64>) -> TelegramBot {
        let ai = Arc::new(AiProvider::new());
        let am = Arc::new(AppchainManager::new());
        let runner = Arc::new(ProcessRunner::new());
        let memory = Arc::new(PilotMemory::new());
        TelegramBot::new_with_token("test:fake_token", allowed, ai, am, runner, memory)
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
    fn test_parse_actions() {
        let text = "앱체인을 중지합니다. [ACTION:stop_appchain:id=abc123]";
        let (clean, actions) = parse_actions(text);
        assert_eq!(clean, "앱체인을 중지합니다.");
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].name, "stop_appchain");
        assert_eq!(actions[0].params.get("id").unwrap(), "abc123");
    }

    #[test]
    fn test_parse_actions_multiple() {
        let text = "작업합니다. [ACTION:stop_appchain:name=test] 그리고 [ACTION:start_deployment:id=dep1]";
        let (clean, actions) = parse_actions(text);
        assert_eq!(clean, "작업합니다.  그리고");
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].name, "stop_appchain");
        assert_eq!(actions[1].name, "start_deployment");
    }

    #[test]
    fn test_parse_actions_no_params() {
        let text = "상태입니다. [ACTION:status]";
        let (_, actions) = parse_actions(text);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].name, "status");
        assert!(actions[0].params.is_empty());
    }

    #[test]
    fn test_parse_actions_comma_in_value() {
        let text = "요약 업데이트합니다. [ACTION:update_summary:content=앱체인 현황, 모두 정상]";
        let (_, actions) = parse_actions(text);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].name, "update_summary");
        assert_eq!(
            actions[0].params.get("content").unwrap(),
            "앱체인 현황, 모두 정상"
        );
    }

    #[test]
    fn test_parse_actions_mixed_params_with_comma_value() {
        let text = "[ACTION:create_appchain:name=test,network=local]";
        let (_, actions) = parse_actions(text);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].params.get("name").unwrap(), "test");
        assert_eq!(actions[0].params.get("network").unwrap(), "local");
    }

    #[test]
    fn test_truncate_utf8() {
        assert_eq!(truncate_utf8("hello", 10), "hello");
        assert_eq!(truncate_utf8("hello world", 5), "hello…");
        // Korean multibyte
        assert_eq!(truncate_utf8("안녕하세요", 6), "안녕…");
    }

    #[test]
    fn test_status_emoji() {
        assert_eq!(status_emoji(&AppchainStatus::Running), "🟢");
        assert_eq!(status_emoji(&AppchainStatus::Stopped), "🔴");
        assert_eq!(status_emoji(&AppchainStatus::Error), "❌");
    }
}
