/**
 * AI Deploy Prompt Generator
 *
 * Generates complete, executable deployment prompts that AI assistants
 * (Claude Code, ChatGPT, etc.) can run step-by-step to deploy a
 * Tokamak L2 appchain on Local Docker, Cloud VM, or Testnet/Mainnet.
 *
 * Deployment targets:
 * - Local Docker: Build from source on user's machine
 * - Cloud (GCP / AWS / Vultr): Pre-built images on remote VM
 * - Testnet/Mainnet: External L1 (Sepolia, Holesky, Mainnet) with any target
 */

const {
  generateComposeFile,
  generateRemoteComposeFile,
  getAppProfile,
  APP_PROFILES,
} = require("./compose-generator");
const path = require("path");

// ---------------------------------------------------------------------------
// Cloud presets
// ---------------------------------------------------------------------------

const CLOUD_PRESETS = {
  local: {
    label: "Local (Docker)",
    regions: [],
    vmTypes: [],
  },
  gcp: {
    label: "Google Cloud (GCP)",
    regions: [
      { id: "asia-northeast3", label: "Seoul (asia-northeast3)" },
      { id: "asia-northeast1", label: "Tokyo (asia-northeast1)" },
      { id: "us-central1", label: "Iowa (us-central1)" },
      { id: "us-east1", label: "South Carolina (us-east1)" },
      { id: "europe-west1", label: "Belgium (europe-west1)" },
    ],
    vmTypes: [
      { id: "e2-standard-4", label: "e2-standard-4 (4 vCPU, 16 GB)", recommended: true },
      { id: "e2-standard-8", label: "e2-standard-8 (8 vCPU, 32 GB)" },
      { id: "n1-standard-4", label: "n1-standard-4 (4 vCPU, 15 GB, GPU-capable)" },
    ],
  },
  aws: {
    label: "Amazon Web Services (AWS)",
    regions: [
      { id: "ap-northeast-2", label: "Seoul (ap-northeast-2)" },
      { id: "ap-northeast-1", label: "Tokyo (ap-northeast-1)" },
      { id: "us-east-1", label: "N. Virginia (us-east-1)" },
      { id: "us-west-2", label: "Oregon (us-west-2)" },
      { id: "eu-west-1", label: "Ireland (eu-west-1)" },
    ],
    vmTypes: [
      { id: "t3.xlarge", label: "t3.xlarge (4 vCPU, 16 GB)", recommended: true },
      { id: "t3.2xlarge", label: "t3.2xlarge (8 vCPU, 32 GB)" },
      { id: "g4dn.xlarge", label: "g4dn.xlarge (4 vCPU, 16 GB + T4 GPU)" },
    ],
  },
  vultr: {
    label: "Vultr",
    regions: [
      { id: "icn", label: "Seoul (icn)" },
      { id: "nrt", label: "Tokyo (nrt)" },
      { id: "sgp", label: "Singapore (sgp)" },
      { id: "ewr", label: "New Jersey (ewr)" },
      { id: "lax", label: "Los Angeles (lax)" },
    ],
    vmTypes: [
      { id: "vc2-6c-16gb", label: "vc2-6c-16gb (6 vCPU, 16 GB, 320 GB SSD)", recommended: true },
      { id: "vc2-8c-32gb", label: "vc2-8c-32gb (8 vCPU, 32 GB, 640 GB SSD)" },
      { id: "vc2-4c-8gb", label: "vc2-4c-8gb (4 vCPU, 8 GB, 160 GB SSD)" },
    ],
  },
};

// Default ports for remote deployments
const DEFAULT_PORTS = {
  l1: 8545,
  l2: 1729,
  proofCoord: 3900,
  l2Explorer: 8082,
  l1Explorer: 8083,
  dashboard: 3000,
  dbPort: 7432,
  metricsPort: 3702,
};

// ETHREX_ROOT relative to this file (lib/ → local-server/ → desktop-app/ → crates/ → repo root)
const ETHREX_ROOT = path.resolve(__dirname, "../../../..");

// ---------------------------------------------------------------------------
// Main generator — routes to local or cloud prompt
// ---------------------------------------------------------------------------

/**
 * Generate a complete AI-executable deployment prompt.
 *
 * @param {Object} opts
 * @param {Object} opts.deployment  - DB deployment row
 * @param {string} opts.cloud       - 'local' | 'gcp' | 'aws' | 'vultr'
 * @param {string} opts.region      - Cloud region ID (ignored for local)
 * @param {string} opts.vmType      - VM instance type (ignored for local)
 * @param {string} opts.l1Mode      - 'local' (built-in L1) or 'testnet' (external L1)
 * @param {string} [opts.l1RpcUrl]  - External L1 RPC URL (when l1Mode === 'testnet')
 * @param {number} [opts.l1ChainId] - L1 chain ID
 * @param {string} [opts.l1Network] - L1 network name (sepolia, holesky, mainnet)
 * @param {boolean} [opts.includeProver] - Whether to include SP1 prover (default: true)
 * @param {Object} [opts.walletConfig] - Wallet configuration for testnet/mainnet
 * @returns {string} Markdown prompt
 */
function generateAIDeployPrompt(opts) {
  const {
    deployment, cloud, region, vmType,
    l1Mode = "local", l1RpcUrl, l1ChainId, l1Network,
    includeProver = true, walletConfig,
  } = opts;

  if (cloud === "local") {
    return generateLocalDeployPrompt(opts);
  }
  return generateCloudDeployPrompt(opts);
}

// ---------------------------------------------------------------------------
// LOCAL DOCKER deployment prompt
// ---------------------------------------------------------------------------

