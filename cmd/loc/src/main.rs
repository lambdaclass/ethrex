use clap::Parser;
use report::{shell_summary, LinesOfCodeReport, LinesOfCodeReporterOptions};
use spinoff::{spinners::Dots, Color, Spinner};
use std::{collections::HashMap, env::current_dir, path::PathBuf};
use tokei::{Config, Language, LanguageType, Languages};

mod report;

fn count_loc(path: PathBuf, config: &Config) -> Language {
    let mut languages = Languages::new();
    languages.get_statistics(&[path], &["tests"], config);
    languages.get(&LanguageType::Rust).unwrap().clone()
}

fn main() {
    let opts = LinesOfCodeReporterOptions::parse();

    let mut spinner = Spinner::new(Dots, "Counting lines of code...", Color::Cyan);

    let ethrex = current_dir().unwrap();
    let ethrex_crates = ethrex.join("crates");
    let levm = ethrex_crates.join("vm");
    let ethrex_l2 = ethrex_crates.join("l2");

    let config = Config::default();

    let ethrex_loc = count_loc(ethrex, &config);
    let levm_loc = count_loc(levm, &config);
    let ethrex_l2_loc = count_loc(ethrex_l2, &config);

    spinner.success("Lines of code calculated!");

    let mut spinner = Spinner::new(Dots, "Generating report...", Color::Cyan);

    let new_report = LinesOfCodeReport {
        ethrex: ethrex_loc.code,
        ethrex_l1: ethrex_loc.code - ethrex_l2_loc.code - levm_loc.code,
        ethrex_l2: ethrex_l2_loc.code,
        levm: levm_loc.code,
    };

    if opts.detailed {
        let mut current_detailed_loc_report = HashMap::new();
        for report in ethrex_loc.reports {
            let file_path = report.name;
            // let file_name = file_path.file_name().unwrap().to_str().unwrap();
            // let dir_path = file_path.parent().unwrap();

            current_detailed_loc_report
                .entry(file_path.as_os_str().to_str().unwrap().to_owned())
                .and_modify(|e: &mut usize| *e += report.stats.code)
                .or_insert_with(|| report.stats.code);
        }

        std::fs::write(
            "current_detailed_loc_report.json",
            serde_json::to_string(&current_detailed_loc_report).unwrap(),
        )
        .expect("current_detailed_loc_report.json could not be written");
    } else if opts.compare_detailed {
        let current_detailed_loc_report: HashMap<String, usize> =
            std::fs::read_to_string("current_detailed_loc_report.json")
                .map(|s| serde_json::from_str(&s).unwrap())
                .expect("current_detailed_loc_report.json could not be read");

        let previous_detailed_loc_report: HashMap<String, usize> =
            std::fs::read_to_string("previous_detailed_loc_report.json")
                .map(|s| serde_json::from_str(&s).unwrap())
                .unwrap_or(current_detailed_loc_report.clone());

        std::fs::write(
            "detailed_loc_report.txt",
            report::pr_message(previous_detailed_loc_report, current_detailed_loc_report),
        )
        .unwrap();
    } else if opts.summary {
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
