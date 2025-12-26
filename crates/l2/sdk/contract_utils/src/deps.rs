use std::{
    path::Path,
    process::{Command, ExitStatus},
    thread,
    time::Duration,
};

use tracing::{info, trace, warn};

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("Failed to clone: {0}")]
    DependencyError(String),
    #[error("Internal error: {0}")]
    InternalError(String),
    #[error("Failed to get string from path")]
    FailedToGetStringFromPath,
}

pub fn git_clone(
    repository_url: &str,
    outdir: &str,
    branch: Option<&str>,
    submodules: bool,
) -> Result<ExitStatus, GitError> {
    info!(repository_url = %repository_url, outdir = %outdir, branch = ?branch, "Cloning or updating git repository");

    const MAX_RETRIES: u32 = 3;
    const INITIAL_RETRY_DELAY_SECS: u64 = 5;

    let mut last_error = None;
    for attempt in 0..=MAX_RETRIES {
        if attempt > 0 {
            let delay = INITIAL_RETRY_DELAY_SECS * (1 << (attempt - 1)); // Exponential backoff: 5s, 10s, 20s
            warn!(
                attempt = attempt,
                max_retries = MAX_RETRIES,
                delay_secs = delay,
                "Retrying git operation after failure"
            );
            thread::sleep(Duration::from_secs(delay));
        }

        match git_clone_internal(repository_url, outdir, branch, submodules) {
            Ok(status) => return Ok(status),
            Err(e) => {
                last_error = Some(e);
                // Only retry on network-related errors
                if attempt < MAX_RETRIES {
                    continue;
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        GitError::DependencyError("Failed to clone repository after retries".to_string())
    }))
}

fn git_clone_internal(
    repository_url: &str,
    outdir: &str,
    branch: Option<&str>,
    submodules: bool,
) -> Result<ExitStatus, GitError> {
    // Helper to create a git command with timeout configuration
    // This helps with CI environments that may have slower network connections
    let git_cmd_with_timeouts = || {
        let mut cmd = Command::new("git");
        cmd.env("GIT_HTTP_LOW_SPEED_LIMIT", "1000")
            .env("GIT_HTTP_LOW_SPEED_TIME", "300")
            .env("GIT_HTTP_TIMEOUT", "300");
        cmd
    };

    if Path::new(outdir).join(".git").exists() {
        info!(outdir = %outdir, "Found existing git repository, updating...");

        let branch_name = if let Some(b) = branch {
            b.to_string()
        } else {
            // Look for default branch name (could be main, master or other)
            let output = git_cmd_with_timeouts()
                .current_dir(outdir)
                .arg("symbolic-ref")
                .arg("refs/remotes/origin/HEAD")
                .output()
                .map_err(|e| {
                    GitError::DependencyError(format!(
                        "Failed to get default branch for {outdir}: {e}"
                    ))
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(GitError::DependencyError(format!(
                    "Failed to get default branch for {outdir}: {stderr}"
                )));
            }

            String::from_utf8(output.stdout)
                .map_err(|_| GitError::InternalError("Failed to parse git output".to_string()))?
                .trim()
                .split('/')
                .next_back()
                .ok_or(GitError::InternalError(
                    "Failed to parse default branch".to_string(),
                ))?
                .to_string()
        };

        trace!(branch = %branch_name, "Updating to branch");

        // Fetch
        let fetch_output = git_cmd_with_timeouts()
            .current_dir(outdir)
            .args(["fetch", "origin"])
            .output()
            .map_err(|err| {
                GitError::DependencyError(format!("Failed to spawn git fetch: {err}"))
            })?;
        if !fetch_output.status.success() {
            let stderr = String::from_utf8_lossy(&fetch_output.stderr);
            return Err(GitError::DependencyError(format!(
                "git fetch failed for {outdir}: {stderr}"
            )));
        }

        // Checkout to branch
        let checkout_status = git_cmd_with_timeouts()
            .current_dir(outdir)
            .arg("checkout")
            .arg(&branch_name)
            .spawn()
            .map_err(|err| {
                GitError::DependencyError(format!("Failed to spawn git checkout: {err}"))
            })?
            .wait()
            .map_err(|err| {
                GitError::DependencyError(format!("Failed to wait for git checkout: {err}"))
            })?;
        if !checkout_status.success() {
            return Err(GitError::DependencyError(format!(
                "git checkout of branch {branch_name} failed for {outdir}, try deleting the repo folder"
            )));
        }

        // Reset branch to origin
        let reset_status = git_cmd_with_timeouts()
            .current_dir(outdir)
            .arg("reset")
            .arg("--hard")
            .arg(format!("origin/{branch_name}"))
            .spawn()
            .map_err(|err| GitError::DependencyError(format!("Failed to spawn git reset: {err}")))?
            .wait()
            .map_err(|err| {
                GitError::DependencyError(format!("Failed to wait for git reset: {err}"))
            })?;

        if !reset_status.success() {
            return Err(GitError::DependencyError(format!(
                "git reset failed for {outdir}"
            )));
        }

        // Update submodules
        if submodules {
            let submodule_status = git_cmd_with_timeouts()
                .current_dir(outdir)
                .arg("submodule")
                .arg("update")
                .arg("--init")
                .arg("--recursive")
                .spawn()
                .map_err(|err| {
                    GitError::DependencyError(format!(
                        "Failed to spawn git submodule update: {err}"
                    ))
                })?
                .wait()
                .map_err(|err| {
                    GitError::DependencyError(format!(
                        "Failed to wait for git submodule update: {err}"
                    ))
                })?;
            if !submodule_status.success() {
                return Err(GitError::DependencyError(format!(
                    "git submodule update failed for {outdir}"
                )));
            }
        }

        Ok(reset_status)
    } else {
        trace!(repository_url = %repository_url, outdir = %outdir, branch = ?branch, "Cloning git repository");
        let mut git_clone_cmd = git_cmd_with_timeouts();
        git_clone_cmd.arg("clone").arg(repository_url);

        if let Some(branch) = branch {
            git_clone_cmd.arg("--branch").arg(branch);
        }

        if submodules {
            git_clone_cmd.arg("--recurse-submodules");
        }

        let output = git_clone_cmd
            .arg(outdir)
            .output()
            .map_err(|err| GitError::DependencyError(format!("Failed to spawn git: {err}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GitError::DependencyError(format!(
                "git clone failed for {repository_url}: {stderr}"
            )));
        }

        Ok(output.status)
    }
}