function generateLocalDeployPrompt(opts) {
  const {
    deployment, l1Mode = "local", l1RpcUrl, l1ChainId, l1Network,
    includeProver = true, walletConfig, ports,
  } = opts;

  const config = deployment.config ? JSON.parse(deployment.config) : {};
  const programSlug = deployment.program_slug || "evm-l2";
  const profile = getAppProfile(programSlug);
  const l2ChainId = deployment.chain_id || 65536999;
  const projectName = `tokamak-${deployment.id.slice(0, 8)}`;
  const isTestnet = l1Mode === "testnet";
  const deployDir = config.deployDir || `~/.tokamak/deployments/${deployment.id.slice(0, 8)}`;

  // Use dynamically allocated ports (checked for real TCP availability)
  // Falls back to DEFAULT_PORTS if not provided
  const P = {
    l1: ports?.l1Port || DEFAULT_PORTS.l1,
    l2: ports?.l2Port || DEFAULT_PORTS.l2,
    proofCoord: ports?.proofCoordPort || DEFAULT_PORTS.proofCoord,
    l2Explorer: ports?.toolsL2ExplorerPort || DEFAULT_PORTS.l2Explorer,
    l1Explorer: ports?.toolsL1ExplorerPort || DEFAULT_PORTS.l1Explorer,
    dashboard: ports?.toolsBridgeUIPort || DEFAULT_PORTS.dashboard,
    dbPort: ports?.toolsDbPort || DEFAULT_PORTS.dbPort,
    metricsPort: ports?.toolsMetricsPort || DEFAULT_PORTS.metricsPort,
  };

  // Always generate the local-L1 compose as template.
  // For testnet, we provide modification instructions instead of using
  // generateTestnetComposeFile (which requires actual private keys for address derivation).
  const composeContent = generateComposeFile({
    programSlug,
    l1Port: P.l1,
    l2Port: P.l2,
    proofCoordPort: P.proofCoord,
    metricsPort: P.metricsPort,
    projectName,
    l2ChainId,
  });

  const sections = [];

  // Header
  sections.push(`# Tokamak L2 Appchain — Local Docker Deployment

> **AI에게**: 사용자가 확인하면 이 문서의 단계를 직접 실행하세요 (안내가 아니라 직접 배포).
> 사용자의 로컬 머신에서 Docker Compose로 L2 앱체인을 빌드하고 실행합니다.
> 각 단계를 직접 실행하고, 진행 상황을 보고하세요 (예: "[Step 2/7] Docker 이미지 빌드 중... ✅ 완료").
> 결과를 확인한 후 다음 단계로 자동 진행하세요.

## Deployment Summary

| Setting | Value |
|---------|-------|
| **App** | ${programSlug} (${profile.description}) |
| **L2 Name** | ${deployment.name} |
| **L2 Chain ID** | ${l2ChainId} |
| **Target** | Local Docker (이 머신) |
| **L1 Mode** | ${isTestnet ? `Testnet (${l1Network || "external"})` : "Built-in (Docker L1)"} |
| **L1 Chain ID** | ${l1ChainId || (isTestnet ? "from network" : 9)} |
${isTestnet ? `| **L1 RPC** | \`${l1RpcUrl || "$L1_RPC_URL"}\` |\n` : ""}| **Prover** | ${includeProver ? "SP1 Prover 포함" : "Prover 미포함"} |
| **Build** | 소스에서 Docker 이미지 빌드 |
| **L2 RPC Port** | ${P.l2} |
${!isTestnet ? `| **L1 RPC Port** | ${P.l1} |\n` : ""}| **Explorer Port** | ${P.l2Explorer} |
| **Dashboard Port** | ${P.dashboard} |

> 포트는 기존 프로세스와 충돌하지 않도록 자동으로 할당되었습니다.`);

  // Prerequisites
  sections.push(`## Step 0: Prerequisites

\`\`\`bash
# Docker Desktop 설치 확인
docker --version
docker compose version

# Docker가 실행 중인지 확인
docker info > /dev/null 2>&1 && echo "✅ Docker is running" || echo "❌ Docker is not running"

# 충분한 디스크 공간 확인 (최소 20GB 필요)
df -h .
\`\`\`

> Docker Desktop이 설치되어 있지 않으면 https://www.docker.com/products/docker-desktop/ 에서 다운로드하세요.
> Docker Desktop → Settings → Resources에서 메모리를 최소 8GB (Prover 포함 시 16GB) 이상으로 설정하세요.`);

  // Source code check
  sections.push(`## Step 1: Verify Source Code

로컬 빌드를 위해 ethrex 소스 코드가 필요합니다.

\`\`\`bash
# ethrex 저장소가 있는지 확인
ETHREX_ROOT="${ETHREX_ROOT}"
if [ -d "$ETHREX_ROOT/crates/l2" ]; then
  echo "✅ ethrex source found at $ETHREX_ROOT"
else
  echo "❌ ethrex source not found — cloning..."
  git clone https://github.com/tokamak-network/ethrex.git "$ETHREX_ROOT"
fi
\`\`\``);

  // Write compose file
  sections.push(`## Step 2: Write Docker Compose File

\`\`\`bash
mkdir -p ${deployDir}
cd ${deployDir}

cat > docker-compose.yaml << 'COMPOSE_EOF'
${composeContent.trimEnd()}
COMPOSE_EOF
\`\`\`

> Docker Compose 프로젝트 이름: \`${projectName}\`
> 이 파일은 소스에서 이미지를 빌드합니다. 최초 빌드에 10-20분이 소요될 수 있습니다.`);

  // Testnet env configuration
  if (isTestnet) {
    sections.push(localTestnetEnvSection({ l1RpcUrl, l1ChainId, l1Network, deployDir, walletConfig }));
  }

  // Build images
  sections.push(`## Step ${isTestnet ? "3.5" : "3"}: Build Docker Images

\`\`\`bash
cd ${deployDir}

# 이미지 빌드 (첫 빌드 시 10-20분 소요)
docker compose -p ${projectName} build

# 빌드 완료 확인
docker images | grep tokamak-appchain
\`\`\`

> 빌드 중 에러가 발생하면:
> - \`docker system prune -f\`로 캐시 정리 후 재시도
> - Docker Desktop 메모리를 8GB 이상으로 설정했는지 확인
> - Rust 빌드에 RAM이 많이 필요합니다 — 다른 무거운 프로세스를 종료하세요`);

  // Deploy
  sections.push(`## Step ${isTestnet ? "4" : "4"}: Start Deployment

\`\`\`bash
cd ${deployDir}

# 전체 서비스 시작
docker compose -p ${projectName} up -d

# Deployer 로그 확인 (컨트랙트 배포 완료까지 대기)
docker logs -f ${projectName}-deployer
\`\`\`

Deployer 컨테이너 동작 순서:
1. ${isTestnet ? "외부 L1 RPC에 연결" : "Built-in L1 노드가 준비될 때까지 대기"}
2. L1 컨트랙트 컴파일 및 배포 (CommonBridge, OnChainProposer, Timelock 등)
3. 배포된 주소를 공유 볼륨(\`/env/.env\`)에 기록
4. 성공 시 exit code 0으로 종료

Deployer가 종료되면 L2 노드와 Prover가 자동으로 시작됩니다.

\`\`\`bash
# Deployer 종료 확인 (exit code 0이어야 함)
docker ps -a --filter "name=${projectName}-deployer" --format "{{.Status}}"

# L2 노드 로그 확인
docker logs --tail 20 ${projectName}-l2
\`\`\``);

  // Verify
  const chainIdHex = "0x" + l2ChainId.toString(16).toUpperCase();
  sections.push(`## Step 5: Verify Deployment

\`\`\`bash
# L2 RPC 확인
curl -s http://localhost:${P.l2} \\
  -X POST -H "Content-Type: application/json" \\
  -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}'
# Expected: {"result":"${chainIdHex}"}

# 블록 번호 확인 (증가하는지)
curl -s http://localhost:${P.l2} \\
  -X POST -H "Content-Type: application/json" \\
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'

${isTestnet ? "" : `# L1 노드 확인
curl -s http://localhost:${P.l1} \\
  -X POST -H "Content-Type: application/json" \\
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
`}
# 전체 컨테이너 상태 확인
docker ps --format "table {{.Names}}\\t{{.Status}}\\t{{.Ports}}"
\`\`\`

모든 컨테이너가 "Up" 상태이고, Deployer가 "Exited (0)"이면 정상입니다.`);

  // Tools
  sections.push(localToolsSection({ deployDir, projectName, l2ChainId, isTestnet, l1ChainId, l1Network, l1RpcUrl, programSlug, P }));

  // Summary
  sections.push(`## Step 7: Access Your L2

배포 완료 후 접속 가능한 서비스:

| Service | URL |
|---------|-----|
| **L2 RPC** | \`http://localhost:${P.l2}\` |
| **L2 Explorer** | \`http://localhost:${P.l2Explorer}\` |
| **Dashboard** | \`http://localhost:${P.dashboard}\` |
${isTestnet ? "" : `| **L1 RPC** | \`http://localhost:${P.l1}\` |
| **L1 Explorer** | \`http://localhost:${P.l1Explorer}\` |
`}
### MetaMask 설정

| Setting | Value |
|---------|-------|
| **Network Name** | ${deployment.name || "Tokamak Appchain"} |
| **RPC URL** | \`http://localhost:${P.l2}\` |
| **Chain ID** | ${l2ChainId} |
| **Currency Symbol** | ETH |`);

  // Troubleshooting
  sections.push(localTroubleshootingSection({ projectName, deployDir }));

  return sections.join("\n\n");
}

function localTestnetEnvSection({ l1RpcUrl, l1ChainId, l1Network, deployDir, walletConfig }) {
  const hasWallet = walletConfig && walletConfig.deployerKey;
  return `## Step 3: Configure Testnet Environment

> **IMPORTANT**: 비밀키를 절대 소스코드에 커밋하지 마세요.
${hasWallet ? `
사용자가 macOS Keychain에 등록한 키를 사용합니다.
먼저 키를 환경변수로 추출해야 합니다.

\`\`\`bash
# Keychain에서 키 추출
DEPLOYER_PRIVATE_KEY=$(security find-generic-password -a "${walletConfig.deployerKey}" -s "tokamak-appchain" -w 2>/dev/null)
echo "Deployer key loaded: ${walletConfig.deployerAddr || '(address)'}"

${walletConfig.committerKey ? `COMMITTER_PK=$(security find-generic-password -a "${walletConfig.committerKey}" -s "tokamak-appchain" -w 2>/dev/null)` : "COMMITTER_PK=$DEPLOYER_PRIVATE_KEY"}
${walletConfig.proofCoordinatorKey ? `PROOF_COORDINATOR_PK=$(security find-generic-password -a "${walletConfig.proofCoordinatorKey}" -s "tokamak-appchain" -w 2>/dev/null)` : "PROOF_COORDINATOR_PK=$DEPLOYER_PRIVATE_KEY"}
${walletConfig.bridgeOwnerKey ? `BRIDGE_OWNER_PK=$(security find-generic-password -a "${walletConfig.bridgeOwnerKey}" -s "tokamak-appchain" -w 2>/dev/null)` : "BRIDGE_OWNER_PK=$DEPLOYER_PRIVATE_KEY"}
\`\`\`
` : `
\`\`\`bash
# 비밀키 설정 (실제 키로 교체하세요)
DEPLOYER_PRIVATE_KEY=0xYOUR_DEPLOYER_PRIVATE_KEY
COMMITTER_PK=$DEPLOYER_PRIVATE_KEY
PROOF_COORDINATOR_PK=$DEPLOYER_PRIVATE_KEY
BRIDGE_OWNER_PK=$DEPLOYER_PRIVATE_KEY
\`\`\`
`}
docker-compose.yaml에서 환경변수를 업데이트합니다:

