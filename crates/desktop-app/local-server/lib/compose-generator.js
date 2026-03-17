/**
 * Docker Compose file generator.
 *
 * Generates a deployment-specific docker-compose.yaml based on:
 * - Selected app/program (evm-l2, zk-dex, tokamon, etc.)
 * - Port assignments (unique per deployment)
 * - Chain configuration
 *
 * Local deployments always build from source (no pull strategy).
 * Each deployment gets a unique image name: tokamak-appchain:{programSlug}-{projectName}
 *
 * Different apps require different:
 * - Docker images (built per-deployment)
 * - Build features (l2,l2-sql vs l2,l2-sql,sp1)
 * - Guest programs (evm-l2 vs zk-dex)
 * - Genesis files (l2.json vs l2-zk-dex.json)
 * - Prover backends (exec vs sp1)
 * - Verification contracts
 */

const fs = require("fs");
const path = require("path");
const { ETHREX_ROOT } = require("./docker-local");

// App-specific configuration profiles
const APP_PROFILES = {
  "evm-l2": {
    dockerfile: null, // uses default Dockerfile
    buildFeatures: "--features l2,l2-sql",
    guestPrograms: null, // no guest program build arg needed
    genesisFile: "l2.json",
    proverBackend: "exec",
    sp1Enabled: false,
    registerGuestPrograms: null,
    programsToml: null,
    deployRich: true,
    description: "Default EVM L2 — full EVM compatibility",
  },
  "zk-dex": {
    dockerfile: "Dockerfile.sp1",
    buildFeatures: "--features l2,l2-sql,sp1",
    guestPrograms: "evm-l2,zk-dex",
    genesisFile: "l2-zk-dex.json",
    proverBackend: "sp1",
    sp1Enabled: true,
    registerGuestPrograms: "zk-dex",
    programsToml: "programs-zk-dex.toml",
    deployRich: false,
    description: "ZK-DEX — decentralized exchange with SP1 ZK proofs",
  },
  "tokamon": {
    dockerfile: null,
    buildFeatures: "--features l2,l2-sql",
    guestPrograms: null,
    genesisFile: "l2.json",
    proverBackend: "exec",
    sp1Enabled: false,
    registerGuestPrograms: null,
    programsToml: null,
    deployRich: true,
    description: "Tokamon — gaming application circuits",
  },
};

/**
 * Sanitize a program slug to prevent YAML injection.
 * Only allows lowercase alphanumeric, hyphens, and underscores.
 */
function sanitizeSlug(slug) {
  return String(slug).replace(/[^a-z0-9_-]/g, "");
}

/**
 * Get the app profile for a given program slug.
 * Falls back to evm-l2 for unknown programs.
 */
function getAppProfile(programSlug) {
  return APP_PROFILES[programSlug] || APP_PROFILES["evm-l2"];
}

/**
 * Generate docker-compose.yaml content for a local deployment (build-only).
 *
 * Each deployment builds its own images with unique names to allow
 * multiple apps on the same machine without image conflicts.
 *
 * @param {Object} opts
 * @param {string} opts.programSlug - App identifier (evm-l2, zk-dex, etc.)
 * @param {number} opts.l1Port - Host port for L1 RPC
 * @param {number} opts.l2Port - Host port for L2 RPC
 * @param {number} opts.proofCoordPort - Host port for proof coordinator
 * @param {string} opts.projectName - Docker Compose project name (e.g. tokamak-08cab1ae)
 * @param {boolean} [opts.gpu=false] - Enable NVIDIA GPU for SP1 prover
 * @param {boolean} [opts.dumpFixtures=false] - Enable ETHREX_DUMP_FIXTURES for offline test data collection
 * @param {boolean} [opts.isPublic=false] - Bind L2 ports to 0.0.0.0 for public access (default: 127.0.0.1)
 * @returns {string} docker-compose.yaml content
 */
