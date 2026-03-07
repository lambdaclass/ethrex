use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::Mutex;

/// Manages the local-server (Node.js Express) lifecycle.
/// Desktop spawns this on startup; it provides deployment management APIs.
pub struct LocalServer {
    child: Mutex<Option<Child>>,
    port: u16,
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
            // macOS: .app/Contents/MacOS/tokamak-desktop → .app/Contents/Resources/local-server/
            if let Some(parent) = exe.parent() {
                let resources = parent.parent().map(|p| p.join("Resources/local-server"));
                if let Some(ref dir) = resources {
                    if dir.join("server.js").exists() {
                        return resources;
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

    /// Find the node binary
    fn find_node() -> Option<PathBuf> {
        if let Ok(node) = std::env::var("NODE_BIN") {
            let p = PathBuf::from(&node);
            if p.exists() {
                return Some(p);
            }
        }

        // Check PATH
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
            log::info!("node_modules not found, running npm install...");
            let npm = if cfg!(target_os = "windows") { "npm.cmd" } else { "npm" };
            let install = std::process::Command::new(npm)
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

    /// Check if a child process is running
    pub async fn is_running(&self) -> bool {
        let child_guard = self.child.lock().await;
        child_guard.is_some()
    }
}