\`\`\`bash
cd ${deployDir}

# 테스트넷 .env 파일 생성
cat > .env << ENV_EOF
L1_RPC_URL=${l1RpcUrl || "https://eth-sepolia.g.alchemy.com/v2/YOUR_API_KEY"}
L1_CHAIN_ID=${l1ChainId || 11155111}
L1_NETWORK=${l1Network || "sepolia"}
DEPLOYER_PRIVATE_KEY=$DEPLOYER_PRIVATE_KEY
COMMITTER_PK=$COMMITTER_PK
PROOF_COORDINATOR_PK=$PROOF_COORDINATOR_PK
BRIDGE_OWNER_PK=$BRIDGE_OWNER_PK
ENV_EOF

chmod 600 .env
\`\`\`

> Deployer 계정에 충분한 ${l1Network || "testnet"} ETH가 있는지 확인하세요.
> Sepolia ETH 파우셋: https://sepoliafaucet.com/ 또는 https://www.alchemy.com/faucets/ethereum-sepolia

**IMPORTANT**: docker-compose.yaml에서 다음 수정이 필요합니다:
1. \`tokamak-app-l1\` 서비스 전체를 제거 (외부 L1 사용)
2. Deployer의 \`ETHREX_ETH_RPC_URL\`을 \`.env\` 파일의 \`$L1_RPC_URL\`로 변경
3. Deployer의 \`ETHREX_DEPLOYER_L1_PRIVATE_KEY\`를 \`$DEPLOYER_PRIVATE_KEY\`로 변경
4. \`ETHREX_DEPLOYER_DEPLOY_RICH=false\`로 변경 (테스트넷은 실제 ETH)
5. L2의 \`ETHREX_ETH_RPC_URL\`도 동일하게 외부 L1 RPC로 변경
6. Deployer의 \`depends_on\`에서 \`tokamak-app-l1\` 제거`;
}

function localToolsSection({ deployDir, projectName, l2ChainId, isTestnet, l1ChainId, l1Network, l1RpcUrl, programSlug, P }) {
  const toolsProjectName = `${projectName}-tools`;
  return `## Step 6: Deploy Tools (Explorer + Dashboard)

Tools 스택:
- **L2 Blockscout Explorer** (port ${P.l2Explorer})
- ${isTestnet ? `**L1 Explorer**: ${l1Network || "external"} Etherscan 사용` : `**L1 Blockscout Explorer** (port ${P.l1Explorer})`}
- **Bridge Dashboard** (port ${P.dashboard})

\`\`\`bash
# Deployer가 기록한 컨트랙트 주소 추출
docker run --rm \\
  -v ${projectName}_env:/env \\
  -v /tmp/ethrex-${projectName}:/out \\
  alpine cp /env/.env /out/.env

# Tools 환경 파일 생성
cp /tmp/ethrex-${projectName}/.env ${ETHREX_ROOT}/crates/l2/.deployed-${projectName}.env

# Tools 환경변수 설정
export TOOLS_ENV_FILE=${ETHREX_ROOT}/crates/l2/.deployed-${projectName}.env
export TOOLS_L2_RPC_PORT=${P.l2}
export TOOLS_L1_RPC_PORT=${P.l1}
export TOOLS_L2_EXPLORER_PORT=${P.l2Explorer}
export TOOLS_L1_EXPLORER_PORT=${P.l1Explorer}
export TOOLS_BRIDGE_UI_PORT=${P.dashboard}
export TOOLS_DB_PORT=${P.dbPort}
export TOOLS_METRICS_PORT=${P.metricsPort}
export L2_CHAIN_ID=${l2ChainId}
${isTestnet ? `export L1_CHAIN_ID=${l1ChainId || 11155111}
export IS_EXTERNAL_L1=true
export L1_RPC_URL=${l1RpcUrl || "$L1_RPC_URL"}
export L1_NETWORK_NAME=${l1Network || "sepolia"}` : `export L1_CHAIN_ID=${l1ChainId || 9}`}

# Tools 이미지 빌드 및 시작
cd ${ETHREX_ROOT}/crates/l2
docker compose -f docker-compose-zk-dex-tools.yaml \\
  -p ${toolsProjectName} \\
  ${isTestnet ? "--profile external-l1 " : ""}build

docker compose -f docker-compose-zk-dex-tools.yaml \\
  -p ${toolsProjectName} \\
  ${isTestnet ? "--profile external-l1 " : ""}up -d
\`\`\`

30초 후 Blockscout 초기화가 완료되면 확인:

\`\`\`bash
# L2 Explorer 확인
curl -s http://localhost:${P.l2Explorer}/api/v2/stats | head -c 200

# Dashboard 확인
curl -s http://localhost:${P.dashboard}/ | head -c 200
\`\`\``;
}

function localTroubleshootingSection({ projectName, deployDir }) {
  return `## Troubleshooting

\`\`\`bash
# 전체 컨테이너 로그
docker compose -p ${projectName} logs --tail=50

# 특정 서비스 로그
docker logs ${projectName}-deployer  # 컨트랙트 배포
docker logs ${projectName}-l2        # L2 노드
docker logs ${projectName}-prover    # Prover

# 서비스 재시작
docker compose -p ${projectName} restart tokamak-app-l2

# 전체 재시작
cd ${deployDir}
docker compose -p ${projectName} down
docker compose -p ${projectName} up -d

# Docker 리소스 확인
docker system df
docker stats --no-stream

# 이미지 재빌드 (캐시 없이)
docker compose -p ${projectName} build --no-cache
\`\`\`

### Common Issues

1. **빌드 실패 (메모리 부족)**: Docker Desktop → Settings → Resources → Memory를 8GB+ 로 설정
2. **Deployer 실패**: L1 연결 확인. 테스트넷의 경우 Deployer 계정 잔액 확인
3. **L2 블록 생성 안됨**: Deployer가 성공적으로 종료되었는지 확인 (\`docker logs ${projectName}-deployer\`)
4. **Explorer 데이터 없음**: Blockscout 인덱서 초기화에 1-2분 소요
5. **포트 충돌**: docker-compose.yaml에서 포트 매핑 변경
6. **빌드 너무 오래 걸림**: 이전 빌드 캐시가 사용되므로 두번째 빌드부터 빨라집니다`;
}

// ---------------------------------------------------------------------------
// CLOUD deployment prompt (GCP / AWS / Vultr)
// ---------------------------------------------------------------------------