function generateComposeFile(opts) {
  const { programSlug: rawSlug, l1Port, l2Port, proofCoordPort = 3900, metricsPort = 3702, projectName, gpu = false, dumpFixtures = false, isPublic = false, customGenesisPath, l2ChainId, customL1GenesisPath } = opts;
  const programSlug = sanitizeSlug(rawSlug);
  const bindAddr = isPublic ? '0.0.0.0' : '127.0.0.1';
  // Proof coordinator and metrics are internal-only — never bind to 0.0.0.0
  const internalBindAddr = '127.0.0.1';
  const profile = getAppProfile(programSlug);
  const workdir = "/usr/local/bin";

  // Unique image names per deployment: tokamak-appchain:{programSlug}-{projectName}
  const l1Image = `tokamak-appchain:l1-${projectName}`;
  const l2Image = `tokamak-appchain:${programSlug}-${projectName}`;

  // Build section for L2 image
  const buildSection = profile.dockerfile
    ? `    build:
      context: ${ETHREX_ROOT}
      dockerfile: ${profile.dockerfile}
      args:
        - BUILD_FLAGS=${profile.buildFeatures}${profile.guestPrograms ? `\n        - GUEST_PROGRAMS=${profile.guestPrograms}` : ""}`
    : `    build:
      context: ${ETHREX_ROOT}
      args:
        - BUILD_FLAGS=${profile.buildFeatures}`;

  // L1 build
  const l1Build = `    build: ${ETHREX_ROOT}`;

  // Genesis sources: use custom genesis if provided, otherwise stock
  const genesisSource = customGenesisPath || `${ETHREX_ROOT}/fixtures/genesis/${profile.genesisFile}`;
  const l1GenesisSource = customL1GenesisPath || `${ETHREX_ROOT}/fixtures/genesis/l1.json`;

  // Deployer env vars (ETHREX_L2_SP1 is set in the base template from profile.sp1Enabled)
  let deployerExtraEnv = "";
  if (profile.registerGuestPrograms) {
    deployerExtraEnv += `      - ETHREX_REGISTER_GUEST_PROGRAMS=${profile.registerGuestPrograms}\n`;
  }
  if (profile.guestPrograms) {
    deployerExtraEnv += `      - GUEST_PROGRAMS=${profile.guestPrograms}\n`;
  }
  deployerExtraEnv += `      - ETHREX_DEPLOYER_GENESIS_L2_PATH=${workdir}/fixtures/genesis/${profile.genesisFile}\n`;
  if (l2ChainId) {
    deployerExtraEnv += `      - ETHREX_L2_CHAIN_ID=${l2ChainId}\n`;
  }

  // No extra deployer genesis volume needed — main mount line already uses genesisSource
  let deployerExtraVolumes = "";

  // L2 extra config
  let l2ExtraVolumes = "";
  let l2Genesis = `/genesis/${profile.genesisFile}`;
  if (profile.programsToml) {
    l2ExtraVolumes += `      - ${ETHREX_ROOT}/crates/l2/${profile.programsToml}:/etc/ethrex/programs.toml\n`;
  }

  // Prover config
  let proverExtraEnv = "";
  let proverExtraVolumes = "";
  let proverCommand = `l2 prover --backend ${profile.proverBackend} --proof-coordinators tcp://tokamak-app-l2:3900`;
  if (profile.proverBackend === "sp1") {
    proverCommand += ` --programs-config /etc/ethrex/programs.toml`;
    proverExtraEnv = `      - ETHREX_PROGRAMS_CONFIG=/etc/ethrex/programs.toml
      - PROVER_CLIENT_TIMED=true
      - DOCKER_HOST=\${DOCKER_HOST:-unix:///var/run/docker.sock}
      - HOME=\${HOME}`;
    proverExtraVolumes = `      - ${ETHREX_ROOT}/crates/l2/${profile.programsToml}:/etc/ethrex/programs.toml
      - /var/run/docker.sock:/var/run/docker.sock
      - \${HOME}/.sp1:\${HOME}/.sp1
      - /tmp:/tmp`;
  }
  if (dumpFixtures) {
    proverExtraEnv += (proverExtraEnv ? "\n" : "") + `      - ETHREX_DUMP_FIXTURES=/tmp/fixtures`;
    // SP1 already has /tmp:/tmp; exec backend needs the volume
    if (profile.proverBackend !== "sp1") {
      proverExtraVolumes += (proverExtraVolumes ? "\n" : "") + `      - /tmp/fixtures:/tmp/fixtures`;
    }
  }

  const yaml = `# Auto-generated by Tokamak Platform
# App: ${programSlug} (${profile.description})
# Project: ${projectName}
# Mode: build from source

volumes:
  env:

services:
  tokamak-app-l1:
    container_name: ${projectName}-l1
    image: "${l1Image}"
${l1Build}
    ports:
      - 127.0.0.1:${l1Port}:8545
    environment:
      - ETHREX_LOG_LEVEL
      - ETHREX_DEV_BLOCK_TIME_MS=2000
    volumes:
      - ${l1GenesisSource}:/genesis/l1.json
    command: --network /genesis/l1.json --http.addr 0.0.0.0 --http.port 8545 --dev

  tokamak-app-deployer:
    container_name: ${projectName}-deployer
    image: "${l2Image}"
    restart: "no"
    volumes:
      - ${ETHREX_ROOT}/crates/l2/contracts:${workdir}/contracts
      - env:/env/
      - ${l1GenesisSource}:${workdir}/fixtures/genesis/l1.json
      - ${genesisSource}:${workdir}/fixtures/genesis/${profile.genesisFile}
      - ${ETHREX_ROOT}/fixtures/keys/private_keys_l1.txt:${workdir}/fixtures/keys/private_keys_l1.txt
${deployerExtraVolumes}    environment:
      - ETHREX_ETH_RPC_URL=http://tokamak-app-l1:8545
      - ETHREX_DEPLOYER_L1_PRIVATE_KEY=0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924
      - ETHREX_DEPLOYER_ENV_FILE_PATH=/env/.env
      - ETHREX_DEPLOYER_GENESIS_L1_PATH=${workdir}/fixtures/genesis/l1.json
      - ETHREX_DEPLOYER_PRIVATE_KEYS_FILE_PATH=${workdir}/fixtures/keys/private_keys_l1.txt
      - ETHREX_DEPLOYER_DEPLOY_RICH=${profile.deployRich}
      - ETHREX_DEPLOYER_RECEIPT_INTERVAL_SECS=2
      - ETHREX_L2_RISC0=false
      - ETHREX_L2_SP1=${profile.sp1Enabled}
      - ETHREX_L2_TDX=false
      - ETHREX_DEPLOYER_ALIGNED=false
      - ETHREX_SP1_VERIFICATION_KEY_PATH=${workdir}/riscv32im-succinct-zkvm-vk-bn254
      - ETHREX_RISC0_VERIFICATION_KEY_PATH=${workdir}/riscv32im-risc0-vk
      - ETHREX_ON_CHAIN_PROPOSER_OWNER=0x4417092b70a3e5f10dc504d0947dd256b965fc62
      - ETHREX_BRIDGE_OWNER=0x4417092b70a3e5f10dc504d0947dd256b965fc62
      - ETHREX_BRIDGE_OWNER_PK=0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e
      - ETHREX_DEPLOYER_SEQUENCER_REGISTRY_OWNER=0x4417092b70a3e5f10dc504d0947dd256b965fc62
      - ETHREX_DEPLOYER_DEPLOY_BASED_CONTRACTS=false
      - ETHREX_L2_VALIDIUM=false
      - COMPILE_CONTRACTS=true
      - ETHREX_USE_COMPILED_GENESIS=true
${deployerExtraEnv}    depends_on:
      - tokamak-app-l1
    entrypoint:
      - /bin/bash
      - -c
      - touch /env/.env; ./ethrex l2 deploy "$$0" "$$@"
    command: >
      --randomize-contract-deployment

  tokamak-app-l2:
    container_name: ${projectName}-l2
    image: "${l2Image}"
${buildSection}
    ports:
      - ${bindAddr}:${l2Port}:1729
      - ${internalBindAddr}:${proofCoordPort}:3900
      - ${internalBindAddr}:${metricsPort}:3702
    environment:
      - ETHREX_ETH_RPC_URL=http://tokamak-app-l1:8545
      - ETHREX_L2_VALIDIUM=false
      - ETHREX_BLOCK_PRODUCER_BLOCK_TIME=\${ETHREX_BLOCK_PRODUCER_BLOCK_TIME:-5000}
      - ETHREX_WATCHER_BLOCK_DELAY=0
      - ETHREX_BASED=false
      - ETHREX_COMMITTER_COMMIT_TIME=\${ETHREX_COMMITTER_COMMIT_TIME:-60000}
      - ETHREX_WATCHER_WATCH_INTERVAL=\${ETHREX_WATCHER_WATCH_INTERVAL:-12000}
      - ETHREX_OSAKA_ACTIVATION_TIME=\${ETHREX_OSAKA_ACTIVATION_TIME:-1761677592}
      - ETHREX_GUEST_PROGRAM_ID=${programSlug}
      - ETHREX_LOG_LEVEL
${dumpFixtures ? `      - ETHREX_DUMP_FIXTURES=/tmp/fixtures\n` : ""}    volumes:
      - ${genesisSource}:/genesis/${profile.genesisFile}
      - env:/env/
${dumpFixtures ? `      - /tmp/fixtures:/tmp/fixtures\n` : ""}${l2ExtraVolumes}    entrypoint:
      - /bin/bash
      - -c
      - export $$(xargs < /env/.env); ./ethrex l2 "$$0" "$$@"
    command: >
      --network ${l2Genesis}
      --http.addr 0.0.0.0
      --http.port 1729
      --authrpc.port 8552
      --proof-coordinator.addr 0.0.0.0
      --block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d
      --committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924
      --proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d
      --no-monitor
      --metrics
      --metrics.port 3702
      --metrics.addr 0.0.0.0
    depends_on:
      tokamak-app-deployer:
        condition: service_completed_successfully

  tokamak-app-prover:
    container_name: ${projectName}-prover
    image: "${l2Image}"
${proverExtraEnv ? `    environment:\n${proverExtraEnv}\n` : ""}${proverExtraVolumes ? `    volumes:\n${proverExtraVolumes}\n` : ""}    command: >
      ${proverCommand}
${gpu && profile.sp1Enabled ? `    deploy:
      resources:
        reservations:
          devices:
            - driver: nvidia
              count: all
              capabilities: [gpu]
` : ""}    depends_on:
      - tokamak-app-l2
`;

  return yaml;
}

// Pre-built image names for remote deployments only.
const IMAGE_REGISTRY = process.env.ETHREX_IMAGE_REGISTRY || "ghcr.io/tokamak-network";

