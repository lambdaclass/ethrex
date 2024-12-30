use clap::Parser;
use report::{shell_summary, LinesOfCodeReport, LinesOfCodeReporterOptions};
use spinoff::{spinners::Dots, Color, Spinner};
use std::path::PathBuf;
use tokei::{Config, LanguageType, Languages};

mod report;

const CARGO_MANIFEST_DIR: &str = std::env!("CARGO_MANIFEST_DIR");

fn main() {
    let opts = LinesOfCodeReporterOptions::parse();

    let mut spinner = Spinner::new(Dots, "Counting lines of code...", Color::Cyan);

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

    spinner.success("Lines of code calculated!");

    let mut spinner = Spinner::new(Dots, "Generating report...", Color::Cyan);

    let new_report = LinesOfCodeReport {
        ethrex: ethrex_loc.code,
        ethrex_l1: ethrex_loc.code - ethrex_l2_loc.code - levm_loc.code,
        ethrex_l2: ethrex_l2_loc.code,
        levm: levm_loc.code,
    };

    if opts.summary {
        spinner.success("Report generated!");
        println!("{}", shell_summary(new_report));
    } else {
        std::fs::write(
            "loc_report.json",
            serde_json::to_string(&new_report).unwrap(),
        )
        .expect("loc_report.json could not be written");

        let old_report: LinesOfCodeReport = std::fs::read_to_string("loc_report.json.old")
            .map(|s| serde_json::from_str(&s).unwrap())
            .unwrap_or(new_report);

        std::fs::write(
            "loc_report_slack.txt",
            report::slack_message(old_report, new_report),
        )
        .unwrap();
        std::fs::write(
            "loc_report_github.txt",
            report::github_step_summary(old_report, new_report),
        )
        .unwrap();

        spinner.success("Report generated!");
    }
}
