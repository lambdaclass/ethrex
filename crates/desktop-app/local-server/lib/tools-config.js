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

module.exports = { getExternalL1Config };
