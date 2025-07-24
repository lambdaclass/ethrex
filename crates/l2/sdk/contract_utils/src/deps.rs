use std::{
    path::PathBuf,
    process::{Command, ExitStatus},
};

use tracing::{info, trace};

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

    if PathBuf::from(outdir).join(".git").exists() {
        info!(outdir = %outdir, "Found existing git repository, updating...");

        let branch_name = if let Some(b) = branch {
            b.to_string()
        } else {
            // Look for default branch name (could be main, master or other)
            let output = Command::new("git")
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
        let fetch_status = Command::new("git")
            .current_dir(outdir)
            .args(["fetch", "origin"])
            .spawn()
            .map_err(|err| GitError::DependencyError(format!("Failed to spawn git fetch: {err}")))?
            .wait()
            .map_err(|err| {
                GitError::DependencyError(format!("Failed to wait for git fetch: {err}"))
            })?;
        if !fetch_status.success() {
            return Err(GitError::DependencyError(format!(
                "git fetch failed for {outdir}"
            )));
        }

        // Checkout to branch
        let checkout_status = Command::new("git")
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
        let reset_status = Command::new("git")
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
            let submodule_status = Command::new("git")
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
        let mut git_cmd = Command::new("git");

        let git_clone_cmd = git_cmd.arg("clone").arg(repository_url);

        if let Some(branch) = branch {
            git_clone_cmd.arg("--branch").arg(branch);
        }

        if submodules {
            git_clone_cmd.arg("--recurse-submodules");
        }

        git_clone_cmd
            .arg(outdir)
            .spawn()
            .map_err(|err| GitError::DependencyError(format!("Failed to spawn git: {err}")))?
            .wait()
            .map_err(|err| GitError::DependencyError(format!("Failed to wait for git: {err}")))
    }
}
