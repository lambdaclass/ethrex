//! File-based locking to prevent concurrent workspace access.

use crate::errors::ReplayError;
use crate::types::LockInfo;
use chrono::Utc;
use std::fs;
use std::path::Path;

/// Duration in seconds after which a lock is considered stale
const STALE_LOCK_SECONDS: i64 = 3600; // 1 hour

/// Acquire a lock for a run. Uses O_EXCL (create_new) for atomic creation.
/// Returns error if lock is already held (and not stale).
pub fn acquire_lock(lock_path: &Path, run_id: &str) -> Result<(), ReplayError> {
    use std::io::Write;

    // Ensure parent directory exists
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let lock_info = LockInfo {
        holder_pid: std::process::id(),
        holder_hostname: hostname(),
        acquired_at: Utc::now(),
        run_id: run_id.to_string(),
    };
    let json = serde_json::to_string_pretty(&lock_info)?;

    // Try atomic create (O_EXCL). If the file doesn't exist, this creates it
    // exclusively — no other process can create it between our check and write.
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
    {
        Ok(mut file) => {
            file.write_all(json.as_bytes())?;
            return Ok(());
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // File exists — check if the existing lock is stale.
        }
        Err(e) => return Err(e.into()),
    }

    // Lock file exists. Read it and check staleness.
    let existing = read_lock(lock_path)?;
    let age = Utc::now().signed_duration_since(existing.acquired_at);
    if age.num_seconds() < STALE_LOCK_SECONDS {
        return Err(ReplayError::LockAlreadyHeld {
            pid: existing.holder_pid,
            hostname: existing.holder_hostname,
        });
    }

    // Stale lock — remove it and retry with atomic create.
    fs::remove_file(lock_path)?;
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
    {
        Ok(mut file) => {
            file.write_all(json.as_bytes())?;
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Another process grabbed the lock between our remove and create.
            // Re-read to report the holder.
            let holder = read_lock(lock_path)?;
            Err(ReplayError::LockAlreadyHeld {
                pid: holder.holder_pid,
                hostname: holder.holder_hostname,
            })
        }
        Err(e) => Err(e.into()),
    }
}

/// Refresh the lock's timestamp to prevent staleness during long-running
/// execution. The executor must call this periodically (e.g., every block).
///
/// Returns an error if the lock is missing or owned by another process,
/// signaling that the executor has lost exclusivity and must stop.
///
/// Uses atomic write (temp file + rename) so concurrent readers never see
/// a truncated or empty lock file.
pub fn refresh_lock(lock_path: &Path, run_id: &str) -> Result<(), ReplayError> {
    if !lock_path.exists() {
        return Err(ReplayError::Internal(
            "lock file disappeared during execution".to_string(),
        ));
    }
    let existing = read_lock(lock_path)?;
    if existing.holder_pid != std::process::id() || existing.run_id != run_id {
        return Err(ReplayError::Internal(format!(
            "lock ownership lost: held by PID {} for run '{}', expected PID {} for run '{}'",
            existing.holder_pid,
            existing.run_id,
            std::process::id(),
            run_id
        )));
    }
    let refreshed = LockInfo {
        acquired_at: Utc::now(),
        ..existing
    };
    let json = serde_json::to_string_pretty(&refreshed)?;
    // Atomic write: write to temp file, then rename. rename on the same
    // filesystem is atomic on POSIX, so concurrent readers of lock_path
    // always see either the old or new complete content, never partial.
    let tmp_path = lock_path.with_extension("lock.tmp");
    fs::write(&tmp_path, json)?;
    fs::rename(&tmp_path, lock_path)?;
    Ok(())
}

/// Release a lock (delete the lock file). No error if already released.
pub fn release_lock(lock_path: &Path) -> Result<(), ReplayError> {
    if lock_path.exists() {
        fs::remove_file(lock_path)?;
    }
    Ok(())
}

/// Release a lock only if it is currently owned by this process for `run_id`.
///
/// This is used by executor-owned cleanup paths to avoid deleting a lock that
/// may have been legitimately acquired by another process after ownership loss.
pub fn release_lock_if_owned(lock_path: &Path, run_id: &str) -> Result<(), ReplayError> {
    if !lock_path.exists() {
        return Ok(());
    }

    let lock_info = match read_lock(lock_path) {
        Ok(info) => info,
        // If lock metadata is unreadable, don't delete it blindly.
        Err(_) => return Ok(()),
    };

    if lock_info.holder_pid == std::process::id() && lock_info.run_id == run_id {
        fs::remove_file(lock_path)?;
    }

    Ok(())
}

/// Read lock info from a lock file
pub fn read_lock(lock_path: &Path) -> Result<LockInfo, ReplayError> {
    let data = fs::read_to_string(lock_path)?;
    Ok(serde_json::from_str(&data)?)
}

/// Check if a lock is currently held (and not stale)
pub fn is_locked(lock_path: &Path) -> bool {
    if !lock_path.exists() {
        return false;
    }
    match read_lock(lock_path) {
        Ok(info) => {
            let age = Utc::now().signed_duration_since(info.acquired_at);
            age.num_seconds() < STALE_LOCK_SECONDS
        }
        Err(_) => false,
    }
}

fn hostname() -> String {
    // Try to get hostname from system
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "unknown".to_string())
}
