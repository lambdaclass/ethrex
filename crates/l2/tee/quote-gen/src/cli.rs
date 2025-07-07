use clap::{ArgAction, Parser};
use secp256k1::SecretKey;

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
        help = "Show the registers value and exit. Useful for the TDXVerifier contract deployment.",
        exclusive = true
    )]
    pub show_registers: bool,
    #[arg(long = "private-key", help = "Private key to sign the messages with")]
    pub private_key: Option<SecretKey>,
}
