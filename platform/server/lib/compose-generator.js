/**
 * Docker Compose file generator.
 *
 * Generates a deployment-specific docker-compose.yaml based on:
 * - Selected app/program (evm-l2, zk-dex, tokamon, etc.)
 * - Port assignments (unique per deployment)
 * - Chain configuration
 *
 * Different apps require different:
 * - Docker images (ethrex:main vs ethrex:sp1)
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
    image: "ethrex:main-l2",
    dockerfile: null, // uses default Dockerfile
    buildFeatures: "--features l2,l2-sql",
    guestPrograms: null, // no guest program build arg needed
    genesisFile: "l2.json",
    proverBackend: "exec",
    sp1Enabled: false,
    registerGuestPrograms: null,
    programsToml: null,
    description: "Default EVM L2 — full EVM compatibility",
  },
  "zk-dex": {
    image: "ethrex:sp1",
    dockerfile: "Dockerfile.sp1",
    buildFeatures: "--features l2,l2-sql,sp1",
    guestPrograms: "evm-l2,zk-dex",
    genesisFile: "l2-zk-dex.json",
    proverBackend: "sp1",
    sp1Enabled: true,
    registerGuestPrograms: "zk-dex",
    programsToml: "programs-zk-dex.toml",
    description: "ZK-DEX — decentralized exchange with SP1 ZK proofs",
  },
  "tokamon": {
    image: "ethrex:main-l2",
    dockerfile: null,
    buildFeatures: "--features l2,l2-sql",
    guestPrograms: null,
    genesisFile: "l2.json",
    proverBackend: "exec",
    sp1Enabled: false,
    registerGuestPrograms: null,
    programsToml: null,
    description: "Tokamon — gaming application circuits",
  },
};

/**
 * Get the app profile for a given program slug.
 * Falls back to evm-l2 for unknown programs.
 */
function getAppProfile(programSlug) {
  return APP_PROFILES[programSlug] || APP_PROFILES["evm-l2"];
}

/**
 * Generate docker-compose.yaml content for a deployment.
 *
 * @param {Object} opts
 * @param {string} opts.programSlug - App identifier (evm-l2, zk-dex, etc.)
 * @param {number} opts.l1Port - Host port for L1 RPC
 * @param {number} opts.l2Port - Host port for L2 RPC
 * @param {number} opts.proofCoordPort - Host port for proof coordinator
 * @param {string} opts.projectName - Docker Compose project name
 * @returns {string} docker-compose.yaml content
 */
