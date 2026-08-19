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
    /// Kept as a raw string here (rather than `Option<Tier>`) so
    /// `load_manifest` can validate it itself and produce an error that
    /// names both the offending workload and the bad value; see
    /// `Tier::parse`.
    #[serde(default)]
    pub tier: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub workload: Vec<WorkloadSpec>,
}

/// A workload's run-tier ceiling. Unlike the raw `WorkloadSpec::tier`
/// string, this only ever holds a value that `Tier::parse` accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Quick,
    Medium,
}

impl Tier {
    /// Parses a manifest `tier` string. Returns `None` for anything other
    /// than `"quick"`/`"medium"` — callers turn that into a hard manifest
    /// load error rather than silently falling back to a default, so a
    /// typo (e.g. `"quik"`) can't be silently dropped from quick/medium.
    fn parse(s: &str) -> Option<Self> {
        match s {
            "quick" => Some(Self::Quick),
            "medium" => Some(Self::Medium),
            _ => None,
        }
    }
}

pub fn load_manifest(path: &str) -> eyre::Result<Manifest> {
    let s = std::fs::read_to_string(path)?;
    let manifest: Manifest = toml::from_str(&s)?;
    for w in &manifest.workload {
        if let Some(tier) = &w.tier
            && Tier::parse(tier).is_none()
        {
            eyre::bail!(
                "workload {:?} has unknown tier {:?} (expected \"quick\" or \"medium\")",
                w.name,
                tier
            );
        }
    }
    Ok(manifest)
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
    ///
    /// `tier` is expected to already be validated (`load_manifest` rejects
    /// unknown values at load time). Defensively, an unrecognized non-empty
    /// string reaching here (e.g. a `Manifest` built by hand, bypassing
    /// `load_manifest`) is treated as neither `"quick"` nor `"medium"` —
    /// visible only under `Slow` — rather than silently defaulting to a
    /// lighter tier.
    pub fn includes_tier(self, tier: Option<&str>) -> bool {
        let effective = match tier {
            None => Some(Tier::Medium),
            Some(t) => Tier::parse(t),
        };
        match effective {
            Some(effective) => match self {
                RunMode::Quick => effective == Tier::Quick,
                RunMode::Medium => matches!(effective, Tier::Quick | Tier::Medium),
                RunMode::Slow => true,
            },
            None => self == RunMode::Slow,
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

    #[test]
    fn includes_tier_defensively_treats_unparseable_string_as_slow_only() {
        // load_manifest rejects this before it can ever reach includes_tier;
        // this only pins the defensive fallback for a hand-built spec that
        // bypasses that validation.
        assert!(!RunMode::Quick.includes_tier(Some("quik")));
        assert!(!RunMode::Medium.includes_tier(Some("quik")));
        assert!(RunMode::Slow.includes_tier(Some("quik")));
    }

    #[test]
    fn load_manifest_rejects_unknown_tier() {
        let toml = r#"
[[workload]]
name = "typo_tier"
type = "micro"
source = "x"
tier = "quik"
"#;
        let path = std::env::temp_dir().join(format!(
            "zkevm_bench_test_manifest_bad_tier_{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, toml).unwrap();

        let err = load_manifest(path.to_str().unwrap()).expect_err("bad tier should be rejected");
        let msg = err.to_string();
        assert!(msg.contains("typo_tier"), "should name the workload: {msg}");
        assert!(
            msg.contains("quik"),
            "should name the bad tier value: {msg}"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_manifest_accepts_known_tiers() {
        let toml = r#"
[[workload]]
name = "a"
type = "micro"
source = "x"
tier = "quick"

[[workload]]
name = "b"
type = "micro"
source = "x"
tier = "medium"

[[workload]]
name = "c"
type = "micro"
source = "x"
"#;
        let path = std::env::temp_dir().join(format!(
            "zkevm_bench_test_manifest_ok_tier_{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, toml).unwrap();

        let m = load_manifest(path.to_str().unwrap()).expect("valid tiers should load fine");
        assert_eq!(m.workload.len(), 3);

        std::fs::remove_file(&path).ok();
    }
}
