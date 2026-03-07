use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

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

fn db_path() -> PathBuf {
    let home = dirs::home_dir().expect("Cannot determine home directory");
    home.join(".tokamak-appchain").join("local.sqlite")
}

/// Read all deployments directly from the SQLite DB (no server needed).
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

/// Get the compose file path for a deployment: ~/.tokamak/deployments/{id}/docker-compose.yaml
fn compose_file_for(id: &str) -> PathBuf {
    let home = dirs::home_dir().expect("Cannot determine home directory");
    home.join(".tokamak")
        .join("deployments")
        .join(id)
        .join("docker-compose.yaml")
}

/// Find the docker binary (macOS apps don't always have /usr/local/bin in PATH)
fn docker_bin() -> String {
    for path in &[
        "/usr/local/bin/docker",
        "/opt/homebrew/bin/docker",
        "/usr/bin/docker",
    ] {
        if std::path::Path::new(path).exists() {
            return path.to_string();
        }
    }
    "docker".to_string()
}

/// Run `docker compose` with the project name and compose file for a deployment.
fn docker_compose(docker_project: &str, compose_file: &std::path::Path, args: &[&str]) -> Result<(), String> {
    let compose_str = compose_file.to_string_lossy();
    let mut cmd_args = vec!["compose".to_string(), "-p".to_string(), docker_project.to_string()];
    if compose_file.exists() {
        cmd_args.push("-f".to_string());
        cmd_args.push(compose_str.to_string());
    }
    for arg in args {
        cmd_args.push(arg.to_string());
    }

    let docker = docker_bin();
    log::info!("Running: {} {}", docker, cmd_args.join(" "));

    let output = Command::new(&docker)
        .args(&cmd_args)
        .output()
        .map_err(|e| format!("Failed to run docker: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::warn!("docker compose failed: {}", stderr);
        return Err(format!("docker compose failed: {}", stderr));
    }
    Ok(())
}

/// Delete a deployment: clean up Docker resources, remove deploy dir, then remove from DB.
pub fn delete_deployment_from_db(id: &str) -> Result<(), String> {
    let path = db_path();
    if !path.exists() {
        return Err("Database not found".to_string());
    }

    let conn = Connection::open(&path)
        .map_err(|e| format!("Failed to open deployment DB: {e}"))?;

    // Get docker_project before deleting
    let docker_project: Option<String> = conn
        .query_row(
            "SELECT docker_project FROM deployments WHERE id = ?",
            [id],
            |row| row.get(0),
        )
        .unwrap_or(None);

    // Clean up Docker resources
    if let Some(ref project) = docker_project {
        let compose = compose_file_for(id);
        if let Err(e) = docker_compose(project, &compose, &["down", "--volumes", "--remove-orphans"]) {
            log::warn!("Docker cleanup failed for {}: {}", id, e);
            // Continue with DB deletion even if Docker cleanup fails
        }
    }

    // Delete from DB
    conn.execute("DELETE FROM deployments WHERE id = ?", [id])
        .map_err(|e| format!("Failed to delete from DB: {e}"))?;

    // Remove deployment directory
    let deploy_dir = compose_file_for(id)
        .parent()
        .unwrap()
        .to_path_buf();
    if deploy_dir.exists() {
        let _ = std::fs::remove_dir_all(&deploy_dir);
    }

    Ok(())
}

/// Stop a deployment's Docker containers.
pub fn stop_deployment_in_db(id: &str) -> Result<(), String> {
    let path = db_path();
    if !path.exists() {
        return Err("Database not found".to_string());
    }

    let conn = Connection::open(&path)
        .map_err(|e| format!("Failed to open deployment DB: {e}"))?;

    let docker_project: Option<String> = conn
        .query_row(
            "SELECT docker_project FROM deployments WHERE id = ?",
            [id],
            |row| row.get(0),
        )
        .unwrap_or(None);

    if let Some(ref project) = docker_project {
        let compose = compose_file_for(id);
        docker_compose(project, &compose, &["stop"])?;
        conn.execute("UPDATE deployments SET status = 'stopped' WHERE id = ?", [id])
            .map_err(|e| format!("Failed to update status: {e}"))?;
    } else {
        return Err("Deployment not found or has no Docker project".to_string());
    }

    Ok(())
}

/// Container info from `docker compose ps --format json`
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

/// Get container status for a deployment via `docker compose ps --format json`.
pub fn get_containers_for_deployment(deployment_id: &str) -> Result<Vec<ContainerInfo>, String> {
    let path = db_path();
    if !path.exists() {
        return Ok(vec![]);
    }

    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("Failed to open DB: {e}"))?;

    let docker_project: Option<String> = conn
        .query_row(
            "SELECT docker_project FROM deployments WHERE id = ?",
            [deployment_id],
            |row| row.get(0),
        )
        .unwrap_or(None);

    let project = match docker_project {
        Some(p) => p,
        None => return Ok(vec![]),
    };

    let compose = compose_file_for(deployment_id);
    let docker = docker_bin();

    let mut cmd_args = vec![
        "compose".to_string(),
        "-p".to_string(),
        project,
    ];
    if compose.exists() {
        cmd_args.push("-f".to_string());
        cmd_args.push(compose.to_string_lossy().to_string());
    }
    cmd_args.extend(["ps".to_string(), "--format".to_string(), "json".to_string()]);

    let output = Command::new(&docker)
        .args(&cmd_args)
        .output()
        .map_err(|e| format!("Failed to run docker: {e}"))?;

    if !output.status.success() {
        return Ok(vec![]);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let containers: Vec<ContainerInfo> = stdout
        .trim()
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();

    Ok(containers)
}

/// Start a stopped deployment's Docker containers.
pub fn start_deployment_in_db(id: &str) -> Result<(), String> {
    let path = db_path();
    if !path.exists() {
        return Err("Database not found".to_string());
    }

    let conn = Connection::open(&path)
        .map_err(|e| format!("Failed to open deployment DB: {e}"))?;

    let docker_project: Option<String> = conn
        .query_row(
            "SELECT docker_project FROM deployments WHERE id = ?",
            [id],
            |row| row.get(0),
        )
        .unwrap_or(None);

    if let Some(ref project) = docker_project {
        let compose = compose_file_for(id);
        docker_compose(project, &compose, &["start"])?;
        conn.execute("UPDATE deployments SET status = 'running' WHERE id = ?", [id])
            .map_err(|e| format!("Failed to update status: {e}"))?;
    } else {
        return Err("Deployment not found or has no Docker project".to_string());
    }

    Ok(())
}
