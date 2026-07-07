use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkloadType {
    RealBlock,
    Micro,
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

impl Manifest {
    pub fn filtered(&self, pattern: Option<&str>) -> Vec<&WorkloadSpec> {
        self.workload
            .iter()
            .filter(|w| pattern.map(|p| w.name.contains(p)).unwrap_or(true))
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
}
