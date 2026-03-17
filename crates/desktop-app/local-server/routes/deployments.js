const express = require("express");
const router = express.Router();
const { v4: uuidv4 } = require("uuid");
const { ethers } = require("ethers");

const {
  provision,
  provisionLocalPrebuilt,
  provisionTestnet,
  provisionRemote,
  provisionRemoteTestnet,
  provisionThanos,
  provisionThanosTestnet,
  stopDeployment,
  startDeployment,
  destroyDeployment,
  enablePublicAccess,
  disablePublicAccess,
  getEmitter,
  isProvisionActive,
  cancelProvision,
  getActiveProvisions,
} = require("../lib/deployment-engine");
const { getDeployEvents, getNextAvailableL2ChainId, getNextAvailableL1ChainId } = require("../db/deployments");
const docker = require("../lib/docker-local");
const remote = require("../lib/docker-remote");
const { getDeploymentDir } = require("../lib/compose-generator");
const rpc = require("../lib/rpc-client");
const keychain = require("../lib/keychain");
const { getExternalL1Config, getPublicAccessConfig, getToolsPorts } = require("../lib/tools-config");
const db = require("../db/db");
const path = require("path");
const fs = require("fs");

// ==========================================
// CRUD (local — no auth required)
// ==========================================

// GET /api/deployments/ai-deploy/presets — cloud provider options (must be before /:id)
const { generateAIDeployPrompt, CLOUD_PRESETS } = require("../lib/ai-prompt-generator");
router.get("/ai-deploy/presets", (_req, res) => {
  res.json(CLOUD_PRESETS);
});

// AI status cache (reported by Messenger via POST, read by Manager UI via GET)
let messengerAIStatus = { configured: false, mode: null, provider: null, model: null };

// GET /api/deployments/ai-deploy/check-ai — get cached Messenger AI status
router.get("/ai-deploy/check-ai", (_req, res) => {
  if (!messengerAIStatus.configured) {
    messengerAIStatus.guide = "메신저 설정에서 AI를 먼저 설정해주세요";
  }
  res.json(messengerAIStatus);
});

// POST /api/deployments/ai-deploy/report-ai — Messenger reports its AI status
router.post("/ai-deploy/report-ai", (req, res) => {
  const { configured, mode, provider, model } = req.body;
  messengerAIStatus = {
    configured: !!configured,
    mode: mode || null,
    provider: provider || null,
    model: model || null,
  };
  res.json({ ok: true });
});

// In-memory pending prompt store (picked up by Messenger)
let pendingAIPrompt = null;

// GET /api/deployments/ai-deploy/pending — Messenger polls for pending prompt
router.get("/ai-deploy/pending", (_req, res) => {
  if (pendingAIPrompt) {
    const prompt = pendingAIPrompt;
    pendingAIPrompt = null; // auto-clear on read
    res.json(prompt);
  } else {
    res.json(null);
  }
});

// POST /api/deployments/ai-deploy/vultr-api-key — store Vultr API key in process.env
router.post("/ai-deploy/vultr-api-key", (req, res) => {
  const { apiKey } = req.body;
  if (!apiKey || typeof apiKey !== "string") {
    return res.status(400).json({ error: "apiKey required" });
  }
  process.env.VULTR_API_KEY = apiKey.trim();
  res.json({ ok: true });
});

router.get("/ai-deploy/check-cli", async (req, res) => {
  const { cloud } = req.query;
  if (!cloud || !["gcp", "aws", "vultr"].includes(cloud)) {
    return res.status(400).json({ error: "cloud must be gcp, aws, or vultr" });
  }
  const { execSync, execFileSync } = require("child_process");
  const result = { cloud, cli: { installed: false, name: "" }, auth: { authenticated: false, account: "" } };

  // Add known SDK paths to PATH for detection
  const extraPaths = ["/usr/local/share/google-cloud-sdk/bin", "/opt/homebrew/share/google-cloud-sdk/bin"];
  const envPATH = `${extraPaths.join(":")}:${process.env.PATH}`;
  const execOpts = { timeout: 5000, env: { ...process.env, PATH: envPATH } };

  if (cloud === "vultr") {
    result.cli.name = "vultr";
    try {
      const ver = execSync("vultr version 2>/dev/null || vultr-cli version 2>/dev/null", execOpts).toString().trim();
      result.cli.installed = true;
      result.cli.version = ver.replace(/.*v?(\d+\.\d+\.\d+).*/, "$1") || ver;
    } catch {
      return res.json(result);
    }
    try {
      const acct = execSync("vultr account 2>/dev/null || vultr-cli account 2>/dev/null", execOpts).toString().trim();
      if (acct && !acct.includes("error") && !acct.includes("401")) {
        result.auth.authenticated = true;
        const emailMatch = acct.match(/EMAIL\s+(.+)/i);
        result.auth.account = emailMatch ? emailMatch[1].trim() : "Authenticated";
      }
    } catch {}
    return res.json(result);
  }

  try {
    if (cloud === "gcp") {
      result.cli.name = "gcloud";
      try {
        const ver = execSync("gcloud version --format=json 2>/dev/null", execOpts).toString();
        const parsed = JSON.parse(ver);
        result.cli.installed = true;
        result.cli.version = parsed["Google Cloud SDK"] || "unknown";
      } catch {
        return res.json(result);
      }
      try {
        const acct = execSync("gcloud config get-value account 2>/dev/null", execOpts).toString().trim();
        if (acct && acct !== "(unset)") {
          result.auth.authenticated = true;
          result.auth.account = acct;
        }
        // Check active project
        const proj = execSync("gcloud config get-value project 2>/dev/null", execOpts).toString().trim();
        if (proj && proj !== "(unset)") {
          result.auth.project = proj;
        }
      } catch {}
    } else if (cloud === "aws") {
      result.cli.name = "aws";
      try {
        const ver = execSync("aws --version 2>&1", { timeout: 5000 }).toString().trim();
        result.cli.installed = true;
        result.cli.version = ver.split(" ")[0]?.replace("aws-cli/", "") || "unknown";
      } catch {
        return res.json(result);
      }
      try {
        const identity = execSync("aws sts get-caller-identity --output json 2>/dev/null", { timeout: 5000 }).toString();
        const parsed = JSON.parse(identity);
        result.auth.authenticated = true;
        result.auth.account = parsed.Arn || parsed.Account || "";
      } catch {}
      // List SSH key pairs
      try {
        const awsRegion = (req.query.region || "ap-northeast-2").replace(/[^a-z0-9-]/g, "");
        const kpJson = execFileSync("aws", ["ec2", "describe-key-pairs", "--query", "KeyPairs[*].[KeyName,KeyPairId]", "--output", "json", "--region", awsRegion], { timeout: 5000, stdio: "pipe" }).toString();
        result.keyPairs = JSON.parse(kpJson).map(([name, id]) => ({ name, id }));
      } catch {
        result.keyPairs = [];
      }
    }
  } catch (e) {
    // Unexpected error — return what we have
  }

  res.json(result);
});

// Input sanitizer for shell-safe values
const SAFE_NAME_RE = /^[a-zA-Z0-9_-]+$/;  // No dots or slashes — prevents path traversal
const SAFE_REGION_RE = /^[a-z]{2}-[a-z]+-\d+$/;

