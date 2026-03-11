/**
 * AI Deploy Prompt Generator
 *
 * Generates complete, executable deployment prompts that AI assistants
 * (Claude Code, ChatGPT, etc.) can run step-by-step to deploy a
 * Tokamak L2 appchain on a cloud VM.
 *
 * The generated prompt includes:
 * - VM creation commands (GCP / AWS)
 * - Docker installation
 * - Compose files (inline, ready to write)
 * - Environment variables
 * - Deployment steps with verification commands
 * - Troubleshooting guide
 */

const { generateRemoteComposeFile, getAppProfile, APP_PROFILES } = require("./compose-generator");

// ---------------------------------------------------------------------------
// Cloud presets
// ---------------------------------------------------------------------------

const CLOUD_PRESETS = {
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

// ---------------------------------------------------------------------------
// Main generator
// ---------------------------------------------------------------------------

/**
 * Generate a complete AI-executable deployment prompt.
 *
 * @param {Object} opts
 * @param {Object} opts.deployment  - DB deployment row
 * @param {string} opts.cloud       - 'gcp' | 'aws'
 * @param {string} opts.region      - Cloud region ID
 * @param {string} opts.vmType      - VM instance type
 * @param {string} opts.l1Mode      - 'local' (built-in L1) or 'testnet' (external L1)
 * @param {string} [opts.l1RpcUrl]  - External L1 RPC URL (when l1Mode === 'testnet')
 * @param {number} [opts.l1ChainId] - L1 chain ID for testnet
 * @param {string} [opts.l1Network] - L1 network name (sepolia, holesky, etc.)
 * @returns {string} Markdown prompt
 */
function generateAIDeployPrompt(opts) {
  const {
    deployment, cloud, region, vmType,
    l1Mode = "local", l1RpcUrl, l1ChainId, l1Network,
  } = opts;

  const config = deployment.config ? JSON.parse(deployment.config) : {};
  const programSlug = deployment.program_slug || "evm-l2";
  const profile = getAppProfile(programSlug);
  const l2ChainId = deployment.chain_id || 65536999;
  const projectName = `tokamak-${deployment.id.slice(0, 8)}`;
  const isTestnet = l1Mode === "testnet";
  const vmName = `tokamak-l2-${deployment.id.slice(0, 8)}`;
  const dataDir = `/opt/tokamak/${deployment.id.slice(0, 8)}`;

  // Generate compose content
  const composeContent = generateRemoteComposeFile({
    programSlug,
    l1Port: DEFAULT_PORTS.l1,
    l2Port: DEFAULT_PORTS.l2,
    proofCoordPort: DEFAULT_PORTS.proofCoord,
    projectName,
    dataDir,
    l2ChainId,
  });

  // Build the prompt sections
  const sections = [];

  sections.push(headerSection({ deployment, programSlug, profile, cloud, region, vmType, l2ChainId, isTestnet, l1RpcUrl, l1Network }));
  sections.push(prerequisitesSection(cloud));
  sections.push(vmCreationSection({ cloud, region, vmType, vmName }));
  sections.push(dockerInstallSection());
  sections.push(composeFileSection({ composeContent, dataDir, projectName }));

  if (isTestnet) {
    sections.push(testnetEnvSection({ l1RpcUrl, l1ChainId, l1Network, dataDir }));
  }

  sections.push(deploySection({ projectName, dataDir, isTestnet }));
  sections.push(verifySection({ l2ChainId, isTestnet }));
  sections.push(toolsSection({ dataDir, l2ChainId, isTestnet, l1ChainId, l1Network, l1RpcUrl, programSlug }));
  sections.push(firewallSection({ cloud, vmName, isTestnet }));
  sections.push(summarySection({ isTestnet }));
  sections.push(troubleshootingSection({ projectName, dataDir }));

  return sections.join("\n\n");
}

// ---------------------------------------------------------------------------
// Section builders
// ---------------------------------------------------------------------------

function headerSection({ deployment, programSlug, profile, cloud, region, vmType, l2ChainId, isTestnet, l1RpcUrl, l1Network }) {
  const cloudLabel = CLOUD_PRESETS[cloud]?.label || cloud;
  return `# Tokamak L2 Appchain — Cloud Deployment

> **This prompt is designed for AI assistants (Claude Code, ChatGPT, etc.).**
> Execute each step in order. All files and commands are complete and ready to run.

## Deployment Summary

| Setting | Value |
|---------|-------|
| **App** | ${programSlug} (${profile.description}) |
| **L2 Name** | ${deployment.name} |
| **L2 Chain ID** | ${l2ChainId} |
| **Cloud** | ${cloudLabel} |
| **Region** | ${region} |
| **VM Type** | ${vmType} |
| **L1 Mode** | ${isTestnet ? `Testnet (${l1Network || "external"})` : "Built-in (Docker L1)"} |
${isTestnet ? `| **L1 RPC** | \`$L1_RPC_URL\` (set in .env) |\n` : ""}| **Docker Images** | \`ghcr.io/tokamak-network/tokamak-appchain:{l1,l2,sp1}\` |`;
}

