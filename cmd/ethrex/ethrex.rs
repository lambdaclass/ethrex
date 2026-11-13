use ethrex::integration::{Ethereum, run};

ethrex::global_allocator!();

fn main() -> eyre::Result<()> {
    run(Ethereum)
}
