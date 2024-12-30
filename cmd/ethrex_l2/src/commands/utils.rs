use clap::Subcommand;
use ethrex_l2_sdk::calldata::encode_calldata;

#[derive(Subcommand)]
pub(crate) enum Command {
    #[clap(about = "Get ABI encode string from a function signature and arguments")]
    Calldata {
        #[clap(long)]
        signature: String,
        #[clap(long)]
        args: String,
        #[clap(long, required = false, default_value = "false")]
        only_args: bool,
    },
}

impl Command {
    pub async fn run(self) -> eyre::Result<()> {
        match self {
            Command::Calldata {
                signature,
                args,
                only_args,
            } => {
                let calldata = encode_calldata(&signature, &args, only_args)?;
                println!("0x{}", hex::encode(calldata));
            }
        };
        Ok(())
    }
}