function imageRef(name) {
  return IMAGE_REGISTRY ? `${IMAGE_REGISTRY}/${name}` : name;
}

const PULL_IMAGES = {
  "tokamak-appchain:l1": imageRef("tokamak-appchain:l1"),
  "tokamak-appchain:l2": imageRef("tokamak-appchain:l2"),
  "tokamak-appchain:sp1": imageRef("tokamak-appchain:sp1"),
};

// Thanos (Optimism) pre-built images — tags from trh-sdk constants
// op-geth has its own tag; all other stack components share ThanosStackImageTag
const THANOS_OP_GETH_TAG = "nightly-f8c04dcb";
const THANOS_STACK_TAG = "nightly-c9d8d16a";
// Hardhat account #0 — well-known devnet key, NEVER use on mainnet/testnet
const THANOS_DEVNET_PRIVATE_KEY = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const THANOS_IMAGES = {
  "l1-geth": "ethereum/client-go:v1.13.15",
  "op-geth": `tokamaknetwork/thanos-op-geth:${THANOS_OP_GETH_TAG}`,
  "op-node": `tokamaknetwork/thanos-op-node:${THANOS_STACK_TAG}`,
  "op-batcher": `tokamaknetwork/thanos-op-batcher:${THANOS_STACK_TAG}`,
  "op-proposer": `tokamaknetwork/thanos-op-proposer:${THANOS_STACK_TAG}`,
};

/**
 * Generate Thanos L1 geth service definition (for local and remote modes).
 */
function thanosL1Service({ projectName, l1Port, l1ChainId, bindAddr }) {
  return `  thanos-l1:
    container_name: ${projectName}-l1
    image: "${THANOS_IMAGES["l1-geth"]}"
    ports:
      - ${bindAddr}:${l1Port}:8545
    volumes:
      - thanos-l1-data:/root/.ethereum
    command:
      - --dev
      - --dev.period=3
      - --http
      - --http.addr=0.0.0.0
      - --http.port=8545
      - --http.api=eth,net,web3,debug,txpool,admin
      - --http.corsdomain=*
      - --http.vhosts=*
      - --ws
      - --ws.addr=0.0.0.0
      - --ws.port=8546
      - --ws.api=eth,net,web3,debug,txpool
      - --networkid=${l1ChainId}
    healthcheck:
      test: ["CMD", "geth", "attach", "--exec", "eth.blockNumber", "http://localhost:8545"]
      interval: 5s
      timeout: 3s
      retries: 20`;
}

/**
 * Generate common Thanos service definitions (L2, op-node, op-batcher, op-proposer).
 * Shared across local, testnet, and remote compose generators.
 */
function thanosServices({ projectName, l2Port, l2ChainId, l1RpcUrl, bindAddr, privateKey }) {
  const pk = privateKey || THANOS_DEVNET_PRIVATE_KEY;
  return `
  thanos-l2:
    container_name: ${projectName}-l2
    image: "${THANOS_IMAGES["op-geth"]}"
    ports:
      - ${bindAddr}:${l2Port}:8545
    volumes:
      - thanos-l2-data:/root/.ethereum
      - thanos-shared:/shared
    environment:
      - OP_GETH_GENESIS_FILE_PATH=/shared/genesis-l2.json
      - OP_GETH_SEQUENCER_HTTP=http://thanos-l2:8545
    command:
      - --http
      - --http.addr=0.0.0.0
      - --http.port=8545
      - --http.api=eth,net,web3,debug,txpool,engine
      - --http.corsdomain=*
      - --http.vhosts=*
      - --ws
      - --ws.addr=0.0.0.0
      - --ws.port=8546
      - --ws.api=eth,net,web3,debug,txpool,engine
      - --authrpc.addr=0.0.0.0
      - --authrpc.port=8551
      - --authrpc.jwtsecret=/shared/jwt-secret.txt
      - --authrpc.vhosts=*
      - --networkid=${l2ChainId}
      - --rollup.sequencerhttp=http://thanos-l2:8545
      - --rollup.disabletxpoolgossip
      - --gcmode=archive
      - --nodiscover
      - --maxpeers=0
    healthcheck:
      test: ["CMD-SHELL", "curl -sf http://localhost:8545 || exit 1"]
      interval: 5s
      timeout: 3s
      retries: 20

  thanos-op-node:
    container_name: ${projectName}-op-node
    image: "${THANOS_IMAGES["op-node"]}"
    depends_on:
      thanos-l2:
        condition: service_healthy
    volumes:
      - thanos-shared:/shared
    environment:
      - OP_NODE_L1_ETH_RPC=${l1RpcUrl}
      - OP_NODE_L1_BEACON=${l1RpcUrl}
      - OP_NODE_L2_ENGINE_RPC=http://thanos-l2:8551
      - OP_NODE_L2_ENGINE_AUTH=/shared/jwt-secret.txt
      - OP_NODE_ROLLUP_CONFIG=/shared/rollup.json
      - OP_NODE_P2P_DISABLE=true
      - OP_NODE_SEQUENCER_ENABLED=true
      - OP_NODE_SEQUENCER_L1_CONFS=0
      - OP_NODE_RPC_ADDR=0.0.0.0
      - OP_NODE_RPC_PORT=9545

  thanos-op-batcher:
    container_name: ${projectName}-batcher
    image: "${THANOS_IMAGES["op-batcher"]}"
    depends_on:
      - thanos-op-node
    volumes:
      - thanos-shared:/shared
    environment:
      - OP_BATCHER_L1_ETH_RPC=${l1RpcUrl}
      - OP_BATCHER_L2_ETH_RPC=http://thanos-l2:8545
      - OP_BATCHER_ROLLUP_RPC=http://thanos-op-node:9545
      - OP_BATCHER_PRIVATE_KEY=${pk}
      - OP_BATCHER_MAX_CHANNEL_DURATION=1
      - OP_BATCHER_SUB_SAFETY_MARGIN=4

  thanos-op-proposer:
    container_name: ${projectName}-proposer
    image: "${THANOS_IMAGES["op-proposer"]}"
    depends_on:
      - thanos-op-node
    volumes:
      - thanos-shared:/shared
    environment:
      - OP_PROPOSER_L1_ETH_RPC=${l1RpcUrl}
      - OP_PROPOSER_ROLLUP_RPC=http://thanos-op-node:9545
      - OP_PROPOSER_PRIVATE_KEY=${pk}
      - OP_PROPOSER_L2OO_ADDRESS_FILE=/shared/L2OutputOracleProxy.json
      - OP_PROPOSER_POLL_INTERVAL=6s
`;
}

/**
 * Generate docker-compose.yaml for REMOTE deployment.
 *
 * Key difference from local: uses `image:` with pre-built registry images
 * instead of `build:` from source. No source code needed on the remote server.
 * Genesis files and config are baked into the images or mounted from a data dir.
 *
 * @param {Object} opts
 * @param {string} opts.programSlug
 * @param {number} opts.l1Port
 * @param {number} opts.l2Port
 * @param {string} opts.projectName
 * @param {number} opts.proofCoordPort - Host port for proof coordinator
 * @param {string} opts.dataDir - Remote data directory (e.g. /opt/tokamak/<id>)
 * @returns {string} docker-compose.yaml content
 */
