use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Represents a deployment row from the local-server SQLite DB.
#[derive(Debug, Serialize, Clone)]
pub struct DeploymentRow {
    pub id: String,
    pub program_slug: String,
    pub stack_type: String,
    pub name: String,
    pub chain_id: Option<i64>,
    pub rpc_url: Option<String>,
    pub status: String,
    pub deploy_method: String,
    pub docker_project: Option<String>,
    pub l1_port: Option<i64>,
    pub l2_port: Option<i64>,
    pub proof_coord_port: Option<i64>,
    pub phase: String,
    pub bridge_address: Option<String>,
    pub proposer_address: Option<String>,
    pub timelock_address: Option<String>,
    pub sp1_verifier_address: Option<String>,
    pub guest_program_registry_address: Option<String>,
    pub verification_status: Option<String>,
    pub error_message: Option<String>,
    pub config: Option<String>,
    pub is_public: i64,
    pub created_at: i64,
    pub tools_l1_explorer_port: Option<i64>,
    pub tools_l2_explorer_port: Option<i64>,
    pub tools_bridge_ui_port: Option<i64>,
    pub hashtags: Option<String>,
    pub ever_running: i64,
    pub l1_chain_id: Option<i64>,
    pub host_id: Option<String>,
    pub platform_deployment_id: Option<String>,
    pub public_l2_rpc_url: Option<String>,
    pub public_domain: Option<String>,
}

/// Container info returned by local-server status endpoint.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContainerInfo {
    #[serde(alias = "Name")]
    pub name: String,
    #[serde(alias = "Service")]
    pub service: String,
    #[serde(alias = "State")]
    pub state: String,
    #[serde(alias = "Status")]
    pub status: String,
    #[serde(alias = "Ports", default)]
    pub ports: String,
    #[serde(alias = "Image", default)]
    pub image: String,
    #[serde(alias = "ID", default)]
    pub id: String,
}

/// Status response from local-server GET /api/deployments/:id/status
#[derive(Debug, Deserialize)]
struct DeploymentStatus {
    containers: Vec<ContainerInfo>,
}

fn db_path() -> PathBuf {
    let home = dirs::home_dir().expect("Cannot determine home directory");
    home.join(".tokamak-appchain").join("local.sqlite")
}

/// Read all deployments directly from the SQLite DB (read-only, no server needed).
pub fn list_deployments_from_db() -> Result<Vec<DeploymentRow>, String> {
    let path = db_path();
    if !path.exists() {
        return Ok(vec![]);
    }

    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("Failed to open deployment DB: {e}"))?;

    let mut stmt = conn
        .prepare(
            "SELECT id, program_slug, stack_type, name, chain_id, rpc_url, status, deploy_method,
                    docker_project, l1_port, l2_port, proof_coord_port, phase,
                    bridge_address, proposer_address, timelock_address, sp1_verifier_address,
                    guest_program_registry_address, verification_status,
                    error_message, config, is_public, created_at,
                    tools_l1_explorer_port, tools_l2_explorer_port, tools_bridge_ui_port,
                    hashtags, ever_running,
                    l1_chain_id, host_id, platform_deployment_id, public_l2_rpc_url, public_domain
             FROM deployments ORDER BY created_at DESC",
        )
        .map_err(|e| format!("SQL prepare error: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(DeploymentRow {
                id: row.get("id")?,
                program_slug: row.get("program_slug")?,
                stack_type: row.get("stack_type")?,
                name: row.get("name")?,
                chain_id: row.get("chain_id")?,
                rpc_url: row.get("rpc_url")?,
                status: row.get("status")?,
                deploy_method: row.get("deploy_method")?,
                docker_project: row.get("docker_project")?,
                l1_port: row.get("l1_port")?,
                l2_port: row.get("l2_port")?,
                proof_coord_port: row.get("proof_coord_port")?,
                phase: row.get("phase")?,
                bridge_address: row.get("bridge_address")?,
                proposer_address: row.get("proposer_address")?,
                timelock_address: row.get("timelock_address")?,
                sp1_verifier_address: row.get("sp1_verifier_address")?,
                guest_program_registry_address: row.get("guest_program_registry_address")?,
                verification_status: row.get("verification_status")?,
                error_message: row.get("error_message")?,
                config: row.get("config")?,
                is_public: row.get("is_public")?,
                created_at: row.get("created_at")?,
                tools_l1_explorer_port: row.get("tools_l1_explorer_port")?,
                tools_l2_explorer_port: row.get("tools_l2_explorer_port")?,
                tools_bridge_ui_port: row.get("tools_bridge_ui_port")?,
                hashtags: row.get("hashtags")?,
                ever_running: row.get::<_, Option<i64>>("ever_running")?.unwrap_or(0),
                l1_chain_id: row.get("l1_chain_id")?,
                host_id: row.get("host_id")?,
                platform_deployment_id: row.get("platform_deployment_id")?,
                public_l2_rpc_url: row.get("public_l2_rpc_url")?,
                public_domain: row.get("public_domain")?,
            })
        })
        .map_err(|e| format!("SQL query error: {e}"))?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| format!("Row read error: {e}"))?);
    }
    Ok(result)
}

