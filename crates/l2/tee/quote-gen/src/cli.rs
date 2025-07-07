use clap::{ArgAction, Parser};

#[derive(Parser)]
#[command(
    name = "ethrex-tdx",
    author = "Lambdaclass",
    about = "ethrex TDX prover"
)]
pub struct CLI {
    #[arg(
        long = "registers",
        action = ArgAction::SetTrue,
        help = "Show the registers value and exit",
        exclusive = true
    )]
    pub show_registers: bool,
}
