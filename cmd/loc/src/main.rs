use std::path::PathBuf;
use tokei::{Config, LanguageType, Languages};

const CARGO_MANIFEST_DIR: &str = std::env!("CARGO_MANIFEST_DIR");

fn main() {
    let ethrex = PathBuf::from(CARGO_MANIFEST_DIR).join("../../");
    let levm = PathBuf::from(CARGO_MANIFEST_DIR).join("../../crates/vm");
    let ethrex_l2 = PathBuf::from(CARGO_MANIFEST_DIR).join("../../crates/l2");

    let config = Config::default();

    let mut languages = Languages::new();
    languages.get_statistics(&[ethrex.clone()], &[], &config);
    let ethrex_loc = &languages.get(&LanguageType::Rust).unwrap();

    let mut languages = Languages::new();
    languages.get_statistics(&[levm], &[], &config);
    let levm_loc = &languages.get(&LanguageType::Rust).unwrap();

    let mut languages = Languages::new();
    languages.get_statistics(&[ethrex_l2], &[], &config);
    let ethrex_l2_loc = &languages.get(&LanguageType::Rust).unwrap();

    let report = format!(
        r#"```
ethrex loc summary
====================
ethrex L1: {}
ethrex L2: {}
levm: {}
ethrex (total): {}
```"#,
        ethrex_loc.code - ethrex_l2_loc.code - levm_loc.code,
        ethrex_l2_loc.code,
        levm_loc.code,
        ethrex_loc.code,
    );

    std::fs::write("loc_report.md", report).unwrap();
}
