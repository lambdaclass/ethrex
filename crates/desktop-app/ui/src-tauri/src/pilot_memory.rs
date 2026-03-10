//! Persistent memory for Tokamak AI Pilot.
//! Stores chat sessions, appchain events, and an AI-managed summary.
//!
//! Storage:
//!   ~/Library/Application Support/tokamak-appchain/pilot-memory/
//!     sessions.jsonl  — all chat messages (append-only)
//!     events.jsonl    — appchain lifecycle events (append-only)
//!     summary.md      — AI-maintained operational summary

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_SESSION_LINES: usize = 500;

/// A single chat message record (persisted to sessions.jsonl)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub ts: DateTime<Utc>,
    pub chat_id: i64,
    pub role: String,      // "user" | "assistant" | "action"
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

/// An appchain lifecycle event (persisted to events.jsonl)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    pub ts: DateTime<Utc>,
    pub event: String, // created, started, stopped, deleted, process_crashed, container_exited
    pub chain_name: String,
    pub chain_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub by: String, // "telegram", "desktop", "system"
}

/// Loaded context for AI prompt injection
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct PilotContext {
    pub recent_messages: Vec<SessionRecord>,
    pub recent_events: Vec<EventRecord>,
    pub summary: String,
}

pub struct PilotMemory {
    dir: PathBuf,
    write_lock: Mutex<()>,
}

