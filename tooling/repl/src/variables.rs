use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// Shared variable store for the REPL.
///
/// Thread-safe wrapper around a `HashMap<String, Value>` that can be shared
/// between the REPL executor (which writes) and the completer (which reads
/// for tab-completion of `$var.field` paths).
#[derive(Clone, Default)]
pub struct VariableStore {
    inner: Arc<Mutex<HashMap<String, Value>>>,
}

impl VariableStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        self.inner.lock().ok()?.get(name).cloned()
    }

    pub fn insert(&self, name: String, value: Value) {
        if let Ok(mut map) = self.inner.lock() {
            map.insert(name, value);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().map_or(true, |m| m.is_empty())
    }

    /// Iterate over all variables. Returns a snapshot.
    pub fn entries(&self) -> Vec<(String, Value)> {
        self.inner
            .lock()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    }

    /// List all variable names. Used by the completer for `$` prefix completion.
    pub fn names(&self) -> Vec<String> {
        self.inner
            .lock()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Given a variable name, list the top-level field names if the value is a JSON object.
    /// Used by the completer for `$name.` field completion.
    pub fn field_names(&self, name: &str) -> Vec<String> {
        self.inner
            .lock()
            .ok()
            .and_then(|m| m.get(name).cloned())
            .and_then(|v| v.as_object().map(|obj| obj.keys().cloned().collect()))
            .unwrap_or_default()
    }

    /// Resolve a nested path like `["executionPayload", "blockHash"]` on a variable.
    /// Returns the field names at the resolved level if it's an object, or empty if not.
    /// Used by the completer for deep `$name.field.` completion.
    pub fn nested_field_names(&self, name: &str, path: &[&str]) -> Vec<String> {
        let value = self.inner.lock().ok().and_then(|m| m.get(name).cloned());
        let Some(mut current) = value else {
            return Vec::new();
        };
        for segment in path {
            match current.get(*segment) {
                Some(v) => current = v.clone(),
                None => return Vec::new(),
            }
        }
        current
            .as_object()
            .map(|obj| obj.keys().cloned().collect())
            .unwrap_or_default()
    }
}
