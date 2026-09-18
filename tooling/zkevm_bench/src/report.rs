use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AirCost {
    pub main: u64,
    pub opcodes: u64,
    pub precompiles: u64,
    pub memory: u64,
    pub base: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadResult {
    pub name: String,
    pub r#type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas: Option<u64>,
    pub air_cost: AirCost,
    pub steps: u64,
    /// Guest's peak zkVM memory footprint in bytes (ziskemu `RAM USAGE`).
    /// A footprint measurement, not a proving-cost component — distinct
    /// from `air_cost.memory`, which is the AIR cost of memory opcodes.
    #[serde(default)]
    pub zkvm_ram_bytes: u64,
    pub guest_output_ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub zisk_version: String,
    pub guest_elf_sha256: String,
    pub generated_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_commit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub meta: Meta,
    pub workloads: Vec<WorkloadResult>,
}

impl Report {
    pub fn write_json(&self, path: &str) -> eyre::Result<()> {
        let f = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(f, self)?;
        Ok(())
    }

    pub fn read_json(path: &str) -> eyre::Result<Report> {
        let f = std::fs::File::open(path)?;
        Ok(serde_json::from_reader(f)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_roundtrips_through_json() {
        let report = Report {
            meta: Meta {
                zisk_version: "v1.0.0-alpha".into(),
                guest_elf_sha256: "abc123".into(),
                generated_by: "ethrex-zkevm-bench".into(),
                git_commit: Some("deadbeef".into()),
            },
            workloads: vec![WorkloadResult {
                name: "mainnet_25087309".into(),
                r#type: "real-block".into(),
                category: Some("typical".into()),
                gas: Some(28_000_000),
                air_cost: AirCost {
                    main: 1,
                    opcodes: 2,
                    precompiles: 3,
                    memory: 4,
                    base: 5,
                    total: 15,
                },
                steps: 42,
                zkvm_ram_bytes: 7_304_122,
                guest_output_ok: true,
            }],
        };
        let json = serde_json::to_string(&report).unwrap();
        let back: Report = serde_json::from_str(&json).unwrap();
        assert_eq!(back.workloads.len(), 1);
        assert_eq!(back.workloads[0].air_cost.total, 15);
        assert_eq!(back.workloads[0].zkvm_ram_bytes, 7_304_122);
        assert_eq!(back.meta.zisk_version, "v1.0.0-alpha");
    }
}