function generateRemoteComposeFile(opts) {
  const { programSlug: rawSlug, l1Port, l2Port, proofCoordPort = 3900, projectName, dataDir, l2ChainId, bindAddress = "0.0.0.0", customL2GenesisPath, customL1GenesisPath } = opts;
  const programSlug = sanitizeSlug(rawSlug);
  const profile = getAppProfile(programSlug);
  const workdir = "/usr/local/bin";

  const l1Image = PULL_IMAGES["tokamak-appchain:l1"];
  // Map profile to the correct pre-built image name for remote
  const remoteImageKey = profile.sp1Enabled ? "tokamak-appchain:sp1" : "tokamak-appchain:l2";
  const l2Image = PULL_IMAGES[remoteImageKey];

  const l2GenesisContainer = profile.genesisFile !== "l2.json"
    ? `/genesis/${profile.genesisFile}`
    : "/genesis/l2.json";

  // Custom genesis volume mounts (for local prebuilt mode — unique chain IDs per deployment)
  const hasCustomGenesis = !!(customL2GenesisPath || customL1GenesisPath);
  let l1ExtraVolumes = "";
  let deployerExtraVolumes = "";
  let l2ExtraVolumes = "";
  let l1GenesisCmd = "/genesis/l1.json";
  let l2Genesis = l2GenesisContainer;
  let deployerL1GenesisPath = `${workdir}/fixtures/genesis/l1.json`;
  let deployerL2GenesisPath = `${workdir}/fixtures/genesis/${profile.genesisFile}`;

  if (customL1GenesisPath) {
    l1ExtraVolumes = `    volumes:\n      - ${customL1GenesisPath}:/custom-genesis/l1.json:ro`;
    deployerExtraVolumes += `      - ${customL1GenesisPath}:/custom-genesis/l1.json:ro\n`;
    l1GenesisCmd = "/custom-genesis/l1.json";
    deployerL1GenesisPath = "/custom-genesis/l1.json";
  }
  if (customL2GenesisPath) {
    deployerExtraVolumes += `      - ${customL2GenesisPath}:/custom-genesis/${profile.genesisFile}:ro\n`;
    l2ExtraVolumes = `      - ${customL2GenesisPath}:/custom-genesis/${profile.genesisFile}:ro\n`;
    l2Genesis = `/custom-genesis/${profile.genesisFile}`;
    deployerL2GenesisPath = `/custom-genesis/${profile.genesisFile}`;
  }

  // Deployer extra env (ETHREX_L2_SP1 is set in the base template from profile.sp1Enabled)
  let deployerExtraEnv = "";
  if (profile.registerGuestPrograms) deployerExtraEnv += `      - ETHREX_REGISTER_GUEST_PROGRAMS=${profile.registerGuestPrograms}\n`;
  if (profile.guestPrograms) deployerExtraEnv += `      - GUEST_PROGRAMS=${profile.guestPrograms}\n`;
  deployerExtraEnv += `      - ETHREX_DEPLOYER_GENESIS_L2_PATH=${deployerL2GenesisPath}\n`;
  if (l2ChainId) {
    deployerExtraEnv += `      - ETHREX_L2_CHAIN_ID=${l2ChainId}\n`;
  }

  // Prover config
  let proverExtraEnv = "";
  let proverExtraVolumes = "";
  let proverCommand = `l2 prover --backend ${profile.proverBackend} --proof-coordinators tcp://tokamak-app-l2:3900`;
  if (profile.proverBackend === "sp1") {
    proverCommand += ` --programs-config /etc/ethrex/programs.toml`;
    proverExtraEnv = `    environment:
      - ETHREX_PROGRAMS_CONFIG=/etc/ethrex/programs.toml
      - PROVER_CLIENT_TIMED=true
      - DOCKER_HOST=unix:///var/run/docker.sock`;
    proverExtraVolumes = `    volumes:
      - ${dataDir}/programs.toml:/etc/ethrex/programs.toml
      - /var/run/docker.sock:/var/run/docker.sock
      - /tmp:/tmp`;
  }

  const yaml = `# Auto-generated by Tokamak Platform (${bindAddress === "127.0.0.1" ? "LOCAL-PREBUILT" : "REMOTE"} mode)
# App: ${programSlug} (${profile.description})
# Project: ${projectName}
# Pre-built images — no build step required

volumes:
  env:

services:
  tokamak-app-l1:
    container_name: ${projectName}-l1
    image: "${l1Image}"
    ports:
      - ${bindAddress}:${l1Port}:8545
${l1ExtraVolumes ? l1ExtraVolumes + "\n      - " + dataDir + "/genesis/l1.json:/genesis/l1.json:ro" : `    volumes:
      - ${dataDir}/genesis/l1.json:/genesis/l1.json:ro`}
    environment:
      - ETHREX_LOG_LEVEL
    command: --network /genesis/l1.json --http.addr 0.0.0.0 --http.port 8545 --dev

  tokamak-app-deployer:
    container_name: ${projectName}-deployer
    image: "${l2Image}"
    restart: "no"
    volumes:
      - env:/env/
      - ${dataDir}/genesis/l1.json:${workdir}/fixtures/genesis/l1.json:ro
      - ${dataDir}/genesis/${profile.genesisFile}:${workdir}/fixtures/genesis/${profile.genesisFile}:ro
      - ${dataDir}/genesis/private_keys_l1.txt:${workdir}/fixtures/keys/private_keys_l1.txt:ro
${deployerExtraVolumes}    environment:
      - ETHREX_ETH_RPC_URL=http://tokamak-app-l1:8545
      - ETHREX_DEPLOYER_L1_PRIVATE_KEY=\${ETHREX_DEPLOYER_L1_PRIVATE_KEY}
      - ETHREX_DEPLOYER_ENV_FILE_PATH=/env/.env
      - ETHREX_DEPLOYER_GENESIS_L1_PATH=${workdir}/fixtures/genesis/l1.json
      - ETHREX_DEPLOYER_PRIVATE_KEYS_FILE_PATH=${workdir}/fixtures/keys/private_keys_l1.txt
      - ETHREX_DEPLOYER_DEPLOY_RICH=true
      - ETHREX_L2_RISC0=false
      - ETHREX_L2_SP1=${profile.sp1Enabled}
      - ETHREX_L2_TDX=false
      - ETHREX_DEPLOYER_ALIGNED=false
      - ETHREX_SP1_VERIFICATION_KEY_PATH=${workdir}/riscv32im-succinct-zkvm-vk-bn254
      - ETHREX_RISC0_VERIFICATION_KEY_PATH=${workdir}/riscv32im-risc0-vk
      - ETHREX_ON_CHAIN_PROPOSER_OWNER=\${ETHREX_ON_CHAIN_PROPOSER_OWNER}
      - ETHREX_BRIDGE_OWNER=\${ETHREX_BRIDGE_OWNER}
      - ETHREX_BRIDGE_OWNER_PK=\${ETHREX_BRIDGE_OWNER_PK}
      - ETHREX_DEPLOYER_SEQUENCER_REGISTRY_OWNER=\${ETHREX_DEPLOYER_SEQUENCER_REGISTRY_OWNER}
      - ETHREX_DEPLOYER_DEPLOY_BASED_CONTRACTS=false
      - ETHREX_L2_VALIDIUM=false
      - COMPILE_CONTRACTS=true
      - ETHREX_USE_COMPILED_GENESIS=true
${deployerExtraEnv}    depends_on:
      - tokamak-app-l1
    entrypoint:
      - /bin/bash
      - -c
      - touch /env/.env; ./ethrex l2 deploy "$$0" "$$@"
    command: >
      --randomize-contract-deployment

  tokamak-app-l2:
    container_name: ${projectName}-l2
    image: "${l2Image}"
    ports:
      - ${bindAddress}:${l2Port}:1729
      - ${bindAddress}:${proofCoordPort}:3900
    environment:
      - ETHREX_ETH_RPC_URL=http://tokamak-app-l1:8545
      - ETHREX_L2_VALIDIUM=false
      - ETHREX_BLOCK_PRODUCER_BLOCK_TIME=5000
      - ETHREX_WATCHER_BLOCK_DELAY=0
      - ETHREX_BASED=false
      - ETHREX_COMMITTER_COMMIT_TIME=60000
      - ETHREX_WATCHER_WATCH_INTERVAL=12000
      - ETHREX_OSAKA_ACTIVATION_TIME=1761677592
      - ETHREX_GUEST_PROGRAM_ID=${programSlug}
      - ETHREX_LOG_LEVEL
    volumes:
      - env:/env/
      - ${dataDir}/genesis/${profile.genesisFile}:${l2Genesis}:ro
${l2ExtraVolumes}    entrypoint:
      - /bin/bash
      - -c
      - export $$(xargs < /env/.env); ./ethrex l2 "$$0" "$$@"
    command: >
      --network ${l2Genesis}
      --http.addr 0.0.0.0
      --http.port 1729
      --authrpc.port 8552
      --proof-coordinator.addr 0.0.0.0
      --block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d
      --committer.l1-private-key \${ETHREX_COMMITTER_L1_PRIVATE_KEY}
      --proof-coordinator.l1-private-key \${ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY}
      --no-monitor
    depends_on:
      tokamak-app-deployer:
        condition: service_completed_successfully

  tokamak-app-prover:
    container_name: ${projectName}-prover
    image: "${l2Image}"
${proverExtraEnv}
${proverExtraVolumes}
    command: >
      ${proverCommand}
    depends_on:
      - tokamak-app-l2
`;

  return yaml;
}

