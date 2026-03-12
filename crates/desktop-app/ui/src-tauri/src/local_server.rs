use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::Mutex;

/// Manages the local-server (Node.js Express) lifecycle.
/// Desktop spawns this on startup; it provides deployment management APIs.
pub struct LocalServer {
    child: Mutex<Option<Child>>,
    port: u16,
    /// When true, the watchdog will not auto-restart the server.
    watchdog_paused: AtomicBool,
}

impl LocalServer {
    pub fn new() -> Self {
        let port = std::env::var("LOCAL_SERVER_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(5002);
        Self {
            child: Mutex::new(None),
            port,
            watchdog_paused: AtomicBool::new(false),
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Find the local-server directory
    fn find_server_dir() -> Option<PathBuf> {
        // 1. Env var override
        if let Ok(dir) = std::env::var("LOCAL_SERVER_DIR") {
            let p = PathBuf::from(&dir);
            if p.join("server.js").exists() {
                return Some(p);
            }
        }

        // 2. Relative to the executable (production bundle)
        if let Ok(exe) = std::env::current_exe() {
            // macOS: .app/Contents/MacOS/exe → .app/Contents/Resources/
            if let Some(parent) = exe.parent() {
                if let Some(contents) = parent.parent() {
                    let resources = contents.join("Resources");
                    // Tauri converts "../../" in resource paths to "_up_/_up_/"
                    let candidates = [
                        resources.join("_up_/_up_/local-server"),
                        resources.join("local-server"),
                    ];
                    for candidate in &candidates {
                        if candidate.join("server.js").exists() {
                            return Some(candidate.clone());
                        }
                    }
                }
            }
        }

        // 3. Development: walk up from exe/cwd to find the workspace
        let start = std::env::current_exe()
            .ok()
            .or_else(|| std::env::current_dir().ok());

        if let Some(start) = start {
            let mut dir = start.as_path();
            for _ in 0..10 {
                let candidate = dir.join("crates/desktop-app/local-server");
                if candidate.join("server.js").exists() {
                    return Some(candidate);
                }
                match dir.parent() {
                    Some(p) => dir = p,
                    None => break,
                }
            }
        }

        None
    }

    /// Find the node binary.
    /// macOS GUI apps (.app bundles) don't inherit the user's shell PATH,
    /// so we probe well-known installation paths in addition to `which`.
    fn find_node() -> Option<PathBuf> {
        // 1. Explicit env var override
        if let Ok(node) = std::env::var("NODE_BIN") {
            let p = PathBuf::from(&node);
            if p.exists() {
                return Some(p);
            }
        }

        // 2. Well-known paths (macOS .app bundles lack shell PATH)
        let well_known: &[&str] = &[
            "/usr/local/bin/node",           // Homebrew (Intel) / official installer
            "/opt/homebrew/bin/node",         // Homebrew (Apple Silicon)
        ];
        for path in well_known {
            let p = PathBuf::from(path);
            if p.exists() {
                return Some(p);
            }
        }

        // 3. nvm — check common nvm directories for any installed version
        if let Ok(home) = std::env::var("HOME") {
            let nvm_dir = PathBuf::from(&home).join(".nvm/versions/node");
            if nvm_dir.is_dir() {
                // Pick the latest version directory
                if let Ok(entries) = std::fs::read_dir(&nvm_dir) {
                    let mut versions: Vec<PathBuf> = entries
                        .filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| p.join("bin/node").exists())
                        .collect();
                    versions.sort();
                    if let Some(latest) = versions.last() {
                        return Some(latest.join("bin/node"));
                    }
                }
            }
        }

        // 4. Fallback: `which node` (works in dev / terminal launches)
        if let Ok(output) = std::process::Command::new("which").arg("node").output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(PathBuf::from(path));
                }
            }
        }