function prerequisitesSection(cloud) {
  const cli = cloud === "gcp" ? "`gcloud` CLI authenticated (`gcloud auth login`)" : "`aws` CLI configured (`aws configure`)";
  return `## Prerequisites

- ${cli}
- SSH key pair available
- For testnet: funded deployer account on the target L1 network`;
}

function vmCreationSection({ cloud, region, vmType, vmName }) {
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

  // AWS
  return `## Step 1: Create VM

\`\`\`bash
# Create a security group (if not exists)
aws ec2 create-security-group \\
  --group-name tokamak-l2-sg \\
  --description "Tokamak L2 appchain" \\
  --region ${region}

# Launch instance
aws ec2 run-instances \\
  --region ${region} \\
  --instance-type ${vmType} \\
  --image-id resolve:ssm:/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id \\
  --block-device-mappings '[{"DeviceName":"/dev/sda1","Ebs":{"VolumeSize":100,"VolumeType":"gp3"}}]' \\
  --tag-specifications 'ResourceType=instance,Tags=[{Key=Name,Value=${vmName}}]' \\
  --key-name YOUR_KEY_NAME \\
  --security-groups tokamak-l2-sg \\
  --count 1

# Get the public IP
aws ec2 describe-instances \\
  --filters "Name=tag:Name,Values=${vmName}" \\
  --query 'Reservations[0].Instances[0].PublicIpAddress' \\
  --output text \\
  --region ${region}

# SSH into the instance
ssh -i YOUR_KEY.pem ubuntu@VM_IP
\`\`\`

Save the public IP as \`VM_IP\` — you'll need it later.
Replace \`YOUR_KEY_NAME\` and \`YOUR_KEY.pem\` with your actual SSH key.`;
}

function dockerInstallSection() {
  return `## Step 2: Install Docker

Run these commands on the VM:

\`\`\`bash
# Install Docker
curl -fsSL https://get.docker.com | sh
sudo usermod -aG docker $USER
newgrp docker

# Verify
docker --version
docker compose version
\`\`\``;
}

function composeFileSection({ composeContent, dataDir, projectName }) {
  return `## Step 3: Write Docker Compose File

\`\`\`bash
sudo mkdir -p ${dataDir}
cd ${dataDir}

cat > docker-compose.yaml << 'COMPOSE_EOF'
${composeContent.trimEnd()}
COMPOSE_EOF
\`\`\`

The compose project name is \`${projectName}\`.`;
}

function testnetEnvSection({ l1RpcUrl, l1ChainId, l1Network, dataDir }) {
  return `## Step 3.5: Configure Testnet Environment

Create an environment file with your L1 connection and private keys.

> **IMPORTANT**: Replace the placeholder values below with your actual keys.
> Never commit private keys to version control.

\`\`\`bash
cat > ${dataDir}/.env << 'ENV_EOF'
# L1 Connection
L1_RPC_URL=${l1RpcUrl || "https://eth-sepolia.g.alchemy.com/v2/YOUR_API_KEY"}
L1_CHAIN_ID=${l1ChainId || 11155111}
L1_NETWORK=${l1Network || "sepolia"}

# Private Keys (REPLACE THESE)
# The deployer key must be funded with testnet ETH on the L1 network.
DEPLOYER_PRIVATE_KEY=0xYOUR_DEPLOYER_PRIVATE_KEY
# Optional: separate keys for each role (defaults to deployer key)
# COMMITTER_PRIVATE_KEY=0x...
# PROOF_COORDINATOR_PRIVATE_KEY=0x...
# BRIDGE_OWNER_PRIVATE_KEY=0x...
ENV_EOF

chmod 600 ${dataDir}/.env
\`\`\`

Then update the docker-compose.yaml deployer and L2 services to use these values:
- Replace all \`ETHREX_ETH_RPC_URL\` values with \`$L1_RPC_URL\`
- Replace \`ETHREX_DEPLOYER_L1_PRIVATE_KEY\` with \`$DEPLOYER_PRIVATE_KEY\`
- Replace committer/proof-coordinator L1 private keys accordingly
- Set \`ETHREX_DEPLOYER_DEPLOY_RICH=false\` (testnet has real ETH)
- Remove the \`tokamak-app-l1\` service (no local L1 needed)`;
}

