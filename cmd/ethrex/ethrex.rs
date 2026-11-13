use ethrex::chain::{Ethereum, run};

ethrex::global_allocator!();

fn main() -> eyre::Result<()> {
    run(Ethereum)
}
