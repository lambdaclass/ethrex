use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokei::{Config, LanguageType, Languages};

const CARGO_MANIFEST_DIR: &str = std::env!("CARGO_MANIFEST_DIR");

#[derive(Default, Serialize, Deserialize, Clone, Copy)]
pub struct LinesOfCodeReport {
    ethrex: usize,
    ethrex_l1: usize,
    ethrex_l2: usize,
    levm: usize,
}

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

    let new_report = LinesOfCodeReport {
        ethrex: ethrex_loc.code,
        ethrex_l1: ethrex_loc.code - ethrex_l2_loc.code - levm_loc.code,
        ethrex_l2: ethrex_l2_loc.code,
        levm: levm_loc.code,
    };

    std::fs::write(
        "loc_report.json",
        serde_json::to_string(&new_report).unwrap(),
    )
    .expect("loc_report.json could not be written");

    let old_report: LinesOfCodeReport = std::fs::read_to_string("loc_report.json.old")
        .map(|s| serde_json::from_str(&s).unwrap())
        .unwrap_or_default();

    std::fs::write(
        "loc_report_slack.txt",
        slack_message(old_report, new_report),
    )
    .unwrap();
    std::fs::write(
        "loc_report_github.txt",
        github_step_summary(old_report, new_report),
    )
    .unwrap();
}

fn slack_message(old_report: LinesOfCodeReport, new_report: LinesOfCodeReport) -> String {
    let ethrex_l1_diff = new_report.ethrex_l1.abs_diff(old_report.ethrex_l1);
    let ethrex_l2_diff = new_report.ethrex_l2.abs_diff(old_report.ethrex_l2);
    let levm_diff = new_report.levm.abs_diff(old_report.levm);
    let ethrex_diff_total = ethrex_l1_diff + ethrex_l2_diff + levm_diff;

    format!(
        r#"*ethrex L1:* {} {}\n*ethrex L2:* {} {}\n*levm:* {} {}\n*ethrex (total):* {} {}"#,
        new_report.ethrex_l1,
        if new_report.ethrex > old_report.ethrex {
            format!("(+{ethrex_l1_diff})")
        } else {
            format!("(-{ethrex_l1_diff})")
        },
        new_report.ethrex_l2,
        if new_report.ethrex_l2 > old_report.ethrex_l2 {
            format!("(+{ethrex_l2_diff})")
        } else {
            format!("(-{ethrex_l2_diff})")
        },
        new_report.levm,
        if new_report.levm > old_report.levm {
            format!("(+{levm_diff})")
        } else {
            format!("(-{levm_diff})")
        },
        new_report.ethrex,
        if new_report.ethrex > old_report.ethrex {
            format!("(+{ethrex_diff_total})")
        } else {
            format!("(-{ethrex_diff_total})")
        },
    )
}

fn github_step_summary(old_report: LinesOfCodeReport, new_report: LinesOfCodeReport) -> String {
    let ethrex_l1_diff = new_report.ethrex_l1.abs_diff(old_report.ethrex_l1);
    let ethrex_l2_diff = new_report.ethrex_l2.abs_diff(old_report.ethrex_l2);
    let levm_diff = new_report.levm.abs_diff(old_report.levm);
    let ethrex_diff_total = ethrex_l1_diff + ethrex_l2_diff + levm_diff;

    format!(
        r#"```
ethrex loc summary
====================
ethrex L1: {} {}
ethrex L2: {} {}
levm: {} ({})
ethrex (total): {} {}
```"#,
        new_report.ethrex_l1,
        if new_report.ethrex > old_report.ethrex {
            format!("(+{ethrex_l1_diff})")
        } else {
            format!("(-{ethrex_l1_diff})")
        },
        new_report.ethrex_l2,
        if new_report.ethrex_l2 > old_report.ethrex_l2 {
            format!("(+{ethrex_l2_diff})")
        } else {
            format!("(-{ethrex_l2_diff})")
        },
        new_report.levm,
        if new_report.levm > old_report.levm {
            format!("(+{levm_diff})")
        } else {
            format!("(-{levm_diff})")
        },
        new_report.ethrex,
        if new_report.ethrex > old_report.ethrex {
            format!("(+{ethrex_diff_total})")
        } else {
            format!("(-{ethrex_diff_total})")
        },
    )
}
