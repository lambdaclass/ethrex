use ef_tests_levm::runner;

fn main() {
    let report = runner::run_ef_tests().unwrap();
    println!("{report}");
}