function generateComposeFile(opts) {
  const { programSlug, l1Port, l2Port, proofCoordPort = 3900, projectName } = opts;
  const profile = getAppProfile(programSlug);
  const workdir = "/usr/local/bin";

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

  // L1 build (always uses default Dockerfile)
  const l1Build = `    build: ${ETHREX_ROOT}`;

  // Deployer env vars
  let deployerExtraEnv = "";
  if (profile.sp1Enabled) {
    deployerExtraEnv += `      - ETHREX_L2_SP1=true\n`;
  }
  if (profile.registerGuestPrograms) {
    deployerExtraEnv += `      - ETHREX_REGISTER_GUEST_PROGRAMS=${profile.registerGuestPrograms}\n`;
  }
  if (profile.guestPrograms) {
    deployerExtraEnv += `      - GUEST_PROGRAMS=${profile.guestPrograms}\n`;
  }
  deployerExtraEnv += `      - ETHREX_DEPLOYER_GENESIS_L2_PATH=${workdir}/fixtures/genesis/${profile.genesisFile}\n`;

  // Extra deployer volumes for custom genesis
  let deployerExtraVolumes = "";
  if (profile.genesisFile !== "l2.json") {
    deployerExtraVolumes = `      - ${ETHREX_ROOT}/fixtures/genesis/${profile.genesisFile}:${workdir}/fixtures/genesis/${profile.genesisFile}\n`;
  }

  // L2 extra config
  let l2ExtraVolumes = "";
  let l2Genesis = "/genesis/l2.json";
  if (profile.genesisFile !== "l2.json") {
    l2ExtraVolumes = `      - ${ETHREX_ROOT}/fixtures/genesis/${profile.genesisFile}:/genesis/${profile.genesisFile}\n`;
    l2Genesis = `/genesis/${profile.genesisFile}`;
  }
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

  const yaml = `# Auto-generated by Tokamak Platform
# App: ${programSlug} (${profile.description})
# Project: ${projectName}

volumes:
  env:

services:
  tokamak-app-l1:
    container_name: ${projectName}-l1
    image: "ethrex:main"
${l1Build}
    ports:
      - 127.0.0.1:${l1Port}:8545
    environment:
      - ETHREX_LOG_LEVEL
    volumes:
      - ${ETHREX_ROOT}/fixtures/genesis/l1.json:/genesis/l1.json
    command: --network /genesis/l1.json --http.addr 0.0.0.0 --http.port 8545 --dev

  tokamak-app-deployer:
    container_name: ${projectName}-deployer
    image: "${profile.image}"
    restart: on-failure:10
${buildSection}
    volumes:
      - ${ETHREX_ROOT}/crates/l2/contracts:${workdir}/contracts
      - env:/env/
      - ${ETHREX_ROOT}/fixtures/genesis/l1.json:${workdir}/fixtures/genesis/l1.json
      - ${ETHREX_ROOT}/fixtures/genesis/l2.json:${workdir}/fixtures/genesis/l2.json
      - ${ETHREX_ROOT}/fixtures/keys/private_keys_l1.txt:${workdir}/fixtures/keys/private_keys_l1.txt
      - ${ETHREX_ROOT}/crates/guest-program/bin/sp1/out/:${workdir}/sp1_out
      - ${ETHREX_ROOT}/crates/guest-program/bin/sp1/out/riscv32im-succinct-zkvm-vk-u32:${workdir}/riscv32im-succinct-zkvm-vk-u32
      - ${ETHREX_ROOT}/crates/guest-program/bin/sp1/out/riscv32im-succinct-zkvm-vk-bn254:${workdir}/riscv32im-succinct-zkvm-vk-bn254
      - ${ETHREX_ROOT}/crates/guest-program/bin/risc0/out/riscv32im-risc0-vk:${workdir}/riscv32im-risc0-vk
${deployerExtraVolumes}    environment:
      - ETHREX_ETH_RPC_URL=http://tokamak-app-l1:8545
      - ETHREX_DEPLOYER_L1_PRIVATE_KEY=0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924
      - ETHREX_DEPLOYER_ENV_FILE_PATH=/env/.env
      - ETHREX_DEPLOYER_GENESIS_L1_PATH=${workdir}/fixtures/genesis/l1.json
      - ETHREX_DEPLOYER_PRIVATE_KEYS_FILE_PATH=${workdir}/fixtures/keys/private_keys_l1.txt
      - ETHREX_DEPLOYER_DEPLOY_RICH=true
      - ETHREX_L2_RISC0=false
      - ETHREX_L2_SP1=\${ETHREX_L2_SP1:-false}
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
    image: "${profile.image}"
${buildSection}
    ports:
      - 127.0.0.1:${l2Port}:1729
      - 127.0.0.1:${proofCoordPort}:3900
    environment:
      - ETHREX_ETH_RPC_URL=http://tokamak-app-l1:8545
      - ETHREX_L2_VALIDIUM=false
      - ETHREX_BLOCK_PRODUCER_BLOCK_TIME=\${ETHREX_BLOCK_PRODUCER_BLOCK_TIME:-5000}
      - ETHREX_WATCHER_BLOCK_DELAY=0
      - ETHREX_BASED=false
      - ETHREX_COMMITTER_COMMIT_TIME=\${ETHREX_COMMITTER_COMMIT_TIME:-60000}
      - ETHREX_WATCHER_WATCH_INTERVAL=\${ETHREX_WATCHER_WATCH_INTERVAL:-12000}
      - ETHREX_GUEST_PROGRAM_ID=${programSlug}
      - ETHREX_LOG_LEVEL
    volumes:
      - ${ETHREX_ROOT}/fixtures/genesis/${profile.genesisFile}:/genesis/${profile.genesisFile}
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
      --committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924
      --proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d
      --no-monitor
    depends_on:
      tokamak-app-deployer:
        condition: service_completed_successfully

  tokamak-app-prover:
    container_name: ${projectName}-prover
    image: "${profile.image}"
${buildSection}
${proverExtraEnv ? `    environment:\n${proverExtraEnv}\n` : ""}${proverExtraVolumes ? `    volumes:\n${proverExtraVolumes}\n` : ""}    command: >
      ${proverCommand}
    depends_on:
      - tokamak-app-l2
`;

  return yaml;
}