/// Proxy Docker lifecycle operations through the local-server HTTP API.
/// This ensures a single source of truth for Docker management.
pub struct DeploymentProxy {
    base_url: String,
}

impl DeploymentProxy {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Validate and sanitize an ID to prevent path traversal attacks.
    fn sanitize_id(id: &str) -> Result<&str, String> {
        if id.is_empty() {
            return Err("ID cannot be empty".to_string());
        }
        if id.contains("..") || id.contains('/') || id.contains('\\') || id.contains('\0') {
            return Err(format!("Invalid ID: {}", id));
        }
        Ok(id)
    }

    /// Stop a deployment via local-server POST /api/deployments/:id/stop
    pub async fn stop_deployment(&self, id: &str) -> Result<(), String> {
        let id = Self::sanitize_id(id)?;
        let url = format!("{}/api/deployments/{}/stop", self.base_url, id);
        let resp = reqwest::Client::new()
            .post(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to reach local-server: {e}"))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Stop failed: {body}"));
        }
        Ok(())
    }

    /// Start a deployment via local-server POST /api/deployments/:id/start
    pub async fn start_deployment(&self, id: &str) -> Result<(), String> {
        let id = Self::sanitize_id(id)?;
        let url = format!("{}/api/deployments/{}/start", self.base_url, id);
        let resp = reqwest::Client::new()
            .post(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to reach local-server: {e}"))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Start failed: {body}"));
        }
        Ok(())
    }

    /// Delete/destroy a deployment via local-server POST /api/deployments/:id/destroy
    pub async fn destroy_deployment(&self, id: &str) -> Result<(), String> {
        let id = Self::sanitize_id(id)?;
        let url = format!("{}/api/deployments/{}/destroy", self.base_url, id);
        let resp = reqwest::Client::new()
            .post(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to reach local-server: {e}"))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Destroy failed: {body}"));
        }
        Ok(())
    }

    /// Get containers for a deployment via local-server GET /api/deployments/:id/status
    pub async fn get_containers(&self, id: &str) -> Result<Vec<ContainerInfo>, String> {
        let id = Self::sanitize_id(id)?;
        let url = format!("{}/api/deployments/{}/status", self.base_url, id);
        let resp = reqwest::Client::new()
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to reach local-server: {e}"))?;

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let status: DeploymentStatus = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse status: {e}"))?;

        Ok(status.containers)
    }

    /// Fetch logs for a specific service via local-server GET /api/deployments/:id/logs
    pub async fn get_logs(
        &self,
        id: &str,
        service: Option<&str>,
        tail: usize,
    ) -> Result<String, String> {
        let id = Self::sanitize_id(id)?;
        let mut url = format!(
            "{}/api/deployments/{}/logs?tail={}",
            self.base_url, id, tail
        );
        if let Some(svc) = service {
            // URL-encode service name to prevent injection
            let encoded: String = svc.chars().map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c.to_string()
                } else {
                    format!("%{:02X}", c as u32)
                }
            }).collect();
            url.push_str(&format!("&service={}", encoded));
        }

        let resp = reqwest::Client::new()
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to reach local-server: {e}"))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Logs fetch failed: {body}"));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse logs response: {e}"))?;

        Ok(json["logs"].as_str().unwrap_or("").to_string())
    }

    /// Get RPC health monitoring via local-server GET /api/deployments/:id/monitoring
    pub async fn get_monitoring(&self, id: &str) -> Result<MonitoringInfo, String> {
        let id = Self::sanitize_id(id)?;
        let url = format!("{}/api/deployments/{}/monitoring", self.base_url, id);
        let resp = reqwest::Client::new()
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to reach local-server: {e}"))?;

        if !resp.status().is_success() {
            return Err("Monitoring endpoint unavailable".to_string());
        }

        resp.json()
            .await
            .map_err(|e| format!("Failed to parse monitoring: {e}"))
    }
}

/// RPC health info from local-server monitoring endpoint
#[derive(Debug, Deserialize, Clone)]
pub struct MonitoringInfo {
    pub l1: Option<RpcHealth>,
    pub l2: Option<RpcHealth>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct RpcHealth {
    pub healthy: bool,
    pub block_number: Option<serde_json::Value>,
    pub chain_id: Option<serde_json::Value>,
    pub rpc_url: Option<String>,
}
