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
    #[error("Could not parse config.toml")]
    TomlFormat,
}
