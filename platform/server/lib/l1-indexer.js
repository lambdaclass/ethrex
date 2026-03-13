/**
 * L1 Indexer — Watches MetadataURIUpdated events on OnChainProposer contracts.
 *
 * When a metadataURI is updated on-chain, this indexer:
 * 1. Fetches the metadata JSON from IPFS
 * 2. Updates the Platform DB cache (deployments table)
 *
 * This makes the Platform DB a cache of on-chain + IPFS data,
 * not the source of truth (Phase 2 architecture).
 */

const { updateDeployment, getActiveDeployments } = require("../db/deployments");

const CHAINS = [
  { name: "sepolia", chainId: 11155111, rpcUrl: process.env.SEPOLIA_RPC_URL },
  { name: "holesky", chainId: 17000, rpcUrl: process.env.HOLESKY_RPC_URL },
  // mainnet added later
];

const IPFS_GATEWAY =
  process.env.IPFS_GATEWAY || "https://gateway.pinata.cloud/ipfs";

/**
 * Compute the MetadataURIUpdated(string) event topic via keccak256.
 * Returns null if ethers is not installed.
 */
let _cachedEventTopic = null;
function computeEventTopic() {
  if (_cachedEventTopic) return _cachedEventTopic;
  try {
    const { keccak256, toUtf8Bytes } = require("ethers");
    _cachedEventTopic = keccak256(toUtf8Bytes("MetadataURIUpdated(string)"));
    return _cachedEventTopic;
  } catch {
    return null;
  }
}

/**
 * Convert ipfs:// URI to HTTP URL
 */
function ipfsToHttp(uri) {
  if (uri.startsWith("ipfs://")) {
    return `${IPFS_GATEWAY}/${uri.replace("ipfs://", "")}`;
  }
  return uri;
}

/**
 * Fetch metadata from IPFS and cache in DB.
 * Finds matching deployment by proposer_address + l1_chain_id directly from DB.
 */
async function fetchAndCacheMetadata(proposerAddr, uri, l1ChainId) {
  try {
    const httpUrl = ipfsToHttp(uri);
    const res = await fetch(httpUrl, { signal: AbortSignal.timeout(30000) });
    if (!res.ok) {
      console.error(`[indexer] Failed to fetch metadata: ${res.status}`);
      return;
    }
    const metadata = await res.json();

    // Find deployment by proposer_address and l1_chain_id
    const deployments = getActiveDeployments({ limit: 1000 });
    const match = deployments.find(
      (d) =>
        d.proposer_address?.toLowerCase() === proposerAddr.toLowerCase() &&
        d.l1_chain_id === l1ChainId
    );

    if (!match) {
      console.log(
        `[indexer] No matching deployment for proposer ${proposerAddr} on chain ${l1ChainId}`
      );
      return;
    }

    // Update DB cache
    const updates = {};
    if (metadata.description) updates.description = metadata.description;
    if (metadata.screenshots)
      updates.screenshots = JSON.stringify(metadata.screenshots);
    if (metadata.services?.explorer)
      updates.explorer_url = metadata.services.explorer;
    if (metadata.services?.bridgeUI)
      updates.dashboard_url = metadata.services.bridgeUI;
    if (metadata.socialLinks)
      updates.social_links = JSON.stringify(metadata.socialLinks);
    if (metadata.network?.networkMode)
      updates.network_mode = metadata.network.networkMode;

    if (Object.keys(updates).length > 0) {
      updateDeployment(match.id, updates);
      console.log(
        `[indexer] Updated deployment ${match.id} from IPFS metadata`
      );
    }
  } catch (err) {
    console.error(`[indexer] fetchAndCacheMetadata error:`, err.message);
  }
}

/**
 * Poll for MetadataURIUpdated events on known proposer contracts.
 * Uses eth_getLogs polling instead of WebSocket subscriptions for reliability.
 */
