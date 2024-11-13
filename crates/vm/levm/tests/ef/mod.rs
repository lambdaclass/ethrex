#![allow(clippy::unwrap_used)]

mod deserialize;
mod report;
mod runner;
mod test;

#[test]
#[ignore]
fn testito() {
    let report = runner::run_ef_tests().unwrap();
    println!("{report}");
}
