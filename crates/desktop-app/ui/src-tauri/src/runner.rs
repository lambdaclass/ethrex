use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::appchain_manager::{AppchainManager, AppchainStatus, StepStatus};

/// Stores running child processes keyed by appchain ID
pub struct ProcessRunner {
    pub children: Mutex<HashMap<String, Arc<Mutex<Child>>>>,
}

impl ProcessRunner {
    pub fn new() -> Self {
        Self {
            children: Mutex::new(HashMap::new()),
        }
    }

    /// Find the workspace root (where the top-level Cargo.toml is)
    fn find_workspace_root() -> Option<PathBuf> {
        // The desktop app is at: <workspace>/crates/desktop-app/ui/src-tauri/
        // Try env var first
        if let Ok(root) = std::env::var("ETHREX_WORKSPACE_ROOT") {
            let p = PathBuf::from(root);
            if p.join("Cargo.toml").exists() {
                return Some(p);
            }
        }

        // Walk up from the current executable or current dir
        let start = std::env::current_exe()
            .ok()
            .or_else(|| std::env::current_dir().ok())?;

        let mut dir = start.as_path();
        for _ in 0..10 {
            // Check if this looks like the ethrex workspace root
            let cargo_toml = dir.join("Cargo.toml");
            if cargo_toml.exists() && dir.join("crates").join("l2").exists() {
                return Some(dir.to_path_buf());
            }
            dir = dir.parent()?;
        }
        None
    }

