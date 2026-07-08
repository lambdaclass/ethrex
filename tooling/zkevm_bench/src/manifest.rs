use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkloadType {
    RealBlock,
    Micro,
    Stress,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkloadSpec {
    pub name: String,
    pub r#type: WorkloadType,
    #[serde(default)]
    pub category: Option<String>,
    pub source: String,
    #[serde(default)]
    pub gas: Option<u64>,
    /// Run-tier ceiling ("quick"|"medium"); `None` is treated as "medium".
    #[serde(default)]
    pub tier: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub workload: Vec<WorkloadSpec>,
}

pub fn load_manifest(path: &str) -> eyre::Result<Manifest> {
    let s = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&s)?)
}

/// Run-tier ceiling selected via `--mode`. Workloads declare an optional
/// `tier` of `"quick"` or `"medium"` (unset == `"medium"`); the run mode
/// decides which tiers are included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    Quick,
    Medium,
    Slow,
}

impl RunMode {
    pub fn parse(s: &str) -> eyre::Result<Self> {
        match s {
            "quick" => Ok(Self::Quick),
            "medium" => Ok(Self::Medium),
            "slow" => Ok(Self::Slow),
            other => eyre::bail!("unknown --mode {other:?} (expected quick|medium|slow)"),
        }
    }

    /// Whether a workload declaring `tier` (`None` defaults to `"medium"`)
    /// should run under this mode. `quick` ⊆ `medium` ⊆ `slow` by
    /// construction: `Slow` always includes, `Medium` includes `Quick`'s
    /// set plus `"medium"`/unset, `Quick` includes only `"quick"`.
    pub fn includes_tier(self, tier: Option<&str>) -> bool {
        let effective = tier.unwrap_or("medium");
        match self {
            RunMode::Quick => effective == "quick",
            RunMode::Medium => matches!(effective, "quick" | "medium"),
            RunMode::Slow => true,
        }
    }
}

impl Manifest {
    pub fn filtered(&self, pattern: Option<&str>) -> Vec<&WorkloadSpec> {
        self.workload
            .iter()
            .filter(|w| pattern.map(|p| w.name.contains(p)).unwrap_or(true))
            .collect()
    }

    /// Combines name-pattern filtering with tier-ceiling selection for a
    /// given run `mode`.
    pub fn filtered_for_run(&self, pattern: Option<&str>, mode: RunMode) -> Vec<&WorkloadSpec> {
        self.filtered(pattern)
            .into_iter()
            .filter(|w| mode.includes_tier(w.tier.as_deref()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_manifest_and_filters() {
        let toml = r#"
[[workload]]
name = "mainnet_25087309_typical"
type = "real-block"
category = "typical"
source = "fixtures/blocks/cache_mainnet_25087309.json.gz"

[[workload]]
name = "eest_100M_keccak"
type = "micro"
category = "keccak"
source = "vectors_zkevm/eest/foo.json"
gas = 100000000
"#;
        let m: Manifest = toml::from_str(toml).unwrap();
        assert_eq!(m.workload.len(), 2);
        assert!(matches!(m.workload[0].r#type, WorkloadType::RealBlock));
        assert_eq!(m.filtered(Some("keccak")).len(), 1);
        assert_eq!(m.filtered(None).len(), 2);
    }

    #[test]
    fn tier_selection_is_nested_quick_medium_slow() {
        let toml = r#"
[[workload]]
name = "a_quick"
type = "micro"
source = "x"
tier = "quick"

[[workload]]
name = "b_medium_explicit"
type = "micro"
source = "x"
tier = "medium"

[[workload]]
name = "c_medium_default"
type = "real-block"
source = "x"

[[workload]]
name = "d_stress"
type = "stress"
source = "x"
"#;
        let m: Manifest = toml::from_str(toml).unwrap();
        assert_eq!(m.workload.len(), 4);

        fn names(specs: Vec<&WorkloadSpec>) -> Vec<&str> {
            specs.into_iter().map(|w| w.name.as_str()).collect()
        }

        let quick = names(m.filtered_for_run(None, RunMode::Quick));
        let medium = names(m.filtered_for_run(None, RunMode::Medium));
        let slow = names(m.filtered_for_run(None, RunMode::Slow));

        assert_eq!(quick, vec!["a_quick"]);
        assert_eq!(
            medium,
            vec![
                "a_quick",
                "b_medium_explicit",
                "c_medium_default",
                "d_stress"
            ]
        );
        assert_eq!(slow, medium);

        // quick ⊆ medium ⊆ slow.
        assert!(quick.iter().all(|n| medium.contains(n)));
        assert!(medium.iter().all(|n| slow.contains(n)));
    }

    #[test]
    fn run_mode_parse_rejects_unknown_mode() {
        assert!(RunMode::parse("quick").is_ok());
        assert!(RunMode::parse("medium").is_ok());
        assert!(RunMode::parse("slow").is_ok());
        assert!(RunMode::parse("bogus").is_err());
    }
}
