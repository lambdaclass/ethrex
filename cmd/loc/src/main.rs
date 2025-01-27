use clap::Parser;
use report::{shell_summary, LinesOfCodeReport, LinesOfCodeReporterOptions};
use spinoff::{spinners::Dots, Color, Spinner};
use std::{collections::HashMap, env::current_dir, fs::DirEntry, path::PathBuf};
use tokei::{Config, Language, LanguageType, Languages};

mod report;

fn count_crates_loc(crates_path: &PathBuf, config: &Config) -> Vec<(String, usize)> {
    let nested_dirs = vec!["networking"];

    let top_level_crate_dirs = std::fs::read_dir(crates_path)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| !nested_dirs.contains(&e.file_name().to_str().unwrap()))
        .collect::<Vec<DirEntry>>();

    let nested_crate_dirs: Vec<DirEntry> = nested_dirs
        .iter()
        .flat_map(|nested_dir| {
            std::fs::read_dir(crates_path.join(nested_dir))
                .unwrap()
                .filter_map(|e| e.ok())
                .collect::<Vec<DirEntry>>()
        })
        .collect();

    let crate_dirs = [top_level_crate_dirs, nested_crate_dirs].concat();

    let mut ethrex_crates_loc: Vec<(String, usize)> = crate_dirs
        .into_iter()
        .filter_map(|crate_dir_entry| {
            let crate_path = crate_dir_entry.path();

            if let Some(crate_loc) = count_loc(crate_path.clone(), config) {
                Some((
                    crate_path.file_name().unwrap().to_str().unwrap().to_owned(),
                    crate_loc.code,
                ))
            } else {
                None
            }
        })
        .collect();

    ethrex_crates_loc.sort_by_key(|(_crate_name, loc)| *loc);
    ethrex_crates_loc.reverse();
    ethrex_crates_loc
}

fn count_loc(path: PathBuf, config: &Config) -> Option<Language> {
    let mut languages = Languages::new();
    languages.get_statistics(&[path], &["tests"], config);
    languages.get(&LanguageType::Rust).cloned()
}

fn main() {
    let opts = LinesOfCodeReporterOptions::parse();

    let mut spinner = Spinner::new(Dots, "Counting lines of code...", Color::Cyan);

    let ethrex_path = current_dir().unwrap();
    let ethrex_crates_path = ethrex_path.join("crates");
    let levm_path = ethrex_crates_path.join("vm");
    let ethrex_l2_path = ethrex_crates_path.join("l2");

    let config = Config::default();

    let ethrex_loc = count_loc(ethrex_path, &config).unwrap();
    let levm_loc = count_loc(levm_path, &config).unwrap();
    let ethrex_l2_loc = count_loc(ethrex_l2_path, &config).unwrap();
    let ethrex_crates_loc = count_crates_loc(&ethrex_crates_path, &config);

    spinner.success("Lines of code calculated!");

    let mut spinner = Spinner::new(Dots, "Generating report...", Color::Cyan);

    let new_report = LinesOfCodeReport {
        ethrex: ethrex_loc.code,
        ethrex_l1: ethrex_loc.code - ethrex_l2_loc.code - levm_loc.code,
        ethrex_l2: ethrex_l2_loc.code,
        levm: levm_loc.code,
        ethrex_crates: ethrex_crates_loc,
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
            .unwrap_or(new_report.clone());

        std::fs::write(
            "loc_report_slack.txt",
            report::slack_message(old_report.clone(), new_report.clone()),
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