function generateCloudDeployPrompt(opts) {
  const {
    deployment, cloud, region, vmType,
    l1Mode = "local", l1RpcUrl, l1ChainId, l1Network,
    includeProver = true, walletConfig,
    storageGB = 30, keyPairName = "",
  } = opts;

  const config = deployment.config ? JSON.parse(deployment.config) : {};
  const programSlug = deployment.program_slug || "evm-l2";
  const profile = getAppProfile(programSlug);
  const l2ChainId = deployment.chain_id || 65536999;
  const shortId = deployment.id.slice(0, 8);
  const safeName = (deployment.name || "l2").replace(/[^a-zA-Z0-9-]/g, "-").toLowerCase().slice(0, 30);
  const projectName = `tokamak-${safeName}-${shortId}`;
  const isTestnet = l1Mode === "testnet";
  const vmName = `tokamak-${safeName}-${shortId}`;
  const sgName = `tokamak-sg-${shortId}`;
  const dataDir = `/opt/tokamak/${shortId}`;

  // Always generate the local-L1 remote compose as template.
  // For testnet, we provide modification instructions instead of using
  // generateRemoteTestnetComposeFile (which requires actual private keys for address derivation).
  const composeContent = generateRemoteComposeFile({
    programSlug,
    l1Port: DEFAULT_PORTS.l1,
    l2Port: DEFAULT_PORTS.l2,
    proofCoordPort: DEFAULT_PORTS.proofCoord,
    projectName,
    dataDir,
    l2ChainId,
  });

  const sections = [];

  sections.push(headerSection({ deployment, programSlug, profile, cloud, region, vmType, l2ChainId, isTestnet, l1RpcUrl, l1Network, l1ChainId, storageGB, keyPairName, includeProver }));
  sections.push(prerequisitesSection(cloud, keyPairName));
  sections.push(vmCreationSection({ cloud, region, vmType, vmName, storageGB, keyPairName, sgName }));
  sections.push(dockerInstallSection());
  sections.push(composeFileSection({ composeContent, dataDir, projectName }));

  if (isTestnet) {
    sections.push(testnetEnvSection({ cloud, l1RpcUrl, l1ChainId, l1Network, dataDir, walletConfig }));
  }

  sections.push(deploySection({ projectName, dataDir, isTestnet }));
  sections.push(verifySection({ l2ChainId, isTestnet }));
  sections.push(toolsSection({ dataDir, l2ChainId, isTestnet, l1ChainId, l1Network, l1RpcUrl, programSlug, projectName }));
  sections.push(firewallSection({ cloud, vmName, isTestnet, sgName, region }));
  sections.push(summarySection({ isTestnet, deployment }));
  sections.push(troubleshootingSection({ projectName, dataDir, sgName, region, vmName }));

  return sections.join("\n\n");
}

// ---------------------------------------------------------------------------
// Cloud section builders
// ---------------------------------------------------------------------------

function headerSection({ deployment, programSlug, profile, cloud, region, vmType, l2ChainId, isTestnet, l1RpcUrl, l1Network, l1ChainId, storageGB = 30, keyPairName = "", includeProver = true }) {
  const cloudLabel = CLOUD_PRESETS[cloud]?.label || cloud;
  const components = ["L2 Node", "Tools (Explorer, Dashboard, Bridge)"];
  if (!isTestnet) components.unshift("L1 Node");
  if (includeProver) components.push("SP1 Prover");

  return `# Tokamak L2 Appchain — Cloud Deployment

> **AI에게**: 사용자가 확인하면 이 프롬프트의 모든 명령을 직접 실행하세요 (안내가 아니라 직접 배포).
> 각 단계를 직접 실행하고, 진행 상황을 보고하세요 (예: "[Step 3/8] Compose 파일 작성 중... ✅ 완료").
> 결과를 확인한 후 다음 단계로 자동 진행하세요.
> ⚠️ SP1 Prover: 현재 Docker 이미지(ghcr.io)는 exec backend만 포함되어 있어 ZK 증명 생성이 안 될 수 있습니다. 이미지가 업데이트되면 자동으로 SP1 증명이 동작합니다. 체인 운영(블록 생성, 트랜잭션)은 정상 동작합니다.

## Deployment Summary

| Setting | Value |
|---------|-------|
| **App** | ${programSlug} (${profile.description}) |
| **L2 Name** | ${deployment.name} |
| **L2 Chain ID** | ${l2ChainId} |
| **Cloud** | ${cloudLabel} |
| **Region** | ${region} |
| **Instance** | ${vmType} |
| **Storage** | ${storageGB}GB gp3 |
${keyPairName ? `| **SSH Key Pair** | ${keyPairName} |\n` : ""}| **Components** | ${components.join(" + ")} |
| **L1 Mode** | ${isTestnet ? `Testnet (${l1Network || "external"})` : "Built-in (Docker L1)"} |
| **L1 Chain ID** | ${l1ChainId || (isTestnet ? "from network" : 9)} |
${isTestnet ? `| **L1 RPC** | \`$L1_RPC_URL\` (set in .env) |\n` : ""}| **Docker Images** | \`ghcr.io/tokamak-network/tokamak-appchain:{l1,l2,sp1}\` |`;
}

function prerequisitesSection(cloud, keyPairName = "") {
  if (cloud === "gcp") {
    return `## Step 0: Prerequisites (gcloud CLI 설치 + 로그인)

아래 명령어를 순서대로 실행하세요. 이미 설치되어 있으면 건너뛰세요.

\`\`\`bash
# 1. gcloud CLI 설치 확인 (없으면 설치)
which gcloud || (curl -fsSL https://sdk.cloud.google.com | bash && exec -l $SHELL)

# 2. 로그인 (브라우저가 열림)
gcloud auth login

# 3. 프로젝트 설정 (프로젝트가 없으면 gcloud projects create로 생성)
gcloud config set project YOUR_PROJECT_ID

# 4. Compute Engine API 활성화 (VM 생성에 필요)
gcloud services enable compute.googleapis.com

# 5. 확인
gcloud config get-value project
gcloud config get-value account
\`\`\`

> 사용자에게 프로젝트 ID를 확인하세요. 빌링이 활성화된 프로젝트여야 합니다.`;
  }

  if (cloud === "vultr") {
    return `## Step 0: Prerequisites (Vultr 설정)

### 방법 1: Vultr CLI 사용 (선택사항)

\`\`\`bash
# vultr CLI 설치 확인
which vultr || which vultr-cli

# 설치 (macOS)
brew install vultr/vultr-cli/vultr-cli

# API 키 설정
export VULTR_API_KEY="YOUR_VULTR_API_KEY"
vultr account

# SSH 키 등록 (없으면)
vultr ssh-key create --name "tokamak" --key "$(cat ~/.ssh/id_rsa.pub)"
\`\`\`

### 방법 2: Vultr 웹 콘솔 (권장)

1. https://my.vultr.com/ 접속
2. Products → Deploy New Server
3. Choose Server: Cloud Compute (Regular Performance)
4. API 키: https://my.vultr.com/settings/#settingsapi 에서 생성

> Vultr API 키는 이미 매니저 앱에서 설정되어 있을 수 있습니다.`;
  }

  // AWS
  const sshKeyName = keyPairName || "tokamak-key";
  return `## Step 0: Prerequisites 확인

\`\`\`bash
# AWS CLI 및 인증 확인
aws --version
aws sts get-caller-identity

# SSH 키 확인
ls -la ~/.ssh/${sshKeyName}.pem || echo "❌ SSH key not found: ~/.ssh/${sshKeyName}.pem"
\`\`\`

> 위 명령어가 모두 정상이면 Step 1로 진행하세요.
> AWS CLI가 없으면: \`brew install awscli\` 후 \`aws configure\`
> SSH 키가 없으면: 매니저 AI Deploy Guide에서 키페어를 생성하세요.`;
}