/**
 * Generate docker-compose.yaml for REMOTE + TESTNET deployment.
 *
 * Combines Remote (pre-built images, no build) with Testnet (external L1, custom keys).
 * No L1 container — deployer + L2 + prover all connect to an external L1 RPC URL.
 *
 * @param {Object} opts
 * @param {string} opts.programSlug
 * @param {number} opts.l2Port
 * @param {number} opts.proofCoordPort
 * @param {string} opts.projectName
 * @param {string} opts.dataDir - Remote data directory
 * @param {string} opts.l1RpcUrl - External L1 RPC URL
 * @param {string} opts.deployerAddress - Deployer address (derived from key)
 * @param {string} opts.committerAddress - Committer address
 * @param {string} opts.proofCoordinatorAddress - Proof coordinator address
 * @param {string} opts.bridgeOwnerAddress - Bridge owner address
 * @param {number} [opts.l2ChainId]
 * @returns {string} docker-compose.yaml content (keys use ${VAR} substitution via --env-file)
 */
function generateRemoteTestnetComposeFile(opts) {
  const {
    programSlug: rawSlug, l2Port, proofCoordPort = 3900, projectName, dataDir,
    l1RpcUrl, deployerAddress, committerAddress, proofCoordinatorAddress,
    bridgeOwnerAddress, l2ChainId,
  } = opts;

  // Validate required addresses to fail fast on caller errors
  const missing = [];
  if (!deployerAddress) missing.push("deployerAddress");
  if (!committerAddress) missing.push("committerAddress");
  if (!proofCoordinatorAddress) missing.push("proofCoordinatorAddress");
  if (!bridgeOwnerAddress) missing.push("bridgeOwnerAddress");
  if (missing.length > 0) {
    throw new Error(`generateRemoteTestnetComposeFile: missing required option(s): ${missing.join(", ")}`);
  }

  const programSlug = sanitizeSlug(rawSlug);
  const profile = getAppProfile(programSlug);
  const workdir = "/usr/local/bin";

  // Pre-built images from registry (no build)
  const remoteImageKey = profile.sp1Enabled ? "tokamak-appchain:sp1" : "tokamak-appchain:l2";
  const l2Image = PULL_IMAGES[remoteImageKey];

  const l2Genesis = profile.genesisFile !== "l2.json"
    ? `/genesis/${profile.genesisFile}`
    : "/genesis/l2.json";

  // Deployer extra env
  let deployerExtraEnv = "";
  if (profile.registerGuestPrograms) deployerExtraEnv += `      - ETHREX_REGISTER_GUEST_PROGRAMS=${profile.registerGuestPrograms}\n`;
  if (profile.guestPrograms) deployerExtraEnv += `      - GUEST_PROGRAMS=${profile.guestPrograms}\n`;
  deployerExtraEnv += `      - ETHREX_DEPLOYER_GENESIS_L2_PATH=${workdir}/fixtures/genesis/${profile.genesisFile}\n`;
  if (l2ChainId) {
    deployerExtraEnv += `      - ETHREX_L2_CHAIN_ID=${l2ChainId}\n`;
  }

  // Prover config
  let proverExtraEnv = "";
  let proverExtraVolumes = "";
  let proverCommand = `l2 prover --backend ${profile.proverBackend} --proof-coordinators tcp://tokamak-app-l2:3900`;
  if (profile.proverBackend === "sp1") {
    proverCommand += ` --programs-config /etc/ethrex/programs.toml`;
    proverExtraEnv = `    environment:
      - ETHREX_PROGRAMS_CONFIG=/etc/ethrex/programs.toml
      - PROVER_CLIENT_TIMED=true
      - DOCKER_HOST=unix:///var/run/docker.sock`;
    proverExtraVolumes = `    volumes:
      - ${dataDir}/programs.toml:/etc/ethrex/programs.toml
      - /var/run/docker.sock:/var/run/docker.sock
      - /tmp:/tmp`;
  }

  const yaml = `# Auto-generated by Tokamak Platform (REMOTE + TESTNET mode)
# App: ${programSlug} (${profile.description})
# Project: ${projectName}
# Pre-built images — no build step required
# L1: External (${l1RpcUrl})

volumes:
  env:

services:
  tokamak-app-deployer:
    container_name: ${projectName}-deployer
    image: "${l2Image}"
    restart: "no"
    volumes:
      - env:/env/
    environment:
      - ETHREX_ETH_RPC_URL=${l1RpcUrl}
      - ETHREX_DEPLOYER_L1_PRIVATE_KEY=\${ETHREX_DEPLOYER_L1_PRIVATE_KEY}
      - ETHREX_DEPLOYER_ENV_FILE_PATH=/env/.env
      - ETHREX_DEPLOYER_GENESIS_L1_PATH=${workdir}/fixtures/genesis/l1.json
      - ETHREX_DEPLOYER_PRIVATE_KEYS_FILE_PATH=${workdir}/fixtures/keys/private_keys_l1.txt
      - ETHREX_DEPLOYER_DEPLOY_RICH=false
      - ETHREX_DEPLOYER_RECEIPT_INTERVAL_SECS=2
      - ETHREX_L2_RISC0=false
      - ETHREX_L2_SP1=${profile.sp1Enabled}
      - ETHREX_L2_TDX=false
      - ETHREX_DEPLOYER_ALIGNED=false
      - ETHREX_SP1_VERIFICATION_KEY_PATH=${workdir}/riscv32im-succinct-zkvm-vk-bn254
      - ETHREX_RISC0_VERIFICATION_KEY_PATH=${workdir}/riscv32im-risc0-vk
      - ETHREX_ON_CHAIN_PROPOSER_OWNER=${bridgeOwnerAddress}
      - ETHREX_BRIDGE_OWNER=${bridgeOwnerAddress}
      - ETHREX_BRIDGE_OWNER_PK=\${ETHREX_BRIDGE_OWNER_PK}
      - ETHREX_DEPLOYER_SEQUENCER_REGISTRY_OWNER=${deployerAddress}
      - ETHREX_DEPLOYER_COMMITTER_L1_ADDRESS=${committerAddress}
      - ETHREX_DEPLOYER_PROOF_SENDER_L1_ADDRESS=${proofCoordinatorAddress}
      - ETHREX_DEPLOYER_DEPLOY_BASED_CONTRACTS=false
      - ETHREX_L2_VALIDIUM=false
      - COMPILE_CONTRACTS=true
      - ETHREX_USE_COMPILED_GENESIS=true
${deployerExtraEnv}    entrypoint:
      - /bin/bash
      - -c
      - touch /env/.env; ./ethrex l2 deploy "$$0" "$$@"
    command: >
      --randomize-contract-deployment

  tokamak-app-l2:
    container_name: ${projectName}-l2
    image: "${l2Image}"
    ports:
      - 0.0.0.0:${l2Port}:1729
      - 127.0.0.1:${proofCoordPort}:3900
    environment:
      - ETHREX_ETH_RPC_URL=${l1RpcUrl}
      - ETHREX_L2_VALIDIUM=false
      - ETHREX_BLOCK_PRODUCER_BLOCK_TIME=5000
      - ETHREX_WATCHER_BLOCK_DELAY=0
      - ETHREX_BASED=false
      - ETHREX_COMMITTER_COMMIT_TIME=60000
      - ETHREX_WATCHER_WATCH_INTERVAL=12000
      - ETHREX_OSAKA_ACTIVATION_TIME=1761677592
      - ETHREX_GUEST_PROGRAM_ID=${programSlug}
      - ETHREX_COMMITTER_L1_PRIVATE_KEY=\${ETHREX_COMMITTER_L1_PRIVATE_KEY}
      - ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY=\${ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY}
      - ETHREX_LOG_LEVEL
    volumes:
      - env:/env/
    entrypoint:
      - /bin/bash
      - -c
      - export $$(xargs < /env/.env); ./ethrex l2 "$$0" "$$@"
    command: >
      --network ${l2Genesis}
      --http.addr 0.0.0.0
      --http.port 1729
      --authrpc.port 8552
      --proof-coordinator.addr 0.0.0.0
      --block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d
      --no-monitor
    depends_on:
      tokamak-app-deployer:
        condition: service_completed_successfully

  tokamak-app-prover:
    container_name: ${projectName}-prover
    image: "${l2Image}"
${proverExtraEnv}
${proverExtraVolumes}
    command: >
      ${proverCommand}
    depends_on:
      - tokamak-app-l2
`;

  return yaml;
}

