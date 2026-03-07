/**
 * JSON-RPC client for L1/L2 health checks and monitoring.
 */

async function rpcCall(url, method, params = []) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 5000);
  try {
    const res = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", method, params, id: 1 }),
      signal: controller.signal,
    });
    const data = await res.json();
    if (data.error) throw new Error(data.error.message);
    return data.result;
  } finally {
    clearTimeout(timeout);
  }
}

async function getBlockNumber(url) {
  const hex = await rpcCall(url, "eth_blockNumber");
  return parseInt(hex, 16);
}

async function getChainId(url) {
  const hex = await rpcCall(url, "eth_chainId");
  return parseInt(hex, 16);
}

async function getBalance(url, address) {
  const hex = await rpcCall(url, "eth_getBalance", [address, "latest"]);
  return hex;
}

async function isHealthy(url) {
  try {
    await getBlockNumber(url);
    return true;
  } catch {
    return false;
  }
}

module.exports = { rpcCall, getBlockNumber, getChainId, getBalance, isHealthy };
