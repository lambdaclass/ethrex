use crate::utils::config::errors::{ConfigError, TomlParserError};
use serde::Deserialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Debug)]
struct ProverClient {
    prover_server_endpoint: String,
    proving_time_ms: u64,
}

impl ProverClient {
    fn to_env(&self) -> String {
        let prefix = "PROVER_CLIENT";
        format!(
            "{prefix}_PROVER_SERVER_ENDPOINT={}
{prefix}_PROVING_TIME_MS={}
",
            self.prover_server_endpoint, self.proving_time_ms
        )
    }
}

fn write_to_env(config: String) -> Result<(), TomlParserError> {
    let env_file_path = {
        let cargo_manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::env::var("PROVER_ENV_FILE")
            .map(Into::into)
            .unwrap_or(cargo_manifest_dir.join(".env.prover"))
    };
    let env_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(env_file_path);
    match env_file {
        Ok(mut file) => {
            file.write_all(&config.into_bytes()).map_err(|_| {
                TomlParserError::EnvWriteError(format!(
                    "Couldn't write file in {}, line: {}",
                    file!(),
                    line!()
                ))
            })?;
        }
        Err(err) => {
            return Err(TomlParserError::EnvWriteError(format!(
                "Error: {}. Couldn't write file in {}, line: {}",
                err,
                file!(),
                line!()
            )));
        }
    };
    Ok(())
}

fn read_config(config_path: String) -> Result<(), ConfigError> {
    let toml_path = {
        let prover_client_config_file_name = std::env::var("PROVER_CLIENT_CONFIG_FILE")
            .unwrap_or("prover_client_config.toml".to_owned());
        Path::new(&config_path).join(prover_client_config_file_name)
    };
    let file = std::fs::read_to_string(toml_path)
        .map_err(|err| TomlParserError::TomlFileNotFound(format!("{err}: prover")))?;

    let config: ProverClient = toml::from_str(&file)
        .map_err(|err| TomlParserError::TomlFormat(format!("{err}: prover")))?;
    write_to_env(config.to_env())?;

    Ok(())
}

pub fn parse_configs() -> Result<(), ConfigError> {
    #[allow(clippy::expect_fun_call, clippy::expect_used)]
    let config_path = std::env::var("CONFIGS_PATH").expect(
        format!(
            "CONFIGS_PATH environment variable not defined. Expected in {}, line: {}
If running locally, a reasonable value would be CONFIGS_PATH=./configs",
            file!(),
            line!()
        )
        .as_str(),
    );

    read_config(config_path).map_err(From::from)
}
