use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    Local,
    Testnet,
    Mainnet,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AppchainStatus {
    Created,
    SettingUp,
    Running,
    Stopped,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    Pending,
    InProgress,
    Done,
    Error,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupStep {
    pub id: String,
    pub label: String,
    pub status: StepStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupProgress {
    pub steps: Vec<SetupStep>,
    pub current_step: usize,
    pub logs: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppchainConfig {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub chain_id: u64,
    pub description: String,
    pub network_mode: NetworkMode,

    // Network
    pub l1_rpc_url: String,
    pub l2_rpc_port: u16,
    pub sequencer_mode: String,

    // Token / Prover
    pub native_token: String,
    pub prover_type: String,

    // Deploy result
    pub bridge_address: Option<String>,
    pub on_chain_proposer_address: Option<String>,

    // Public
    pub is_public: bool,
    pub hashtags: Vec<String>,

    // Status
    pub status: AppchainStatus,
    pub created_at: String,
}

pub struct AppchainManager {
    pub appchains: Mutex<HashMap<String, AppchainConfig>>,
    pub setup_progress: Mutex<HashMap<String, SetupProgress>>,
    pub config_dir: PathBuf,
}


impl AppchainManager {
    pub fn new() -> Self {
        let config_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".tokamak-appchain");
        fs::create_dir_all(&config_dir).ok();
        fs::create_dir_all(config_dir.join("chains")).ok();

        let mut manager = Self {
            appchains: Mutex::new(HashMap::new()),
            setup_progress: Mutex::new(HashMap::new()),
            config_dir,
        };
        manager.load_appchains();
        manager
    }

    fn appchains_file(&self) -> PathBuf {
        self.config_dir.join("appchains.json")
    }

    fn load_appchains(&mut self) {
        let path = self.appchains_file();
        if path.exists() {
            if let Ok(data) = fs::read_to_string(&path) {
                if let Ok(list) = serde_json::from_str::<Vec<AppchainConfig>>(&data) {
                    let mut map = self.appchains.lock().unwrap();
                    for chain in list {
                        map.insert(chain.id.clone(), chain);
                    }
                }
            }
        }
    }

    fn save_appchains(&self) {
        let map = self.appchains.lock().unwrap();
        let list: Vec<&AppchainConfig> = map.values().collect();
        if let Ok(json) = serde_json::to_string_pretty(&list) {
            fs::write(self.appchains_file(), json).ok();
        }
    }

    pub fn create_appchain(&self, config: AppchainConfig) -> Result<String, String> {
        let id = config.id.clone();

        // Save chain-specific dir
        let chain_dir = self.config_dir.join("chains").join(&id);
        fs::create_dir_all(&chain_dir).map_err(|e| e.to_string())?;

        let config_path = chain_dir.join("config.json");
        let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
        fs::write(config_path, json).map_err(|e| e.to_string())?;

        // Add to map
        {
            let mut map = self.appchains.lock().unwrap();
            map.insert(id.clone(), config);
        }
        self.save_appchains();

        Ok(id)
    }

    pub fn list_appchains(&self) -> Vec<AppchainConfig> {
        let map = self.appchains.lock().unwrap();
        map.values().cloned().collect()
    }

    pub fn get_appchain(&self, id: &str) -> Option<AppchainConfig> {
        let map = self.appchains.lock().unwrap();
        map.get(id).cloned()
    }

    pub fn update_status(&self, id: &str, status: AppchainStatus) {
        let mut map = self.appchains.lock().unwrap();
        if let Some(chain) = map.get_mut(id) {
            chain.status = status;
        }
        drop(map);
        self.save_appchains();
    }

    pub fn update_public(&self, id: &str, is_public: bool) {
        let mut map = self.appchains.lock().unwrap();
        if let Some(chain) = map.get_mut(id) {
            chain.is_public = is_public;
        }
        drop(map);
        self.save_appchains();
    }

    pub fn delete_appchain(&self, id: &str) -> Result<(), String> {
        {
            let mut map = self.appchains.lock().unwrap();
            map.remove(id);
        }
        self.save_appchains();

        // Remove chain dir
        let chain_dir = self.config_dir.join("chains").join(id);
        if chain_dir.exists() {
            fs::remove_dir_all(chain_dir).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn get_setup_progress(&self, id: &str) -> Option<SetupProgress> {
        let map = self.setup_progress.lock().unwrap();
        map.get(id).cloned()
    }

    pub fn init_setup_progress(&self, id: &str, network_mode: &NetworkMode, has_prover: bool) {
        let mut steps = vec![
            SetupStep {
                id: "config".to_string(),
                label: "Creating config".to_string(),
                status: StepStatus::Pending,
            },
        ];

        if *network_mode == NetworkMode::Local {
            steps.push(SetupStep {
                id: "dev".to_string(),
                label: "Starting L1 + Deploy + L2 (dev mode)".to_string(),
                status: StepStatus::Pending,
            });
        } else {
            steps.push(SetupStep {
                id: "l1_check".to_string(),
                label: "Checking L1 connection".to_string(),
                status: StepStatus::Pending,
            });
            steps.push(SetupStep {
                id: "deploy".to_string(),
                label: "Deploying contracts".to_string(),
                status: StepStatus::Pending,
            });
            steps.push(SetupStep {
                id: "l2".to_string(),
                label: "Starting L2 node".to_string(),
                status: StepStatus::Pending,
            });
        }

        if has_prover {
            steps.push(SetupStep {
                id: "prover".to_string(),
                label: "Starting prover".to_string(),
                status: StepStatus::Pending,
            });
        }

        steps.push(SetupStep {
            id: "done".to_string(),
            label: "Done".to_string(),
            status: StepStatus::Pending,
        });

        let progress = SetupProgress {
            steps,
            current_step: 0,
            logs: vec![],
            error: None,
        };

        let mut map = self.setup_progress.lock().unwrap();
        map.insert(id.to_string(), progress);
    }

    pub fn update_step_status(&self, id: &str, step_id: &str, status: StepStatus) {
        let mut map = self.setup_progress.lock().unwrap();
        if let Some(progress) = map.get_mut(id) {
            for step in &mut progress.steps {
                if step.id == step_id {
                    step.status = status;
                    break;
                }
            }
        }
    }

    pub fn advance_step(&self, id: &str) {
        let mut map = self.setup_progress.lock().unwrap();
        if let Some(progress) = map.get_mut(id) {
            if progress.current_step < progress.steps.len() - 1 {
                progress.current_step += 1;
            }
        }
    }

    pub fn add_log(&self, id: &str, log: String) {
        let mut map = self.setup_progress.lock().unwrap();
        if let Some(progress) = map.get_mut(id) {
            progress.logs.push(log);
            // Keep only the last 500 log lines
            if progress.logs.len() > 500 {
                let drain_count = progress.logs.len() - 500;
                progress.logs.drain(..drain_count);
            }
        }
    }

    pub fn set_setup_error(&self, id: &str, error: String) {
        let mut map = self.setup_progress.lock().unwrap();
        if let Some(progress) = map.get_mut(id) {
            progress.error = Some(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_manager() -> AppchainManager {
        let dir = std::env::temp_dir().join(format!("tokamak-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("chains")).unwrap();
        AppchainManager {
            appchains: Mutex::new(HashMap::new()),
            setup_progress: Mutex::new(HashMap::new()),
            config_dir: dir,
        }
    }

    fn sample_config(id: &str) -> AppchainConfig {
        AppchainConfig {
            id: id.to_string(),
            name: "Test Chain".to_string(),
            icon: "🔷".to_string(),
            chain_id: 17001,
            description: "test".to_string(),
            network_mode: NetworkMode::Local,
            l1_rpc_url: "http://localhost:8545".to_string(),
            l2_rpc_port: 1729,
            sequencer_mode: "standalone".to_string(),
            native_token: "TON".to_string(),
            prover_type: "sp1".to_string(),
            bridge_address: None,
            on_chain_proposer_address: None,
            is_public: false,
            hashtags: vec!["test".to_string()],
            status: AppchainStatus::Created,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_create_and_list() {
        let am = test_manager();
        let config = sample_config("chain-1");
        am.create_appchain(config).unwrap();

        let list = am.list_appchains();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Test Chain");
    }

    #[test]
    fn test_get_appchain() {
        let am = test_manager();
        am.create_appchain(sample_config("chain-2")).unwrap();

        assert!(am.get_appchain("chain-2").is_some());
        assert!(am.get_appchain("nonexistent").is_none());
    }

    #[test]
    fn test_update_status() {
        let am = test_manager();
        am.create_appchain(sample_config("chain-3")).unwrap();

        am.update_status("chain-3", AppchainStatus::Running);
        let chain = am.get_appchain("chain-3").unwrap();
        assert_eq!(chain.status, AppchainStatus::Running);
    }

    #[test]
    fn test_update_public() {
        let am = test_manager();
        am.create_appchain(sample_config("chain-4")).unwrap();

        assert!(!am.get_appchain("chain-4").unwrap().is_public);
        am.update_public("chain-4", true);
        assert!(am.get_appchain("chain-4").unwrap().is_public);
    }

    #[test]
    fn test_delete_appchain() {
        let am = test_manager();
        am.create_appchain(sample_config("chain-5")).unwrap();
        assert_eq!(am.list_appchains().len(), 1);

        am.delete_appchain("chain-5").unwrap();
        assert_eq!(am.list_appchains().len(), 0);
        assert!(am.get_appchain("chain-5").is_none());
    }

    #[test]
    fn test_setup_progress_local() {
        let am = test_manager();
        am.init_setup_progress("chain-6", &NetworkMode::Local, false);

        let progress = am.get_setup_progress("chain-6").unwrap();
        // Local without prover: config, dev, done = 3 steps
        assert_eq!(progress.steps.len(), 3);
        assert_eq!(progress.steps[0].id, "config");
        assert_eq!(progress.steps[1].id, "dev");
        assert_eq!(progress.steps[2].id, "done");
    }

    #[test]
    fn test_setup_progress_testnet_with_prover() {
        let am = test_manager();
        am.init_setup_progress("chain-7", &NetworkMode::Testnet, true);

        let progress = am.get_setup_progress("chain-7").unwrap();
        // Testnet with prover: config, l1_check, deploy, l2, prover, done = 6 steps
        assert_eq!(progress.steps.len(), 6);
        assert_eq!(progress.steps[1].id, "l1_check");
        assert_eq!(progress.steps[4].id, "prover");
    }

    #[test]
    fn test_step_status_and_advance() {
        let am = test_manager();
        am.init_setup_progress("chain-8", &NetworkMode::Local, false);

        am.update_step_status("chain-8", "config", StepStatus::Done);
        am.advance_step("chain-8");

        let progress = am.get_setup_progress("chain-8").unwrap();
        assert_eq!(progress.steps[0].status, StepStatus::Done);
        assert_eq!(progress.current_step, 1);
    }

    #[test]
    fn test_add_log_and_limit() {
        let am = test_manager();
        am.init_setup_progress("chain-9", &NetworkMode::Local, false);

        for i in 0..600 {
            am.add_log("chain-9", format!("log line {i}"));
        }

        let progress = am.get_setup_progress("chain-9").unwrap();
        assert_eq!(progress.logs.len(), 500);
        assert!(progress.logs[0].contains("100")); // first 100 were drained
    }

    #[test]
    fn test_set_setup_error() {
        let am = test_manager();
        am.init_setup_progress("chain-10", &NetworkMode::Local, false);

        am.set_setup_error("chain-10", "something failed".to_string());
        let progress = am.get_setup_progress("chain-10").unwrap();
        assert_eq!(progress.error, Some("something failed".to_string()));
    }

    #[test]
    fn test_persistence() {
        let dir = std::env::temp_dir().join(format!("tokamak-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("chains")).unwrap();

        // Create and save
        {
            let am = AppchainManager {
                appchains: Mutex::new(HashMap::new()),
                setup_progress: Mutex::new(HashMap::new()),
                config_dir: dir.clone(),
            };
            am.create_appchain(sample_config("persist-1")).unwrap();
        }

        // Load from disk
        {
            let mut am = AppchainManager {
                appchains: Mutex::new(HashMap::new()),
                setup_progress: Mutex::new(HashMap::new()),
                config_dir: dir.clone(),
            };
            am.load_appchains();
            let list = am.list_appchains();
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].id, "persist-1");
        }

        fs::remove_dir_all(&dir).ok();
    }
}
