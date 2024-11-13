use crate::{
    commands::{self, config::EditConfigOpts},
    config::{ContractsConfig, EthereumRustL2Config, NetworkConfig, WalletConfig},
    utils::messages::{
        ADDRESS_PROMPT_MSG, CONFIG_CREATE_PROMPT_MSG, CONFIG_EDIT_PROMPT_MSG,
        L1_CHAIN_ID_PROMPT_MSG, L1_EXPLORER_URL_PROMPT_MSG, L1_RPC_URL_PROMPT_MSG,
        L2_CHAIN_ID_PROMPT_MSG, L2_EXPLORER_URL_PROMPT_MSG, L2_RPC_URL_PROMPT_MSG,
        PRIVATE_KEY_PROMPT_MSG,
    },
};
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Select};
use ethereum_types::H256;
use eyre::ContextCompat;
use secp256k1::SecretKey;
use std::{path::PathBuf, str::FromStr};

pub mod default_values;
use default_values::{
    DEFAULT_ADDRESS, DEFAULT_CONTRACTS_COMMON_BRIDGE_ADDRESS, DEFAULT_L1_CHAIN_ID,
    DEFAULT_L1_EXPLORER_URL, DEFAULT_L1_RPC_URL, DEFAULT_L2_CHAIN_ID, DEFAULT_L2_EXPLORER_URL,
    DEFAULT_L2_RPC_URL, DEFAULT_PRIVATE_KEY,
};

use super::messages::CONTRACTS_COMMON_BRIDGE_PROMPT_MSG;

pub const SELECTED_CONFIG_FILE_NAME: &str = ".selected";

pub fn configs_dir_path() -> eyre::Result<std::path::PathBuf> {
    let configs_dir_path = dirs::config_dir()
        .ok_or_else(|| eyre::eyre!("Could not find user's config directory"))?
        .join("ethereum-rust-l2-cli")
        .join("configs");
    if !configs_dir_path.exists() {
        std::fs::create_dir_all(&configs_dir_path)?;
    }
    Ok(configs_dir_path)
}

pub fn config_path(config_name: &str) -> eyre::Result<std::path::PathBuf> {
    Ok(configs_dir_path()?.join(format!("{config_name}.toml")))
}

pub fn prompt<T>(prompt: &str, default: T) -> eyre::Result<T>
where
    T: Clone + ToString + FromStr,
    <T as FromStr>::Err: ToString,
{
    Input::<T>::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .default(default)
        .show_default(true)
        .interact_text()
        .map_err(Into::into)
}

pub fn confirm(prompt: &str) -> eyre::Result<bool> {
    Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .show_default(true)
        .default(false)
        .interact()
        .map_err(Into::into)
}

pub fn config_file_names() -> eyre::Result<Vec<String>> {
    let config_file_names_with_selected_config_file = std::fs::read_dir(configs_dir_path()?)?
        .map(|entry| {
            entry
                .map_err(Into::into)
                .and_then(|entry| {
                    entry
                        .file_name()
                        .into_string()
                        .map_err(|e| eyre::eyre!("Invalid file name: {:?}", e.into_string()))
                })
                .map(|file_name| file_name.replace(".toml", ""))
        })
        .collect::<Result<Vec<String>, eyre::Error>>()?;
    let config_file_names = config_file_names_with_selected_config_file
        .into_iter()
        .filter(|file_name| file_name != SELECTED_CONFIG_FILE_NAME)
        .collect();
    Ok(config_file_names)
}

pub fn config_path_interactive_selection(prompt: &str) -> eyre::Result<PathBuf> {
    let configs = config_file_names()?;
    if configs.is_empty() {
        return Err(eyre::eyre!("No configs found"));
    }
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .items(&configs)
        .interact()?;
    config_path(configs.get(selection).context("No config selected")?)
}

pub fn prompt_config() -> eyre::Result<EthereumRustL2Config> {
    let prompted_config = EthereumRustL2Config {
        network: NetworkConfig {
            l1_rpc_url: prompt(L1_RPC_URL_PROMPT_MSG, DEFAULT_L1_RPC_URL.into())?,
            l1_chain_id: prompt(L1_CHAIN_ID_PROMPT_MSG, DEFAULT_L1_CHAIN_ID)?,
            l2_rpc_url: prompt(L2_RPC_URL_PROMPT_MSG, DEFAULT_L2_RPC_URL.into())?,
            l2_chain_id: prompt(L2_CHAIN_ID_PROMPT_MSG, DEFAULT_L2_CHAIN_ID)?,
            l2_explorer_url: prompt(L2_EXPLORER_URL_PROMPT_MSG, DEFAULT_L2_EXPLORER_URL.into())?,
            l1_explorer_url: prompt(L1_EXPLORER_URL_PROMPT_MSG, DEFAULT_L1_EXPLORER_URL.into())?,
        },
        wallet: WalletConfig {
            private_key: {
                let prompted_private_key = prompt(
                    PRIVATE_KEY_PROMPT_MSG,
                    format!(
                        "0x{}",
                        hex::encode(
                            SecretKey::from_slice(DEFAULT_PRIVATE_KEY.as_bytes())?.secret_bytes(),
                        )
                    ),
                )?;
                SecretKey::from_slice(H256::from_str(&prompted_private_key[2..])?.as_fixed_bytes())?
            },
            address: prompt(ADDRESS_PROMPT_MSG, DEFAULT_ADDRESS)?,
        },
        contracts: ContractsConfig {
            common_bridge: prompt(
                CONTRACTS_COMMON_BRIDGE_PROMPT_MSG,
                DEFAULT_CONTRACTS_COMMON_BRIDGE_ADDRESS,
            )?,
        },
    };
    Ok(prompted_config)
}