        None
    }

    /// Start the local-server process
    pub async fn start(&self) -> Result<(), String> {
        let mut child_guard = self.child.lock().await;
        if child_guard.is_some() {
            return Ok(()); // Already running
        }

        let node = Self::find_node().ok_or("Node.js not found. Please install Node.js.")?;
        let server_dir =
            Self::find_server_dir().ok_or("local-server directory not found")?;

        // Auto-install dependencies if node_modules is missing
        if !server_dir.join("node_modules").exists() {
            log::info!(
                "node_modules not found in {}, running npm install...",
                server_dir.display()
            );
            // Derive npm path from node binary (npm lives in the same bin directory)
            let npm = {
                let npm_name = if cfg!(target_os = "windows") { "npm.cmd" } else { "npm" };
                let sibling = node.parent().map(|p| p.join(npm_name));
                match sibling {
                    Some(ref p) if p.exists() => p.clone(),
                    _ => PathBuf::from(npm_name), // fallback to PATH
                }
            };
            log::info!("Using npm: {}", npm.display());
            let install = std::process::Command::new(&npm)
                .arg("install")
                .current_dir(&server_dir)
                .output();
            match install {
                Ok(out) if out.status.success() => {
                    log::info!("npm install completed successfully");
                }
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    log::warn!("npm install finished with warnings: {}", stderr);
                }
                Err(e) => {
                    return Err(format!("Failed to run npm install: {e}"));
                }
            }
        }

        log::info!(
            "Starting local-server: {} {} (port {})",
            node.display(),
            server_dir.join("server.js").display(),
            self.port
        );

        let mut child = tokio::process::Command::new(&node)
            .arg("server.js")
            .current_dir(&server_dir)
            .env("LOCAL_SERVER_PORT", self.port.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to spawn local-server: {e}"))?;

        // Spawn log forwarders
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    log::info!("[local-server] {}", line);
                }
            });
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    log::warn!("[local-server] {}", line);
                }
            });
        }

        *child_guard = Some(child);
        Ok(())
    }

    /// Stop the local-server process
    pub async fn stop(&self) -> Result<(), String> {
        let mut child_guard = self.child.lock().await;
        if let Some(ref mut child) = *child_guard {
            child
                .kill()
                .await
                .map_err(|e| format!("Failed to kill local-server: {e}"))?;
        }
        *child_guard = None;
        Ok(())
    }

    /// Check if the server is healthy via HTTP
    pub async fn health_check(&self) -> bool {
        let url = format!("{}/api/health", self.url());
        match reqwest::get(&url).await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    /// Check if a child process is running (actually polls the process)
    pub async fn is_running(&self) -> bool {
        let mut child_guard = self.child.lock().await;
        if let Some(ref mut child) = *child_guard {
            match child.try_wait() {
                Ok(Some(_)) => {
                    // Process exited — clean up
                    *child_guard = None;
                    false
                }
                Ok(None) => true,  // Still running
                Err(_) => false,
            }
        } else {
            false
        }
    }

    /// Pause the watchdog (prevents auto-restart after explicit stop).
    pub fn pause_watchdog(&self) {
        self.watchdog_paused.store(true, Ordering::Relaxed);
    }

    /// Resume the watchdog (re-enables auto-restart).
    pub fn resume_watchdog(&self) {
        self.watchdog_paused.store(false, Ordering::Relaxed);
    }

    /// Start a watchdog that auto-restarts the server if it crashes.
    /// Should be called once after the initial start.
    pub fn start_watchdog(server: std::sync::Arc<Self>) {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                if server.watchdog_paused.load(Ordering::Relaxed) {
                    continue;
                }
                let running = server.is_running().await;
                if !running {
                    log::warn!("[watchdog] Local server process died, restarting...");
                    match server.start().await {
                        Ok(()) => log::info!("[watchdog] Local server restarted successfully"),
                        Err(e) => log::error!("[watchdog] Failed to restart local server: {e}"),
                    }
                }
            }
        });
    }
}
