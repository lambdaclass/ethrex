use clap::Subcommand;
use secp256k1::SecretKey;

#[derive(Subcommand)]
pub(crate) enum Command {
    #[clap(
        name = "address",
        visible_aliases = ["a"],
        about = "Convert private key to address."
    )]
    PrivateKeyToAddress {
        #[arg(help = "Private key in hex format.", required = true)]
        private_key: String,
    },
}

impl Command {
    pub fn run(self) -> eyre::Result<()> {
        match self {
            Command::PrivateKeyToAddress { private_key } => {
                let secret_key = private_key
                    .strip_prefix("0x")
                    .unwrap_or(&private_key)
                    .parse::<SecretKey>()?;
                let address = ethrex_l2_sdk::get_address_from_secret_key(&secret_key)?;
                println!("{address:#x}");
            }
        }
        Ok(())
    }
}
