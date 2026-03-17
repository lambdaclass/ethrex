use serde::Deserialize;
use std::path::Path;

/// Runtime configuration for the guest program registry.
///
/// Controls which guest programs are loaded and which is the default.
/// When no config file is provided, all built-in programs are enabled.
#[derive(Debug, Clone, Deserialize)]
pub struct ProgramsConfig {
    /// The program_id used when no specific program is requested.
    #[serde(default = "default_program")]
    pub default_program: String,
    /// List of program_ids to register.  Only these programs will be available.
    #[serde(default = "default_enabled")]
    pub enabled_programs: Vec<String>,
}

fn default_program() -> String {
    "evm-l2".to_string()
}

fn default_enabled() -> Vec<String> {
    vec![
        "evm-l2".to_string(),
        "zk-dex".to_string(),
        "tokamon".to_string(),
    ]
}

impl Default for ProgramsConfig {
    fn default() -> Self {
        Self {
            default_program: default_program(),
            enabled_programs: default_enabled(),
        }
    }
}

impl ProgramsConfig {
    /// Load config from a TOML file.  If the file does not exist, returns the default config.
    pub fn load(path: &str) -> Result<Self, String> {
        let path = Path::new(path);
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        toml::from_str(&content).map_err(|e| format!("failed to parse {}: {e}", path.display()))
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn default_config_enables_all() {
        let cfg = ProgramsConfig::default();
        assert_eq!(cfg.default_program, "evm-l2");
        assert_eq!(cfg.enabled_programs.len(), 3);
        assert!(cfg.enabled_programs.contains(&"evm-l2".to_string()));
        assert!(cfg.enabled_programs.contains(&"zk-dex".to_string()));
        assert!(cfg.enabled_programs.contains(&"tokamon".to_string()));
    }

    #[test]
    fn load_missing_returns_default() {
        let cfg =
            ProgramsConfig::load("/nonexistent/path/programs.toml").expect("should return default");
        assert_eq!(cfg.default_program, "evm-l2");
        assert_eq!(cfg.enabled_programs.len(), 3);
    }

    #[test]
    fn load_valid_toml() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let path = dir.path().join("programs.toml");
        std::fs::write(
            &path,
            r#"
default_program = "zk-dex"
enabled_programs = ["zk-dex", "tokamon"]
"#,
        )
        .expect("write");
        let cfg = ProgramsConfig::load(path.to_str().expect("utf8")).expect("should parse");
        assert_eq!(cfg.default_program, "zk-dex");
        assert_eq!(cfg.enabled_programs, vec!["zk-dex", "tokamon"]);
    }

    #[test]
    fn filtered_registry() {
        use crate::registry::GuestProgramRegistry;
        use ethrex_guest_program::programs::{
            EvmL2GuestProgram, TokammonGuestProgram, ZkDexGuestProgram,
        };
        use std::sync::Arc;

        // Config that only enables zk-dex.
        let config = ProgramsConfig {
            default_program: "zk-dex".to_string(),
            enabled_programs: vec!["zk-dex".to_string()],
        };

        let mut registry = GuestProgramRegistry::new(&config.default_program);
        let all_programs: Vec<(String, Arc<dyn ethrex_guest_program::traits::GuestProgram>)> = vec![
            ("evm-l2".to_string(), Arc::new(EvmL2GuestProgram)),
            ("zk-dex".to_string(), Arc::new(ZkDexGuestProgram)),
            ("tokamon".to_string(), Arc::new(TokammonGuestProgram)),
        ];
        for (id, program) in all_programs {
            if config.enabled_programs.contains(&id) {
                registry.register(program);
            }
        }

        assert!(registry.get("zk-dex").is_some());
        assert!(registry.get("evm-l2").is_none());
        assert!(registry.get("tokamon").is_none());
        assert_eq!(registry.program_ids().len(), 1);
    }
}