// POST /api/deployments/ai-deploy/monitor — check EC2 + container status via AWS CLI + SSH
router.post("/ai-deploy/monitor", async (req, res) => {
  let { vmName, region, keyPairName, deploymentId } = req.body;
  const { execFileSync } = require("child_process");

  // Load config from DB once (reused for parameter extraction and instance ID save)
  let deployConfig = null;
  if (deploymentId) {
    try {
      const dep = db.prepare("SELECT config FROM deployments WHERE id = ?").get(deploymentId);
      if (dep?.config) deployConfig = JSON.parse(dep.config);
    } catch {}
  }
  if (deployConfig && !vmName) {
    vmName = deployConfig.vmName || vmName;
    region = deployConfig.region || region;
    keyPairName = deployConfig.keyPairName || keyPairName;
  }

  if (!vmName) return res.status(400).json({ error: "vmName required" });
  // Validate inputs to prevent injection
  if (!SAFE_NAME_RE.test(vmName)) return res.status(400).json({ error: "Invalid vmName" });
  const awsRegion = SAFE_REGION_RE.test(region) ? region : "ap-northeast-2";
  if (keyPairName && !SAFE_NAME_RE.test(keyPairName)) return res.status(400).json({ error: "Invalid keyPairName" });

  const result = { vmName, ec2: null, containers: null, services: {} };
  const jmesQuery = "Reservations[].Instances[] | sort_by(@, &LaunchTime) | [-1].{State:State.Name,IP:PublicIpAddress,Id:InstanceId,Type:InstanceType,LaunchTime:LaunchTime,Name:Tags[?Key=='Name'].Value|[0]}";

  try {
    // 1. Get EC2 instance info — search exact name first, then wildcard
    let ec2Json = execFileSync("aws", [
      "ec2", "describe-instances",
      "--filters", `Name=tag:Name,Values=${vmName}`, "Name=instance-state-name,Values=pending,running,stopping,stopped",
      "--query", jmesQuery, "--output", "json", "--region", awsRegion,
    ], { timeout: 10000, stdio: "pipe" }).toString().trim();
    let parsed = JSON.parse(ec2Json);
    // Fallback: search all tokamak-* instances
    if (!parsed) {
      ec2Json = execFileSync("aws", [
        "ec2", "describe-instances",
        "--filters", "Name=tag:Name,Values=tokamak-*", "Name=instance-state-name,Values=pending,running,stopping,stopped",
        "--query", jmesQuery, "--output", "json", "--region", awsRegion,
      ], { timeout: 10000, stdio: "pipe" }).toString().trim();
      parsed = JSON.parse(ec2Json);
    }
    result.ec2 = parsed || { State: "not_found" };
    if (result.ec2.Name) result.vmName = result.ec2.Name;
    // Save instance ID and IP to DB for reliable future lookups (reuse deployConfig)
    if (deploymentId && result.ec2.Id && deployConfig) {
      if (deployConfig.ec2InstanceId !== result.ec2.Id || deployConfig.ec2IP !== result.ec2.IP) {
        deployConfig.ec2InstanceId = result.ec2.Id;
        deployConfig.ec2IP = result.ec2.IP;
        try { db.prepare("UPDATE deployments SET config = ? WHERE id = ?").run(JSON.stringify(deployConfig), deploymentId); } catch {}
      }
    }
  } catch (e) {
    result.ec2 = { State: "not_found", error: (e.message || "").slice(0, 200) };
    return res.json(result);
  }

  if (result.ec2.State !== "running" || !result.ec2.IP) {
    return res.json(result);
  }

  const ip = result.ec2.IP;
  // Validate IP is a public address (prevent SSRF to internal services)
  const ipParts = ip.split(".").map(Number);
  const isPrivate = (ipParts[0] === 10) ||
    (ipParts[0] === 172 && ipParts[1] >= 16 && ipParts[1] <= 31) ||
    (ipParts[0] === 192 && ipParts[1] === 168) ||
    (ipParts[0] === 127);
  if (isPrivate) {
    result.services = {};
    return res.json(result);
  }
  const os = require("os");
  const path = require("path");
  const keyPath = keyPairName ? path.join(os.homedir(), ".ssh", `${keyPairName}.pem`) : "";

  // 2. Check containers via SSH (execFileSync — no shell)
  if (keyPath) {
    try {
      const containers = execFileSync("ssh", [
        "-o", "StrictHostKeyChecking=no", "-o", "ConnectTimeout=5",
        "-i", keyPath, `ubuntu@${ip}`,
        "docker ps --format '{{.Names}}|{{.Status}}|{{.Ports}}' 2>/dev/null",
      ], { timeout: 15000, stdio: "pipe" }).toString().trim();
      result.containers = containers.split("\n").filter(Boolean).map(line => {
        const [name, status, ports] = line.split("|");
        return { name, status, ports };
      });
    } catch {
      result.containers = null;
    }
  }

  // 2.5. Extract contract addresses from deployer env file via SSH
  if (keyPath) {
    try {
      const envContent = execFileSync("ssh", [
        "-o", "StrictHostKeyChecking=no", "-o", "ConnectTimeout=5",
        "-i", keyPath, `ubuntu@${ip}`,
        "docker cp $(docker ps -aq -f name=deployer | head -1):/env/.env /dev/stdout 2>/dev/null || cat /opt/tokamak/*/deployed.env 2>/dev/null || echo ''",
      ], { timeout: 15000, stdio: "pipe" }).toString().trim();
      if (envContent) {
        const contracts = {};
        for (const line of envContent.split("\n")) {
          const [key, val] = line.split("=");
          if (key && val) contracts[key.trim()] = val.trim();
        }
        result.contracts = {
          bridge: contracts.ETHREX_WATCHER_BRIDGE_ADDRESS || null,
          proposer: contracts.ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS || null,
          timelock: contracts.ETHREX_TIMELOCK_ADDRESS || null,
          sp1Verifier: contracts.ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS || null,
          guestProgramRegistry: contracts.ETHREX_DEPLOYER_GUEST_PROGRAM_REGISTRY_ADDRESS || null,
        };
      }
    } catch {
      // Contract extraction is best-effort
    }
  }

  // 3. Check HTTP endpoints (parallel)
  const endpoints = [
    { name: "L2 RPC", port: 1729, type: "rpc" },
    { name: "L1 RPC", port: 8545, type: "rpc" },
    { name: "L2 Explorer", port: 8082, type: "http" },
    { name: "Dashboard", port: 3000, type: "http" },
  ];
  const checks = endpoints.map(async (ep) => {
    try {
      const signal = AbortSignal.timeout(3000);
      if (ep.type === "rpc") {
        const r = await fetch(`http://${ip}:${ep.port}`, {
          method: "POST", headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ jsonrpc: "2.0", method: "eth_blockNumber", params: [], id: 1 }),
          signal,
        });
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        const data = await r.json();
        if (!data.result) throw new Error("Missing result in RPC response");
        result.services[ep.name] = { ok: true, block: parseInt(data.result, 16) };
      } else {
        const r = await fetch(`http://${ip}:${ep.port}/`, { signal });
        result.services[ep.name] = { ok: r.ok, status: r.status };
      }
    } catch {
      result.services[ep.name] = { ok: false };
    }
  });
  await Promise.allSettled(checks);

  res.json(result);
});

// POST /api/deployments/ai-deploy/create-key-pair — create AWS SSH key pair
router.post("/ai-deploy/create-key-pair", (req, res) => {
  const { keyName, region } = req.body;
  if (!keyName || !SAFE_NAME_RE.test(keyName)) return res.status(400).json({ error: "Invalid keyName (alphanumeric, dash, underscore only)" });
  const awsRegion = SAFE_REGION_RE.test(region) ? region : "ap-northeast-2";
  const { execFileSync } = require("child_process");
  try {
    const result = execFileSync("aws", [
      "ec2", "create-key-pair", "--key-name", keyName,
      "--query", "KeyMaterial", "--output", "text", "--region", awsRegion,
    ], { timeout: 10000, stdio: "pipe" }).toString();
    // Save the private key to ~/.ssh/
    const fs = require("fs");
    const path = require("path");
    const sshDir = path.join(require("os").homedir(), ".ssh");
    if (!fs.existsSync(sshDir)) fs.mkdirSync(sshDir, { mode: 0o700 });
    const pemPath = path.join(sshDir, `${keyName}.pem`);
    fs.writeFileSync(pemPath, result, { mode: 0o400 });
    res.json({ ok: true, keyName, pemPath });
  } catch (e) {
    const msg = (e.message || "") + (e.stderr ? e.stderr.toString() : "");
    if (msg.includes("InvalidKeyPair.Duplicate")) {
      res.status(409).json({ error: `Key pair "${keyName}" already exists` });
    } else {
      res.status(500).json({ error: msg.slice(0, 300) });
    }
  }
});

