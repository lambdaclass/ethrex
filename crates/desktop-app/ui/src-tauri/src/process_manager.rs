use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProcessStatus {
    Stopped,
    Starting,
    Running,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub name: String,
    pub status: ProcessStatus,
    pub pid: Option<u32>,
}

pub struct ProcessManager {
    pub processes: Mutex<HashMap<String, NodeInfo>>,
}

impl ProcessManager {
    pub fn new() -> Self {
        let mut processes = HashMap::new();
        for name in &["ethrex-l1", "ethrex-l2", "prover", "sequencer"] {
            processes.insert(
                name.to_string(),
                NodeInfo {
                    name: name.to_string(),
                    status: ProcessStatus::Stopped,
                    pid: None,
                },
            );
        }
        Self {
            processes: Mutex::new(processes),
        }
    }

    pub fn get_all(&self) -> Vec<NodeInfo> {
        let procs = self.processes.lock().unwrap();
        procs.values().cloned().collect()
    }

    pub fn get_status(&self, name: &str) -> Option<NodeInfo> {
        let procs = self.processes.lock().unwrap();
        procs.get(name).cloned()
    }

    pub fn set_status(&self, name: &str, status: ProcessStatus, pid: Option<u32>) {
        let mut procs = self.processes.lock().unwrap();
        if let Some(info) = procs.get_mut(name) {
            info.status = status;
            info.pid = pid;
        }
    }
}
