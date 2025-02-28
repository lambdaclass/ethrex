#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(
        "\x1b[91mCould not find crates/l2/config.toml
Have you tried copying the provided example? Try:
cp {manifest_dir}/config_example.toml {manifest_dir}/config.toml
\x1b[0m
",
        manifest_dir = env!("CARGO_MANIFEST_DIR")

    )]
    TomlFileNotFound,

    #[error(
        "\x1b[91mCould not parse config.toml
Check the provided example to see if you have all the required fields.
The example can be found in:
crates/l2/config_example.toml
You can also see the differences with:
diff {manifest_dir}/config_example.toml {manifest_dir}/config.toml
\x1b[0m
",
        manifest_dir = env!("CARGO_MANIFEST_DIR")

    )]
    TomlFormat,
    #[error(
        "\x1b[91mCould not write to .env file.\x1b[0m
"
    )]
    EnvWriteError(String),
}