    /// Find the ethrex binary, checking multiple locations
    fn find_ethrex_binary() -> Option<PathBuf> {
        // 1. ETHREX_BIN env var
        if let Ok(bin) = std::env::var("ETHREX_BIN") {
            let p = PathBuf::from(&bin);
            if p.exists() {
                return Some(p);
            }
        }

        // 2. Check PATH
        if let Ok(output) = std::process::Command::new("which")
            .arg("ethrex")
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(PathBuf::from(path));
                }
            }
        }

        // 3. Check workspace target directories
        if let Some(root) = Self::find_workspace_root() {
            for profile in &["release", "debug"] {
                let bin = root.join("target").join(profile).join("ethrex");
                if bin.exists() {
                    return Some(bin);
                }
            }
        }

        None
    }

    /// Build the ethrex binary with required features
    async fn build_ethrex(
        am: &AppchainManager,
        chain_id: &str,
    ) -> Result<PathBuf, String> {
        let workspace_root =
            Self::find_workspace_root().ok_or("Cannot find ethrex workspace root")?;

        am.add_log(
            chain_id,
            "Building ethrex binary (this may take a few minutes)...".to_string(),
        );

        let output = Command::new("cargo")
            .args([
                "build",
                "--release",
                "--features",
                "l2,dev,rocksdb",
            ])
            .current_dir(&workspace_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| format!("Failed to run cargo build: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("cargo build failed: {stderr}"));
        }

        let bin = workspace_root.join("target/release/ethrex");
        if bin.exists() {
            am.add_log(chain_id, "Build complete.".to_string());
            Ok(bin)
        } else {
            Err("Binary not found after build".to_string())
        }
    }

    /// Start the local dev mode: `ethrex l2 --dev --no-monitor`
    pub async fn start_local_dev(
        runner: Arc<ProcessRunner>,
        am: Arc<AppchainManager>,
        chain_id: String,
    ) {
        // Find or build the binary
        let binary = match Self::find_ethrex_binary() {
            Some(bin) => {
                am.add_log(&chain_id, format!("Found ethrex binary: {}", bin.display()));
                bin
            }
            None => {
                am.add_log(&chain_id, "ethrex binary not found, building...".to_string());
                match Self::build_ethrex(&am, &chain_id).await {
                    Ok(bin) => bin,
                    Err(e) => {
                        am.update_step_status(&chain_id, "dev", StepStatus::Error);
                        am.set_setup_error(&chain_id, format!("Build failed: {e}"));
                        am.update_status(&chain_id, AppchainStatus::Error);
                        return;
                    }
                }
            }
        };

        // Get workspace root for working directory
        let workspace_root = match Self::find_workspace_root() {
            Some(root) => root,
            None => {
                am.update_step_status(&chain_id, "dev", StepStatus::Error);
                am.set_setup_error(&chain_id, "Cannot find workspace root".to_string());
                am.update_status(&chain_id, AppchainStatus::Error);
                return;
            }
        };

        // Spawn ethrex l2 --dev --no-monitor
        am.add_log(&chain_id, format!("Spawning: {} l2 --dev --no-monitor", binary.display()));

        let child_result = Command::new(&binary)
            .args(["l2", "--dev", "--no-monitor"])
            .current_dir(&workspace_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn();

        let mut child = match child_result {
            Ok(c) => c,
            Err(e) => {
                am.update_step_status(&chain_id, "dev", StepStatus::Error);
                am.set_setup_error(&chain_id, format!("Failed to spawn ethrex: {e}"));
                am.update_status(&chain_id, AppchainStatus::Error);
                return;
            }
        };

        // Take stdout and stderr
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // Store child handle
        let child_handle = Arc::new(Mutex::new(child));
        {
            let mut children = runner.children.lock().await;
            children.insert(chain_id.clone(), child_handle.clone());
        }

        // Spawn readers for stdout and stderr
        let am_stdout = am.clone();
        let chain_id_stdout = chain_id.clone();
        let stdout_handle = if let Some(stdout) = stdout {
            Some(tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    Self::process_dev_log_line(&am_stdout, &chain_id_stdout, &line);
                }
            }))
        } else {
            None
        };

        let am_stderr = am.clone();
        let chain_id_stderr = chain_id.clone();
        let stderr_handle = if let Some(stderr) = stderr {
            Some(tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    // stderr also contains tracing logs
                    Self::process_dev_log_line(&am_stderr, &chain_id_stderr, &line);
                }
            }))
        } else {
            None
        };

        // Wait for child to exit
        let exit_status = {
            let mut child = child_handle.lock().await;
            child.wait().await
        };

        // Wait for readers to finish
        if let Some(h) = stdout_handle {
            let _ = h.await;
        }
        if let Some(h) = stderr_handle {
            let _ = h.await;
        }

        // Clean up
        {
            let mut children = runner.children.lock().await;
            children.remove(&chain_id);
        }

        match exit_status {
            Ok(status) if status.success() => {
                am.add_log(&chain_id, "ethrex process exited normally.".to_string());
                am.update_status(&chain_id, AppchainStatus::Stopped);
            }
            Ok(status) => {
                let msg = format!("ethrex process exited with status: {status}");
                am.add_log(&chain_id, msg.clone());
                // Only mark as error if we haven't reached running state
                let progress = am.get_setup_progress(&chain_id);
                let all_done = progress
                    .map(|p| {
                        p.steps
                            .iter()
                            .all(|s| s.status == StepStatus::Done || s.status == StepStatus::Skipped)
                    })
                    .unwrap_or(false);
                if !all_done {
                    am.set_setup_error(&chain_id, msg);
                    am.update_status(&chain_id, AppchainStatus::Error);
                }
            }
            Err(e) => {
                let msg = format!("Failed to wait for ethrex: {e}");
                am.add_log(&chain_id, msg.clone());
                am.set_setup_error(&chain_id, msg);
                am.update_status(&chain_id, AppchainStatus::Error);
            }
        }
    }

    /// Parse a log line from ethrex l2 --dev and update progress
    fn process_dev_log_line(am: &AppchainManager, chain_id: &str, line: &str) {
        // Add all lines to logs (limit to last 500)
        am.add_log(chain_id, line.to_string());

        // Detect stage transitions from the println! output in command.rs
        if line.contains("Removing L1 and L2 databases") {
            am.add_log(chain_id, "[stage] Cleaning up old databases".to_string());
        } else if line.contains("Initializing L1") {
            am.add_log(chain_id, "[stage] Starting local L1 node".to_string());
        } else if line.contains("Deploying contracts") {
            am.add_log(
                chain_id,
                "[stage] Deploying L2 contracts to L1".to_string(),
            );
        } else if line.contains("Initializing L2") {
            am.add_log(chain_id, "[stage] Starting L2 node".to_string());
        }

        // Detect when L2 is fully running
        // The L2 is considered ready when we see RPC server started
        if line.contains("Starting rpc server")
            || line.contains("RPC server started")
            || line.contains("Blockchain is ready")
            || line.contains("started on")
            || line.contains("listening on")
        {
            am.update_step_status(chain_id, "dev", StepStatus::Done);
            am.advance_step(chain_id);
            am.update_step_status(chain_id, "done", StepStatus::Done);
            am.update_status(chain_id, AppchainStatus::Running);
            am.add_log(chain_id, "[ready] Appchain is running!".to_string());
        }

        // Detect errors
        if line.contains("panic") || line.contains("FATAL") || line.contains("Error:") {
            // Don't immediately mark as error for warnings
            if line.contains("panic") || line.contains("FATAL") {
                am.update_step_status(chain_id, "dev", StepStatus::Error);
                am.set_setup_error(chain_id, line.to_string());
                am.update_status(chain_id, AppchainStatus::Error);
            }
        }
    }

    /// Stop a running appchain process
    pub async fn stop_chain(&self, chain_id: &str) -> Result<(), String> {
        let mut children = self.children.lock().await;
        if let Some(child_handle) = children.remove(chain_id) {
            let mut child = child_handle.lock().await;
            child.kill().await.map_err(|e| format!("Failed to kill process: {e}"))?;
            Ok(())
        } else {
            Err(format!("No running process for chain: {chain_id}"))
        }
    }

    /// Check if a chain has a running process
    pub async fn is_running(&self, chain_id: &str) -> bool {
        let children = self.children.lock().await;
        children.contains_key(chain_id)
    }
}
