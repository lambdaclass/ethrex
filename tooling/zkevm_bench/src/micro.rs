pub fn micro_to_program_input(
    _source: &str,
    _gas: Option<u64>,
) -> eyre::Result<ethrex_guest_program::input::ProgramInput> {
    eyre::bail!("micro workloads not implemented until Task 10")
}