function vmCreationSection({ cloud, region, vmType, vmName, storageGB = 30, keyPairName = "", sgName = "tokamak-l2-sg" }) {
  if (cloud === "gcp") {
    return `## Step 1: Create VM

\`\`\`bash
gcloud compute instances create ${vmName} \\
  --zone=${region}-a \\
  --machine-type=${vmType} \\
  --image-family=ubuntu-2404-lts-amd64 \\
  --image-project=ubuntu-os-cloud \\
  --boot-disk-size=100GB \\
  --boot-disk-type=pd-ssd \\
  --tags=tokamak-l2

# Get the external IP
gcloud compute instances describe ${vmName} \\
  --zone=${region}-a \\
  --format='get(networkInterfaces[0].accessConfigs[0].natIP)'

# SSH into the VM
gcloud compute ssh ${vmName} --zone=${region}-a
\`\`\`

Save the external IP as \`VM_IP\` — you'll need it later.`;
  }

  if (cloud === "vultr") {
    return `## Step 1: Create VM

### 방법 1: Vultr CLI

\`\`\`bash
# 사용 가능한 플랜 확인
vultr plans list | grep -E "vc2-(4|6|8)c"

# 리전 확인
vultr regions list | grep -E "(icn|nrt|sgp|ewr)"

# OS ID 확인 (Ubuntu 22.04 LTS)
vultr os list | grep "Ubuntu 22.04"

# 서버 생성
vultr instance create \\
  --region ${region} \\
  --plan ${vmType} \\
  --os 1743 \\
  --label "${vmName}" \\
  --host "${vmName}"

# 서버 IP 확인
vultr instance list
\`\`\`

### 방법 2: Vultr 웹 콘솔

1. https://my.vultr.com/ → Deploy New Server
2. **Type**: Cloud Compute
3. **Location**: ${region === "icn" ? "Seoul" : region === "nrt" ? "Tokyo" : region === "sgp" ? "Singapore" : region}
4. **OS**: Ubuntu 22.04 LTS x64
5. **Plan**: ${vmType} 이상
6. **SSH Keys**: 기존 키 선택 또는 새로 등록
7. Deploy Now 클릭

서버 준비 후 SSH 접속:

\`\`\`bash
ssh root@VM_IP
\`\`\`

Save the server IP as \`VM_IP\` — you'll need it later.`;
  }

  // AWS
  const keyName = keyPairName || "tokamak-key";
  const diskSize = storageGB || 30;
  return `## Step 1: Create VM

> **IMPORTANT**: 같은 이름의 인스턴스가 이미 존재하면 생성을 건너뛰세요. 중복 생성 시 불필요한 비용이 발생합니다.

\`\`\`bash
# Check if instance already exists
EXISTING=$(aws ec2 describe-instances \\
  --filters "Name=tag:Name,Values=${vmName}" "Name=instance-state-name,Values=pending,running,stopping,stopped" \\
  --query "Reservations[].Instances[0].PublicIpAddress" \\
  --output text --region ${region} 2>/dev/null)

if [ -n "$EXISTING" ] && [ "$EXISTING" != "None" ]; then
  echo "✅ Instance already exists: $EXISTING"
  VM_IP=$EXISTING
else
  # Find default VPC and a public subnet (with internet gateway route)
  VPC_ID=$(aws ec2 describe-vpcs --filters "Name=is-default,Values=true" \\
    --query "Vpcs[0].VpcId" --output text --region ${region} 2>/dev/null)
  if [ -z "$VPC_ID" ] || [ "$VPC_ID" = "None" ]; then
    VPC_ID=$(aws ec2 describe-vpcs --query "Vpcs[0].VpcId" --output text --region ${region})
  fi
  echo "VPC: $VPC_ID"

  # Find a public subnet (with IGW route, not NAT)
  SUBNET_ID=""
  for RT_ID in $(aws ec2 describe-route-tables --filters "Name=vpc-id,Values=$VPC_ID" \\
    --query "RouteTables[?Routes[?GatewayId!=\`local\` && starts_with(GatewayId,\`igw-\`)]].Associations[].SubnetId" \\
    --output text --region ${region} 2>/dev/null); do
    if [ -n "$RT_ID" ] && [ "$RT_ID" != "None" ]; then
      SUBNET_ID=$RT_ID
      break
    fi
  done
  if [ -z "$SUBNET_ID" ]; then
    echo "❌ No public subnet with IGW route found. Check VPC route tables."
    exit 1
  fi
  echo "Subnet: $SUBNET_ID (IGW route confirmed)"

  # Create a security group in the VPC (if not exists)
  SG_ID=$(aws ec2 create-security-group \\
    --group-name ${sgName} --vpc-id $VPC_ID \\
    --description "Tokamak L2 appchain" \\
    --query "GroupId" --output text \\
    --region ${region} 2>/dev/null) || \\
  SG_ID=$(aws ec2 describe-security-groups \\
    --filters "Name=group-name,Values=${sgName}" "Name=vpc-id,Values=$VPC_ID" \\
    --query "SecurityGroups[0].GroupId" --output text --region ${region})
  echo "Security Group: $SG_ID"

  # Open SSH port with current IP
  MY_IP=$(curl -s ifconfig.me 2>/dev/null || echo "0.0.0.0")
  aws ec2 authorize-security-group-ingress \\
    --group-id $SG_ID --protocol tcp --port 22 --cidr $MY_IP/32 \\
    --region ${region} 2>/dev/null || true

  # Launch instance in the public subnet
  aws ec2 run-instances \\
    --region ${region} \\
    --instance-type ${vmType} \\
    --image-id resolve:ssm:/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id \\
    --block-device-mappings '[{"DeviceName":"/dev/sda1","Ebs":{"VolumeSize":${diskSize},"VolumeType":"gp3"}}]' \\
    --tag-specifications 'ResourceType=instance,Tags=[{Key=Name,Value=${vmName}}]' \\
    --key-name ${keyName} \\
    --network-interfaces "SubnetId=$SUBNET_ID,AssociatePublicIpAddress=true,DeviceIndex=0,Groups=[$SG_ID]" \\
    --count 1

  # Wait for instance to be running
  echo "Waiting for instance to start..."
  aws ec2 wait instance-running \\
    --filters "Name=tag:Name,Values=${vmName}" \\
    --region ${region}

  # Get the public IP
  VM_IP=$(aws ec2 describe-instances \\
    --filters "Name=tag:Name,Values=${vmName}" "Name=instance-state-name,Values=running" \\
    --query 'Reservations[0].Instances[0].PublicIpAddress' \\
    --output text --region ${region})
  echo "✅ Instance created: $VM_IP"
fi

echo "VM_IP=$VM_IP"

# SSH into the instance (may need to wait 30s for SSH to be ready)
ssh -o StrictHostKeyChecking=no -i ~/.ssh/${keyName}.pem ubuntu@$VM_IP
\`\`\`

Save the public IP as \`VM_IP\` — you'll need it later.
SSH Key: \`~/.ssh/${keyName}.pem\``;
}

function dockerInstallSection() {
  return `## Step 2: Install Docker

\`\`\`bash
# Install Docker
curl -fsSL https://get.docker.com | sh
sudo usermod -aG docker $USER

# newgrp으로 그룹 적용 (세션이 끊기면 SSH 재접속)
newgrp docker

# Verify
docker --version
docker compose version
\`\`\`

> \`newgrp docker\` 실행 후 셸이 끊기면 SSH로 재접속하세요. 재접속 후 \`docker ps\`가 sudo 없이 동작하면 OK.`;
}

function composeFileSection({ composeContent, dataDir, projectName }) {
  return `## Step 3: Write Docker Compose File

\`\`\`bash
sudo mkdir -p ${dataDir}/genesis
cd ${dataDir}

# Download genesis files (L1 + L2) from repository
curl -fsSL https://raw.githubusercontent.com/tokamak-network/ethrex/tokamak-dev/fixtures/genesis/l1.json \\
  -o ${dataDir}/genesis/l1.json 2>/dev/null || \\
curl -fsSL https://raw.githubusercontent.com/tokamak-network/ethrex/feat/app-customized-framework/fixtures/genesis/l1.json \\
  -o ${dataDir}/genesis/l1.json
curl -fsSL https://raw.githubusercontent.com/tokamak-network/ethrex/tokamak-dev/fixtures/genesis/${profile.genesisFile} \\
  -o ${dataDir}/genesis/${profile.genesisFile} 2>/dev/null || \\
curl -fsSL https://raw.githubusercontent.com/tokamak-network/ethrex/feat/app-customized-framework/fixtures/genesis/${profile.genesisFile} \\
  -o ${dataDir}/genesis/${profile.genesisFile}
curl -fsSL https://raw.githubusercontent.com/tokamak-network/ethrex/tokamak-dev/fixtures/keys/private_keys_l1.txt \\
  -o ${dataDir}/genesis/private_keys_l1.txt 2>/dev/null || \\
curl -fsSL https://raw.githubusercontent.com/tokamak-network/ethrex/feat/app-customized-framework/fixtures/keys/private_keys_l1.txt \\
  -o ${dataDir}/genesis/private_keys_l1.txt
echo "✅ Genesis files downloaded"

cat > docker-compose.yaml << 'COMPOSE_EOF'
${composeContent.trimEnd()}
COMPOSE_EOF
\`\`\`

The compose project name is \`${projectName}\`.
이 파일은 사전 빌드된 Docker 이미지(\`ghcr.io/tokamak-network/tokamak-appchain\`)를 pull합니다.

\`\`\`bash
# 키 환경변수 파일 생성 (compose에서 참조)
cat > ${dataDir}/.env << 'ENV_EOF'
# Default built-in L1 keys (test-only, no real assets)
ETHREX_DEPLOYER_L1_PRIVATE_KEY=0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924
ETHREX_ON_CHAIN_PROPOSER_OWNER=0x4417092b70a3e5f10dc504d0947dd256b965fc62
ETHREX_BRIDGE_OWNER=0x4417092b70a3e5f10dc504d0947dd256b965fc62
ETHREX_BRIDGE_OWNER_PK=0x941e103320615d394a55708be13e45994c7d93b932b064dbcb2b511fe3254e2e
ETHREX_DEPLOYER_SEQUENCER_REGISTRY_OWNER=0x4417092b70a3e5f10dc504d0947dd256b965fc62
ETHREX_COMMITTER_L1_PRIVATE_KEY=0x385c546456b6a603a1cfcaa9ec9494ba4832da08dd6bcf4de9a71e4a01b74924
ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY=0x39725efee3fb28614de3bacaffe4cc4bd8c436257e2c8bb887c4b5c4be45e76d
ENV_EOF
chmod 600 ${dataDir}/.env

# Prover에 필요한 programs.toml 생성
cat > ${dataDir}/programs.toml << 'TOML_EOF'
default_program = "evm-l2"
enabled_programs = ["evm-l2", "zk-dex", "tokamon"]
TOML_EOF
\`\`\`

> ⚠️ \`.env\` 파일에 프라이빗 키가 포함됩니다. 서버 접근 권한을 관리하세요.
> 위 키는 내장 L1 테스트용입니다. 테스트넷/메인넷 배포 시 실제 키로 교체하세요.`;
}