// Pre-built image registry (Docker Hub / GHCR)
const IMAGE_REGISTRY = process.env.ETHREX_IMAGE_REGISTRY || "ghcr.io/tokamak-network/ethrex";

// Remote image tags per app profile
const REMOTE_IMAGES = {
  "ethrex:main": `${IMAGE_REGISTRY}:main`,
  "ethrex:main-l2": `${IMAGE_REGISTRY}:main-l2`,
  "ethrex:sp1": `${IMAGE_REGISTRY}:sp1`,
};

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
  const { programSlug, l1Port, l2Port, proofCoordPort = 3900, projectName, dataDir } = opts;
  const profile = getAppProfile(programSlug);
  const workdir = "/usr/local/bin";

  const l1Image = REMOTE_IMAGES["ethrex:main"];
  const l2Image = REMOTE_IMAGES[profile.image] || REMOTE_IMAGES["ethrex:main-l2"];

  const l2Genesis = profile.genesisFile !== "l2.json"
    ? `/genesis/${profile.genesisFile}`
    : "/genesis/l2.json";

  // Deployer extra env
  let deployerExtraEnv = "";
  if (profile.sp1Enabled) deployerExtraEnv += `      - ETHREX_L2_SP1=true\n`;
  if (profile.registerGuestPrograms) deployerExtraEnv += `      - ETHREX_REGISTER_GUEST_PROGRAMS=${profile.registerGuestPrograms}\n`;
  if (profile.guestPrograms) deployerExtraEnv += `      - GUEST_PROGRAMS=${profile.guestPrograms}\n`;
  deployerExtraEnv += `      - ETHREX_DEPLOYER_GENESIS_L2_PATH=${workdir}/fixtures/genesis/${profile.genesisFile}\n`;

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

  const yaml = `# Auto-generated by Tokamak Platform (REMOTE mode)
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
      - 0.0.0.0:${l1Port}:8545
    environment:
      - ETHREX_LOG_LEVEL
    command: --network /genesis/l1.json --http.addr 0.0.0.0 --http.port 8545 --dev

  tokamak-app-deployer:
    container_name: ${projectName}-deployer
    image: "${l2Image}"
    restart: on-failure:10
    volumes:
      - env:/env/
    environment:
      - ETHREX_ETH_RPC_URL=http://tokamak-app-l1:8545
      - ETHREX_DEPLOYER_L1_PRIVATE_KEY=0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924
      - ETHREX_DEPLOYER_ENV_FILE_PATH=/env/.env
      - ETHREX_DEPLOYER_GENESIS_L1_PATH=${workdir}/fixtures/genesis/l1.json
      - ETHREX_DEPLOYER_PRIVATE_KEYS_FILE_PATH=${workdir}/fixtures/keys/private_keys_l1.txt
      - ETHREX_DEPLOYER_DEPLOY_RICH=true
      - ETHREX_L2_RISC0=false
      - ETHREX_L2_SP1=\${ETHREX_L2_SP1:-false}
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
    ports:
      - 0.0.0.0:${l2Port}:1729
      - 0.0.0.0:${proofCoordPort}:3900
    environment:
      - ETHREX_ETH_RPC_URL=http://tokamak-app-l1:8545
      - ETHREX_L2_VALIDIUM=false
      - ETHREX_BLOCK_PRODUCER_BLOCK_TIME=5000
      - ETHREX_WATCHER_BLOCK_DELAY=0
      - ETHREX_BASED=false
      - ETHREX_COMMITTER_COMMIT_TIME=60000
      - ETHREX_WATCHER_WATCH_INTERVAL=12000
      - ETHREX_GUEST_PROGRAM_ID=${programSlug}
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
      --committer.l1-private-key 0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924
      --proof-coordinator.l1-private-key 0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d
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

module.exports = {
  generateComposeFile,
  generateRemoteComposeFile,
  generateProgramsToml,
  writeComposeFile,
  getDeploymentDir,
  getAppProfile,
  APP_PROFILES,
  IMAGE_REGISTRY,
};
