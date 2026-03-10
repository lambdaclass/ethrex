/** Build external L1 config props from deployment config (shared by routes + engine) */
function getExternalL1Config(deployment) {
  const depConfig = deployment.config ? JSON.parse(deployment.config) : {};
  const isExternal = depConfig.mode === 'testnet';
  const testnetCfg = depConfig.testnet || {};
  return {
    skipL1Explorer: isExternal,
    ...(isExternal && {
      l1RpcUrl: testnetCfg.l1RpcUrl,
      l1ChainId: testnetCfg.l1ChainId,
      l1ExplorerUrl: testnetCfg.l1ExplorerUrl,
      l1NetworkName: testnetCfg.network,
      isExternalL1: true,
    }),
  };
}

module.exports = { getExternalL1Config };