function testnetEnvSection({ cloud, l1RpcUrl, l1ChainId, l1Network, dataDir, walletConfig }) {
  const hasWallet = walletConfig && walletConfig.deployerKey;

  let keySetupGuide = "";
  if (hasWallet) {
    if (cloud === "gcp") {
      keySetupGuide = `
### Private Key 전달 방법 (GCP Secret Manager 권장)

사용자의 로컬 머신에서 실행:

\`\`\`bash
# 1. Secret Manager API 활성화
gcloud services enable secretmanager.googleapis.com

# 2. Keychain에서 키를 읽어 Secret Manager에 등록
security find-generic-password -a "${walletConfig.deployerKey}" -s "tokamak-appchain" -w | \\
  gcloud secrets create tokamak-deployer-key --data-file=-

${walletConfig.committerKey ? `security find-generic-password -a "${walletConfig.committerKey}" -s "tokamak-appchain" -w | \\
  gcloud secrets create tokamak-committer-key --data-file=-` : "# Committer = Deployer (동일 키)"}
${walletConfig.proofCoordinatorKey ? `security find-generic-password -a "${walletConfig.proofCoordinatorKey}" -s "tokamak-appchain" -w | \\
  gcloud secrets create tokamak-proof-coordinator-key --data-file=-` : "# Proof Coordinator = Deployer (동일 키)"}
${walletConfig.bridgeOwnerKey ? `security find-generic-password -a "${walletConfig.bridgeOwnerKey}" -s "tokamak-appchain" -w | \\
  gcloud secrets create tokamak-bridge-owner-key --data-file=-` : "# Bridge Owner = Deployer (동일 키)"}
\`\`\`

VM에서 키 가져오기:

\`\`\`bash
DEPLOYER_PRIVATE_KEY=$(gcloud secrets versions access latest --secret=tokamak-deployer-key)
${walletConfig.committerKey ? `COMMITTER_PK=$(gcloud secrets versions access latest --secret=tokamak-committer-key)` : "COMMITTER_PK=$DEPLOYER_PRIVATE_KEY"}
${walletConfig.proofCoordinatorKey ? `PROOF_COORDINATOR_PK=$(gcloud secrets versions access latest --secret=tokamak-proof-coordinator-key)` : "PROOF_COORDINATOR_PK=$DEPLOYER_PRIVATE_KEY"}
${walletConfig.bridgeOwnerKey ? `BRIDGE_OWNER_PK=$(gcloud secrets versions access latest --secret=tokamak-bridge-owner-key)` : "BRIDGE_OWNER_PK=$DEPLOYER_PRIVATE_KEY"}
\`\`\``;
    } else if (cloud === "aws") {
      keySetupGuide = `
### Private Key 전달 방법 (AWS Secrets Manager 권장)

사용자의 로컬 머신에서 실행:

\`\`\`bash
# 1. Keychain에서 키를 읽어 Secrets Manager에 등록
DEPLOYER_KEY=$(security find-generic-password -a "${walletConfig.deployerKey}" -s "tokamak-appchain" -w)
aws secretsmanager create-secret --name tokamak-deployer-key --secret-string "$DEPLOYER_KEY"

${walletConfig.committerKey ? `COMMITTER_KEY=$(security find-generic-password -a "${walletConfig.committerKey}" -s "tokamak-appchain" -w)
aws secretsmanager create-secret --name tokamak-committer-key --secret-string "$COMMITTER_KEY"` : "# Committer = Deployer (동일 키)"}
${walletConfig.proofCoordinatorKey ? `PC_KEY=$(security find-generic-password -a "${walletConfig.proofCoordinatorKey}" -s "tokamak-appchain" -w)
aws secretsmanager create-secret --name tokamak-proof-coordinator-key --secret-string "$PC_KEY"` : "# Proof Coordinator = Deployer (동일 키)"}
${walletConfig.bridgeOwnerKey ? `BO_KEY=$(security find-generic-password -a "${walletConfig.bridgeOwnerKey}" -s "tokamak-appchain" -w)
aws secretsmanager create-secret --name tokamak-bridge-owner-key --secret-string "$BO_KEY"` : "# Bridge Owner = Deployer (동일 키)"}
\`\`\`

VM에서 키 가져오기:

\`\`\`bash
DEPLOYER_PRIVATE_KEY=$(aws secretsmanager get-secret-value --secret-id tokamak-deployer-key --query SecretString --output text)
${walletConfig.committerKey ? `COMMITTER_PK=$(aws secretsmanager get-secret-value --secret-id tokamak-committer-key --query SecretString --output text)` : "COMMITTER_PK=$DEPLOYER_PRIVATE_KEY"}
${walletConfig.proofCoordinatorKey ? `PROOF_COORDINATOR_PK=$(aws secretsmanager get-secret-value --secret-id tokamak-proof-coordinator-key --query SecretString --output text)` : "PROOF_COORDINATOR_PK=$DEPLOYER_PRIVATE_KEY"}
${walletConfig.bridgeOwnerKey ? `BRIDGE_OWNER_PK=$(aws secretsmanager get-secret-value --secret-id tokamak-bridge-owner-key --query SecretString --output text)` : "BRIDGE_OWNER_PK=$DEPLOYER_PRIVATE_KEY"}
\`\`\``;
    } else {
      // Vultr or other — use SCP
      keySetupGuide = `
### Private Key 전달 방법 (SCP 사용)

사용자의 로컬 머신에서 실행:

\`\`\`bash
# 1. 로컬에서 .env 파일 생성
DEPLOYER_KEY=$(security find-generic-password -a "${walletConfig.deployerKey}" -s "tokamak-appchain" -w)
${walletConfig.committerKey ? `COMMITTER_KEY=$(security find-generic-password -a "${walletConfig.committerKey}" -s "tokamak-appchain" -w)` : "COMMITTER_KEY=$DEPLOYER_KEY"}
${walletConfig.proofCoordinatorKey ? `PC_KEY=$(security find-generic-password -a "${walletConfig.proofCoordinatorKey}" -s "tokamak-appchain" -w)` : "PC_KEY=$DEPLOYER_KEY"}
${walletConfig.bridgeOwnerKey ? `BO_KEY=$(security find-generic-password -a "${walletConfig.bridgeOwnerKey}" -s "tokamak-appchain" -w)` : "BO_KEY=$DEPLOYER_KEY"}

cat > /tmp/tokamak-keys.env << EOF
DEPLOYER_PRIVATE_KEY=$DEPLOYER_KEY
COMMITTER_PK=$COMMITTER_KEY
PROOF_COORDINATOR_PK=$PC_KEY
BRIDGE_OWNER_PK=$BO_KEY
EOF

# 2. SCP로 서버에 전송
scp /tmp/tokamak-keys.env root@VM_IP:${dataDir}/.env
rm /tmp/tokamak-keys.env  # 로컬에서 삭제
\`\`\`

VM에서 확인:

\`\`\`bash
# 키 파일 확인 (내용은 절대 출력하지 마세요)
wc -l ${dataDir}/.env
chmod 600 ${dataDir}/.env

# 환경변수 로드
source ${dataDir}/.env
\`\`\``;
    }
  }

  return `## Step 3.5: Configure Testnet Environment

Create an environment file with your L1 connection and private keys.

> **IMPORTANT**: 비밀키를 shell history나 소스코드에 절대 남기지 마세요.
${keySetupGuide}
${!hasWallet ? `
\`\`\`bash
cat > ${dataDir}/.env << 'ENV_EOF'
# L1 Connection
L1_RPC_URL=${l1RpcUrl || "https://eth-sepolia.g.alchemy.com/v2/YOUR_API_KEY"}
L1_CHAIN_ID=${l1ChainId || 11155111}
L1_NETWORK=${l1Network || "sepolia"}

# Private Keys (REPLACE THESE — 절대 커밋하지 마세요)
DEPLOYER_PRIVATE_KEY=0xYOUR_DEPLOYER_PRIVATE_KEY
# Optional: 각 역할에 별도 키 사용 (기본값: Deployer 키)
# COMMITTER_PK=0x...
# PROOF_COORDINATOR_PK=0x...
# BRIDGE_OWNER_PK=0x...
ENV_EOF

chmod 600 ${dataDir}/.env
\`\`\`
` : ""}
docker-compose.yaml에서 환경변수를 업데이트:
- \`ETHREX_ETH_RPC_URL\` → \`$L1_RPC_URL\`
- \`ETHREX_DEPLOYER_L1_PRIVATE_KEY\` → \`$DEPLOYER_PRIVATE_KEY\`
- Committer/Proof Coordinator 키도 해당하는 환경변수에 설정
- \`ETHREX_DEPLOYER_DEPLOY_RICH=false\` (테스트넷은 실제 ETH 사용)
- L1 서비스(\`tokamak-app-l1\`) 제거 (외부 L1 사용)

> Deployer 계정에 충분한 ${l1Network || "testnet"} ETH가 있는지 확인하세요.
> Sepolia ETH: https://sepoliafaucet.com/ | Holesky ETH: https://holesky-faucet.pk910.de/`;
}

