use clap::Parser;
use colored::Colorize;
use prettytable::{row, Table};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Parser)]
pub struct LinesOfCodeReporterOptions {
    #[arg(short, long, value_name = "SUMMARY", default_value = "false")]
    pub summary: bool,
    #[arg(short, long, value_name = "DETAILED", default_value = "false")]
    pub detailed: bool,
    #[arg(short, long, value_name = "PR_SUMMARY", default_value = "false")]
    pub compare_detailed: bool,
}

#[derive(Default, Serialize, Deserialize, Clone, Copy)]
pub struct LinesOfCodeReport {
    pub ethrex: usize,
    pub ethrex_l1: usize,
    pub ethrex_l2: usize,
    pub levm: usize,
}

pub fn pr_message(
    old_report: HashMap<String, usize>,
    new_report: HashMap<String, usize>,
) -> String {
    let sorted_file_paths = {
        let mut keys: Vec<_> = new_report.keys().collect();
        keys.sort();
        keys
    };

    let mut table = Table::new();

    table.add_row(row!["File", "Lines", "Diff"]);

    let mut total_lines_changed: i64 = 0;

    for file_path in sorted_file_paths {
        let current_loc = *new_report.get(file_path).unwrap() as i64;
        let previous_loc = *old_report.get(file_path).unwrap_or(&0) as i64;
        let loc_diff = current_loc - previous_loc;

        if loc_diff == 0 {
            continue;
        }

        total_lines_changed = if current_loc > previous_loc {
            total_lines_changed + loc_diff
        } else {
            total_lines_changed - loc_diff
        };

        table.add_row(row![
            file_path,
            current_loc,
            match current_loc.cmp(&previous_loc) {
                std::cmp::Ordering::Greater => format!("+{loc_diff}"),
                std::cmp::Ordering::Less => format!("{loc_diff}"),
                std::cmp::Ordering::Equal => "-".to_owned(),
            }
        ]);
    }

    if total_lines_changed != 0 {
        format!(
            "```\n{table}\nTotal lines changed: {}\n```",
            match total_lines_changed.cmp(&0) {
                std::cmp::Ordering::Greater => format!("+{total_lines_changed}"),
                std::cmp::Ordering::Less => format!("{total_lines_changed}"),
                std::cmp::Ordering::Equal => "-".to_owned(),
            }
        )
    } else {
        "The amount of lines of code in the project has not changed.".to_owned()
    }
}

pub fn slack_message(old_report: LinesOfCodeReport, new_report: LinesOfCodeReport) -> String {
    let ethrex_l1_diff = new_report.ethrex_l1.abs_diff(old_report.ethrex_l1);
    let ethrex_l2_diff = new_report.ethrex_l2.abs_diff(old_report.ethrex_l2);
    let levm_diff = new_report.levm.abs_diff(old_report.levm);
    let ethrex_diff_total = ethrex_l1_diff + ethrex_l2_diff + levm_diff;

    format!(
        r#"{{
    "blocks": [
        {{
            "type": "header",
            "text": {{
                "type": "plain_text",
                "text": "Daily Lines of Code Report"
            }}
        }},
        {{
            "type": "divider"
        }},
        {{
            "type": "section",
            "text": {{
                "type": "mrkdwn",
                "text": "*ethrex L1:* {} {}\n*ethrex L2:* {} {}\n*levm:* {} {}\n*ethrex (total):* {} {}"
            }}             
        }}
    ]
}}"#,
        new_report.ethrex_l1,
        match new_report.ethrex_l1.cmp(&old_report.ethrex_l1) {
            std::cmp::Ordering::Greater => format!("(+{ethrex_l1_diff})"),
            std::cmp::Ordering::Less => format!("(-{ethrex_l1_diff})"),
            std::cmp::Ordering::Equal => "".to_string(),
        },
        new_report.ethrex_l2,
        match new_report.ethrex_l2.cmp(&old_report.ethrex_l2) {
            std::cmp::Ordering::Greater => format!("(+{ethrex_l2_diff})"),
            std::cmp::Ordering::Less => format!("(-{ethrex_l2_diff})"),
            std::cmp::Ordering::Equal => "".to_string(),
        },
        new_report.levm,
        match new_report.levm.cmp(&old_report.levm) {
            std::cmp::Ordering::Greater => format!("(+{levm_diff})"),
            std::cmp::Ordering::Less => format!("(-{levm_diff})"),
            std::cmp::Ordering::Equal => "".to_string(),
        },
        new_report.ethrex,
        match new_report.ethrex.cmp(&old_report.ethrex) {
            std::cmp::Ordering::Greater => format!("(+{ethrex_diff_total})"),
            std::cmp::Ordering::Less => format!("(-{ethrex_diff_total})"),
            std::cmp::Ordering::Equal => "".to_string(),
        },
    )
}

pub fn github_step_summary(old_report: LinesOfCodeReport, new_report: LinesOfCodeReport) -> String {
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

pub fn shell_summary(new_report: LinesOfCodeReport) -> String {
    format!(
        "{}\n{}\n{} {}\n{} {}\n{} {}\n{} {}",
        "Lines of Code".bold(),
        "=============".bold(),
        "ethrex L1:".bold(),
        new_report.ethrex_l1,
        "ethrex L2:".bold(),
        new_report.ethrex_l2,
        "levm:".bold(),
        new_report.levm,
        "ethrex (total):".bold(),
        new_report.ethrex,
    )
}
