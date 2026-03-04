pub mod calldata;
pub mod merkle_tree;
pub mod messages;
pub mod privileged_transactions;
pub mod prover;
pub mod sequencer_state;
pub mod utils;

/// Maps a guest program ID string to its on-chain `programTypeId`.
///
/// This is the single source of truth for program type IDs used by
/// the deployer, committer, and proof sender.
///
/// Returns 0 for unknown programs.
pub fn resolve_program_type_id(program_id: &str) -> u8 {
    match program_id {
        "evm-l2" => 1,
        "zk-dex" => 2,
        "tokamon" => 3,
        _ => 0,
    }
}