function deploySection({ projectName, dataDir, isTestnet }) {
  return `## Step 4: Pull Images and Deploy

\`\`\`bash
cd ${dataDir}

# Pull all images (사전 빌드된 이미지 — 2-3분 소요)
docker compose -p ${projectName} pull

# Start the deployment
docker compose -p ${projectName} up -d

# Deployer 로그 확인 (완료될 때까지 대기 — 보통 3-5분)
docker logs -f ${projectName}-deployer

# Deployer 종료 확인 (exit code 0이어야 정상)
docker wait ${projectName}-deployer
docker inspect ${projectName}-deployer --format='{{.State.ExitCode}}'
\`\`\`

Deployer 동작 순서:
1. ${isTestnet ? "외부 L1 RPC에 연결" : "Built-in L1이 준비될 때까지 대기"}
2. L1 컨트랙트 컴파일 및 배포 (CommonBridge, OnChainProposer, Timelock, SP1Verifier, GuestProgramRegistry)
3. 배포된 주소를 공유 볼륨(\`/env/.env\`)에 기록
4. 성공 시 exit code 0으로 종료

> Deployer가 종료되면 L2 노드와 Prover가 자동으로 시작됩니다.
> exit code가 0이 아니면 \`docker logs ${projectName}-deployer\`로 에러를 확인하세요.`;
}

function verifySection({ l2ChainId, isTestnet }) {
  const chainIdHex = "0x" + l2ChainId.toString(16).toUpperCase();
  return `## Step 5: Verify Deployment

\`\`\`bash
# Check L2 RPC is responding
curl -s http://localhost:${DEFAULT_PORTS.l2} \\
  -X POST -H "Content-Type: application/json" \\
  -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}'
# Expected: {"result":"${chainIdHex}"}

# Check latest block number
curl -s http://localhost:${DEFAULT_PORTS.l2} \\
  -X POST -H "Content-Type: application/json" \\
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
# Should return an incrementing block number

${isTestnet ? "" : `# Check L1 is running
curl -s http://localhost:${DEFAULT_PORTS.l1} \\
  -X POST -H "Content-Type: application/json" \\
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
`}
# View all container statuses
docker ps --format "table {{.Names}}\\t{{.Status}}\\t{{.Ports}}"
\`\`\`

All containers should show "Up" status. The deployer container should show "Exited (0)".`;
}

function toolsSection({ dataDir, l2ChainId, isTestnet, l1ChainId, l1Network, l1RpcUrl, programSlug, projectName }) {
  return `## Step 6: Deploy Tools (Explorer + Dashboard)

Tools 스택:
- **L2 Blockscout Explorer** (port ${DEFAULT_PORTS.l2Explorer})
- ${isTestnet ? `**L1 Explorer**: ${l1Network || "external"} Etherscan 사용` : `**L1 Blockscout Explorer** (port ${DEFAULT_PORTS.l1Explorer})`}
- **Bridge Dashboard** (port ${DEFAULT_PORTS.dashboard})

\`\`\`bash
cd ${dataDir}

# Download tools compose (tokamak-dev, fallback to feature branch)
curl -fsSL https://raw.githubusercontent.com/tokamak-network/ethrex/tokamak-dev/crates/l2/docker-compose-zk-dex-tools.yaml \\
  -o docker-compose-tools.yaml 2>/dev/null || \\
curl -fsSL https://raw.githubusercontent.com/tokamak-network/ethrex/feat/app-customized-framework/crates/l2/docker-compose-zk-dex-tools.yaml \\
  -o docker-compose-tools.yaml

# Get the deployed contract addresses from the deployer
docker cp ${projectName}-deployer:/env/.env ${dataDir}/deployed.env 2>/dev/null || echo "No deployed env found"

# Set tools environment variables
export TOOLS_L2_RPC_PORT=${DEFAULT_PORTS.l2}
export TOOLS_L1_RPC_PORT=${DEFAULT_PORTS.l1}
export TOOLS_L2_EXPLORER_PORT=${DEFAULT_PORTS.l2Explorer}
export TOOLS_L1_EXPLORER_PORT=${DEFAULT_PORTS.l1Explorer}
export TOOLS_BRIDGE_UI_PORT=${DEFAULT_PORTS.dashboard}
export TOOLS_DB_PORT=${DEFAULT_PORTS.dbPort}
export TOOLS_METRICS_PORT=${DEFAULT_PORTS.metricsPort}
export TOOLS_BIND_ADDR=0.0.0.0
export TOOLS_ENV_FILE=${dataDir}/deployed.env
export L2_CHAIN_ID=${l2ChainId}
${isTestnet ? `export L1_CHAIN_ID=${l1ChainId || 11155111}
export IS_EXTERNAL_L1=true
export L1_RPC_URL=${l1RpcUrl || "$L1_RPC_URL"}
export L1_NETWORK_NAME=${l1Network || "sepolia"}` : `export L1_CHAIN_ID=${l1ChainId || 9}`}

# Public URLs — VM IP 또는 도메인으로 설정 (나중에 도메인 연결 시 변경)
VM_IP=$(curl -s http://169.254.169.254/latest/meta-data/public-ipv4 2>/dev/null || echo "localhost")
export PUBLIC_L1_EXPLORER_HOST=$VM_IP:${DEFAULT_PORTS.l1Explorer}
export PUBLIC_L2_EXPLORER_HOST=$VM_IP:${DEFAULT_PORTS.l2Explorer}
export PUBLIC_L1_EXPLORER_URL=http://$VM_IP:${DEFAULT_PORTS.l1Explorer}
export PUBLIC_L2_EXPLORER_URL=http://$VM_IP:${DEFAULT_PORTS.l2Explorer}

# Start tools (use external-l1 profile for testnet to skip L1 Blockscout)
docker compose -f docker-compose-tools.yaml \\
  -p ${projectName}-tools \\
  ${isTestnet ? "--profile external-l1 " : ""}up -d
\`\`\`

30초 후 Blockscout 초기화가 완료되면 확인:

\`\`\`bash
# L2 Explorer
curl -s http://localhost:${DEFAULT_PORTS.l2Explorer}/api/v2/stats | head -c 200

# Dashboard
curl -s http://localhost:${DEFAULT_PORTS.dashboard}/ | head -c 200
\`\`\``;
}