/**
 * Generate docker-compose.yaml for TESTNET deployment.
 *
 * Key difference from local: NO L1 container — uses external L1 RPC URL.
 * Builds from source like local, but deployer/L2/prover connect to external L1.
 *
 * @param {Object} opts
 * @param {string} opts.programSlug - App identifier
 * @param {number} opts.l2Port - Host port for L2 RPC
 * @param {number} opts.proofCoordPort - Host port for proof coordinator
 * @param {number} opts.metricsPort - Host port for metrics
 * @param {string} opts.projectName - Docker Compose project name
 * @param {string} opts.l1RpcUrl - External L1 RPC URL (e.g. https://sepolia.infura.io/v3/...)
 * @param {string} opts.deployerAddress - Deployer address (derived from key)
 * @param {string} opts.committerAddress - Committer address
 * @param {string} opts.proofCoordinatorAddress - Proof coordinator address
 * @param {string} opts.bridgeOwnerAddress - Bridge owner address
 * @param {boolean} [opts.gpu=false] - Enable NVIDIA GPU
 * @returns {string} docker-compose.yaml content (keys use ${VAR} substitution via --env-file)
 */
function generateTestnetComposeFile(opts) {
  const {
    programSlug: rawSlug, l2Port, proofCoordPort = 3900, metricsPort = 3702,
    projectName, l1RpcUrl, gpu = false,
    deployerAddress, committerAddress, proofCoordinatorAddress, bridgeOwnerAddress,
    isPublic = false, customGenesisPath, l2ChainId,
  } = opts;

  // Validate required addresses to fail fast on caller errors
  const missing = [];
  if (!deployerAddress) missing.push("deployerAddress");
  if (!committerAddress) missing.push("committerAddress");
  if (!proofCoordinatorAddress) missing.push("proofCoordinatorAddress");
  if (!bridgeOwnerAddress) missing.push("bridgeOwnerAddress");
  if (missing.length > 0) {
    throw new Error(`generateTestnetComposeFile: missing required option(s): ${missing.join(", ")}`);
  }

  const programSlug = sanitizeSlug(rawSlug);
  const bindAddr = isPublic ? '0.0.0.0' : '127.0.0.1';
  // Proof coordinator and metrics are internal-only — never bind to 0.0.0.0
  const internalBindAddr = '127.0.0.1';
  const profile = getAppProfile(programSlug);
  const workdir = "/usr/local/bin";

  // Shared image name per programSlug — reuse across deployments to avoid redundant builds
  const l2Image = `tokamak-appchain:${programSlug}`;

  const buildSection = profile.dockerfile
    ? `    build:
      context: ${ETHREX_ROOT}
      dockerfile: ${profile.dockerfile}
      args:
        - BUILD_FLAGS=${profile.buildFeatures}${profile.guestPrograms ? `\n        - GUEST_PROGRAMS=${profile.guestPrograms}` : ""}`
    : `    build:
      context: ${ETHREX_ROOT}
      args:
        - BUILD_FLAGS=${profile.buildFeatures}`;

  // Genesis source: use custom genesis if provided, otherwise stock
  const genesisSource = customGenesisPath || `${ETHREX_ROOT}/fixtures/genesis/${profile.genesisFile}`;

  let deployerExtraEnv = "";
  if (profile.registerGuestPrograms) {
    deployerExtraEnv += `      - ETHREX_REGISTER_GUEST_PROGRAMS=${profile.registerGuestPrograms}\n`;
  }
  if (profile.guestPrograms) {
    deployerExtraEnv += `      - GUEST_PROGRAMS=${profile.guestPrograms}\n`;
  }
  deployerExtraEnv += `      - ETHREX_DEPLOYER_GENESIS_L2_PATH=${workdir}/fixtures/genesis/${profile.genesisFile}\n`;
  if (l2ChainId) {
    deployerExtraEnv += `      - ETHREX_L2_CHAIN_ID=${l2ChainId}\n`;
  }

  let deployerExtraVolumes = "";

  let l2ExtraVolumes = "";
  let l2Genesis = `/genesis/${profile.genesisFile}`;
  if (profile.programsToml) {
    l2ExtraVolumes += `      - ${ETHREX_ROOT}/crates/l2/${profile.programsToml}:/etc/ethrex/programs.toml\n`;
  }

  let proverExtraEnv = "";
  let proverExtraVolumes = "";
  let proverCommand = `l2 prover --backend ${profile.proverBackend} --proof-coordinators tcp://tokamak-app-l2:3900`;
  if (profile.proverBackend === "sp1") {
    proverCommand += ` --programs-config /etc/ethrex/programs.toml`;
    proverExtraEnv = `      - ETHREX_PROGRAMS_CONFIG=/etc/ethrex/programs.toml
      - PROVER_CLIENT_TIMED=true
      - DOCKER_HOST=\${DOCKER_HOST:-unix:///var/run/docker.sock}
      - HOME=\${HOME}`;
    proverExtraVolumes = `      - ${ETHREX_ROOT}/crates/l2/${profile.programsToml}:/etc/ethrex/programs.toml
      - /var/run/docker.sock:/var/run/docker.sock
      - \${HOME}/.sp1:\${HOME}/.sp1
      - /tmp:/tmp`;
  }

  const yaml = `# Auto-generated by Tokamak Platform
# App: ${programSlug} (${profile.description})
# Project: ${projectName}
# Mode: testnet (external L1)
# L1 RPC: ${l1RpcUrl}

volumes:
  env:

services:
  tokamak-app-deployer:
    container_name: ${projectName}-deployer
    image: "${l2Image}"
${buildSection}
    restart: "no"
    volumes:
      - ${ETHREX_ROOT}/crates/l2/contracts:${workdir}/contracts
      - env:/env/
      - ${ETHREX_ROOT}/fixtures/genesis/l1.json:${workdir}/fixtures/genesis/l1.json
      - ${genesisSource}:${workdir}/fixtures/genesis/${profile.genesisFile}
      - ${ETHREX_ROOT}/fixtures/keys/private_keys_l1.txt:${workdir}/fixtures/keys/private_keys_l1.txt
${deployerExtraVolumes}    environment:
      - ETHREX_ETH_RPC_URL=${l1RpcUrl}
      - ETHREX_DEPLOYER_L1_PRIVATE_KEY=\${ETHREX_DEPLOYER_L1_PRIVATE_KEY}
      - ETHREX_DEPLOYER_ENV_FILE_PATH=/env/.env
      - ETHREX_DEPLOYER_GENESIS_L1_PATH=${workdir}/fixtures/genesis/l1.json
      - ETHREX_DEPLOYER_PRIVATE_KEYS_FILE_PATH=${workdir}/fixtures/keys/private_keys_l1.txt
      - ETHREX_DEPLOYER_DEPLOY_RICH=false
      - ETHREX_L2_RISC0=false
      - ETHREX_L2_SP1=${profile.sp1Enabled}
      - ETHREX_L2_TDX=false
      - ETHREX_DEPLOYER_ALIGNED=false
      - ETHREX_SP1_VERIFICATION_KEY_PATH=${workdir}/riscv32im-succinct-zkvm-vk-bn254
      - ETHREX_RISC0_VERIFICATION_KEY_PATH=${workdir}/riscv32im-risc0-vk
      - ETHREX_ON_CHAIN_PROPOSER_OWNER=${bridgeOwnerAddress}
      - ETHREX_BRIDGE_OWNER=${bridgeOwnerAddress}
      - ETHREX_BRIDGE_OWNER_PK=\${ETHREX_BRIDGE_OWNER_PK}
      - ETHREX_DEPLOYER_SEQUENCER_REGISTRY_OWNER=${deployerAddress}
      - ETHREX_DEPLOYER_COMMITTER_L1_ADDRESS=${committerAddress}
      - ETHREX_DEPLOYER_PROOF_SENDER_L1_ADDRESS=${proofCoordinatorAddress}
      - ETHREX_DEPLOYER_DEPLOY_BASED_CONTRACTS=false
      - ETHREX_L2_VALIDIUM=false
      - COMPILE_CONTRACTS=true
      - ETHREX_USE_COMPILED_GENESIS=true
${deployerExtraEnv}    entrypoint:
      - /bin/bash
      - -c
      - touch /env/.env; ./ethrex l2 deploy "$$0" "$$@"
    command: >
      --randomize-contract-deployment

  tokamak-app-l2:
    container_name: ${projectName}-l2
    image: "${l2Image}"
    ports:
      - ${bindAddr}:${l2Port}:1729
      - ${internalBindAddr}:${proofCoordPort}:3900
      - ${internalBindAddr}:${metricsPort}:3702
    environment:
      - ETHREX_ETH_RPC_URL=${l1RpcUrl}
      - ETHREX_L2_VALIDIUM=false
      - ETHREX_BLOCK_PRODUCER_BLOCK_TIME=\${ETHREX_BLOCK_PRODUCER_BLOCK_TIME:-5000}
      - ETHREX_WATCHER_BLOCK_DELAY=0
      - ETHREX_BASED=false
      - ETHREX_COMMITTER_COMMIT_TIME=\${ETHREX_COMMITTER_COMMIT_TIME:-60000}
      - ETHREX_WATCHER_WATCH_INTERVAL=\${ETHREX_WATCHER_WATCH_INTERVAL:-12000}
      - ETHREX_OSAKA_ACTIVATION_TIME=\${ETHREX_OSAKA_ACTIVATION_TIME:-1761677592}
      - ETHREX_GUEST_PROGRAM_ID=${programSlug}
      - ETHREX_COMMITTER_L1_PRIVATE_KEY=\${ETHREX_COMMITTER_L1_PRIVATE_KEY}
      - ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY=\${ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY}
      - ETHREX_LOG_LEVEL
    volumes:
      - ${genesisSource}:/genesis/${profile.genesisFile}
      - env:/env/
${l2ExtraVolumes}    entrypoint:
      - /bin/bash
      - -c
      - export $$(xargs < /env/.env); ./ethrex l2 "$$0" "$$@"
    command: >
      --network ${l2Genesis}
      --http.addr 0.0.0.0
      --http.port 1729
      --authrpc.port 8552
      --proof-coordinator.addr 0.0.0.0
      --block-producer.coinbase-address 0x0007a881CD95B1484fca47615B64803dad620C8d
      --no-monitor
      --metrics
      --metrics.port 3702
      --metrics.addr 0.0.0.0
    depends_on:
      tokamak-app-deployer:
        condition: service_completed_successfully

  tokamak-app-prover:
    container_name: ${projectName}-prover
    image: "${l2Image}"
${proverExtraEnv ? `    environment:\n${proverExtraEnv}\n` : ""}${proverExtraVolumes ? `    volumes:\n${proverExtraVolumes}\n` : ""}    command: >
      ${proverCommand}
${gpu && profile.sp1Enabled ? `    deploy:
      resources:
        reservations:
          devices:
            - driver: nvidia
              count: all
              capabilities: [gpu]
` : ""}    depends_on:
      - tokamak-app-l2
`;

  return yaml;
}

