#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(
        "Could not find crates/l2/configs/{0}
Have you tried copying the provided example? Try:
cp {manifest_dir}/configs/*_example.toml {manifest_dir}/configs/*.toml
",
        manifest_dir = env!("CARGO_MANIFEST_DIR")

    )]
    TomlFileNotFound(String),

    #[error(
        "Could not parse crates/l2/configs/{0}
Check the provided example to see if you have all the required fields.
The example can be found at:
crates/l2/configs/*_example.toml
You can also see the differences with:
diff {manifest_dir}/configs/*_example.toml {manifest_dir}/configs/*.toml
",
        manifest_dir = env!("CARGO_MANIFEST_DIR")

    )]
    TomlFormat(String),
    #[error(
        "\x1b[91mCould not write to .env file.\x1b[0m
"
    )]
    EnvWriteError(String),
    #[error("Internal parsing error.")]
    InternalParsingError,
}
