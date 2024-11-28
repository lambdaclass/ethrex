use std::path::PathBuf;
use tokei::{Config, LanguageType, Languages};

const CARGO_MANIFEST_DIR: &str = std::env!("CARGO_MANIFEST_DIR");

fn main() {
    let ethrex = PathBuf::from(CARGO_MANIFEST_DIR).join("../../");
    let levm = PathBuf::from(CARGO_MANIFEST_DIR).join("../../crates/vm");
    let ethrex_l2 = PathBuf::from(CARGO_MANIFEST_DIR).join("../../crates/l2");

    let config = Config::default();

    let mut languages = Languages::new();
    languages.get_statistics(&[ethrex.clone()], &["tests"], &config);
    let ethrex_loc = &languages.get(&LanguageType::Rust).unwrap();

    let mut languages = Languages::new();
    languages.get_statistics(&[levm], &["tests"], &config);
    let levm_loc = &languages.get(&LanguageType::Rust).unwrap();

    let mut languages = Languages::new();
    languages.get_statistics(&[ethrex_l2], &["tests"], &config);
    let ethrex_l2_loc = &languages.get(&LanguageType::Rust).unwrap();

    std::fs::write(
        "loc_report_slack.txt",
        slack_message(ethrex_loc.code, ethrex_l2_loc.code, levm_loc.code),
    )
    .unwrap();
    std::fs::write(
        "loc_report_github.txt",
        github_step_summary(ethrex_loc.code, ethrex_l2_loc.code, levm_loc.code),
    )
    .unwrap();
}

fn slack_message(ethrex_loc: usize, ethrex_l2_loc: usize, levm_loc: usize) -> String {
    format!(
        r#"*ethrex L1:* {}\n*ethrex L2:* {}\n*levm:* {}\n*ethrex (total):* {}"#,
        ethrex_loc - ethrex_l2_loc - levm_loc,
        ethrex_l2_loc,
        levm_loc,
        ethrex_loc,
    )
}

fn github_step_summary(ethrex_loc: usize, ethrex_l2_loc: usize, levm_loc: usize) -> String {
    format!(
        r#"```
ethrex loc summary
====================
ethrex L1: {}
ethrex L2: {}
levm: {}
ethrex (total): {}
```"#,
        ethrex_loc - ethrex_l2_loc - levm_loc,
        ethrex_l2_loc,
        levm_loc,
        ethrex_loc,
    )
}