async function pollChain(chain, knownProposers, lastBlock) {
  if (!chain.rpcUrl) return lastBlock;

  try {
    // Get latest block
    const blockRes = await fetch(chain.rpcUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: "eth_blockNumber",
        params: [],
      }),
      signal: AbortSignal.timeout(5000),
    });
    const blockData = await blockRes.json();
    const latestBlock = parseInt(blockData.result, 16);

    if (latestBlock <= lastBlock) return lastBlock;

    const eventTopic = computeEventTopic();
    if (!eventTopic) {
      console.error(`[indexer] Cannot compute event topic — ethers not available`);
      return latestBlock;
    }

    // Query logs for each known proposer
    for (const proposerAddr of knownProposers) {
      try {
        const logsRes = await fetch(chain.rpcUrl, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            jsonrpc: "2.0",
            id: 1,
            method: "eth_getLogs",
            params: [
              {
                fromBlock: `0x${(lastBlock + 1).toString(16)}`,
                toBlock: `0x${latestBlock.toString(16)}`,
                address: proposerAddr,
                topics: [eventTopic],
              },
            ],
          }),
          signal: AbortSignal.timeout(10000),
        });
        const logsData = await logsRes.json();

        if (logsData.result && logsData.result.length > 0) {
          for (const log of logsData.result) {
            // Decode the MetadataURIUpdated event data (ABI-encoded string)
            const data = log.data;
            try {
              const { AbiCoder } = require("ethers");
              const coder = new AbiCoder();
              const [newURI] = coder.decode(["string"], data);
              if (!newURI || typeof newURI !== "string") {
                console.error(`[indexer] Invalid decoded URI from ${proposerAddr}`);
                continue;
              }
              console.log(
                `[indexer][${chain.name}] MetadataURIUpdated: ${proposerAddr} → ${newURI}`
              );
              await fetchAndCacheMetadata(
                proposerAddr,
                newURI,
                chain.chainId
              );
            } catch (decodeErr) {
              console.error(
                `[indexer] Failed to decode event data:`,
                decodeErr.message
              );
            }
          }
        }
      } catch (err) {
        console.error(
          `[indexer] Error polling ${proposerAddr} on ${chain.name}:`,
          err.message
        );
      }
    }

    return latestBlock;
  } catch (err) {
    console.error(`[indexer] pollChain error on ${chain.name}:`, err.message);
    return lastBlock;
  }
}

/**
 * Start the L1 indexer.
 * Polls each configured L1 chain every `intervalMs` for MetadataURIUpdated events.
 */
function startIndexer(intervalMs = 30000) {
  // Check if ethers is available
  try {
    require("ethers");
  } catch {
    console.log(
      "[indexer] ethers not installed — L1 indexer disabled. Install with: npm i ethers"
    );
    return null;
  }

  const activeChains = CHAINS.filter((c) => c.rpcUrl);
  if (activeChains.length === 0) {
    console.log(
      "[indexer] No L1 RPC URLs configured — indexer disabled. Set SEPOLIA_RPC_URL or HOLESKY_RPC_URL."
    );
    return null;
  }

  console.log(
    `[indexer] Starting L1 indexer for chains: ${activeChains.map((c) => c.name).join(", ")}`
  );

  // Track last polled block per chain — start from "latest" (will be fetched on first tick)
  const lastBlocks = {};
  for (const chain of activeChains) {
    lastBlocks[chain.name] = -1; // -1 means "fetch current block on first poll, skip historical"
  }

  // Guard against overlapping polls
  let polling = false;

  const interval = setInterval(async () => {
    if (polling) return;
    polling = true;

    try {
      const deployments = getActiveDeployments({ limit: 1000 });

      for (const chain of activeChains) {
        // On first poll, initialize lastBlock to current head (skip genesis scan)
        if (lastBlocks[chain.name] === -1) {
          try {
            const res = await fetch(chain.rpcUrl, {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({
                jsonrpc: "2.0", id: 1,
                method: "eth_blockNumber", params: [],
              }),
              signal: AbortSignal.timeout(5000),
            });
            const data = await res.json();
            lastBlocks[chain.name] = parseInt(data.result, 16);
            console.log(`[indexer] ${chain.name}: starting from block ${lastBlocks[chain.name]}`);
            continue; // skip this tick, start polling from next interval
          } catch (err) {
            console.error(`[indexer] Failed to get initial block for ${chain.name}:`, err.message);
            continue;
          }
        }

        const proposers = deployments
          .filter(
            (d) => d.proposer_address && d.l1_chain_id === chain.chainId
          )
          .map((d) => d.proposer_address);

        if (proposers.length === 0) continue;

        const uniqueProposers = [...new Set(proposers)];
        lastBlocks[chain.name] = await pollChain(
          chain,
          uniqueProposers,
          lastBlocks[chain.name]
        );
      }
    } finally {
      polling = false;
    }
  }, intervalMs);

  return interval;
}

module.exports = { startIndexer, fetchAndCacheMetadata };