/**
 * Generate programs.toml content for a program slug.
 */
function generateProgramsToml(programSlug) {
  return `default_program = "${programSlug}"\nenabled_programs = ["${programSlug}"]\n`;
}

/**
 * Write compose file to the deployment directory.
 * @returns {string} Path to the generated compose file
 */
function writeComposeFile(deploymentId, composeContent, customDir) {
  const deployDir = getDeploymentDir(deploymentId, customDir);
  fs.mkdirSync(deployDir, { recursive: true });
  const filePath = path.join(deployDir, "docker-compose.yaml");
  fs.writeFileSync(filePath, composeContent, "utf-8");
  return filePath;
}

function getDeploymentDir(deploymentId, customDir) {
  if (customDir) {
    return path.resolve(customDir);
  }
  const home = process.env.HOME || require("os").homedir();
  return path.join(home, ".tokamak", "deployments", deploymentId);
}

/**
 * Create a deployment-specific genesis file with a custom L2 chain ID.
 * Reads the stock genesis, replaces the chainId, and writes to the deployment dir.
 *
 * @param {string} programSlug - App identifier (determines which genesis template to use)
 * @param {number} l2ChainId - The custom L2 chain ID
 * @param {string} deploymentId - Deployment ID (for directory)
 * @param {string} [customDir] - Optional custom deployment directory
 * @returns {Promise<string>} Path to the generated genesis file
 */