function deploySection({ projectName, dataDir, isTestnet }) {
  return `## Step 4: Pull Images and Deploy

\`\`\`bash
cd ${dataDir}

# Pull all images
docker compose -p ${projectName} pull

# Start the deployment
docker compose -p ${projectName} up -d

# Watch the deployer logs (wait for "Contract deployment complete")
docker logs -f ${projectName}-deployer
\`\`\`

The deployer container will:
1. ${isTestnet ? "Connect to the external L1 RPC" : "Wait for the built-in L1 to be ready"}
2. Compile and deploy L1 contracts (CommonBridge, OnChainProposer, etc.)
3. Write deployed addresses to a shared volume (\`/env/.env\`)
4. Exit with code 0 on success

Once the deployer exits, the L2 node and prover will start automatically.`;
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

function toolsSection({ dataDir, l2ChainId, isTestnet, l1ChainId, l1Network, l1RpcUrl, programSlug }) {
  // Tools compose is large — include inline for the AI to write
  return `## Step 6: Deploy Tools (Explorer + Dashboard)

The tools stack includes:
- **L2 Blockscout Explorer** (port ${DEFAULT_PORTS.l2Explorer})
- ${isTestnet ? `**L1 Explorer**: Use ${l1Network || "external"} Etherscan` : `**L1 Blockscout Explorer** (port ${DEFAULT_PORTS.l1Explorer})`}
- **Bridge Dashboard** (port ${DEFAULT_PORTS.dashboard})

\`\`\`bash
cd ${dataDir}

# Download the tools compose file from the repository
curl -fsSL https://raw.githubusercontent.com/tokamak-network/ethrex/tokamak-dev/crates/l2/docker-compose-zk-dex-tools.yaml \\
  -o docker-compose-tools.yaml

# Get the deployed contract addresses from the deployer
docker cp $(docker ps -aq -f name=deployer | head -1):/env/.env ${dataDir}/deployed.env 2>/dev/null || echo "No deployed env found"

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
export L1_NETWORK_NAME=${l1Network || "sepolia"}` : `export L1_CHAIN_ID=9`}

# Start tools (use external-l1 profile for testnet to skip L1 Blockscout)
docker compose -f docker-compose-tools.yaml \\
  -p tools-${programSlug} \\
  ${isTestnet ? "--profile external-l1 " : ""}up -d
\`\`\`

Wait ~30 seconds for Blockscout to initialize, then verify:

\`\`\`bash
# L2 Explorer
curl -s http://localhost:${DEFAULT_PORTS.l2Explorer}/api/v2/stats | head -c 200

# Dashboard
curl -s http://localhost:${DEFAULT_PORTS.dashboard}/ | head -c 200
\`\`\``;
}

function firewallSection({ cloud, vmName, isTestnet }) {
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

  // AWS
  const sgRules = [
    { port: DEFAULT_PORTS.l2, desc: "L2 RPC" },
    { port: DEFAULT_PORTS.l2Explorer, desc: "L2 Explorer" },
    { port: DEFAULT_PORTS.dashboard, desc: "Dashboard" },
  ];
  if (!isTestnet) {
    sgRules.push({ port: DEFAULT_PORTS.l1, desc: "L1 RPC" });
    sgRules.push({ port: DEFAULT_PORTS.l1Explorer, desc: "L1 Explorer" });
  }

  const rules = sgRules.map(r =>
    `aws ec2 authorize-security-group-ingress --group-name tokamak-l2-sg --protocol tcp --port ${r.port} --cidr 0.0.0.0/0  # ${r.desc}`
  ).join("\n");

  return `## Step 7: Open Firewall Ports

\`\`\`bash
${rules}
\`\`\``;
}

function summarySection({ isTestnet }) {
  return `## Step 8: Access Your L2

After completing all steps, your L2 appchain is accessible at:

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

Add this L2 network to MetaMask:
- **Network Name**: ${isTestnet ? "Tokamak Appchain (Testnet)" : "Tokamak Appchain"}
- **RPC URL**: \`http://VM_IP:${DEFAULT_PORTS.l2}\`
- **Chain ID**: (see deployment summary above)
- **Currency Symbol**: ETH`;
}

function troubleshootingSection({ projectName, dataDir }) {
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

1. **Deployer fails**: Check L1 connectivity and that the deployer account has sufficient ETH.
2. **L2 not producing blocks**: Ensure the deployer exited successfully (\`docker logs ${projectName}-deployer\`).
3. **Explorer shows no data**: Wait 1-2 minutes for Blockscout indexer to catch up.
4. **Port already in use**: Change the port mapping in docker-compose.yaml.`;
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

module.exports = {
  generateAIDeployPrompt,
  CLOUD_PRESETS,
  DEFAULT_PORTS,
};
