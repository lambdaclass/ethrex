use clap::Args;
use ethrex_l2::utils::test_data_io::{generate_program_input, read_chain_file, read_genesis_file};
use ethrex_prover_lib::prover::create_prover;

#[derive(Args)]
pub(crate) struct Command {
    #[clap(
        short = 'g',
        long = "genesis",
        help = "Path to the file containing the genesis block."
    )]
    genesis: String,
    #[clap(
        short = 'c',
        long = "chain",
        help = "Path to the file containing the test chain."
    )]
    chain: String,
    #[clap(
        short = 'n',
        long = "block-number",
        help = "Number of the block in the test chain to prove."
    )]
    block_number: usize,
}

impl Command {
    pub fn run(self) -> eyre::Result<()> {
        let genesis = read_genesis_file(&self.genesis);
        let chain = read_chain_file(&self.chain);
        let program_input = generate_program_input(genesis, chain, self.block_number)?;

        let mut prover = create_prover(ethrex_l2::proposer::prover_server::ProverType::RISC0);
        prover.prove(program_input).expect("proving failed");
        println!(
            "Total gas consumption: {}",
            prover
                .get_gas()
                .expect("failed to deserialize gas consumption")
        );
        Ok(())
    }
}
