use ethrex_l2::{errors::ConfigError, parse_toml::TomlParserMode};
use std::env;

#[derive(Debug, thiserror::Error)]
pub enum TomlParserError {
    #[error("Failed to interact with .env file, error: {0}")]
    ConfigError(#[from] ConfigError),
    #[error("Missing Argument. Please use 'full' or 'prover'.")]
    MissingArgument,
    #[error("Invalid Mode. Please use 'full' or 'prover', used {0}.")]
    InvalidMode(String),
}

// Used for the binary,
// throws an error without the allow
// because the deployer.rs is importing the module
// and doesn't use the main function.
#[allow(dead_code)]
fn main() -> Result<(), TomlParserError> {
    let args: Vec<String> = env::args().collect();

    println!("arguments: {args:?}");
    if args.len() < 2 {
        return Err(TomlParserError::MissingArgument);
    }

    let mode_str = &args.get(1).ok_or(TomlParserError::MissingArgument)?;
    let mode = match mode_str.to_lowercase().as_str() {
        "full" => TomlParserMode::Full,
        "prover" => TomlParserMode::ProverClient,
        _ => {
            return Err(TomlParserError::InvalidMode(mode_str.to_string()));
        }
    };

    parse_toml(mode)
}

pub fn parse_toml(mode: TomlParserMode) -> Result<(), TomlParserError> {
    #[allow(clippy::expect_fun_call, clippy::expect_used)]
    let toml_config = std::env::var("CONFIG_FILE").expect(
        format!(
            "CONFIG_FILE environment variable not defined. Expected in {}, line: {}
If running locally, a reasonable value would be CONFIG_FILE=config.toml",
            file!(),
            line!()
        )
        .as_str(),
    );

    ethrex_l2::parse_toml::read_toml(toml_config, mode).map_err(From::from)
}
