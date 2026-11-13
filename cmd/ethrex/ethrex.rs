use ethrex::chain::{ETHEREUM, run};

ethrex::global_allocator!();

fn main() -> eyre::Result<()> {
    run(ETHEREUM, GLOBAL_ALLOCATOR)
}