pub async fn confirm_config_creation(config_name: String) -> eyre::Result<()> {
    let create_confirmation = confirm(CONFIG_CREATE_PROMPT_MSG)?;
    if create_confirmation {
        Box::pin(async {
            commands::config::Command::Create { config_name }
                .run()
                .await
        })
        .await
    } else {
        println!("Aborted");
        Ok(())
    }
}

pub fn selected_config_path() -> eyre::Result<PathBuf> {
    Ok(configs_dir_path()?.join(SELECTED_CONFIG_FILE_NAME))
}

pub fn edit_config_by_name_interactively(
    config_path: &PathBuf,
) -> eyre::Result<EthereumRustL2Config> {
    let existing_config: EthereumRustL2Config =
        toml::from_str(&std::fs::read_to_string(config_path)?)?;
    let new_config = edit_existing_config_interactively(existing_config)?;
    Ok(new_config)
}

pub fn edit_config_by_name_with_args(
    config_path: &PathBuf,
    opts: EditConfigOpts,
) -> eyre::Result<EthereumRustL2Config> {
    let existing_config: EthereumRustL2Config =
        toml::from_str(&std::fs::read_to_string(config_path)?)?;
    let new_config = edit_existing_config_non_interactively(existing_config, opts)?;
    Ok(new_config)
}

pub fn edit_config_interactively() -> eyre::Result<(EthereumRustL2Config, PathBuf)> {
    let config_path = config_path_interactive_selection(CONFIG_EDIT_PROMPT_MSG)?;
    let existing_config: EthereumRustL2Config =
        toml::from_str(&std::fs::read_to_string(config_path.clone())?)?;
    let new_config = edit_existing_config_interactively(existing_config)?;
    Ok((new_config, config_path))
}

pub async fn set_new_config(config_path: PathBuf) -> eyre::Result<()> {
    Box::pin(async {
        commands::config::Command::Set {
            config_name: Some(
                config_path
                    .file_stem()
                    .context("There's no file name")?
                    .to_os_string()
                    .into_string()
                    .map_err(|e| eyre::eyre!("Invalid file name: {:?}", e.into_string()))?,
            ),
        }
        .run()
        .await
    })
    .await
}

pub fn edit_existing_config_interactively(
    existing_config: EthereumRustL2Config,
) -> eyre::Result<EthereumRustL2Config> {
    let config = EthereumRustL2Config {
        network: NetworkConfig {
            l1_rpc_url: prompt(L1_RPC_URL_PROMPT_MSG, existing_config.network.l1_rpc_url)?,
            l1_chain_id: prompt(L1_CHAIN_ID_PROMPT_MSG, existing_config.network.l1_chain_id)?,
            l2_rpc_url: prompt(L2_RPC_URL_PROMPT_MSG, existing_config.network.l2_rpc_url)?,
            l2_chain_id: prompt(L2_CHAIN_ID_PROMPT_MSG, existing_config.network.l2_chain_id)?,
            l2_explorer_url: prompt(
                L2_EXPLORER_URL_PROMPT_MSG,
                existing_config.network.l2_explorer_url,
            )?,
            l1_explorer_url: prompt(
                L1_EXPLORER_URL_PROMPT_MSG,
                existing_config.network.l1_explorer_url,
            )?,
        },
        wallet: WalletConfig {
            private_key: {
                let prompted_private_key = prompt(
                    PRIVATE_KEY_PROMPT_MSG,
                    format!(
                        "0x{}",
                        hex::encode(existing_config.wallet.private_key.secret_bytes())
                    ),
                )?;
                SecretKey::from_slice(H256::from_str(&prompted_private_key[2..])?.as_fixed_bytes())?
            },
            address: prompt(ADDRESS_PROMPT_MSG, existing_config.wallet.address)?,
        },
        contracts: ContractsConfig {
            common_bridge: prompt(
                CONTRACTS_COMMON_BRIDGE_PROMPT_MSG,
                existing_config.contracts.common_bridge,
            )?,
        },
    };
    Ok(config)
}

pub fn edit_existing_config_non_interactively(
    existing_config: EthereumRustL2Config,
    opts: EditConfigOpts,
) -> eyre::Result<EthereumRustL2Config> {
    let config = EthereumRustL2Config {
        network: NetworkConfig {
            l1_rpc_url: opts
                .l1_rpc_url
                .unwrap_or(existing_config.network.l1_rpc_url),
            l1_chain_id: opts
                .l1_chain_id
                .unwrap_or(existing_config.network.l1_chain_id),
            l2_rpc_url: opts
                .l2_rpc_url
                .unwrap_or(existing_config.network.l2_rpc_url),
            l2_chain_id: opts
                .l2_chain_id
                .unwrap_or(existing_config.network.l2_chain_id),
            l2_explorer_url: opts
                .l2_explorer_url
                .unwrap_or(existing_config.network.l2_explorer_url),
            l1_explorer_url: opts
                .l1_explorer_url
                .unwrap_or(existing_config.network.l1_explorer_url),
        },
        wallet: WalletConfig {
            address: opts.address.unwrap_or(existing_config.wallet.address),
            private_key: opts
                .private_key
                .map(|pk| {
                    SecretKey::from_slice(H256::from_str(&pk[2..]).unwrap().as_bytes()).unwrap()
                })
                .unwrap_or(existing_config.wallet.private_key),
        },
        contracts: ContractsConfig {
            common_bridge: opts
                .common_bridge
                .unwrap_or(existing_config.contracts.common_bridge),
        },
    };
    Ok(config)
}