// GET /api/deployments/next-chain-id — get unique L1 and L2 chain IDs
router.get("/next-chain-id", (req, res) => {
  try {
    res.json({ chainId: getNextAvailableL2ChainId(), l1ChainId: getNextAvailableL1ChainId() });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments — create a new deployment
router.post("/", (req, res) => {
  try {
    const { programSlug, name, chainId, config, stackType } = req.body;
    if (!name) {
      return res.status(400).json({ error: "name is required" });
    }

    const id = uuidv4();
    const now = Date.now();

    db.prepare(`
      INSERT INTO deployments (id, program_slug, stack_type, name, chain_id, config, created_at)
      VALUES (?, ?, ?, ?, ?, ?, ?)
    `).run(id, programSlug || "evm-l2", stackType || "ethrex", name.trim(), chainId || null, config ? JSON.stringify(config) : null, now);

    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(id);
    res.status(201).json({ deployment });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments — list all local deployments
router.get("/", (req, res) => {
  try {
    const deployments = db.prepare("SELECT * FROM deployments ORDER BY created_at DESC").all();
    res.json({ deployments });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments/docker/status — check if Docker daemon is available
router.get("/docker/status", (req, res) => {
  try {
    const available = docker.isDockerAvailable();
    res.json({ available });
  } catch (e) {
    res.json({ available: false });
  }
});

// Estimated gas per role:
// Deployer: ~5 contracts × 2 txs (impl + proxy) + timelock init = ~25M gas (one-time)
// Committer: batch commit ~200K gas per tx (recurring, estimate first month ~100 txs)
// Proof Coordinator: proof submission ~300K gas per tx (recurring, estimate first month ~100 txs)
// Bridge Owner: VK registration + accept ownership ~500K gas (one-time setup)
const ROLE_GAS_ESTIMATES = {
  deployer: {
    gas: 25_000_000,
    label: "Contract deployment (one-time)",
    detail: "5 contracts (Bridge, Proposer, Timelock, SP1Verifier, SequencerRegistry) × 2 txs (impl + proxy) ≈ 2.5M gas/pair × 5 = ~25M gas",
  },
  committer: {
    gas: 8_640_000_000,
    label: "Batch commits (1 month, 60s interval)",
    detail: "commit_batch() ≈ 200K gas/tx. Commits every 60s including empty blocks. 1,440/day × 30 days = 43,200 tx/month. 200K × 43,200 = ~8.64B gas/month.",
    interval: "60s",
  },
  "proof-coordinator": {
    gas: 12_960_000_000,
    label: "Proof submissions (1 month, 1:1 with commits)",
    detail: "verify() ≈ 300K gas/tx (ZK proof on-chain). 1 proof per committed batch. 43,200 tx/month. 300K × 43,200 = ~12.96B gas/month.",
    interval: "60s (1:1 with commits)",
  },
  "bridge-owner": {
    gas: 500_000,
    label: "VK registration + ownership (one-time)",
    detail: "setVerificationKey() ≈ 200K gas + acceptOwnership() × 2 contracts ≈ 100K each + register() ≈ 100K = ~500K gas total",
  },
};

/** Validate RPC URL: must be http(s), no private IPs or metadata endpoints.
 *  allowLocal=true permits localhost/127.0.0.1 (for local L1 dev mode). */
function validateRpcUrl(rpcUrl, { allowLocal = false } = {}) {
  let parsed;
  try { parsed = new URL(rpcUrl); } catch { throw new Error("Invalid URL format"); }
  if (!["http:", "https:"].includes(parsed.protocol)) throw new Error("URL must use http or https");
  const host = parsed.hostname;
  // Block cloud metadata endpoints
  if (host === "169.254.169.254" || host === "metadata.google.internal") throw new Error("Blocked: cloud metadata endpoint");
  // Block private IPs (RFC1918)
  if (/^(10\.|172\.(1[6-9]|2\d|3[01])\.|192\.168\.)/.test(host)) throw new Error("Blocked: private IP range");
  // Block loopback unless explicitly allowed (local L1 dev mode)
  if (!allowLocal && (host === "127.0.0.1" || host === "::1" || host === "0.0.0.0" || host === "localhost")) {
    throw new Error("Blocked: localhost/loopback address. Use an external RPC URL for testnet.");
  }
  return parsed;
}

// Shared RPC call helper with 10s timeout
function makeRpcCaller(rpcUrl) {
  validateRpcUrl(rpcUrl);
  return async (method, params = []) => {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 10000);
    try {
      const r = await globalThis.fetch(rpcUrl, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
        signal: controller.signal,
      });
      const data = await r.json();
      if (data.error) throw new Error(data.error.message || "RPC error");
      return data.result;
    } finally {
      clearTimeout(timeout);
    }
  };
}

const CHAIN_NAMES = { 1: "Ethereum Mainnet", 11155111: "Sepolia", 17000: "Holesky" };

function formatGwei(gasPriceGwei) {
  return gasPriceGwei < 0.0001 ? gasPriceGwei.toPrecision(4) : gasPriceGwei.toFixed(4);
}

/** Convert wei BigInt to ETH string with 6 decimal precision (safe for large values) */
function weiToEth(wei) {
  const ETH = 1000000000000000000n; // 1e18
  const whole = wei / ETH;
  const remainder = wei % ETH;
  const decimal = remainder * 1000000n / ETH; // 6 decimal places
  return `${whole}.${decimal.toString().padStart(6, "0")}`;
}

// POST /api/deployments/testnet/check-balance — check account balance on testnet
router.post("/testnet/check-balance", async (req, res) => {
  try {
    const { rpcUrl, address, role } = req.body;
    if (!rpcUrl || !address) {
      return res.status(400).json({ error: "rpcUrl and address are required" });
    }
    if (!/^0x[0-9a-fA-F]{40}$/.test(address)) {
      return res.status(400).json({ error: "Invalid Ethereum address format" });
    }

    const rpcCall = makeRpcCaller(rpcUrl);
    const [balanceHex, chainIdHex, gasPriceHex] = await Promise.all([
      rpcCall("eth_getBalance", [address, "latest"]),
      rpcCall("eth_chainId"),
      rpcCall("eth_gasPrice"),
    ]);

    const balanceWei = BigInt(balanceHex || "0x0");
    const chainId = parseInt(chainIdHex || "0x0", 16);
    const gasPriceWei = BigInt(gasPriceHex || "0x0");
    const gasPriceGwei = Number(gasPriceWei / 1000n) / 1e6;

    const roleInfo = ROLE_GAS_ESTIMATES[role] || ROLE_GAS_ESTIMATES.deployer;
    const estimatedGas = roleInfo.gas;
    const estimatedCostWei = gasPriceWei * BigInt(estimatedGas);
    const sufficient = balanceWei >= estimatedCostWei;

    res.json({
      address,
      role: role || "deployer",
      balanceEth: weiToEth(balanceWei),
      chainId,
      gasPriceGwei: formatGwei(gasPriceGwei),
      estimatedGas,
      gasLabel: roleInfo.label,
      gasDetail: roleInfo.detail,
      interval: roleInfo.interval || null,
      estimatedCostEth: weiToEth(estimatedCostWei),
      sufficient,
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/testnet/check-rpc — test L1 RPC connectivity
router.post("/testnet/check-rpc", async (req, res) => {
  try {
    const { rpcUrl } = req.body;
    if (!rpcUrl) {
      return res.status(400).json({ error: "rpcUrl is required" });
    }

    const rpcCall = makeRpcCaller(rpcUrl);
    const [chainIdHex, blockHex] = await Promise.all([
      rpcCall("eth_chainId"),
      rpcCall("eth_blockNumber"),
    ]);

    const chainId = parseInt(chainIdHex || "0x0", 16);
    const blockNumber = parseInt(blockHex || "0x0", 16);
    const chainName = CHAIN_NAMES[chainId] || `Chain ${chainId}`;

    res.json({ ok: true, chainId, chainName, blockNumber });
  } catch (e) {
    res.status(500).json({ ok: false, error: e.message });
  }
});

// GET /api/deployments/keychain/accounts — list keychain accounts
router.get("/keychain/accounts", (req, res) => {
  try {
    const accounts = keychain.listAccounts();
    res.json({ accounts });
  } catch (e) {
    res.json({ accounts: [] });
  }
});

// POST /api/deployments/testnet/resolve-keys — resolve keychain keys to addresses + balances
router.post("/testnet/resolve-keys", async (req, res) => {
  try {
    const { rpcUrl, deployerKey, committerKey, proofCoordinatorKey, bridgeOwnerKey } = req.body;
    if (!rpcUrl) {
      return res.status(400).json({ error: "rpcUrl is required" });
    }

    const resolveRole = (keychainName, label) => {
      if (!keychainName) return null;
      let pk = keychain.getSecret(keychainName);
      if (!pk) return { error: `Key "${keychainName}" not found in Keychain`, label };
      try {
        const wallet = new ethers.Wallet(pk);
        const address = wallet.address;
        return { address, label, keychainName };
      } catch {
        return { error: `Invalid key format for "${keychainName}"`, label };
      } finally {
        pk = null; // Best-effort: dereference private key (actual GC timing is V8-controlled)
      }
    };

    const roles = {
      deployer: resolveRole(deployerKey, "Deployer"),
      committer: resolveRole(committerKey, "Committer") || resolveRole(deployerKey, "Committer"),
      proofCoordinator: resolveRole(proofCoordinatorKey, "Proof Coordinator") || resolveRole(deployerKey, "Proof Coordinator"),
      bridgeOwner: resolveRole(bridgeOwnerKey, "Bridge Owner") || resolveRole(deployerKey, "Bridge Owner"),
    };

    const errors = Object.values(roles).filter(r => r?.error);
    if (errors.length > 0) {
      return res.status(400).json({ error: errors[0].error });
    }

    // Fetch balances in parallel using BigInt for precision
    const rpcCall = makeRpcCaller(rpcUrl);
    const uniqueAddresses = [...new Set(Object.values(roles).map(r => r?.address).filter(Boolean))];
    const balanceWeis = {};
    const [gasPriceHex] = await Promise.all([
      rpcCall("eth_gasPrice"),
      ...uniqueAddresses.map(async (addr) => {
        const hex = await rpcCall("eth_getBalance", [addr, "latest"]);
        balanceWeis[addr] = BigInt(hex || "0x0");
      }),
    ]);

    const gasPriceWei = BigInt(gasPriceHex || "0x0");
    const gasPriceGwei = Number(gasPriceWei / 1000n) / 1e6;

    // Estimate total deployment cost using BigInt
    const deployerGas = ROLE_GAS_ESTIMATES.deployer.gas;
    const estimatedCostWei = gasPriceWei * BigInt(deployerGas);

    // Enrich roles with balances
    for (const role of Object.values(roles)) {
      if (role?.address) {
        const wei = balanceWeis[role.address] || 0n;
        role.balance = weiToEth(wei);
      }
    }

    const deployerWei = roles.deployer?.address ? (balanceWeis[roles.deployer.address] || 0n) : 0n;

    res.json({
      roles,
      gasPriceGwei: formatGwei(gasPriceGwei),
      estimatedDeployCostEth: weiToEth(estimatedCostWei),
      deployerSufficient: deployerWei >= estimatedCostWei,
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/testnet/estimate-gas — estimate total deployment + operational costs
router.post("/testnet/estimate-gas", async (req, res) => {
  try {
    const { rpcUrl } = req.body;
    if (!rpcUrl) {
      return res.status(400).json({ error: "rpcUrl is required" });
    }

    const rpcCall = makeRpcCaller(rpcUrl);
    const [gasPriceHex, chainIdHex] = await Promise.all([
      rpcCall("eth_gasPrice"),
      rpcCall("eth_chainId"),
    ]);

    const gasPriceWei = BigInt(gasPriceHex || "0x0");
    const gasPriceGwei = Number(gasPriceWei / 1000n) / 1e6;
    const chainId = parseInt(chainIdHex || "0x0", 16);

    const breakdown = {};
    let totalGas = 0n;
    for (const [role, info] of Object.entries(ROLE_GAS_ESTIMATES)) {
      const gasBI = BigInt(info.gas);
      const costWei = gasPriceWei * gasBI;
      totalGas += gasBI;
      breakdown[role] = {
        gas: info.gas,
        label: info.label,
        detail: info.detail,
        interval: info.interval || null,
        costEth: weiToEth(costWei),
      };
    }

    const totalCostWei = gasPriceWei * totalGas;

    res.json({
      chainId,
      chainName: CHAIN_NAMES[chainId] || `Chain ${chainId}`,
      gasPriceGwei: formatGwei(gasPriceGwei),
      breakdown,
      totalGas: totalGas.toString(),
      totalCostEth: weiToEth(totalCostWei),
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments/active/provisions — list currently running provisions
// NOTE: Must be defined before /:id to avoid route shadowing
router.get("/active/provisions", (req, res) => {
  res.json({ provisions: getActiveProvisions() });
});

// GET /api/deployments/check-image/:slug — check if Docker image exists for a program
router.get("/check-image/:slug", (req, res) => {
  try {
    const image = docker.findImage(req.params.slug);
    res.json({ exists: !!image, image: image || null });
  } catch (e) {
    res.json({ exists: false, image: null });
  }
});

// GET /api/deployments/:id — get deployment detail
router.get("/:id", (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) {
      return res.status(404).json({ error: "Deployment not found" });
    }
    res.json({ deployment });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// PUT /api/deployments/:id — update deployment
function updateDeploymentHandler(req, res) {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    const allowedFields = ["name", "chain_id", "l1_chain_id", "rpc_url", "config", "is_public", "hashtags", "platform_deployment_id", "phase", "status", "bridge_address", "proposer_address", "timelock_address", "sp1_verifier_address", "guest_program_registry_address"];
    const updates = [];
    const values = [];

    for (const field of allowedFields) {
      if (req.body[field] !== undefined) {
        updates.push(`${field} = ?`);
        values.push(field === 'config' && typeof req.body[field] === 'object' ? JSON.stringify(req.body[field]) : req.body[field]);
      }
    }

    if (updates.length > 0) {
      values.push(req.params.id);
      db.prepare(`UPDATE deployments SET ${updates.join(", ")} WHERE id = ?`).run(...values);
    }

    const updated = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    res.json({ deployment: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
}
router.put("/:id", updateDeploymentHandler);
router.patch("/:id", updateDeploymentHandler);

// DELETE /api/deployments/:id — remove deployment
router.delete("/:id", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    // Cancel active provision if running
    cancelProvision(req.params.id);

    // Cleanup Docker resources if any
    if (deployment.docker_project && deployment.phase !== "configured") {
      try {
        await destroyDeployment(deployment);
      } catch {
        // Continue with DB deletion
      }
    }

    db.prepare("DELETE FROM deploy_events WHERE deployment_id = ?").run(req.params.id);
    db.prepare("DELETE FROM deployments WHERE id = ?").run(req.params.id);
    res.json({ ok: true });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// ==========================================
// Docker Deployment Lifecycle
// ==========================================

// Helper: update deployment in DB
function updateDeployment(id, fields) {
  const updates = [];
  const values = [];
  for (const [key, val] of Object.entries(fields)) {
    updates.push(`${key} = ?`);
    values.push(val);
  }
  if (updates.length > 0) {
    values.push(id);
    db.prepare(`UPDATE deployments SET ${updates.join(", ")} WHERE id = ?`).run(...values);
  }
  return db.prepare("SELECT * FROM deployments WHERE id = ?").get(id);
}

// POST /api/deployments/:id/provision — start full deployment
router.post("/:id/provision", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) {
      return res.status(404).json({ error: "Deployment not found" });
    }

    if (deployment.phase === "running") {
      return res.status(400).json({ error: "Deployment is already running" });
    }

    const inProgressPhases = ["checking_docker", "building", "pulling", "l1_starting", "deploying_contracts", "l2_starting", "starting_prover", "starting_tools", "starting_op_node", "starting_batcher", "starting_proposer"];
    if (inProgressPhases.includes(deployment.phase)) {
      return res.status(400).json({ error: "Deployment is already in progress" });
    }

    const { hostId } = req.body;
    // Determine mode from deployment config
    let deployMode = 'local';
    try {
      const config = deployment.config ? JSON.parse(deployment.config) : {};
      deployMode = config.mode || 'local';
    } catch {}

    // Determine stack type from deployment DB row
    const stackType = deployment.stack_type || 'ethrex';

    // Validate unsupported combinations before responding
    if (stackType === 'thanos' && hostId) {
      return res.status(400).json({ error: "Remote deployment is not yet supported for Thanos stack" });
    }

    res.json({ ok: true, message: "Provisioning started", remote: !!hostId, mode: deployMode });

    let provisionFn;
    if (stackType === 'thanos') {
      if (deployMode === 'testnet') {
        provisionFn = () => provisionThanosTestnet(deployment);
      } else {
        provisionFn = () => provisionThanos(deployment);
      }
    } else if (hostId && deployMode === 'testnet') {
      provisionFn = () => provisionRemoteTestnet(deployment, hostId);
    } else if (hostId) {
      provisionFn = () => provisionRemote(deployment, hostId);
    } else if (deployMode === 'testnet') {
      provisionFn = () => provisionTestnet(deployment);
    } else if (deployMode === 'ai-deploy') {
      provisionFn = () => provisionLocalPrebuilt(deployment);
    } else {
      provisionFn = () => provision(deployment);
    }

    provisionFn().catch((err) => {
      console.error(`Provision failed for ${deployment.id}:`, err.message);
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/start
router.post("/:id/start", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });
    if (!deployment.docker_project) return res.status(400).json({ error: "Not provisioned yet" });

    const updated = await startDeployment(deployment);
    res.json({ deployment: updated });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/stop
router.post("/:id/stop", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });

    // Cancel active provision first (removes from registry)
    const wasProvisioning = cancelProvision(req.params.id);

    if (deployment.docker_project) {
      // Stop all containers including deployer
      try {
        const path = require("path");
        const { getDeploymentDir } = require("../lib/compose-generator");
        const composeFile = path.join(getDeploymentDir(deployment.id), "docker-compose.yaml");
        const docker = require("../lib/docker-local");
        // Stop tools first, then core services
        try { await docker.stopTools(`${deployment.docker_project}-tools`); } catch { /* tools may not be running */ }
        await docker.stop(deployment.docker_project, composeFile);
      } catch (e) {
        console.log(`[stop] docker stop failed: ${e.message}`);
      }
      const { updateDeployment: updateDep } = require("../db/deployments");
      const updated = updateDep(deployment.id, {
        phase: "stopped",
        error_message: wasProvisioning ? "Cancelled by user" : null,
      });
      res.json({ deployment: updated });
    } else {
      // No docker project yet — just mark as configured
      const { updateDeployment: updateDep } = require("../db/deployments");
      const updated = updateDep(deployment.id, {
        phase: "configured",
        error_message: wasProvisioning ? "Cancelled by user" : null,
      });
      res.json({ deployment: updated });
    }
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/destroy
router.post("/:id/destroy", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });

    // Cancel active provision if running
    cancelProvision(req.params.id);

    // Destroy Docker containers if provisioned
    if (deployment.docker_project) {
      await destroyDeployment(deployment);
    }

    // Remove from DB
    db.prepare("DELETE FROM deploy_events WHERE deployment_id = ?").run(req.params.id);
    db.prepare("DELETE FROM deployments WHERE id = ?").run(req.params.id);
    res.json({ ok: true, deleted: true });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// Tools services live in a separate compose file
const TOOLS_SERVICES = new Set(["frontend-l1", "backend-l1", "frontend-l2", "backend-l2", "db", "db-init", "redis-db", "proxy", "proxy-l2-only", "function-selectors", "function-selectors-l2", "bridge-ui"]);

// POST /api/deployments/:id/service/:service/stop — stop a single service
router.post("/:id/service/:service/stop", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });
    if (!deployment.docker_project) return res.status(400).json({ error: "Not provisioned yet" });
    if (TOOLS_SERVICES.has(req.params.service)) {
      // Tools use separate compose — stop via tools compose (per-deployment)
      await docker.stopTools(`${deployment.docker_project}-tools`);
      return res.json({ ok: true, message: `Tools stopped` });
    }
    const composeFile = path.join(getDeploymentDir(deployment.id, deployment.deploy_dir), "docker-compose.yaml");
    await docker.stopService(deployment.docker_project, composeFile, req.params.service);
    res.json({ ok: true, message: `Service ${req.params.service} stopped` });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/service/:service/start — start a single service
router.post("/:id/service/:service/start", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });
    if (!deployment.docker_project) return res.status(400).json({ error: "Not provisioned yet" });
    if (TOOLS_SERVICES.has(req.params.service)) {
      // Tools use separate compose — start all tools together (they depend on each other)
      const composeFile = path.join(getDeploymentDir(deployment.id, deployment.deploy_dir), "docker-compose.yaml");
      const envVars = await docker.extractEnv(deployment.docker_project, composeFile);
      await docker.startTools(`${deployment.docker_project}-tools`, envVars, getToolsPorts(deployment));
      return res.json({ ok: true, message: `Tools started` });
    }
    const composeFile = path.join(getDeploymentDir(deployment.id, deployment.deploy_dir), "docker-compose.yaml");
    await docker.startService(deployment.docker_project, composeFile, req.params.service, { DOCKER_ETHREX_WORKDIR: "/usr/local/bin" });
    res.json({ ok: true, message: `Service ${req.params.service} started` });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/build-tools
router.post("/:id/build-tools", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });
    if (!deployment.docker_project) return res.status(400).json({ error: "Not provisioned yet" });

    await docker.buildTools(`${deployment.docker_project}-tools`, getToolsPorts(deployment));
    res.json({ ok: true, message: "Tools images rebuilt" });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/restart-tools
router.post("/:id/restart-tools", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });
    if (!deployment.docker_project) return res.status(400).json({ error: "Not provisioned yet" });

    let envVars = {};
    try {
      const composeFile = path.join(getDeploymentDir(deployment.id, deployment.deploy_dir), "docker-compose.yaml");
      const volumeEnv = await docker.extractEnv(deployment.docker_project, composeFile);
      if (volumeEnv.ETHREX_WATCHER_BRIDGE_ADDRESS) envVars.ETHREX_WATCHER_BRIDGE_ADDRESS = volumeEnv.ETHREX_WATCHER_BRIDGE_ADDRESS;
      if (volumeEnv.ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS) envVars.ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS = volumeEnv.ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS;
    } catch {
      if (deployment.bridge_address) envVars.ETHREX_WATCHER_BRIDGE_ADDRESS = deployment.bridge_address;
      if (deployment.proposer_address) envVars.ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS = deployment.proposer_address;
    }

    // Respond immediately — docker compose up can take 30s+ and WebKit times out
    res.json({ ok: true, message: "Tools starting..." });
    docker.restartTools(`${deployment.docker_project}-tools`, envVars, getToolsPorts(deployment)).catch(e => {
      console.error("Tools restart failed:", e.message);
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// POST /api/deployments/:id/stop-tools
router.post("/:id/stop-tools", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });
    if (!deployment.docker_project) return res.status(400).json({ error: "Not provisioned yet" });
    res.json({ ok: true, message: "Tools stopping..." });
    docker.stopTools(`${deployment.docker_project}-tools`).catch(e => {
      console.error("Tools stop failed:", e.message);
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// ==========================================
// External Access (Public Domain/IP)
// ==========================================

// POST /api/deployments/:id/public-access
router.post("/:id/public-access", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });
    if (!deployment.docker_project) return res.status(400).json({ error: "Not provisioned yet" });

    const { publicDomain } = req.body;
    if (!publicDomain) return res.status(400).json({ error: "publicDomain is required" });
    // Validate domain/IP: only allow safe hostname characters
    if (!/^[a-zA-Z0-9._:-]+$/.test(publicDomain)) {
      return res.status(400).json({ error: "publicDomain contains invalid characters" });
    }

    // Save to DB
    updateDeployment(deployment.id, {
      is_public: 1,
      public_domain: publicDomain,
      public_l2_rpc_url: req.body.l2RpcUrl || null,
      public_l2_explorer_url: req.body.l2ExplorerUrl || null,
      public_l1_explorer_url: req.body.l1ExplorerUrl || null,
      public_dashboard_url: req.body.dashboardUrl || null,
    });

    const updated = db.prepare("SELECT * FROM deployments WHERE id = ?").get(deployment.id);
    const publicConfig = getPublicAccessConfig(updated);

    // Regenerate compose (0.0.0.0 binding) + restart L2 + tools (async — can take 60s+)
    res.json({ ok: true, message: "Enabling public access...", publicConfig });

    enablePublicAccess(deployment).catch(e => {
      console.error(`[public-access] Enable failed: ${e.message}`);
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// DELETE /api/deployments/:id/public-access
router.delete("/:id/public-access", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });
    if (!deployment.docker_project) return res.status(400).json({ error: "Not provisioned yet" });

    // Clear public access in DB
    updateDeployment(deployment.id, {
      is_public: 0,
      public_domain: null,
      public_l2_rpc_url: null,
      public_l2_explorer_url: null,
      public_l1_explorer_url: null,
      public_dashboard_url: null,
    });

    // Regenerate compose (127.0.0.1 binding) + restart L2 + tools (async)
    res.json({ ok: true, message: "Disabling public access..." });

    disablePublicAccess(deployment).catch(e => {
      console.error(`[public-access] Disable failed: ${e.message}`);
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// ==========================================
// Monitoring & Logs
// ==========================================

// GET /api/deployments/:id/status
router.get("/:id/status", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });

    // Skip SSH polling for stopped/configured deployments (no need to probe unreachable servers)
    if (deployment.phase === 'stopped' || deployment.phase === 'configured' || deployment.status === 'stopped') {
      return res.json({ phase: deployment.phase, containers: [], endpoints: {} });
    }

    // Determine if this is a remote deployment (SSH host or AI-deploy with EC2)
    const dConfig = deployment.config ? (typeof deployment.config === 'string' ? JSON.parse(deployment.config) : deployment.config) : {};
    const isHostRemote = !!deployment.host_id;
    const isAIDeployRemote = dConfig.mode === 'ai-deploy' && !!dConfig.ec2IP;

    if (!deployment.docker_project && !isAIDeployRemote) {
      return res.json({ phase: deployment.phase, containers: [], endpoints: {} });
    }

    let containers = [];
    let host = null;

    if (isHostRemote) {
      // Remote deployment via hosts table: query containers via SSH
      const hostRecord = db.prepare("SELECT * FROM hosts WHERE id = ?").get(deployment.host_id);
      if (hostRecord) {
        host = {
          hostname: hostRecord.hostname,
          username: hostRecord.username,
          port: hostRecord.port || 22,
          deployDir: deployment.deploy_dir || `/opt/tokamak/${deployment.id}`,
        };
        let conn;
        try {
          conn = await remote.connect(hostRecord);
          const remoteDir = host.deployDir;
          containers = await remote.getStatusRemote(conn, deployment.docker_project, remoteDir);
          try {
            const toolsContainers = await remote.getStatusRemote(conn, `${deployment.docker_project}-tools`, remoteDir);
            if (toolsContainers.length > 0) containers = containers.concat(toolsContainers);
          } catch (e) {
            console.warn(`[status] Failed to get tools containers for ${deployment.id}: ${e.message}`);
          }
        } catch (e) {
          console.error(`[status] Remote SSH failed for ${deployment.id}: ${e.message}`);
        } finally {
          if (conn) try { conn.end(); } catch {}
        }
      }
    } else if (isAIDeployRemote) {
      // AI Deploy with EC2 instance: query via SSH
      const ip = dConfig.ec2IP;
      const keyPairName = dConfig.keyPairName || null;
      const username = 'ubuntu';
      const shortId = deployment.id.slice(0, 8);
      const remoteDir = `/opt/tokamak/${shortId}`;
      host = {
        hostname: ip,
        username,
        port: 22,
        deployDir: remoteDir,
        keyPairName,
        vmName: dConfig.vmName || null,
        region: dConfig.region || null,
        cloud: dConfig.cloud || 'aws',
        vmType: dConfig.vmType || null,
        awsAccount: dConfig.awsAccount || null,
      };
      if (ip && keyPairName) {
        const os = require("os");
        const keyPath = path.join(os.homedir(), ".ssh", `${keyPairName}.pem`);
        if (fs.existsSync(keyPath)) {
          let conn;
          try {
            const hostRecord = { hostname: ip, port: 22, username, auth_method: 'key', private_key: fs.readFileSync(keyPath, 'utf-8') };
            conn = await remote.connect(hostRecord);
            const projectName = deployment.docker_project || dConfig.vmName || deployment.id;
            try { containers = await remote.getStatusRemote(conn, projectName, remoteDir); } catch {}
            try {
              const toolsContainers = await remote.getStatusRemote(conn, `${projectName}-tools`, remoteDir);
              if (toolsContainers.length > 0) containers = containers.concat(toolsContainers);
            } catch {}
          } catch (e) {
            console.error(`[status] AI-deploy SSH failed for ${deployment.id}: ${e.message}`);
          } finally {
            if (conn) try { conn.end(); } catch {}
          }
        }
      }
    } else {
      // Local deployment: query containers via local Docker
      const composeFile = path.join(getDeploymentDir(deployment.id, deployment.deploy_dir), "docker-compose.yaml");
      if (fs.existsSync(composeFile)) {
        containers = await docker.getStatus(deployment.docker_project, composeFile);
      }
      try {
        const toolsContainers = await docker.getToolsStatus(`${deployment.docker_project}-tools`);
        if (toolsContainers.length > 0) containers = containers.concat(toolsContainers);
      } catch {}
    }

    const isRemote = isHostRemote || isAIDeployRemote;
    const rpcHost = (isRemote && host?.hostname) ? host.hostname : "127.0.0.1";

    res.json({
      phase: deployment.phase,
      containers,
      endpoints: {
        l1Rpc: (rpcHost && deployment.l1_port) ? `http://${rpcHost}:${deployment.l1_port}` : null,
        l2Rpc: (rpcHost && deployment.l2_port) ? `http://${rpcHost}:${deployment.l2_port}` : null,
      },
      contracts: {
        bridge: deployment.bridge_address,
        proposer: deployment.proposer_address,
      },
      error: deployment.error_message,
      host,
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments/:id/events — SSE stream
router.get("/:id/events", (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });

    res.writeHead(200, {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache",
      Connection: "keep-alive",
    });

    res.write(`data: ${JSON.stringify({ event: "phase", phase: deployment.phase, timestamp: Date.now() })}\n\n`);

    const emitter = getEmitter(deployment.id);
    const handler = (data) => {
      res.write(`data: ${JSON.stringify(data)}\n\n`);
      if (data.phase === "running" || data.event === "error") {
        setTimeout(() => res.end(), 1000);
      }
    };

    emitter.on("event", handler);
    req.on("close", () => emitter.removeListener("event", handler));
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments/:id/events/history — get stored events from DB
router.get("/:id/events/history", (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });

    const since = req.query.since ? parseInt(req.query.since) : undefined;
    const limit = req.query.limit ? parseInt(req.query.limit) : 1000;
    const events = getDeployEvents(deployment.id, { since, limit });

    res.json({
      events,
      isActive: isProvisionActive(deployment.id),
      phase: deployment.phase,
      createdAt: deployment.created_at,
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// (moved above /:id to avoid route shadowing)

// GET /api/deployments/:id/logs
router.get("/:id/logs", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });

    const service = req.query.service || null;
    const follow = req.query.follow === "true";
    const tail = parseInt(req.query.tail) || 100;

    // AI Deploy: fetch logs via SSH
    const dConfig = deployment.config ? (typeof deployment.config === 'string' ? JSON.parse(deployment.config) : deployment.config) : {};
    if (dConfig.mode === 'ai-deploy' && dConfig.ec2IP && dConfig.keyPairName) {
      const ip = dConfig.ec2IP;
      const keyPairName = dConfig.keyPairName;
      const username = 'ubuntu';
      const shortId = deployment.id.slice(0, 8);
      const remoteDir = `/opt/tokamak/${shortId}`;
      const projectName = deployment.docker_project || dConfig.vmName || deployment.id;
      const os = require("os");
      const keyPath = path.join(os.homedir(), ".ssh", `${keyPairName}.pem`);
      if (!fs.existsSync(keyPath)) return res.status(400).json({ error: "SSH key not found" });

      const hostRecord = { hostname: ip, port: 22, username, auth_method: 'key', private_key: fs.readFileSync(keyPath, 'utf-8') };

      if (follow) {
        // Stream logs via SSH
        let conn;
        try {
          conn = await remote.connect(hostRecord);
          res.writeHead(200, { "Content-Type": "text/event-stream", "Cache-Control": "no-cache", Connection: "keep-alive" });
          const stream = await remote.streamLogsRemote(conn, projectName, remoteDir, service);
          stream.on("data", (chunk) => {
            for (const line of chunk.toString().split("\n").filter(Boolean)) {
              res.write(`data: ${JSON.stringify({ line })}\n\n`);
            }
          });
          stream.stderr.on("data", (chunk) => {
            for (const line of chunk.toString().split("\n").filter(Boolean)) {
              res.write(`data: ${JSON.stringify({ line })}\n\n`);
            }
          });
          stream.on("close", () => { res.end(); conn.end(); });
          req.on("close", () => { try { stream.close(); } catch {} conn.end(); });
        } catch (e) {
          if (conn) conn.end();
          if (!res.headersSent) res.status(500).json({ error: e.message });
          else res.end();
        }
      } else {
        let conn;
        try {
          conn = await remote.connect(hostRecord);
          const logs = await remote.getLogsRemote(conn, projectName, remoteDir, service, tail);
          conn.end();
          res.json({ logs });
        } catch (e) {
          if (conn) conn.end();
          res.status(500).json({ error: `SSH log fetch failed: ${e.message}` });
        }
      }
      return;
    }

    // Local deployment
    if (!deployment.docker_project) return res.status(400).json({ error: "Not provisioned yet" });

    const composeFile = path.join(getDeploymentDir(deployment.id, deployment.deploy_dir), "docker-compose.yaml");
    if (!fs.existsSync(composeFile)) {
      return res.status(400).json({ error: "Compose file not found" });
    }

    const toolsServices = ["bridge-ui", "db", "db-init", "backend-l1", "backend-l2", "frontend-l1", "frontend-l2", "proxy", "proxy-l2-only", "redis-db", "function-selectors", "function-selectors-l2"];
    const isToolsService = service && toolsServices.includes(service);

    if (follow) {
      res.writeHead(200, {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache",
        Connection: "keep-alive",
      });

      const proc = isToolsService
        ? docker.streamToolsLogs(`${deployment.docker_project}-tools`, service)
        : docker.streamLogs(deployment.docker_project, composeFile, service);

      proc.stdout.on("data", (chunk) => {
        for (const line of chunk.toString().split("\n").filter(Boolean)) {
          res.write(`data: ${JSON.stringify({ line })}\n\n`);
        }
      });
      proc.stderr.on("data", (chunk) => {
        for (const line of chunk.toString().split("\n").filter(Boolean)) {
          res.write(`data: ${JSON.stringify({ line })}\n\n`);
        }
      });
      proc.on("close", () => res.end());
      req.on("close", () => proc.kill("SIGTERM"));
    } else {
      const logs = isToolsService
        ? await docker.getToolsLogs(`${deployment.docker_project}-tools`, service, tail)
        : await docker.getLogs(deployment.docker_project, composeFile, service, tail);
      res.json({ logs });
    }
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// GET /api/deployments/:id/monitoring
router.get("/:id/monitoring", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });

    if (!deployment.l2_port) {
      return res.json({ l1: null, l2: null });
    }

    let rpcHost = "127.0.0.1";
    if (deployment.host_id) {
      const host = db.prepare("SELECT * FROM hosts WHERE id = ?").get(deployment.host_id);
      if (host) rpcHost = host.hostname;
    }

    // Testnet: no local L1 port, use external rpc_url instead
    const l1Url = deployment.l1_port
      ? `http://${rpcHost}:${deployment.l1_port}`
      : deployment.rpc_url || null;
    const l2Url = `http://${rpcHost}:${deployment.l2_port}`;
    const prefundedAddress = "0x3d1e15a1a55578f7c920884a9943b3b35d0d885b";

    const reject = Promise.reject(new Error("no url"));
    reject.catch(() => {}); // suppress unhandled rejection
    const [l1Block, l2Block, l1Chain, l2Chain, l1Balance, l2Balance] = await Promise.allSettled([
      l1Url ? rpc.getBlockNumber(l1Url) : reject,
      rpc.getBlockNumber(l2Url),
      l1Url ? rpc.getChainId(l1Url) : reject,
      rpc.getChainId(l2Url),
      l1Url ? rpc.getBalance(l1Url, prefundedAddress) : reject,
      rpc.getBalance(l2Url, prefundedAddress),
    ]);

    res.json({
      l1: l1Url ? {
        healthy: l1Block.status === "fulfilled",
        blockNumber: l1Block.status === "fulfilled" ? l1Block.value : null,
        chainId: l1Chain.status === "fulfilled" ? l1Chain.value : null,
        balance: l1Balance.status === "fulfilled" ? l1Balance.value : null,
        rpcUrl: l1Url,
      } : null,
      l2: {
        healthy: l2Block.status === "fulfilled",
        blockNumber: l2Block.status === "fulfilled" ? l2Block.value : null,
        chainId: l2Chain.status === "fulfilled" ? l2Chain.value : null,
        balance: l2Balance.status === "fulfilled" ? l2Balance.value : null,
        rpcUrl: l2Url,
      },
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// ---------------------------------------------------------------------------
// AI Deploy Prompt
// ---------------------------------------------------------------------------

/** POST /api/deployments/:id/ai-prompt — generate AI deployment prompt */
router.post("/:id/ai-prompt", async (req, res) => {
  try {
    const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
    if (!deployment) return res.status(404).json({ error: "Deployment not found" });

    const { cloud, region, vmType, l1Mode, l1RpcUrl, l1ChainId, l1Network, includeProver, walletConfig, storageGB, keyPairName, awsAccount, programName } = req.body;
    if (!cloud) {
      return res.status(400).json({ error: "cloud is required" });
    }

    // Smart defaults per cloud provider
    const defaults = cloud === "local"
      ? { region: "local", vmType: "local" }
      : cloud === "gcp"
      ? { region: "asia-northeast3", vmType: "e2-standard-4" }
      : cloud === "vultr"
      ? { region: "icn", vmType: "vc2-6c-16gb" }
      : { region: "ap-northeast-2", vmType: "t3.xlarge" };

    // For local deployments, allocate real free ports to avoid conflicts
    // with other running processes. Cloud VMs use DEFAULT_PORTS (fresh server).
    let ports;
    if (cloud === "local") {
      const { getNextAvailablePorts } = require("../db/deployments");
      ports = await getNextAvailablePorts();
    }

    const prompt = generateAIDeployPrompt({
      deployment, cloud,
      region: region || defaults.region,
      vmType: vmType || defaults.vmType,
      l1Mode: l1Mode || "local",
      l1RpcUrl, l1ChainId, l1Network,
      includeProver, walletConfig,
      storageGB: storageGB || 30,
      keyPairName: keyPairName || "",
      ports,
    });

    // Save cloud config to deployment for persistent monitoring
    const safeName = (deployment.name || "l2").replace(/[^a-zA-Z0-9-]/g, "-").replace(/-+/g, "-").replace(/^-|-$/g, "").toLowerCase().slice(0, 30);
    const vmName = `tokamak-${safeName}-${deployment.id.slice(0, 8)}`;
    const cloudConfig = {
      mode: "ai-deploy",
      cloud,
      region: region || defaults.region,
      vmType: vmType || defaults.vmType,
      vmName,
      storageGB: storageGB || 30,
      keyPairName: keyPairName || "",
      l1Mode: l1Mode || "local",
      l1RpcUrl, l1ChainId, l1Network,
      includeProver,
      awsAccount: awsAccount || "",
      programName: programName || deployment.program_slug || "evm-l2",
      prompt,
    };
    db.prepare("UPDATE deployments SET config = ?, phase = 'ai-deploy' WHERE id = ?")
      .run(JSON.stringify(cloudConfig), deployment.id);

    // Store as pending for Messenger to pick up
    pendingAIPrompt = {
      deploymentId: deployment.id,
      deploymentName: deployment.name,
      cloud,
      prompt,
      createdAt: new Date().toISOString(),
    };

    res.json({ prompt, sentToMessenger: true });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

// ---------------------------------------------------------------------------
// AI Chat (direct in Manager)
// ---------------------------------------------------------------------------

let aiConfig = { provider: null, apiKey: null, model: null };

// Try to load AI config from Messenger's Keychain on startup
try {
  const kcConfigStr = keychain.getSecret("ai-config");
  const kcApiKey = keychain.getSecret("ai-api-key");
  if (kcConfigStr && kcApiKey) {
    const kcConfig = JSON.parse(kcConfigStr);
    aiConfig = { provider: kcConfig.provider || "claude", apiKey: kcApiKey, model: kcConfig.model || null };
    console.log(`[ai] Loaded AI config from Messenger Keychain: provider=${aiConfig.provider}, model=${aiConfig.model || "default"}`);
  }
} catch (e) {
  console.log("[ai] No Messenger AI config found in Keychain");
}

router.post("/ai-deploy/ai-config", (req, res) => {
  const { provider, apiKey, model } = req.body;
  if (!provider || !apiKey) return res.status(400).json({ error: "provider and apiKey required" });
  aiConfig = { provider, apiKey, model: model || null };

  // Sync to Messenger Keychain only if Messenger doesn't already have a key
  let messengerSynced = false;
  try {
    const existingKey = keychain.getSecret("ai-api-key");
    if (!existingKey) {
      const configObj = JSON.stringify({ provider, model: model || null });
      const ok1 = keychain.setSecret("ai-config", configObj);
      const ok2 = keychain.setSecret("ai-api-key", apiKey);
      messengerSynced = ok1 && ok2;
      if (messengerSynced) console.log(`[ai] Synced AI config to Messenger Keychain (was empty): provider=${provider}`);
    } else {
      console.log(`[ai] Messenger Keychain already has AI key, skipping sync`);
    }
  } catch (e) {
    console.error("[ai] Failed to check/sync Messenger Keychain:", e.message);
  }
  res.json({ ok: true, messengerSynced });
});

// Check if Messenger app has AI settings in Keychain
router.get("/ai-deploy/messenger-ai-config", (_req, res) => {
  try {
    const configStr = keychain.getSecret("ai-config");
    const apiKey = keychain.getSecret("ai-api-key");
    if (configStr && apiKey) {
      const config = JSON.parse(configStr);
      const maskedKey = apiKey.length > 8
        ? apiKey.slice(0, 4) + "..." + apiKey.slice(-4)
        : "****";
      return res.json({
        available: true,
        provider: config.provider || "claude",
        model: config.model || null,
        maskedKey,
      });
    }
    res.json({ available: false });
  } catch {
    res.json({ available: false });
  }
});

// Apply Messenger AI config to Manager (Keychain API key only)
router.post("/ai-deploy/use-messenger-ai", (_req, res) => {
  try {
    const configStr = keychain.getSecret("ai-config");
    const apiKey = keychain.getSecret("ai-api-key");
    if (!configStr || !apiKey) return res.status(404).json({ error: "Messenger AI config not found in Keychain" });
    const config = JSON.parse(configStr);
    aiConfig = { provider: config.provider || "claude", apiKey, model: config.model || null };
    res.json({ ok: true, provider: aiConfig.provider, model: aiConfig.model });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

router.get("/ai-deploy/ai-config", (_req, res) => {
  res.json({
    configured: !!aiConfig.apiKey,
    provider: aiConfig.provider,
    model: aiConfig.model,
    hasKey: !!aiConfig.apiKey,
  });
});

router.post("/ai-deploy/chat", async (req, res) => {
  const { messages, systemPrompt } = req.body;
  if (!aiConfig.apiKey) return res.status(400).json({ error: "AI not configured. Set API key first." });
  if (!messages || !messages.length) return res.status(400).json({ error: "messages required" });

  try {
    const provider = aiConfig.provider || "claude";
    let responseText;

    if (provider === "claude") {
      const apiMessages = messages.map(m => ({ role: m.role, content: m.content }));
      const body = {
        model: aiConfig.model || "claude-sonnet-4-6",
        max_tokens: 8192,
        messages: apiMessages,
      };
      if (systemPrompt) body.system = systemPrompt;

      const r = await fetch("https://api.anthropic.com/v1/messages", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "x-api-key": aiConfig.apiKey,
          "anthropic-version": "2023-06-01",
        },
        body: JSON.stringify(body),
      });
      if (!r.ok) {
        const err = await r.json().catch(() => ({}));
        throw new Error(err.error?.message || `Claude API error: ${r.status}`);
      }
      const data = await r.json();
      responseText = data.content?.[0]?.text || "";

    } else if (provider === "openai" || provider === "gpt") {
      const apiMessages = [];
      if (systemPrompt) apiMessages.push({ role: "system", content: systemPrompt });
      messages.forEach(m => apiMessages.push({ role: m.role, content: m.content }));

      const r = await fetch("https://api.openai.com/v1/chat/completions", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "Authorization": `Bearer ${aiConfig.apiKey}`,
        },
        body: JSON.stringify({
          model: aiConfig.model || "gpt-4o",
          messages: apiMessages,
          max_completion_tokens: 8192,
        }),
      });
      if (!r.ok) {
        const err = await r.json().catch(() => ({}));
        throw new Error(err.error?.message || `OpenAI API error: ${r.status}`);
      }
      const data = await r.json();
      responseText = data.choices?.[0]?.message?.content || "";

    } else if (provider === "gemini") {
      const apiMessages = [];
      if (systemPrompt) apiMessages.push({ role: "system", content: systemPrompt });
      messages.forEach(m => apiMessages.push({ role: m.role, content: m.content }));

      const r = await fetch("https://generativelanguage.googleapis.com/v1beta/openai/chat/completions", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "Authorization": `Bearer ${aiConfig.apiKey}`,
        },
        body: JSON.stringify({
          model: aiConfig.model || "gemini-2.0-flash",
          messages: apiMessages,
          max_tokens: 8192,
        }),
      });
      if (!r.ok) {
        const err = await r.json().catch(() => ({}));
        throw new Error(err.error?.message || `Gemini API error: ${r.status}`);
      }
      const data = await r.json();
      responseText = data.choices?.[0]?.message?.content || "";

    } else {
      return res.status(400).json({ error: `Unknown provider: ${provider}` });
    }

    res.json({ role: "assistant", content: responseText });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});

module.exports = router;
