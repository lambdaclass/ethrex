use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Represents a deployment row from the local-server SQLite DB.
#[derive(Debug, Serialize, Clone)]
pub struct DeploymentRow {
    pub id: String,
    pub program_slug: String,
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
    pub error_message: Option<String>,
    pub is_public: i64,
    pub created_at: i64,
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
            "SELECT id, program_slug, name, chain_id, rpc_url, status, deploy_method,
                    docker_project, l1_port, l2_port, proof_coord_port, phase,
                    bridge_address, proposer_address, error_message, is_public, created_at
             FROM deployments ORDER BY created_at DESC",
        )
        .map_err(|e| format!("SQL prepare error: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(DeploymentRow {
                id: row.get(0)?,
                program_slug: row.get(1)?,
                name: row.get(2)?,
                chain_id: row.get(3)?,
                rpc_url: row.get(4)?,
                status: row.get(5)?,
                deploy_method: row.get(6)?,
                docker_project: row.get(7)?,
                l1_port: row.get(8)?,
                l2_port: row.get(9)?,
                proof_coord_port: row.get(10)?,
                phase: row.get(11)?,
                bridge_address: row.get(12)?,
                proposer_address: row.get(13)?,
                error_message: row.get(14)?,
                is_public: row.get(15)?,
                created_at: row.get(16)?,
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

    /// Stop a deployment via local-server POST /api/deployments/:id/stop
    pub async fn stop_deployment(&self, id: &str) -> Result<(), String> {
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
}