impl PilotMemory {
    pub fn new() -> Self {
        let dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("tokamak-appchain")
            .join("pilot-memory");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("Failed to create pilot-memory dir: {e}");
        }
        Self {
            dir,
            write_lock: Mutex::new(()),
        }
    }

    fn sessions_path(&self) -> PathBuf {
        self.dir.join("sessions.jsonl")
    }

    fn events_path(&self) -> PathBuf {
        self.dir.join("events.jsonl")
    }

    fn summary_path(&self) -> PathBuf {
        self.dir.join("summary.md")
    }

    // ── Write ──

    pub fn append_message(&self, chat_id: i64, role: &str, content: &str) {
        let record = SessionRecord {
            ts: Utc::now(),
            chat_id,
            role: role.to_string(),
            content: content.to_string(),
            action: None,
            result: None,
        };
        self.append_jsonl(&self.sessions_path(), &record);
        // Probabilistic cleanup: ~1% chance per message to avoid unbounded growth
        if rand::random::<u8>() < 3 {
            self.cleanup_sessions();
        }
    }

    pub fn append_action(&self, chat_id: i64, action: &str, result: &str) {
        let record = SessionRecord {
            ts: Utc::now(),
            chat_id,
            role: "action".to_string(),
            content: String::new(),
            action: Some(action.to_string()),
            result: Some(result.to_string()),
        };
        self.append_jsonl(&self.sessions_path(), &record);
    }

    pub fn append_event(&self, event: &str, chain_name: &str, chain_id: &str, detail: &str, by: &str) {
        let record = EventRecord {
            ts: Utc::now(),
            event: event.to_string(),
            chain_name: chain_name.to_string(),
            chain_id: chain_id.to_string(),
            detail: detail.to_string(),
            by: by.to_string(),
        };
        self.append_jsonl(&self.events_path(), &record);
    }

    pub fn update_summary(&self, content: &str) {
        if let Err(e) = std::fs::write(self.summary_path(), content) {
            log::warn!("Failed to write pilot summary: {e}");
        }
    }

    // ── Read ──

    /// Load recent context for AI prompt: last N messages for this chat, last N events, summary
    pub fn load_recent_context(&self, chat_id: i64, msg_limit: usize, event_limit: usize) -> PilotContext {
        let all_messages = self.read_jsonl::<SessionRecord>(&self.sessions_path());
        let filtered: Vec<_> = all_messages
            .into_iter()
            .filter(|r| r.chat_id == chat_id)
            .collect();
        let recent_messages = filtered[filtered.len().saturating_sub(msg_limit)..].to_vec();

        let all_events = self.read_jsonl::<EventRecord>(&self.events_path());
        let recent_events = all_events[all_events.len().saturating_sub(event_limit)..].to_vec();

        let summary = std::fs::read_to_string(self.summary_path()).unwrap_or_default();

        PilotContext {
            recent_messages,
            recent_events,
            summary,
        }
    }

    /// Get time of last message from a specific chat.
    /// Reads from the end of the file for efficiency (most recent entries are at the bottom).
    pub fn last_message_time(&self, chat_id: i64) -> Option<DateTime<Utc>> {
        let content = std::fs::read_to_string(self.sessions_path()).ok()?;
        // Scan from the end — the last matching line is what we need
        content
            .lines()
            .rev()
            .filter_map(|line| serde_json::from_str::<SessionRecord>(line).ok())
            .find(|r| r.chat_id == chat_id && r.role != "action")
            .map(|r| r.ts)
    }

    /// Get events since a specific time
    pub fn events_since(&self, since: DateTime<Utc>) -> Vec<EventRecord> {
        let events = self.read_jsonl::<EventRecord>(&self.events_path());
        events.into_iter().filter(|e| e.ts > since).collect()
    }

    /// Cleanup old session records (keep last N lines)
    pub fn cleanup_sessions(&self) {
        let path = self.sessions_path();
        let records = self.read_jsonl::<SessionRecord>(&path);
        if records.len() <= MAX_SESSION_LINES {
            return;
        }
        let keep = &records[records.len() - MAX_SESSION_LINES..];
        let content: String = keep
            .iter()
            .filter_map(|r| serde_json::to_string(r).ok())
            .map(|s| s + "\n")
            .collect();
        let _ = std::fs::write(&path, content);
    }

    // ── Helpers ──

    fn append_jsonl<T: Serialize>(&self, path: &PathBuf, record: &T) {
        use std::io::Write;
        let _guard = self.write_lock.lock().expect("write_lock poisoned");
        if let Ok(json) = serde_json::to_string(record) {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(file, "{}", json);
            }
        }
    }

    fn read_jsonl<T: for<'de> Deserialize<'de>>(&self, path: &PathBuf) -> Vec<T> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        content
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_memory() -> (PilotMemory, TempDir) {
        let tmp = TempDir::new().unwrap();
        let memory = PilotMemory {
            dir: tmp.path().to_path_buf(),
            write_lock: Mutex::new(()),
        };
        (memory, tmp)
    }

    #[test]
    fn test_append_and_read_messages() {
        let (memory, _tmp) = make_memory();
        memory.append_message(123, "user", "hello");
        memory.append_message(123, "assistant", "hi there");
        memory.append_message(456, "user", "other chat");

        let ctx = memory.load_recent_context(123, 10, 10);
        assert_eq!(ctx.recent_messages.len(), 2);
        assert_eq!(ctx.recent_messages[0].content, "hello");
        assert_eq!(ctx.recent_messages[1].content, "hi there");
    }

    #[test]
    fn test_append_and_read_events() {
        let (memory, _tmp) = make_memory();
        memory.append_event("created", "test-chain", "abc", "", "telegram");
        memory.append_event("started", "test-chain", "abc", "", "telegram");

        let ctx = memory.load_recent_context(123, 10, 10);
        assert_eq!(ctx.recent_events.len(), 2);
        assert_eq!(ctx.recent_events[0].event, "created");
        assert_eq!(ctx.recent_events[1].event, "started");
    }

    #[test]
    fn test_summary_read_write() {
        let (memory, _tmp) = make_memory();
        assert_eq!(memory.load_recent_context(1, 1, 1).summary, "");

        memory.update_summary("# Test Summary\nAll good.");
        let ctx = memory.load_recent_context(1, 1, 1);
        assert_eq!(ctx.summary, "# Test Summary\nAll good.");
    }

    #[test]
    fn test_last_message_time() {
        let (memory, _tmp) = make_memory();
        assert!(memory.last_message_time(123).is_none());

        memory.append_message(123, "user", "hello");
        assert!(memory.last_message_time(123).is_some());
        assert!(memory.last_message_time(999).is_none());
    }

    #[test]
    fn test_events_since() {
        let (memory, _tmp) = make_memory();
        let before = Utc::now();
        memory.append_event("created", "chain", "1", "", "test");
        let events = memory.events_since(before - chrono::Duration::seconds(1));
        assert_eq!(events.len(), 1);

        let events = memory.events_since(Utc::now() + chrono::Duration::seconds(1));
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn test_append_action() {
        let (memory, _tmp) = make_memory();
        memory.append_action(123, "stop_appchain:id=abc", "ok");
        let ctx = memory.load_recent_context(123, 10, 10);
        assert_eq!(ctx.recent_messages.len(), 1);
        assert_eq!(ctx.recent_messages[0].role, "action");
        assert_eq!(ctx.recent_messages[0].action.as_deref(), Some("stop_appchain:id=abc"));
    }

    #[test]
    fn test_cleanup_sessions() {
        let (memory, _tmp) = make_memory();
        for i in 0..600 {
            memory.append_message(1, "user", &format!("msg {i}"));
        }
        memory.cleanup_sessions();
        let ctx = memory.load_recent_context(1, 1000, 0);
        assert!(ctx.recent_messages.len() <= MAX_SESSION_LINES);
    }
}
