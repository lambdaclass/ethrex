use clap::Subcommand;
use keccak_hash::H256;
use secp256k1::SecretKey;

#[derive(Subcommand)]
pub(crate) enum Command {
    #[clap(about = "Convert private key to address.")]
    PrivateKeyToAddress {
        #[arg(long = "pk", help = "Private key in hex format.", required = true)]
        private_key: String,
    },
}

impl Command {
    pub fn run(self) -> eyre::Result<()> {
        match self {
            Command::PrivateKeyToAddress { private_key } => {
                let pk_str = private_key.strip_prefix("0x").unwrap_or(&private_key);
                let pk_h256 = pk_str.parse::<H256>()?;
                let pk_bytes = pk_h256.as_bytes();
                let secret_key = SecretKey::from_slice(pk_bytes)?;
                let address = ethrex_l2_sdk::get_address_from_secret_key(&secret_key)?;
                println!("{address:#x}");
            }
        }
        Ok(())
    }
}