function firewallSection({ cloud, vmName, isTestnet, sgName = "tokamak-l2-sg", region = "ap-northeast-2" }) {
  if (cloud === "gcp") {
    const ports = isTestnet
      ? `${DEFAULT_PORTS.l2},${DEFAULT_PORTS.l2Explorer},${DEFAULT_PORTS.dashboard}`
      : `${DEFAULT_PORTS.l1},${DEFAULT_PORTS.l2},${DEFAULT_PORTS.l2Explorer},${DEFAULT_PORTS.l1Explorer},${DEFAULT_PORTS.dashboard}`;
    return `## Step 7: Open Firewall Ports

\`\`\`bash
gcloud compute firewall-rules create tokamak-l2-allow \\
  --allow=tcp:${ports} \\
  --target-tags=tokamak-l2 \\
  --description="Tokamak L2 appchain ports"
\`\`\``;
  }

  if (cloud === "vultr") {
    const ports = isTestnet
      ? [DEFAULT_PORTS.l2, DEFAULT_PORTS.l2Explorer, DEFAULT_PORTS.dashboard]
      : [DEFAULT_PORTS.l1, DEFAULT_PORTS.l2, DEFAULT_PORTS.l2Explorer, DEFAULT_PORTS.l1Explorer, DEFAULT_PORTS.dashboard];

    return `## Step 7: Open Firewall Ports

### 방법 1: Vultr Firewall Group (웹 콘솔)

1. https://my.vultr.com/firewall/ → Add Firewall Group
2. 규칙 추가:
${ports.map(p => `   - Protocol: TCP, Port: ${p}, Source: 0.0.0.0/0`).join("\n")}
3. 서버에 Firewall Group 연결

### 방법 2: UFW (서버 내)

\`\`\`bash
sudo ufw allow 22/tcp  # SSH
${ports.map(p => `sudo ufw allow ${p}/tcp`).join("\n")}
sudo ufw enable
\`\`\``;
  }

  // AWS
  // Only open ports that external users need (L2 services + tools)
  // L1 RPC/Explorer are internal — accessed within Docker network only
  const sgRules = [
    { port: DEFAULT_PORTS.l2, desc: "L2 RPC" },
    { port: DEFAULT_PORTS.l2Explorer, desc: "L2 Explorer" },
    { port: DEFAULT_PORTS.l1Explorer, desc: "L1 Explorer" },
    { port: DEFAULT_PORTS.dashboard, desc: "Bridge Dashboard" },
  ];

  const rules = sgRules.map(r =>
    `aws ec2 authorize-security-group-ingress --group-id $SG_ID --protocol tcp --port ${r.port} --cidr 0.0.0.0/0 --region ${region}  # ${r.desc}`
  ).join("\n");

  return `## Step 7: Open Firewall Ports

> L2 RPC, Explorer, Dashboard 포트를 외부에서 접근할 수 있도록 개방합니다 (SSH 제외).
> 실 운영 환경에서는 필요한 포트만 특정 IP로 제한하세요.

\`\`\`bash
# SG_ID가 없으면 조회
SG_ID=\${SG_ID:-$(aws ec2 describe-security-groups --filters "Name=group-name,Values=${sgName}" --query "SecurityGroups[0].GroupId" --output text --region ${region})}
${rules}
\`\`\``;
}

function summarySection({ isTestnet, deployment }) {
  const name = deployment?.name || "Tokamak Appchain";
  return `## Step 8: Access Your L2

배포 완료 후 접속 가능한 서비스:

| Service | URL |
|---------|-----|
| **L2 RPC** | \`http://VM_IP:${DEFAULT_PORTS.l2}\` |
| **L2 Explorer** | \`http://VM_IP:${DEFAULT_PORTS.l2Explorer}\` |
| **Dashboard** | \`http://VM_IP:${DEFAULT_PORTS.dashboard}\` |
${isTestnet ? "" : `| **L1 RPC** | \`http://VM_IP:${DEFAULT_PORTS.l1}\` |
| **L1 Explorer** | \`http://VM_IP:${DEFAULT_PORTS.l1Explorer}\` |
`}
Replace \`VM_IP\` with the actual IP from Step 1.

### MetaMask Configuration

| Setting | Value |
|---------|-------|
| **Network Name** | ${name} |
| **RPC URL** | \`http://VM_IP:${DEFAULT_PORTS.l2}\` |
| **Chain ID** | (see deployment summary above) |
| **Currency Symbol** | ETH |`;
}

function troubleshootingSection({ projectName, dataDir, sgName = "", region = "ap-northeast-2", vmName = "" }) {
  return `## Troubleshooting

\`\`\`bash
# View all container logs
docker compose -p ${projectName} logs --tail=50

# View specific container logs
docker logs ${projectName}-deployer  # Contract deployment
docker logs ${projectName}-l2        # L2 node
docker logs ${projectName}-prover    # Prover

# Restart a service
docker compose -p ${projectName} restart tokamak-app-l2

# Full restart
cd ${dataDir}
docker compose -p ${projectName} down
docker compose -p ${projectName} up -d

# Check disk space
df -h

# Check Docker resources
docker system df
\`\`\`

### Common Issues

1. **Deployer 실패**: L1 연결 확인, Deployer 계정 잔액 확인
2. **L2 블록 생성 안됨**: Deployer가 성공적으로 종료되었는지 확인 (\`docker logs ${projectName}-deployer\`)
3. **Explorer 데이터 없음**: Blockscout 인덱서 초기화에 1-2분 소요
4. **포트 충돌**: docker-compose.yaml에서 포트 매핑 변경
5. **이미지 pull 실패**: \`docker login ghcr.io\` 또는 네트워크 확인
6. **Prover 크래시**: 메모리 부족일 수 있음. VM 사양 업그레이드 고려

### 리소스 정리 (비용 절약)

> ⚠️ terminate는 복구 불가합니다. 단계별로 확인하면서 진행하세요.

**1단계: 현재 상태 확인**
\`\`\`bash
aws ec2 describe-instances --filters "Name=tag:Name,Values=${vmName}" \\
  --query "Reservations[].Instances[].{Id:InstanceId,State:State.Name,IP:PublicIpAddress}" \\
  --output table --region ${region}
\`\`\`

**2단계: 인스턴스 중지 (일시 정지 — 나중에 재시작 가능)**
\`\`\`bash
aws ec2 stop-instances --instance-ids INSTANCE_ID --region ${region}
\`\`\`
> stop하면 인스턴스 비용 즉시 중지. EBS($0.096/GB/월)와 IP($3.60/월)는 계속 과금.
> 💡 당장 삭제하지 않아도 stop만으로 인스턴스 비용을 절약할 수 있습니다. 나중에 start로 재시작 가능.

**참고: AWS EC2 과금 방식**
- 인스턴스는 **초 단위 과금** (최소 1분). stop/terminate 즉시 과금 중지.
- 안 쓸 때는 **stop하면 인스턴스 비용 0원**. start로 언제든 재시작 가능.
- 단, stop하면 Public IP가 바뀌고 Docker 컨테이너 재시작이 필요합니다.

**3단계: 인스턴스 완전 삭제 (복구 불가 — 필요할 때만)**
\`\`\`bash
aws ec2 terminate-instances --instance-ids INSTANCE_ID --region ${region}
\`\`\`
> terminate: EBS, Public IP 모두 삭제. 과금 즉시 중지.

**4단계: Security Group 삭제 (인스턴스 terminate 후)**
\`\`\`bash
aws ec2 delete-security-group --group-id $(aws ec2 describe-security-groups \\
  --filters "Name=group-name,Values=${sgName}" \\
  --query "SecurityGroups[0].GroupId" --output text --region ${region}) --region ${region}
\`\`\``;
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

module.exports = {
  generateAIDeployPrompt,
  CLOUD_PRESETS,
  DEFAULT_PORTS,
};
