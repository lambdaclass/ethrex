use crate::sequencer::errors::ProverServerError;
use ethrex_rpc::clients::eth::EthClient;

use std::process::Command;

const QPL_TOOL_PATH: &str =
    "tee/automata-dcap-qpl/automata-dcap-qpl-tool/target/release/automata-dcap-qpl-tool";

pub async fn prepare_quote_prerequisites(
    eth_client: &EthClient,
    rpc_url: &str,
    private_key_str: &str,
    quote: &str,
) -> Result<(), ProverServerError> {
    let chain_id = eth_client
        .get_chain_id()
        .await
        .map_err(ProverServerError::EthClientError)?;

    Command::new(QPL_TOOL_PATH)
        .args([
            "--chain_id",
            &chain_id.to_string(),
            "--rpc_url",
            rpc_url,
            "-p",
            private_key_str,
            "--quote_hex",
            quote,
        ])
        .output()
        .map_err(ProverServerError::ComandError)?;
    Ok(())
}