async function writeCustomGenesis(programSlug, l2ChainId, deploymentId, customDir) {
  const fsp = require("fs").promises;
  const profile = getAppProfile(programSlug);
  const stockPath = path.join(ETHREX_ROOT, "fixtures", "genesis", profile.genesisFile);
  const genesis = JSON.parse(await fsp.readFile(stockPath, "utf-8"));

  genesis.config.chainId = l2ChainId;

  const deployDir = getDeploymentDir(deploymentId, customDir);
  await fsp.mkdir(deployDir, { recursive: true });
  const outPath = path.join(deployDir, profile.genesisFile);
  await fsp.writeFile(outPath, JSON.stringify(genesis, null, 2), "utf-8");
  return outPath;
}

/**
 * Create a deployment-specific L1 genesis file with a custom chain ID.
 */
async function writeCustomL1Genesis(l1ChainId, deploymentId, customDir) {
  const fsp = require("fs").promises;
  const stockPath = path.join(ETHREX_ROOT, "fixtures", "genesis", "l1.json");
  const genesis = JSON.parse(await fsp.readFile(stockPath, "utf-8"));

  genesis.config.chainId = l1ChainId;

  const deployDir = getDeploymentDir(deploymentId, customDir);
  await fsp.mkdir(deployDir, { recursive: true });
  const outPath = path.join(deployDir, "l1.json");
  await fsp.writeFile(outPath, JSON.stringify(genesis, null, 2), "utf-8");
  return outPath;
}

/**
 * Generate docker-compose.yaml for Thanos (OP Stack) LOCAL deployment.
 *
 * Thanos uses separate services: L1 geth, contract deployer, op-geth (L2),
 * op-node, op-batcher, op-proposer. All pre-built images (no build step).
 *
 * Tools (Blockscout, Bridge UI) are started separately via the existing tools pipeline.
 *
 * @param {Object} opts
 * @param {number} opts.l1Port - Host port for L1 RPC
 * @param {number} opts.l2Port - Host port for L2 RPC (maps to 8545 inside op-geth)
 * @param {string} opts.projectName - Docker Compose project name
 * @param {boolean} [opts.isPublic=false] - Bind to 0.0.0.0 for public access
 * @param {number} [opts.l2ChainId] - L2 chain ID
 * @param {number} [opts.l1ChainId] - L1 chain ID
 * @returns {string} docker-compose.yaml content
 */
function generateThanosComposeFile(opts) {
  const { l1Port, l2Port, projectName, isPublic = false, l2ChainId = 901, l1ChainId = 900 } = opts;
  const bindAddr = isPublic ? '0.0.0.0' : '127.0.0.1';
  const l1RpcUrl = 'http://thanos-l1:8545';

  return `# Auto-generated by Tokamak Platform
# Stack: Thanos (Optimism)
# Project: ${projectName}
# Mode: local (built-in L1 geth)

volumes:
  thanos-shared:
  thanos-l1-data:
  thanos-l2-data:

services:
${thanosL1Service({ projectName, l1Port, l1ChainId, bindAddr })}
${thanosServices({ projectName, l2Port, l2ChainId, l1RpcUrl, bindAddr, privateKey: THANOS_DEVNET_PRIVATE_KEY })}
`;
}

/**
 * Generate docker-compose.yaml for Thanos TESTNET deployment.
 * No L1 container — uses external L1 RPC URL.
 *
 * @param {Object} opts
 * @param {number} opts.l2Port - Host port for L2 RPC
 * @param {string} opts.projectName
 * @param {string} opts.l1RpcUrl - External L1 RPC URL
 * @param {string} opts.deployerPrivateKey
 * @param {number} [opts.l2ChainId]
 * @param {boolean} [opts.isPublic=false]
 * @returns {string}
 */
function generateThanosTestnetComposeFile(opts) {
  const { l2Port, projectName, l1RpcUrl, deployerPrivateKey, l2ChainId = 901, isPublic = false } = opts;
  const bindAddr = isPublic ? '0.0.0.0' : '127.0.0.1';

  return `# Auto-generated by Tokamak Platform
# Stack: Thanos (Optimism)
# Project: ${projectName}
# Mode: testnet (external L1: ${l1RpcUrl})

volumes:
  thanos-shared:
  thanos-l2-data:

services:
${thanosServices({ projectName, l2Port, l2ChainId, l1RpcUrl, bindAddr, privateKey: deployerPrivateKey })}
`;
}

/**
 * Generate docker-compose.yaml for Thanos REMOTE deployment.
 * Pre-built images, deployed via SSH to a remote server.
 *
 * @param {Object} opts
 * @param {number} opts.l1Port
 * @param {number} opts.l2Port
 * @param {string} opts.projectName
 * @param {string} opts.dataDir - Remote data directory
 * @param {number} [opts.l2ChainId]
 * @param {number} [opts.l1ChainId]
 * @param {string} [opts.bindAddress='0.0.0.0']
 * @returns {string}
 */
function generateThanosRemoteComposeFile(opts) {
  const { l1Port, l2Port, projectName, deployerPrivateKey, l2ChainId = 901, l1ChainId = 900, bindAddress = '0.0.0.0' } = opts;
  if (!deployerPrivateKey) throw new Error("Remote Thanos deployment requires an explicit private key");
  const l1RpcUrl = 'http://thanos-l1:8545';

  return `# Auto-generated by Tokamak Platform (REMOTE mode)
# Stack: Thanos (Optimism)
# Project: ${projectName}
# Pre-built images — no build step required

volumes:
  thanos-shared:
  thanos-l1-data:
  thanos-l2-data:

services:
${thanosL1Service({ projectName, l1Port, l1ChainId, bindAddr: bindAddress })}
${thanosServices({ projectName, l2Port, l2ChainId, l1RpcUrl, bindAddr: bindAddress, privateKey: deployerPrivateKey })}
`;
}

module.exports = {
  generateComposeFile,
  generateTestnetComposeFile,
  generateRemoteComposeFile,
  generateRemoteTestnetComposeFile,
  generateThanosComposeFile,
  generateThanosTestnetComposeFile,
  generateThanosRemoteComposeFile,
  generateProgramsToml,
  writeComposeFile,
  writeCustomGenesis,
  writeCustomL1Genesis,
  getDeploymentDir,
  getAppProfile,
  APP_PROFILES,
  THANOS_IMAGES,
};
