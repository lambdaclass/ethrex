/** Map network name to public explorer URL */
const EXPLORER_URLS = {
  sepolia: 'https://sepolia.etherscan.io',
  holesky: 'https://holesky.etherscan.io',
  mainnet: 'https://etherscan.io',
};

/** Build external L1 config props from deployment config (shared by routes + engine) */
function getExternalL1Config(deployment) {
  const depConfig = deployment.config ? JSON.parse(deployment.config) : {};
  const isExternal = depConfig.mode === 'testnet';
  const testnetCfg = depConfig.testnet || {};
  const explorerUrl = testnetCfg.l1ExplorerUrl || EXPLORER_URLS[testnetCfg.network] || '';
  return {
    skipL1Explorer: isExternal,
    ...(isExternal && {
      l1RpcUrl: testnetCfg.l1RpcUrl,
      l1ChainId: testnetCfg.l1ChainId,
      l1ExplorerUrl: explorerUrl,
      l1NetworkName: testnetCfg.network,
      isExternalL1: true,
    }),
  };
}

/** Build public access config from deployment DB row */
function getPublicAccessConfig(deployment) {
  if (!deployment.is_public || !deployment.public_domain) return {};

  const domain = deployment.public_domain;
  return {
    publicDomain: domain,
    publicBaseUrl: `http://${domain}`,
    publicL2RpcUrl: deployment.public_l2_rpc_url || `http://${domain}:${deployment.l2_port}`,
    publicL2ExplorerUrl: deployment.public_l2_explorer_url || `http://${domain}:${deployment.tools_l2_explorer_port}`,
    publicL1ExplorerUrl: deployment.public_l1_explorer_url || (
      deployment.l1_port ? `http://${domain}:${deployment.tools_l1_explorer_port}` : null
    ),
    publicDashboardUrl: deployment.public_dashboard_url || `http://${domain}:${deployment.tools_bridge_ui_port}`,
  };
}

/** Build toolsPorts object from deployment DB row (shared by routes + engine) */
function getToolsPorts(deployment) {
  return {
    toolsL1ExplorerPort: deployment.tools_l1_explorer_port,
    toolsL2ExplorerPort: deployment.tools_l2_explorer_port,
    toolsBridgeUIPort: deployment.tools_bridge_ui_port,
    toolsDbPort: deployment.tools_db_port,
    toolsMetricsPort: deployment.tools_metrics_port,
    l1Port: deployment.l1_port,
    l2Port: deployment.l2_port,
    ...getExternalL1Config(deployment),
    ...getPublicAccessConfig(deployment),
  };
}

module.exports = { getExternalL1Config, getPublicAccessConfig, getToolsPorts };
