//! Unified L2 State Layer
//!
//! Single source of truth for all L2 instance state (appchains + Docker deployments).
//! Background-refreshed every 5 seconds, consumed by Telegram Bot, AI Messenger, and Frontend.

use crate::appchain_manager::{AppchainManager, AppchainStatus};
use crate::deployment_db::{self, DeploymentProxy, MonitoringInfo};
use crate::pilot_memory::PilotMemory;
use crate::runner::ProcessRunner;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tokio::sync::broadcast;

const LOCAL_SERVER_URL: &str = "http://127.0.0.1:5002";
const REFRESH_INTERVAL_SECS: u64 = 5;
const EVENT_CHANNEL_CAPACITY: usize = 64;

// ── Data Models ──

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum L2Source {
    Appchain,
    Deployment,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum L2Status {
    Created,
    SettingUp,
    Running,
    Stopped,
    Error,
    Partial,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L2Health {
    pub l1_healthy: bool,
    pub l2_healthy: bool,
    pub l1_block_number: Option<serde_json::Value>,
    pub l2_block_number: Option<serde_json::Value>,
    pub l1_chain_id: Option<serde_json::Value>,
    pub l2_chain_id: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L2Contracts {
    pub bridge: Option<String>,
    pub proposer: Option<String>,
    pub timelock: Option<String>,
    pub sp1_verifier: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L2Container {
    pub service: String,
    pub state: String,
    pub status: String,
}

/// Unified L2 instance info combining appchain config + deployment runtime state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L2Info {
    pub id: String,
    pub name: String,
    pub source: L2Source,

    // Config
    pub chain_id: Option<u64>,
    pub network_mode: String,
    pub native_token: String,

    // Runtime state
    pub status: L2Status,
    pub health: Option<L2Health>,

    // Endpoints
    pub l1_rpc_url: Option<String>,
    pub l2_rpc_url: Option<String>,

    // Contracts
    pub contracts: Option<L2Contracts>,

    // Docker (Deployment only)
    pub containers: Option<Vec<L2Container>>,
    pub phase: Option<String>,
    pub error_message: Option<String>,

    // Meta
    pub is_public: bool,
    pub created_at: String,
}

// ── Events ──

#[derive(Debug, Clone, Serialize)]
pub struct L2Event {
    pub event_type: String,
    pub l2_id: String,
    pub l2_name: String,
    pub source_type: String, // "appchain" | "deployment"
    pub detail: String,
    pub timestamp: String,
}

// ── Snapshot ──

#[derive(Debug, Clone)]
struct StateSnapshot {
    items: Vec<L2Info>,
    #[allow(dead_code)]
    refreshed_at: Instant,
}

// ── UnifiedL2State ──

pub struct UnifiedL2State {
    snapshot: RwLock<StateSnapshot>,
    event_tx: broadcast::Sender<L2Event>,
}

impl UnifiedL2State {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            snapshot: RwLock::new(StateSnapshot {
                items: Vec::new(),
                refreshed_at: Instant::now(),
            }),
            event_tx,
        }
    }

    /// Get all L2 instances (cached, instant)
    pub fn get_all(&self) -> Vec<L2Info> {
        self.snapshot.read().unwrap().items.clone()
    }

    /// Get a specific L2 instance by ID
    #[allow(dead_code)]
    pub fn get_by_id(&self, id: &str) -> Option<L2Info> {
        self.snapshot
            .read()
            .unwrap()
            .items
            .iter()
            .find(|l| l.id == id)
            .cloned()
    }

    /// Build JSON context for AI system prompts (Telegram + Desktop Chat)
    pub fn to_context_json(&self) -> serde_json::Value {
        let snap = self.snapshot.read().unwrap();
        let all: &Vec<L2Info> = &snap.items;

        serde_json::json!({
            "my_appchains": all,
            "total_count": all.len(),
        })
    }

    /// Subscribe to state change events
    pub fn subscribe_events(&self) -> broadcast::Receiver<L2Event> {
        self.event_tx.subscribe()
    }

    /// Emit an event
    #[allow(dead_code)]
    fn emit_event(&self, event: L2Event) {
        // Ignore send errors (no receivers)
        let _ = self.event_tx.send(event);
    }

    /// Force immediate refresh (after an action)
    pub async fn refresh_now(
        &self,
        am: &AppchainManager,
        runner: &ProcessRunner,
    ) {
        self.do_refresh(am, runner).await;
    }

    /// Core refresh logic: collect state from AppchainManager + local-server
    async fn do_refresh(
        &self,
        _am: &AppchainManager,
        _runner: &ProcessRunner,
    ) {
        let old_items = self.snapshot.read().unwrap().items.clone();
        let mut new_items = Vec::new();

        // NOTE: AppchainManager (local process appchains) is not collected here.
        // All real deployments go through the Docker-based L2 Manager (local-server).
        // AppchainManager contains only legacy test data that is not actively used.

        // Docker deployments from local-server
        let deployments = deployment_db::list_deployments_from_db().unwrap_or_default();
        let proxy = DeploymentProxy::new(LOCAL_SERVER_URL);

        for dep in &deployments {
            // Fetch container status
            let containers = proxy.get_containers(&dep.id).await.unwrap_or_default();
            let container_info: Vec<L2Container> = containers
                .iter()
                .map(|c| L2Container {
                    service: c.service.clone(),
                    state: c.state.clone(),
                    status: c.status.clone(),
                })
                .collect();

            // Determine live status from containers
            let status = if containers.is_empty() {
                match dep.phase.as_str() {
                    "running" | "active" => L2Status::Partial, // phase says running but no containers found
                    "error" => L2Status::Error,
                    "stopped" => L2Status::Stopped,
                    "configured" => L2Status::Created,
                    _ => {
                        // Provisioning phases
                        if ["checking_docker", "building", "pulling", "l1_starting",
                            "deploying_contracts", "verifying_contracts", "l2_starting",
                            "starting_prover", "starting_tools"]
                            .contains(&dep.phase.as_str())
                        {
                            L2Status::SettingUp
                        } else {
                            L2Status::Stopped
                        }
                    }
                }
            } else if containers.iter().all(|c| c.state == "running") {
                L2Status::Running
            } else if containers.iter().all(|c| c.state == "exited" || c.state == "dead") {
                L2Status::Stopped
            } else {
                L2Status::Partial
            };

            // Fetch monitoring (best-effort)
            let monitoring = proxy.get_monitoring(&dep.id).await.ok();
            let health = monitoring.as_ref().map(|m| build_health(m));

            // Contracts
            let contracts = if dep.bridge_address.is_some() || dep.proposer_address.is_some() {
                Some(L2Contracts {
                    bridge: dep.bridge_address.clone(),
                    proposer: dep.proposer_address.clone(),
                    timelock: dep.timelock_address.clone(),
                    sp1_verifier: dep.sp1_verifier_address.clone(),
                })
            } else {
                None
            };

            let chain_id = dep.chain_id.map(|c| c as u64);

            new_items.push(L2Info {
                id: dep.id.clone(),
                name: dep.name.clone(),
                source: L2Source::Deployment,
                chain_id,
                network_mode: dep
                    .config
                    .as_ref()
                    .and_then(|c| {
                        serde_json::from_str::<serde_json::Value>(c)
                            .ok()
                            .and_then(|v| v["mode"].as_str().map(String::from))
                    })
                    .unwrap_or_else(|| "local".to_string()),
                native_token: "TON".to_string(),
                status,
                health,
                l1_rpc_url: dep.rpc_url.clone().or_else(|| {
                    dep.l1_port.map(|p| format!("http://127.0.0.1:{}", p))
                }),
                l2_rpc_url: dep.l2_port.map(|p| format!("http://127.0.0.1:{}", p)),
                contracts,
                containers: if container_info.is_empty() {
                    None
                } else {
                    Some(container_info)
                },
                phase: Some(dep.phase.clone()),
                error_message: dep.error_message.clone(),
                is_public: dep.is_public != 0,
                created_at: dep.created_at.to_string(),
            });
        }

        // 3. Diff and emit events
        diff_and_emit(&old_items, &new_items, &self.event_tx);

        // 4. Update snapshot
        *self.snapshot.write().unwrap() = StateSnapshot {
            items: new_items,
            refreshed_at: Instant::now(),
        };
    }
}

// ── Background refresh task ──

pub async fn spawn_state_refresh(
    state: Arc<UnifiedL2State>,
    am: Arc<AppchainManager>,
    runner: Arc<ProcessRunner>,
    memory: Arc<PilotMemory>,
    app_handle: tauri::AppHandle,
) {
    // Pipe events to PilotMemory + Tauri frontend events (separate task)
    let mut event_rx = state.subscribe_events();
    let memory_clone = memory.clone();
    let app_clone = app_handle.clone();
    tokio::spawn(async move {
        while let Ok(event) = event_rx.recv().await {
            // Record in PilotMemory
            memory_clone.append_event(
                &event.event_type,
                &event.l2_name,
                &event.l2_id,
                &event.detail,
                "system",
            );
            // Emit to frontend via Tauri event
            use tauri::Emitter;
            let _ = app_clone.emit("l2-state-changed", &event);
        }
    });

    // Periodic refresh loop (runs directly in this task)
    loop {
        state.do_refresh(&am, &runner).await;
        tokio::time::sleep(tokio::time::Duration::from_secs(REFRESH_INTERVAL_SECS)).await;
    }
}

// ── Helpers ──

#[allow(dead_code)]
fn reconcile_appchain_status(status: &AppchainStatus, process_alive: bool) -> L2Status {
    match status {
        AppchainStatus::Running if !process_alive => L2Status::Error,
        AppchainStatus::Running => L2Status::Running,
        AppchainStatus::Stopped => L2Status::Stopped,
        AppchainStatus::SettingUp => L2Status::SettingUp,
        AppchainStatus::Error => L2Status::Error,
        AppchainStatus::Created => L2Status::Created,
    }
}

#[allow(dead_code)]
fn appchain_status_to_l2(status: &AppchainStatus) -> L2Status {
    match status {
        AppchainStatus::Running => L2Status::Running,
        AppchainStatus::Stopped => L2Status::Stopped,
        AppchainStatus::SettingUp => L2Status::SettingUp,
        AppchainStatus::Error => L2Status::Error,
        AppchainStatus::Created => L2Status::Created,
    }
}

fn build_health(m: &MonitoringInfo) -> L2Health {
    let (l1_healthy, l1_block, l1_chain) = match &m.l1 {
        Some(l) => (l.healthy, l.block_number.clone(), l.chain_id.clone()),
        None => (false, None, None),
    };
    let (l2_healthy, l2_block, l2_chain) = match &m.l2 {
        Some(l) => (l.healthy, l.block_number.clone(), l.chain_id.clone()),
        None => (false, None, None),
    };
    L2Health {
        l1_healthy,
        l2_healthy,
        l1_block_number: l1_block,
        l2_block_number: l2_block,
        l1_chain_id: l1_chain,
        l2_chain_id: l2_chain,
    }
}

fn diff_and_emit(
    old: &[L2Info],
    new: &[L2Info],
    tx: &broadcast::Sender<L2Event>,
) {
    let now = chrono::Utc::now().to_rfc3339();

    for new_item in new {
        let old_item = old.iter().find(|o| o.id == new_item.id && o.source == new_item.source);

        match old_item {
            Some(old_item) => {
                // Status change
                if old_item.status != new_item.status {
                    let _ = tx.send(L2Event {
                        event_type: "status_changed".to_string(),
                        l2_id: new_item.id.clone(),
                        l2_name: new_item.name.clone(),
                        source_type: format!("{:?}", new_item.source).to_lowercase(),
                        detail: format!(
                            "{:?} → {:?}",
                            old_item.status, new_item.status
                        ),
                        timestamp: now.clone(),
                    });
                }

                // Health change detection
                match (&old_item.health, &new_item.health) {
                    (Some(old_h), Some(new_h)) => {
                        if old_h.l1_healthy != new_h.l1_healthy || old_h.l2_healthy != new_h.l2_healthy {
                            let _ = tx.send(L2Event {
                                event_type: "health_changed".to_string(),
                                l2_id: new_item.id.clone(),
                                l2_name: new_item.name.clone(),
                                source_type: format!("{:?}", new_item.source).to_lowercase(),
                                detail: format!(
                                    "L1: {} → {}, L2: {} → {}",
                                    old_h.l1_healthy, new_h.l1_healthy,
                                    old_h.l2_healthy, new_h.l2_healthy
                                ),
                                timestamp: now.clone(),
                            });
                        }
                    }
                    (None, Some(new_h)) => {
                        // Health first appeared (e.g. monitoring became available)
                        let _ = tx.send(L2Event {
                            event_type: "health_changed".to_string(),
                            l2_id: new_item.id.clone(),
                            l2_name: new_item.name.clone(),
                            source_type: format!("{:?}", new_item.source).to_lowercase(),
                            detail: format!(
                                "L1: {}, L2: {} (initial)",
                                new_h.l1_healthy, new_h.l2_healthy
                            ),
                            timestamp: now.clone(),
                        });
                    }
                    _ => {}
                }
            }
            None => {
                // New instance
                let _ = tx.send(L2Event {
                    event_type: "created".to_string(),
                    l2_id: new_item.id.clone(),
                    l2_name: new_item.name.clone(),
                    source_type: format!("{:?}", new_item.source).to_lowercase(),
                    detail: format!("New {:?} instance", new_item.source),
                    timestamp: now.clone(),
                });
            }
        }
    }

    // Detect deletions
    for old_item in old {
        if !new.iter().any(|n| n.id == old_item.id && n.source == old_item.source) {
            let _ = tx.send(L2Event {
                event_type: "deleted".to_string(),
                l2_id: old_item.id.clone(),
                l2_name: old_item.name.clone(),
                source_type: format!("{:?}", old_item.source).to_lowercase(),
                detail: format!("{:?} instance removed", old_item.source),
                timestamp: now.clone(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Test helpers ──

    fn make_appchain(id: &str, name: &str, status: L2Status) -> L2Info {
        L2Info {
            id: id.to_string(),
            name: name.to_string(),
            source: L2Source::Appchain,
            chain_id: Some(17001),
            network_mode: "Local".to_string(),
            native_token: "TON".to_string(),
            status,
            health: None,
            l1_rpc_url: Some("http://localhost:8545".to_string()),
            l2_rpc_url: Some("http://localhost:1729".to_string()),
            contracts: None,
            containers: None,
            phase: None,
            error_message: None,
            is_public: false,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn make_deployment(id: &str, name: &str, status: L2Status) -> L2Info {
        L2Info {
            id: id.to_string(),
            name: name.to_string(),
            source: L2Source::Deployment,
            chain_id: Some(17001),
            network_mode: "local".to_string(),
            native_token: "TON".to_string(),
            status,
            health: None,
            l1_rpc_url: Some("http://127.0.0.1:8545".to_string()),
            l2_rpc_url: Some("http://127.0.0.1:8546".to_string()),
            contracts: None,
            containers: None,
            phase: Some("running".to_string()),
            error_message: None,
            is_public: false,
            created_at: "1700000000".to_string(),
        }
    }

    fn make_deployment_with_health(id: &str, name: &str, l1_healthy: bool, l2_healthy: bool) -> L2Info {
        let mut d = make_deployment(id, name, L2Status::Running);
        d.health = Some(L2Health {
            l1_healthy,
            l2_healthy,
            l1_block_number: Some(serde_json::json!(100)),
            l2_block_number: Some(serde_json::json!(50)),
            l1_chain_id: Some(serde_json::json!(1)),
            l2_chain_id: Some(serde_json::json!(17001)),
        });
        d
    }

    #[allow(dead_code)]
    fn make_deployment_with_containers(id: &str, name: &str, containers: Vec<(&str, &str)>) -> L2Info {
        let mut d = make_deployment(id, name, L2Status::Running);
        d.containers = Some(
            containers
                .into_iter()
                .map(|(svc, state)| L2Container {
                    service: svc.to_string(),
                    state: state.to_string(),
                    status: format!("Up 5 min"),
                })
                .collect(),
        );
        d
    }

    // ── Unit Tests: Serialization ──

    #[test]
    fn test_l2_status_serialize_all_variants() {
        assert_eq!(serde_json::to_string(&L2Status::Created).unwrap(), "\"created\"");
        assert_eq!(serde_json::to_string(&L2Status::SettingUp).unwrap(), "\"settingup\"");
        assert_eq!(serde_json::to_string(&L2Status::Running).unwrap(), "\"running\"");
        assert_eq!(serde_json::to_string(&L2Status::Stopped).unwrap(), "\"stopped\"");
        assert_eq!(serde_json::to_string(&L2Status::Error).unwrap(), "\"error\"");
        assert_eq!(serde_json::to_string(&L2Status::Partial).unwrap(), "\"partial\"");
    }

    #[test]
    fn test_l2_status_deserialize() {
        assert_eq!(serde_json::from_str::<L2Status>("\"running\"").unwrap(), L2Status::Running);
        assert_eq!(serde_json::from_str::<L2Status>("\"partial\"").unwrap(), L2Status::Partial);
    }

    #[test]
    fn test_l2_source_serialize() {
        assert_eq!(serde_json::to_string(&L2Source::Appchain).unwrap(), "\"appchain\"");
        assert_eq!(serde_json::to_string(&L2Source::Deployment).unwrap(), "\"deployment\"");
    }

    #[test]
    fn test_l2_info_full_json_roundtrip() {
        let info = make_appchain("abc", "My Chain", L2Status::Running);
        let json = serde_json::to_string(&info).unwrap();
        let parsed: L2Info = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "abc");
        assert_eq!(parsed.name, "My Chain");
        assert_eq!(parsed.status, L2Status::Running);
        assert_eq!(parsed.source, L2Source::Appchain);
        assert_eq!(parsed.native_token, "TON");
    }

    #[test]
    fn test_l2_info_with_contracts_serializes() {
        let mut info = make_deployment("dep-1", "Test Deploy", L2Status::Running);
        info.contracts = Some(L2Contracts {
            bridge: Some("0xabc".to_string()),
            proposer: Some("0xdef".to_string()),
            timelock: None,
            sp1_verifier: None,
        });
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("0xabc"));
        assert!(json.contains("0xdef"));
    }

    #[test]
    fn test_l2_health_serializes() {
        let health = L2Health {
            l1_healthy: true,
            l2_healthy: false,
            l1_block_number: Some(serde_json::json!(12345)),
            l2_block_number: None,
            l1_chain_id: Some(serde_json::json!(1)),
            l2_chain_id: None,
        };
        let json = serde_json::to_string(&health).unwrap();
        assert!(json.contains("\"l1_healthy\":true"));
        assert!(json.contains("\"l2_healthy\":false"));
        assert!(json.contains("12345"));
    }

    // ── Unit Tests: Status reconciliation ──

    #[test]
    fn test_reconcile_all_appchain_statuses() {
        assert_eq!(reconcile_appchain_status(&AppchainStatus::Running, true), L2Status::Running);
        assert_eq!(reconcile_appchain_status(&AppchainStatus::Running, false), L2Status::Error);
        assert_eq!(reconcile_appchain_status(&AppchainStatus::Stopped, false), L2Status::Stopped);
        assert_eq!(reconcile_appchain_status(&AppchainStatus::Stopped, true), L2Status::Stopped);
        assert_eq!(reconcile_appchain_status(&AppchainStatus::SettingUp, false), L2Status::SettingUp);
        assert_eq!(reconcile_appchain_status(&AppchainStatus::Error, false), L2Status::Error);
        assert_eq!(reconcile_appchain_status(&AppchainStatus::Created, false), L2Status::Created);
    }

    #[test]
    fn test_appchain_status_to_l2_all() {
        assert_eq!(appchain_status_to_l2(&AppchainStatus::Running), L2Status::Running);
        assert_eq!(appchain_status_to_l2(&AppchainStatus::Stopped), L2Status::Stopped);
        assert_eq!(appchain_status_to_l2(&AppchainStatus::SettingUp), L2Status::SettingUp);
        assert_eq!(appchain_status_to_l2(&AppchainStatus::Error), L2Status::Error);
        assert_eq!(appchain_status_to_l2(&AppchainStatus::Created), L2Status::Created);
    }

    // ── Unit Tests: build_health ──

    #[test]
    fn test_build_health_both_present() {
        let m = MonitoringInfo {
            l1: Some(RpcHealth {
                healthy: true,
                block_number: Some(serde_json::json!(100)),
                chain_id: Some(serde_json::json!(1)),
                rpc_url: Some("http://localhost:8545".to_string()),
            }),
            l2: Some(RpcHealth {
                healthy: false,
                block_number: Some(serde_json::json!(50)),
                chain_id: Some(serde_json::json!(17001)),
                rpc_url: None,
            }),
        };
        let h = build_health(&m);
        assert!(h.l1_healthy);
        assert!(!h.l2_healthy);
        assert_eq!(h.l1_block_number, Some(serde_json::json!(100)));
        assert_eq!(h.l2_chain_id, Some(serde_json::json!(17001)));
    }

    #[test]
    fn test_build_health_none() {
        let m = MonitoringInfo { l1: None, l2: None };
        let h = build_health(&m);
        assert!(!h.l1_healthy);
        assert!(!h.l2_healthy);
        assert!(h.l1_block_number.is_none());
        assert!(h.l2_block_number.is_none());
    }

    // ── Unit Tests: UnifiedL2State core ──

    #[test]
    fn test_empty_state() {
        let state = UnifiedL2State::new();
        assert!(state.get_all().is_empty());
        assert!(state.get_by_id("nonexistent").is_none());
    }

    #[test]
    fn test_context_json_empty() {
        let state = UnifiedL2State::new();
        let ctx = state.to_context_json();
        assert_eq!(ctx["total_count"], 0);
        assert!(ctx["my_appchains"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_context_json_lists_all() {
        let state = UnifiedL2State::new();
        {
            let mut snap = state.snapshot.write().unwrap();
            snap.items = vec![
                make_deployment("d1", "Deploy 1", L2Status::Running),
                make_deployment("d2", "Deploy 2", L2Status::Stopped),
            ];
        }
        let ctx = state.to_context_json();
        assert_eq!(ctx["total_count"], 2);
        let all = ctx["my_appchains"].as_array().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0]["name"], "Deploy 1");
        assert_eq!(all[1]["name"], "Deploy 2");
    }

    #[test]
    fn test_get_by_id_found() {
        let state = UnifiedL2State::new();
        {
            let mut snap = state.snapshot.write().unwrap();
            snap.items = vec![
                make_appchain("a1", "Chain A", L2Status::Running),
                make_deployment("d1", "Deploy D", L2Status::Stopped),
            ];
        }
        let found = state.get_by_id("a1").unwrap();
        assert_eq!(found.name, "Chain A");
        assert_eq!(found.source, L2Source::Appchain);
    }

    #[test]
    fn test_get_by_id_not_found() {
        let state = UnifiedL2State::new();
        {
            let mut snap = state.snapshot.write().unwrap();
            snap.items = vec![make_appchain("a1", "Chain A", L2Status::Running)];
        }
        assert!(state.get_by_id("nonexistent").is_none());
    }

    #[test]
    fn test_subscribe_events() {
        let state = UnifiedL2State::new();
        let mut rx = state.subscribe_events();
        state.emit_event(L2Event {
            event_type: "test".to_string(),
            l2_id: "id".to_string(),
            l2_name: "name".to_string(),
            source_type: "appchain".to_string(),
            detail: "detail".to_string(),
            timestamp: "now".to_string(),
        });
        let event = rx.try_recv().unwrap();
        assert_eq!(event.event_type, "test");
        assert_eq!(event.l2_id, "id");
    }

    #[test]
    fn test_multiple_subscribers_receive_events() {
        let state = UnifiedL2State::new();
        let mut rx1 = state.subscribe_events();
        let mut rx2 = state.subscribe_events();
        state.emit_event(L2Event {
            event_type: "broadcast".to_string(),
            l2_id: "x".to_string(),
            l2_name: "x".to_string(),
            source_type: "appchain".to_string(),
            detail: "".to_string(),
            timestamp: "".to_string(),
        });
        assert_eq!(rx1.try_recv().unwrap().event_type, "broadcast");
        assert_eq!(rx2.try_recv().unwrap().event_type, "broadcast");
    }

    // ── Unit Tests: diff_and_emit ──

    #[test]
    fn test_diff_detects_new_item() {
        let (tx, mut rx) = broadcast::channel(16);
        let new = vec![make_appchain("test-1", "Test Chain", L2Status::Created)];
        diff_and_emit(&[], &new, &tx);
        let event = rx.try_recv().unwrap();
        assert_eq!(event.event_type, "created");
        assert_eq!(event.l2_id, "test-1");
        assert_eq!(event.source_type, "appchain");
    }

    #[test]
    fn test_diff_detects_status_change() {
        let (tx, mut rx) = broadcast::channel(16);
        let old = vec![make_appchain("test-1", "Test Chain", L2Status::Running)];
        let new = vec![make_appchain("test-1", "Test Chain", L2Status::Stopped)];
        diff_and_emit(&old, &new, &tx);
        let event = rx.try_recv().unwrap();
        assert_eq!(event.event_type, "status_changed");
        assert!(event.detail.contains("Running"));
        assert!(event.detail.contains("Stopped"));
    }

    #[test]
    fn test_diff_detects_deletion() {
        let (tx, mut rx) = broadcast::channel(16);
        let old = vec![make_appchain("test-1", "Test Chain", L2Status::Running)];
        diff_and_emit(&old, &[], &tx);
        let event = rx.try_recv().unwrap();
        assert_eq!(event.event_type, "deleted");
        assert_eq!(event.l2_id, "test-1");
    }

    #[test]
    fn test_diff_no_change_no_events() {
        let (tx, mut rx) = broadcast::channel(16);
        let items = vec![make_appchain("test-1", "Test Chain", L2Status::Running)];
        diff_and_emit(&items, &items.clone(), &tx);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_diff_multiple_changes() {
        let (tx, mut rx) = broadcast::channel(16);
        let old = vec![
            make_appchain("a1", "Chain A", L2Status::Running),
            make_appchain("a2", "Chain B", L2Status::Stopped),
            make_appchain("a3", "Chain C", L2Status::Running),
        ];
        let new = vec![
            make_appchain("a1", "Chain A", L2Status::Stopped),   // status changed
            // a2 deleted
            make_appchain("a3", "Chain C", L2Status::Running),   // no change
            make_appchain("a4", "Chain D", L2Status::Created),   // new
        ];
        diff_and_emit(&old, &new, &tx);

        let e1 = rx.try_recv().unwrap();
        assert_eq!(e1.event_type, "status_changed");
        assert_eq!(e1.l2_id, "a1");

        let e2 = rx.try_recv().unwrap();
        assert_eq!(e2.event_type, "created");
        assert_eq!(e2.l2_id, "a4");

        let e3 = rx.try_recv().unwrap();
        assert_eq!(e3.event_type, "deleted");
        assert_eq!(e3.l2_id, "a2");

        // No more events
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_diff_same_id_different_source_are_independent() {
        let (tx, mut rx) = broadcast::channel(16);
        let old = vec![make_appchain("same-id", "Appchain", L2Status::Running)];
        let new = vec![
            make_appchain("same-id", "Appchain", L2Status::Running),
            make_deployment("same-id", "Deployment", L2Status::Running),
        ];
        diff_and_emit(&old, &new, &tx);
        let event = rx.try_recv().unwrap();
        assert_eq!(event.event_type, "created");
        assert_eq!(event.source_type, "deployment");
        // No more events (appchain unchanged)
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_diff_health_change() {
        let (tx, mut rx) = broadcast::channel(16);
        let old = vec![make_deployment_with_health("d1", "Deploy", true, true)];
        let new = vec![make_deployment_with_health("d1", "Deploy", true, false)];
        diff_and_emit(&old, &new, &tx);
        let event = rx.try_recv().unwrap();
        assert_eq!(event.event_type, "health_changed");
        assert!(event.detail.contains("L2: true → false"));
    }

    #[test]
    fn test_diff_health_no_change() {
        let (tx, mut rx) = broadcast::channel(16);
        let old = vec![make_deployment_with_health("d1", "Deploy", true, true)];
        let new = vec![make_deployment_with_health("d1", "Deploy", true, true)];
        diff_and_emit(&old, &new, &tx);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_diff_health_added_first_time() {
        let (tx, mut rx) = broadcast::channel(16);
        let old = vec![make_deployment("d1", "Deploy", L2Status::Running)]; // no health
        let new = vec![make_deployment_with_health("d1", "Deploy", true, true)]; // health added
        diff_and_emit(&old, &new, &tx);
        // Health first appeared → emits health_changed with "(initial)"
        let event = rx.try_recv().unwrap();
        assert_eq!(event.event_type, "health_changed");
        assert!(event.detail.contains("initial"));
    }

    // ── Unit Tests: L2Container ──

    #[test]
    fn test_container_info_serializes() {
        let c = L2Container {
            service: "l2".to_string(),
            state: "running".to_string(),
            status: "Up 5 minutes".to_string(),
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"service\":\"l2\""));
        assert!(json.contains("\"state\":\"running\""));
    }

    // ── E2E-like Tests: Full state lifecycle ──

    #[test]
    fn test_e2e_state_write_read_cycle() {
        let state = UnifiedL2State::new();
        let mut rx = state.subscribe_events();

        // Initially empty
        assert!(state.get_all().is_empty());
        assert_eq!(state.to_context_json()["total_count"], 0);

        // Simulate first refresh: 2 appchains appear
        {
            let mut snap = state.snapshot.write().unwrap();
            let old = snap.items.clone();
            snap.items = vec![
                make_appchain("a1", "Chain A", L2Status::Running),
                make_appchain("a2", "Chain B", L2Status::Created),
            ];
            diff_and_emit(&old, &snap.items, &state.event_tx);
        }
        assert_eq!(state.get_all().len(), 2);
        assert_eq!(state.to_context_json()["total_count"], 2);

        // 2 created events
        assert_eq!(rx.try_recv().unwrap().event_type, "created");
        assert_eq!(rx.try_recv().unwrap().event_type, "created");
        assert!(rx.try_recv().is_err());

        // Simulate second refresh: a1 stops, a2 starts, d1 appears
        {
            let mut snap = state.snapshot.write().unwrap();
            let old = snap.items.clone();
            snap.items = vec![
                make_appchain("a1", "Chain A", L2Status::Stopped),
                make_appchain("a2", "Chain B", L2Status::Running),
                make_deployment("d1", "Deploy 1", L2Status::Running),
            ];
            diff_and_emit(&old, &snap.items, &state.event_tx);
        }
        assert_eq!(state.get_all().len(), 3);
        assert_eq!(state.to_context_json()["total_count"], 3);

        // a1 status_changed, a2 status_changed, d1 created
        let e1 = rx.try_recv().unwrap();
        assert_eq!(e1.l2_id, "a1");
        assert_eq!(e1.event_type, "status_changed");

        let e2 = rx.try_recv().unwrap();
        assert_eq!(e2.l2_id, "a2");
        assert_eq!(e2.event_type, "status_changed");

        let e3 = rx.try_recv().unwrap();
        assert_eq!(e3.l2_id, "d1");
        assert_eq!(e3.event_type, "created");

        assert!(rx.try_recv().is_err());

        // Simulate third refresh: a1 deleted, d1 health degrades
        {
            let mut snap = state.snapshot.write().unwrap();
            let old = snap.items.clone();
            snap.items = vec![
                make_appchain("a2", "Chain B", L2Status::Running),
                make_deployment_with_health("d1", "Deploy 1", true, false),
            ];
            diff_and_emit(&old, &snap.items, &state.event_tx);
        }
        assert_eq!(state.get_all().len(), 2);
        assert!(state.get_by_id("a1").is_none());

        // d1 health appeared (None→Some) → health_changed with "(initial)"
        let e4 = rx.try_recv().unwrap();
        assert_eq!(e4.l2_id, "d1");
        assert_eq!(e4.event_type, "health_changed");
        assert!(e4.detail.contains("initial"));

        // a1 deleted
        let e5 = rx.try_recv().unwrap();
        assert_eq!(e5.l2_id, "a1");
        assert_eq!(e5.event_type, "deleted");

        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_e2e_context_json_includes_all_fields() {
        let state = UnifiedL2State::new();
        {
            let mut snap = state.snapshot.write().unwrap();
            let mut dep = make_deployment("d1", "Full Deploy", L2Status::Running);
            dep.health = Some(L2Health {
                l1_healthy: true,
                l2_healthy: true,
                l1_block_number: Some(serde_json::json!(999)),
                l2_block_number: Some(serde_json::json!(500)),
                l1_chain_id: Some(serde_json::json!(1)),
                l2_chain_id: Some(serde_json::json!(17001)),
            });
            dep.contracts = Some(L2Contracts {
                bridge: Some("0xBridge".to_string()),
                proposer: Some("0xProposer".to_string()),
                timelock: Some("0xTimelock".to_string()),
                sp1_verifier: None,
            });
            dep.containers = Some(vec![
                L2Container { service: "l1".to_string(), state: "running".to_string(), status: "Up".to_string() },
                L2Container { service: "l2".to_string(), state: "running".to_string(), status: "Up".to_string() },
            ]);
            snap.items = vec![dep];
        }

        let ctx = state.to_context_json();
        let dep = &ctx["my_appchains"][0];
        assert_eq!(dep["name"], "Full Deploy");
        assert_eq!(dep["status"], "running");
        assert_eq!(dep["health"]["l1_healthy"], true);
        assert_eq!(dep["health"]["l1_block_number"], 999);
        assert_eq!(dep["contracts"]["bridge"], "0xBridge");
        assert_eq!(dep["contracts"]["proposer"], "0xProposer");
        assert_eq!(dep["containers"].as_array().unwrap().len(), 2);
        assert_eq!(dep["phase"], "running");
    }

    #[test]
    fn test_e2e_concurrent_read_write() {
        use std::sync::Arc;
        use std::thread;

        let state = Arc::new(UnifiedL2State::new());

        // Writer thread
        let state_w = state.clone();
        let writer = thread::spawn(move || {
            for i in 0..100 {
                let mut snap = state_w.snapshot.write().unwrap();
                snap.items = vec![make_appchain(
                    &format!("a-{}", i),
                    &format!("Chain {}", i),
                    if i % 2 == 0 { L2Status::Running } else { L2Status::Stopped },
                )];
            }
        });

        // Reader threads
        let mut readers = vec![];
        for _ in 0..4 {
            let state_r = state.clone();
            readers.push(thread::spawn(move || {
                for _ in 0..100 {
                    let all = state_r.get_all();
                    // Should always be consistent (0 or 1 items, never corrupt)
                    assert!(all.len() <= 1);
                    let _ = state_r.to_context_json();
                    let _ = state_r.get_by_id("a-50");
                }
            }));
        }

        writer.join().unwrap();
        for r in readers {
            r.join().unwrap();
        }

        // Final state should have exactly 1 item
        assert_eq!(state.get_all().len(), 1);
    }

    #[test]
    fn test_e2e_event_ordering() {
        let (tx, mut rx) = broadcast::channel(64);

        // Simulate a full lifecycle: create → start → health degrade → stop → delete
        let v0: Vec<L2Info> = vec![];
        let v1 = vec![make_appchain("a1", "Chain", L2Status::Created)];
        let v2 = vec![make_appchain("a1", "Chain", L2Status::Running)];
        let v3 = vec![make_appchain("a1", "Chain", L2Status::Error)];
        let v4 = vec![make_appchain("a1", "Chain", L2Status::Stopped)];
        let v5: Vec<L2Info> = vec![];

        diff_and_emit(&v0, &v1, &tx); // created
        diff_and_emit(&v1, &v2, &tx); // created → running
        diff_and_emit(&v2, &v3, &tx); // running → error
        diff_and_emit(&v3, &v4, &tx); // error → stopped
        diff_and_emit(&v4, &v5, &tx); // deleted

        let events: Vec<String> = (0..5).map(|_| rx.try_recv().unwrap().event_type).collect();
        assert_eq!(events, vec!["created", "status_changed", "status_changed", "status_changed", "deleted"]);
        assert!(rx.try_recv().is_err());
    }

    // ── Unit Tests: context JSON (my_appchains flat structure) ──

    #[test]
    fn test_context_json_flat_structure() {
        let state = UnifiedL2State::new();
        {
            let mut snap = state.snapshot.write().unwrap();
            snap.items = vec![
                make_deployment("d1", "Sepolia-2", L2Status::Stopped),
                make_deployment("d2", "Local-1", L2Status::Running),
            ];
        }
        let ctx = state.to_context_json();

        // Flat list, no appchains/deployments split
        assert_eq!(ctx["total_count"], 2);
        assert!(ctx.get("appchains").is_none());
        assert!(ctx.get("deployments").is_none());

        let all = ctx["my_appchains"].as_array().unwrap();
        assert_eq!(all[0]["name"], "Sepolia-2");
        assert_eq!(all[0]["status"], "stopped");
        assert_eq!(all[1]["name"], "Local-1");
        assert_eq!(all[1]["status"], "running");
    }

    #[test]
    fn test_context_json_includes_source_field() {
        let state = UnifiedL2State::new();
        {
            let mut snap = state.snapshot.write().unwrap();
            snap.items = vec![make_deployment("d1", "Deploy", L2Status::Running)];
        }
        let ctx = state.to_context_json();
        let item = &ctx["my_appchains"][0];
        assert_eq!(item["source"], "deployment");
    }

    // ── Unit Tests: do_refresh skips AppchainManager ──

    #[test]
    fn test_refresh_only_collects_deployments() {
        // After removing AppchainManager collection, the L2Source::Appchain
        // items should never appear in the state from do_refresh.
        // This is tested indirectly: any items in state must be Deployment source.
        let state = UnifiedL2State::new();
        {
            let mut snap = state.snapshot.write().unwrap();
            // Simulate what do_refresh would produce (only deployments)
            snap.items = vec![
                make_deployment("d1", "Chain-A", L2Status::Running),
                make_deployment("d2", "Chain-B", L2Status::Stopped),
            ];
        }
        let all = state.get_all();
        assert!(all.iter().all(|l| l.source == L2Source::Deployment));
        assert_eq!(all.len(), 2);
    }

    // ── Unit Tests: deployment status from containers ──

    #[test]
    fn test_deployment_status_partial() {
        let state = UnifiedL2State::new();
        {
            let mut snap = state.snapshot.write().unwrap();
            let mut dep = make_deployment("d1", "Partial", L2Status::Partial);
            dep.containers = Some(vec![
                L2Container { service: "l1".into(), state: "running".into(), status: "Up".into() },
                L2Container { service: "l2".into(), state: "exited".into(), status: "Exited (1)".into() },
            ]);
            snap.items = vec![dep];
        }
        let l2 = state.get_by_id("d1").unwrap();
        assert_eq!(l2.status, L2Status::Partial);
        assert_eq!(l2.containers.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_deployment_status_transitions_emit_events() {
        let (tx, mut rx) = broadcast::channel(16);

        // stopped → running
        let old = vec![make_deployment("d1", "Chain", L2Status::Stopped)];
        let new = vec![make_deployment("d1", "Chain", L2Status::Running)];
        diff_and_emit(&old, &new, &tx);
        let e = rx.try_recv().unwrap();
        assert_eq!(e.event_type, "status_changed");
        assert!(e.detail.contains("Stopped"));
        assert!(e.detail.contains("Running"));
        assert_eq!(e.source_type, "deployment");

        // running → partial
        let old = new;
        let new = vec![make_deployment("d1", "Chain", L2Status::Partial)];
        diff_and_emit(&old, &new, &tx);
        let e = rx.try_recv().unwrap();
        assert!(e.detail.contains("Partial"));

        // partial → error
        let old = new;
        let new = vec![make_deployment("d1", "Chain", L2Status::Error)];
        diff_and_emit(&old, &new, &tx);
        let e = rx.try_recv().unwrap();
        assert!(e.detail.contains("Error"));

        assert!(rx.try_recv().is_err());
    }

    // ── E2E: deployment-only lifecycle ──

    #[test]
    fn test_e2e_deployment_only_lifecycle() {
        let state = UnifiedL2State::new();
        let mut rx = state.subscribe_events();

        // Phase 1: empty
        assert_eq!(state.to_context_json()["total_count"], 0);

        // Phase 2: two deployments appear
        {
            let mut snap = state.snapshot.write().unwrap();
            let old = snap.items.clone();
            snap.items = vec![
                make_deployment("d1", "Sepolia-2", L2Status::Stopped),
                make_deployment("d2", "Local-1", L2Status::Running),
            ];
            diff_and_emit(&old, &snap.items, &state.event_tx);
        }
        assert_eq!(state.get_all().len(), 2);
        assert_eq!(state.to_context_json()["total_count"], 2);
        assert_eq!(rx.try_recv().unwrap().event_type, "created");
        assert_eq!(rx.try_recv().unwrap().event_type, "created");
        assert!(rx.try_recv().is_err());

        // Phase 3: d1 starts, d2 stops
        {
            let mut snap = state.snapshot.write().unwrap();
            let old = snap.items.clone();
            snap.items = vec![
                make_deployment("d1", "Sepolia-2", L2Status::Running),
                make_deployment("d2", "Local-1", L2Status::Stopped),
            ];
            diff_and_emit(&old, &snap.items, &state.event_tx);
        }
        let e1 = rx.try_recv().unwrap();
        assert_eq!(e1.l2_id, "d1");
        assert!(e1.detail.contains("Running"));
        let e2 = rx.try_recv().unwrap();
        assert_eq!(e2.l2_id, "d2");
        assert!(e2.detail.contains("Stopped"));
        assert!(rx.try_recv().is_err());

        // Phase 4: d2 deleted
        {
            let mut snap = state.snapshot.write().unwrap();
            let old = snap.items.clone();
            snap.items = vec![
                make_deployment("d1", "Sepolia-2", L2Status::Running),
            ];
            diff_and_emit(&old, &snap.items, &state.event_tx);
        }
        assert_eq!(state.get_all().len(), 1);
        let e3 = rx.try_recv().unwrap();
        assert_eq!(e3.l2_id, "d2");
        assert_eq!(e3.event_type, "deleted");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_e2e_mixed_appchain_deployment_lifecycle() {
        let (tx, mut rx) = broadcast::channel(64);

        // Step 1: appchain + deployment created together
        let old: Vec<L2Info> = vec![];
        let new = vec![
            make_appchain("a1", "My Appchain", L2Status::Created),
            make_deployment("d1", "Docker L2", L2Status::SettingUp),
        ];
        diff_and_emit(&old, &new, &tx);
        let e1 = rx.try_recv().unwrap();
        assert_eq!(e1.source_type, "appchain");
        let e2 = rx.try_recv().unwrap();
        assert_eq!(e2.source_type, "deployment");

        // Step 2: deployment finishes setup, appchain starts
        let old = new;
        let new = vec![
            make_appchain("a1", "My Appchain", L2Status::Running),
            make_deployment("d1", "Docker L2", L2Status::Running),
        ];
        diff_and_emit(&old, &new, &tx);
        let e3 = rx.try_recv().unwrap();
        assert_eq!(e3.event_type, "status_changed");
        assert!(e3.detail.contains("Running"));
        let e4 = rx.try_recv().unwrap();
        assert_eq!(e4.event_type, "status_changed");

        // Step 3: appchain crashes, deployment still running
        let old = new;
        let new = vec![
            make_appchain("a1", "My Appchain", L2Status::Error),
            make_deployment("d1", "Docker L2", L2Status::Running),
        ];
        diff_and_emit(&old, &new, &tx);
        let e5 = rx.try_recv().unwrap();
        assert_eq!(e5.l2_id, "a1");
        assert_eq!(e5.event_type, "status_changed");
        assert!(e5.detail.contains("Error"));
        assert!(rx.try_recv().is_err()); // deployment unchanged
    }
}
