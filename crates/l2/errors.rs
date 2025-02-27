#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(
        "\x1b[91mCould not find crates/l2/config.toml
Have you tried copying the provided example? Try:
cp crates/l2/config_example.toml crates/l2/config.toml
\x1b[0m
"
    )]
    TomlFileNotFound,

    #[error(
        "\x1b[91mCould not parse config.toml
Check the provided example to see if you have all the required fields.
The example can be found in:
crates/l2/config_example.toml

You can also see the differences with:
diff crates/l2/config_example.toml crates/l2/config.toml
\x1b[0m
"
    )]
    TomlFormat,
    #[error(
        "\x1b[91mCould not write to .env file.\x1b[0m
"
    )]
    EnvWriteError(String),
    #[error(
        "\x1b[91m .env file already exits, please check if it has any valuable information. If not, delete it
\x1b[0m
"
    )]
    EnvFileAlreadyExists,
}
