//! Workspace management: directory layout and artifact storage.

use crate::errors::ReplayError;
use crate::types::{CheckpointMeta, ReplayEvent, RunManifest, RunStatus, RunSummary};
use std::fs;
use std::path::{Path, PathBuf};

pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Initialize workspace at the given root, creating directory structure if needed.
    pub fn init(root: impl Into<PathBuf>) -> Result<Self, ReplayError> {
        let root = root.into();
        fs::create_dir_all(root.join("checkpoints"))?;
        fs::create_dir_all(root.join("runs"))?;
        Ok(Self { root })
    }

    /// Open existing workspace (error if doesn't exist).
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, ReplayError> {
        let root = root.into();
        if !root.join("checkpoints").exists() || !root.join("runs").exists() {
            return Err(ReplayError::InvalidArgument(format!(
                "not a valid workspace: {}",
                root.display()
            )));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn checkpoints_dir(&self) -> PathBuf {
        self.root.join("checkpoints")
    }

    pub fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }

    // Checkpoint paths

    pub fn checkpoint_dir(&self, id: &str) -> PathBuf {
        self.checkpoints_dir().join(id)
    }

    pub fn checkpoint_db_dir(&self, id: &str) -> PathBuf {
        self.checkpoint_dir(id).join("db")
    }

    pub fn checkpoint_meta_path(&self, id: &str) -> PathBuf {
        self.checkpoint_dir(id).join("checkpoint.json")
    }

    // Run paths

    pub fn run_dir(&self, id: &str) -> PathBuf {
        self.runs_dir().join(id)
    }

    pub fn run_manifest_path(&self, id: &str) -> PathBuf {
        self.run_dir(id).join("run_manifest.json")
    }

    pub fn run_status_path(&self, id: &str) -> PathBuf {
        self.run_dir(id).join("status.json")
    }

    pub fn run_summary_path(&self, id: &str) -> PathBuf {
        self.run_dir(id).join("summary.json")
    }

    pub fn run_events_path(&self, id: &str) -> PathBuf {
        self.run_dir(id).join("events.ndjson")
    }

    pub fn run_log_path(&self, id: &str) -> PathBuf {
        self.run_dir(id).join("logs").join("replay.log")
    }

    pub fn run_db_dir(&self, id: &str) -> PathBuf {
        self.run_dir(id).join("db")
    }

    pub fn run_lock_path(&self, id: &str) -> PathBuf {
        self.run_dir(id).join("locks").join("run.lock")
    }

    pub fn cancel_flag_path(&self, id: &str) -> PathBuf {
        self.run_dir(id).join("cancel.flag")
    }

    // --- Directory creation helpers ---

    /// Create run directory structure.
    pub fn create_run_dirs(&self, id: &str) -> Result<(), ReplayError> {
        let run_dir = self.run_dir(id);
        fs::create_dir_all(run_dir.join("logs"))?;
        fs::create_dir_all(run_dir.join("locks"))?;
        fs::create_dir_all(run_dir.join("db"))?;
        Ok(())
    }

    /// Create checkpoint directory structure.
    pub fn create_checkpoint_dirs(&self, id: &str) -> Result<(), ReplayError> {
        fs::create_dir_all(self.checkpoint_db_dir(id))?;
        Ok(())
    }

    // --- Checkpoint meta read/write ---

    pub fn write_checkpoint_meta(&self, meta: &CheckpointMeta) -> Result<(), ReplayError> {
        let path = self.checkpoint_meta_path(&meta.checkpoint_id);
        let json = serde_json::to_string_pretty(meta)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn read_checkpoint_meta(&self, id: &str) -> Result<CheckpointMeta, ReplayError> {
        let path = self.checkpoint_meta_path(id);
        if !path.exists() {
            return Err(ReplayError::InvalidArgument(format!(
                "checkpoint not found: {id}"
            )));
        }
        let data = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&data)?)
    }

    pub fn list_checkpoints(&self) -> Result<Vec<CheckpointMeta>, ReplayError> {
        let dir = self.checkpoints_dir();
        let mut checkpoints = Vec::new();
        if dir.exists() {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    let meta_path = entry.path().join("checkpoint.json");
                    if meta_path.exists() {
                        let data = fs::read_to_string(meta_path)?;
                        if let Ok(meta) = serde_json::from_str::<CheckpointMeta>(&data) {
                            checkpoints.push(meta);
                        }
                    }
                }
            }
        }
        checkpoints.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(checkpoints)
    }

    /// Find a checkpoint by label (for idempotency checks).
    pub fn find_checkpoint_by_label(
        &self,
        label: &str,
    ) -> Result<Option<CheckpointMeta>, ReplayError> {
        let checkpoints = self.list_checkpoints()?;
        Ok(checkpoints.into_iter().find(|c| c.label == label))
    }

    // --- Run manifest read/write ---

    pub fn write_run_manifest(&self, manifest: &RunManifest) -> Result<(), ReplayError> {
        let json = serde_json::to_string_pretty(manifest)?;
        fs::write(self.run_manifest_path(&manifest.run_id), json)?;
        Ok(())
    }

    pub fn read_run_manifest(&self, id: &str) -> Result<RunManifest, ReplayError> {
        let path = self.run_manifest_path(id);
        if !path.exists() {
            return Err(ReplayError::RunNotFound(id.to_string()));
        }
        let data = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&data)?)
    }

    // --- Run status read/write ---

    pub fn write_run_status(&self, status: &RunStatus) -> Result<(), ReplayError> {
        let json = serde_json::to_string_pretty(status)?;
        fs::write(self.run_status_path(&status.run_id), json)?;
        Ok(())
    }

    pub fn read_run_status(&self, id: &str) -> Result<RunStatus, ReplayError> {
        let path = self.run_status_path(id);
        if !path.exists() {
            return Err(ReplayError::RunNotFound(id.to_string()));
        }
        let data = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&data)?)
    }

    // --- Run summary read/write ---

    pub fn write_run_summary(&self, summary: &RunSummary) -> Result<(), ReplayError> {
        let json = serde_json::to_string_pretty(summary)?;
        fs::write(self.run_summary_path(&summary.run_id), json)?;
        Ok(())
    }

    pub fn read_run_summary(&self, id: &str) -> Result<RunSummary, ReplayError> {
        let path = self.run_summary_path(id);
        if !path.exists() {
            return Err(ReplayError::RunNotFound(id.to_string()));
        }
        let data = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&data)?)
    }

    // --- Event log (append-only NDJSON) ---

    pub fn append_event(&self, event: &ReplayEvent) -> Result<(), ReplayError> {
        use std::io::Write;
        let path = self.run_events_path(&event.run_id);
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let line = serde_json::to_string(event)?;
        writeln!(file, "{}", line)?;
        Ok(())
    }

    pub fn read_events(&self, id: &str) -> Result<Vec<ReplayEvent>, ReplayError> {
        let path = self.run_events_path(id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let data = fs::read_to_string(path)?;
        let events: Vec<ReplayEvent> = data
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        Ok(events)
    }
}
