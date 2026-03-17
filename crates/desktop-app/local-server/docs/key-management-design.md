# Key Management Design for L2 Deployments

> Related: [GitHub Issue #45](https://github.com/tokamak-network/ethrex/issues/45) — Move private keys out of docker-compose.yaml

## 1. Overview

L2 배포에는 4개의 키 역할이 필요하다. 배포 경로(Manual vs AI Deploy)와 타겟(Local / Testnet / Mainnet)에 따라 키의 소스와 전달 방식이 다르다.

### 1.1 Key Roles

| Role | Compose Env Var (deployer) | L2 Node CLI Arg | Rust Env Var Fallback | 용도 |
|------|--------------------------|-----------------|----------------------|------|
| **Deployer** | `ETHREX_DEPLOYER_L1_PRIVATE_KEY` | N/A | N/A | L1에 컨트랙트 배포 |
| **Committer** | N/A | `--committer.l1-private-key` | `ETHREX_COMMITTER_L1_PRIVATE_KEY` | L2 배치를 L1에 커밋 |
| **Proof Coordinator** | N/A | `--proof-coordinator.l1-private-key` | `ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY` | L1에 증명 전송 |
| **Bridge Owner** | `ETHREX_BRIDGE_OWNER_PK` | N/A | N/A | 브리지 컨트랙트 소유자 |

- Local/테스트: 모든 역할에 같은 키 사용 가능
- Production: 각 역할별 개별 키 필수
- 키에서 파생되는 **주소(address)** 도 deployer에 전달됨 (비밀 아님, compose에 남아도 됨)

### 1.2 Two Deployment Paths

```
┌─────────────────────────────────────────────────────────────────┐
│                     Manager (macOS)                              │
│                                                                  │
│  ┌──────────┐                                                    │
│  │ Keychain  │──────────────┬──────────────────┐                 │
│  │ (macOS)   │              │                  │                 │
│  └──────────┘              ▼                  ▼                 │
│                   ┌─────────────────┐  ┌──────────────────┐     │
│                   │ Path A: Manual  │  │ Path B: AI Deploy│     │
│                   │ deployment-     │  │ ai-prompt-       │     │
│                   │ engine.js       │  │ generator.js     │     │
│                   └────────┬────────┘  └────────┬─────────┘     │
│                            │                    │               │
│                   compose.yaml           AI Prompt              │
│                   (keys inline!)     (cloud-specific            │
│                                       key instructions)         │
└────────────────────┬────────────────────────────┬───────────────┘
                     │                            │
        ┌────────────┼────────────┐    ┌──────────┼──────────┐
        ▼            ▼            ▼    ▼          ▼          ▼
   ┌────────┐  ┌──────────┐ ┌──────┐ ┌────┐  ┌─────┐  ┌───────┐
   │ Local  │  │ Local    │ │Remote│ │GCP │  │ AWS │  │ Vultr │
   │(L1 in) │  │ Testnet  │ │Testnt│ │    │  │     │  │       │
   └────────┘  └──────────┘ └──────┘ └────┘  └─────┘  └───────┘
   test keys   Keychain     SFTP     Secret  Secrets   SCP
   hardcoded   →compose     upload   Manager Manager   upload
```

## 2. 현재 상태 (As-Is)

### 2.1 Path A: Manual Deploy — 키 노출 현황

키가 compose yaml에 평문으로 삽입되는 위치 (`compose-generator.js`):

| 컨테이너 | 노출 위치 | 노출 방식 | 라인 |
|----------|----------|----------|------|
| **deployer** | `environment:` | `ETHREX_DEPLOYER_L1_PRIVATE_KEY=${deployerPk}` | 595, 797 |
| **deployer** | `environment:` | `ETHREX_BRIDGE_OWNER_PK=${bridgeOwnerPk}` | 609, 810 |
| **l2 node** | `command:` | `--committer.l1-private-key ${committerPk}` | 654, 857 |
| **l2 node** | `command:` | `--proof-coordinator.l1-private-key ${proofCoordinatorPk}` | 655, 858 |

**총 4곳에서 키가 평문 노출**, deployer 컨테이너 env + l2 node CLI args 양쪽 모두.

추가로, `compose-generator.js`에서 **키 → 주소 파생**도 수행한다 (L758-772):
```javascript
const deployerAddress = new ethers.Wallet(deployerPk).address;
const committerAddress = new ethers.Wallet(committerPk).address;
// → ETHREX_BRIDGE_OWNER, ETHREX_DEPLOYER_COMMITTER_L1_ADDRESS 등에 사용
```
주소는 공개 정보이므로 compose에 남아도 된다. 하지만 주소 파생을 위해 키가 compose-generator에 전달되어야 하므로, **키 → 주소 파생은 deployment-engine.js에서 수행**하고 compose-generator에는 주소만 전달하는 것이 맞다.

#### Local (로컬 L1, 하드코딩 테스트 키)

```
하드코딩 키 (0x385c...)  →  compose yaml (env + CLI arg)  →  Docker 컨테이너
```
- **위험도: 낮음** — 테스트 키만 사용, 실제 가치 없음
- 수정 불필요

#### Local Testnet (세폴리아 L1, 로컬 Docker)

```
macOS Keychain
  → deployment-engine.js (getPrivateKey)
    → compose-generator.js (generateLocalTestnetComposeFile)
      → ~/.tokamak/deployments/{id}/docker-compose.yaml
        ├── deployer env: ETHREX_DEPLOYER_L1_PRIVATE_KEY=0x실제키
        ├── deployer env: ETHREX_BRIDGE_OWNER_PK=0x실제키
        ├── l2 command: --committer.l1-private-key 0x실제키
        └── l2 command: --proof-coordinator.l1-private-key 0x실제키
```
- **위험도: 높음** — 실제 세폴리아 ETH가 있는 키가 `~/.tokamak/deployments/` 아래 평문 저장
- `docker inspect`, `docker compose config`, 파일 읽기로 키 노출 가능

#### Remote Testnet (세폴리아 L1, 원격 서버 via SSH)

```
macOS Keychain
  → deployment-engine.js (getPrivateKey)
    → compose-generator.js (generateRemoteTestnetComposeFile)
      → docker-compose.yaml (평문 키 포함!)
        → SFTP 업로드 → /opt/tokamak/{id}/docker-compose.yaml (원격)
          → Docker 컨테이너 (env + CLI arg)
```
- **위험도: 매우 높음** — 실제 키가 네트워크로 전송되고 원격 디스크에 저장
- SSH 접근 가능한 모든 사용자가 키를 읽을 수 있음

### 2.2 Path B: AI Deploy — 키 처리 현황

AI Deploy는 이미 클라우드별 네이티브 시크릿 매니저를 활용한다:

| 클라우드 | 키 저장소 | 전송 방식 | 디스크 노출 | 접근 제어 |
|---------|----------|----------|-----------|----------|
| **Local** | `.env` (chmod 600) | 로컬 파일 | `.env`만 | 파일 권한 |
| **GCP** | Secret Manager | 파이프 (디스크 X) | 없음 | IAM |
| **AWS** | Secrets Manager | CLI arg | 없음 | IAM |
| **Vultr** | `.env` on VM | SCP → rm 로컬 | VM의 `.env`만 | SSH + 파일 권한 |

- GCP: `security ... -w | gcloud secrets create ...` 파이프로 키 전송, VM에서 `gcloud secrets versions access`로 읽기
- AWS: `aws secretsmanager create-secret --secret-string "$KEY"` → VM에서 `get-secret-value`로 읽기
- Vultr: `.env` 파일 SCP 전송 후 로컬 삭제, `chmod 600`

**AI Deploy는 이미 안전한 방식을 사용** → Manual Deploy에도 같은 패턴 적용 필요

### 2.3 보안 문제 요약

| 문제 | Path A (Manual) | Path B (AI) |
|------|---------------|-------------|
| compose yaml에 키 평문 | **모든 testnet/mainnet** | 없음 |
| `docker inspect`로 키 노출 | deployer env에서 노출 | 없음 (`docker inspect` 자체는 AI도 동일) |
| CLI arg에 키 평문 (`ps aux`) | l2 node `--*-private-key` | 없음 |
| 원격 디스크에 키 파일 | SFTP 업로드된 compose | `.env` (chmod 600) |

## 3. 키 관리의 두 가지 문제

키 관리는 **호스트 측 저장**과 **호스트 → 컨테이너 전달** 두 가지 문제로 나뉜다.

### 3.1 문제 1: 호스트 측 키 저장 (OS별 시크릿 저장소)

Manager가 사용자의 프라이빗 키를 어디에 저장하는가?

| OS | 시크릿 저장소 | CLI | 암호화 | 현재 지원 |
|---|---|---|---|---|
| **macOS** | Keychain | `security find-generic-password` | AES-256 (Secure Enclave) | **구현됨** (`keychain.js`) |
| **Windows** | Credential Manager (DPAPI) | `cmdkey` / PowerShell `Get-Credential` | DPAPI (사용자 로그인 키 기반) | 미구현 |
| **Linux** | `gnome-keyring` / `kwallet` / `pass` | `secret-tool` / `kwallet-query` / `pass` | 다양 (GPG, AES 등) | 미구현 |

현재는 macOS Keychain만 지원한다. Windows/Linux 지원은 크로스플랫폼 확장 시 필요하며, 각 OS별 네이티브 시크릿 저장소를 사용하는 것이 원칙이다.

**대안: 통합 시크릿 관리**

OS별 네이티브 저장소 대신 통합 방식을 사용할 수도 있다:

| 방식 | 장점 | 단점 |
|------|------|------|
| 암호화된 키 파일 (AES-256 + 사용자 비밀번호) | 크로스플랫폼, 구현 간단 | 비밀번호 관리 부담 |
| OS keyring 라이브러리 (e.g. `keytar`) | 네이티브 통합, 단일 API | 라이브러리 의존성, 플랫폼별 빌드 |
| 브라우저 기반 Web Crypto API | UI에서 직접 관리 | 브라우저 닫으면 소멸, 영속성 문제 |

### 3.2 문제 2: 호스트 → Docker 컨테이너 키 전달

OS 시크릿 저장소에서 읽은 키를 Docker 컨테이너에 전달하는 방법:

```
┌──────────────────┐     전달 방식?      ┌─────────────────┐
│  호스트           │  ──────────────▶   │  Docker 컨테이너  │
│  (Keychain 등)   │                    │  (deployer, l2)  │
└──────────────────┘                    └─────────────────┘
```

| 전달 방식 | compose에 키 노출 | 호스트 디스크 | `docker inspect` | `ps aux` |
|----------|----------------|-------------|-----------------|---------|
| **환경변수** (`environment:`) | **예** | compose 파일 | 보임 | 안 보임 |
| **CLI arg** (`command:`) | **예** | compose 파일 | 안 보임 | **보임** |
| **compose 변수 치환** (`${VAR}`) | **아니오** (변수명만) | `.env` 파일 | 보임 (주입 후) | 안 보임 |
| **env_file** 디렉티브 | **아니오** | `.keys.env` 파일 | 보임 (주입 후) | 안 보임 |
| **Docker secrets** (Swarm) | **아니오** | **없음** (tmpfs) | 안 보임 | 안 보임 |
| **Docker secrets** (Compose) | **아니오** | secret 파일 (bind mount) | 안 보임 | 안 보임 |

**핵심**: Docker 컨테이너는 호스트의 macOS Keychain에 직접 접근할 수 없다. 어떤 방식이든 호스트에서 읽어서 전달해야 한다. 문제는 그 **전달 과정에서 키가 compose yaml 또는 디스크에 남는가**이다.

> **주의**: Docker Compose의 `secrets`는 Swarm 모드에서만 tmpfs로 마운트된다. 일반 Compose에서는 **bind mount**이므로 호스트 디스크에 secret 파일이 존재해야 한다. 다만, compose yaml 자체에는 키 값이 들어가지 않고, `docker inspect`에도 노출되지 않는다.

**선택: compose 변수 치환 방식 (`--env-file`)**

`env_file` 디렉티브보다 **compose 변수 치환 + `docker compose --env-file`** 방식이 더 적합하다:

```yaml
# compose yaml에는 변수명만 (키 값 없음)
services:
  tokamak-app-deployer:
    environment:
      - ETHREX_DEPLOYER_L1_PRIVATE_KEY=${ETHREX_DEPLOYER_L1_PRIVATE_KEY}
```

```bash
# 실행 시 .keys.env에서 변수 값 주입
docker compose --env-file .keys.env up -d
```

이유:
- compose yaml에 키 값이 **절대** 들어가지 않음 (변수명만)
- `.keys.env` 파일은 `chmod 600`으로 보호
- `env_file` 디렉티브와 달리, compose-level 변수 치환이므로 모든 서비스에 선택적으로 적용 가능
- AI Deploy의 Local 모드와 동일한 패턴

### 3.3 타겟별 전략

| 타겟 | 호스트 키 저장 | 컨테이너 전달 | 우선순위 |
|------|-------------|-------------|---------|
| **Local (테스트 L1)** | 하드코딩 테스트 키 | compose env (현행 유지) | 변경 불필요 |
| **Local Testnet (세폴리아)** | OS Keychain | compose 변수 치환 + `.keys.env` | 높음 |
| **Remote Testnet (세폴리아)** | OS Keychain | `.keys.env` SFTP 분리 업로드 | 높음 |
| **Mainnet** | OS Keychain | Docker secrets (Compose) + 외부 시크릿 매니저 | 최우선 |

## 4. 개선 설계 (To-Be)

### 4.1 Phase 1: compose 변수 치환 + `.keys.env` 분리 (Testnet 즉시 적용)

compose yaml에서 키 값을 제거하고, `${변수}` 치환 + `.keys.env` 파일로 분리한다.

#### 4.1.1 compose-generator.js 변경

**Before** — 키 값이 yaml에 직접 삽입:
```yaml
services:
  tokamak-app-deployer:
    environment:
      - ETHREX_DEPLOYER_L1_PRIVATE_KEY=0xREAL_KEY_HERE           # ← 평문!
      - ETHREX_BRIDGE_OWNER_PK=0xREAL_KEY_HERE                   # ← 평문!
  tokamak-app-l2:
    command: >
      --committer.l1-private-key 0xREAL_KEY_HERE                 # ← 평문!
      --proof-coordinator.l1-private-key 0xREAL_KEY_HERE          # ← 평문!
```

**After** — 변수명만 사용, CLI arg는 env var로 대체:
```yaml
services:
  tokamak-app-deployer:
    environment:
      - ETHREX_DEPLOYER_L1_PRIVATE_KEY=${ETHREX_DEPLOYER_L1_PRIVATE_KEY}
      - ETHREX_BRIDGE_OWNER_PK=${ETHREX_BRIDGE_OWNER_PK}
      - ETHREX_BRIDGE_OWNER=${ETHREX_BRIDGE_OWNER}                # 주소 (공개)
      - ETHREX_ON_CHAIN_PROPOSER_OWNER=${ETHREX_BRIDGE_OWNER}     # 주소 (공개)
      - ETHREX_DEPLOYER_COMMITTER_L1_ADDRESS=${COMMITTER_ADDRESS}  # 주소 (공개)
      - ETHREX_DEPLOYER_PROOF_SENDER_L1_ADDRESS=${PROOF_COORDINATOR_ADDRESS}
  tokamak-app-l2:
    environment:
      - ETHREX_COMMITTER_L1_PRIVATE_KEY=${ETHREX_COMMITTER_L1_PRIVATE_KEY}
      - ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY=${ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY}
    command: >
      --network ...
      --http.addr 0.0.0.0
      # CLI arg에서 키 제거 — env var로 대체 (Rust clap이 이미 지원)
```

Rust CLI는 이미 env var fallback을 지원 (`options.rs`):
- `--committer.l1-private-key` → `env = "ETHREX_COMMITTER_L1_PRIVATE_KEY"` (line 576)
- `--proof-coordinator.l1-private-key` → `env = "ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY"` (line 705)

CLI arg에서 키를 제거해도 env var로 전달되므로 Rust 코드 변경 불필요.

#### 4.1.2 주소 파생 위치 변경

**Before** — `compose-generator.js`에서 키 → 주소 파생:
```javascript
// compose-generator.js (line 762-772)
const deployerAddress = new ethers.Wallet(deployerPk).address;
const committerAddress = new ethers.Wallet(committerPk).address;
```

**After** — `deployment-engine.js`에서 주소 파생 후 전달:
```javascript
// deployment-engine.js
const { ethers } = require("ethers");
const deployerAddress = new ethers.Wallet(deployerPk).address;
const committerAddress = new ethers.Wallet(committerPk).address;
const proofCoordinatorAddress = new ethers.Wallet(proofCoordinatorPk).address;
const bridgeOwnerAddress = new ethers.Wallet(bridgeOwnerPk).address;

// compose-generator에는 주소만 전달 (키 전달하지 않음)
const { composeYaml } = generateLocalTestnetComposeFile({
  deployerAddress, committerAddress, proofCoordinatorAddress, bridgeOwnerAddress,
  // deployerPk, committerPk 등은 전달하지 않음!
  ...otherParams,
});

// 키는 .keys.env로 분리
const keysEnv = buildKeysEnvContent({
  deployerPk, committerPk, proofCoordinatorPk, bridgeOwnerPk,
  deployerAddress, committerAddress, proofCoordinatorAddress, bridgeOwnerAddress,
});
```

이렇게 하면 **compose-generator가 프라이빗 키를 아예 모르게** 된다.

#### 4.1.3 `.keys.env` 파일 형식

```env
# Auto-generated by Tokamak Manager — DO NOT COMMIT
# Deployment: {id}
ETHREX_DEPLOYER_L1_PRIVATE_KEY=0x...
ETHREX_COMMITTER_L1_PRIVATE_KEY=0x...
ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY=0x...
ETHREX_BRIDGE_OWNER_PK=0x...
ETHREX_BRIDGE_OWNER=0x...address...
ETHREX_ON_CHAIN_PROPOSER_OWNER=0x...address...
ETHREX_DEPLOYER_SEQUENCER_REGISTRY_OWNER=0x...address...
COMMITTER_ADDRESS=0x...address...
PROOF_COORDINATOR_ADDRESS=0x...address...
```

> 주소(address)도 `.keys.env`에 포함시키는 이유: compose yaml의 변수 치환에서 사용하기 위함. 주소는 비밀이 아니지만, compose yaml에 하드코딩하는 것보다 `.keys.env`에서 통합 관리하는 것이 깔끔함.

#### 4.1.4 파일 저장 위치

| 파일 | 로컬 배포 경로 | 원격 배포 경로 |
|------|-------------|-------------|
| `docker-compose.yaml` | `~/.tokamak/deployments/{id}/` | `/opt/tokamak/{id}/` |
| `.keys.env` | `~/.tokamak/deployments/{id}/` (chmod 600) | `/opt/tokamak/{id}/` (chmod 600) |

#### 4.1.5 deployment-engine.js 변경

```javascript
// writeKeysEnvFile 함수 추가
function writeKeysEnvFile(deploymentId, keysEnvContent, customDir) {
  const deployDir = getDeploymentDir(deploymentId, customDir);
  const keysPath = path.join(deployDir, ".keys.env");
  fs.writeFileSync(keysPath, keysEnvContent, { mode: 0o600 });
  return keysPath;
}

// buildKeysEnvContent 함수 추가
function buildKeysEnvContent({ deployerPk, committerPk, proofCoordinatorPk, bridgeOwnerPk,
                                deployerAddress, committerAddress, proofCoordinatorAddress, bridgeOwnerAddress }) {
  return [
    "# Auto-generated by Tokamak Manager — DO NOT COMMIT",
    `ETHREX_DEPLOYER_L1_PRIVATE_KEY=${deployerPk}`,
    `ETHREX_COMMITTER_L1_PRIVATE_KEY=${committerPk}`,
    `ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY=${proofCoordinatorPk}`,
    `ETHREX_BRIDGE_OWNER_PK=${bridgeOwnerPk}`,
    `ETHREX_BRIDGE_OWNER=${bridgeOwnerAddress}`,
    `ETHREX_ON_CHAIN_PROPOSER_OWNER=${bridgeOwnerAddress}`,
    `ETHREX_DEPLOYER_SEQUENCER_REGISTRY_OWNER=${deployerAddress}`,
    `COMMITTER_ADDRESS=${committerAddress}`,
    `PROOF_COORDINATOR_ADDRESS=${proofCoordinatorAddress}`,
    "",
  ].join("\n");
}
```

#### 4.1.6 docker-local.js 변경

```javascript
// docker compose 실행 시 --env-file 추가
// Before
execSync(`docker compose -f ${composeFile} -p ${projectName} up -d`);

// After
const keysFile = path.join(path.dirname(composeFile), ".keys.env");
const envFileFlag = fs.existsSync(keysFile) ? `--env-file ${keysFile}` : "";
execSync(`docker compose ${envFileFlag} -f ${composeFile} -p ${projectName} up -d`);
```

> `.keys.env`가 없으면 (Local 테스트 L1) 기존처럼 동작. 있으면 (Testnet) 변수 치환.

#### 4.1.7 docker-remote.js 변경

```javascript
// Before: compose yaml에 키 포함된 채 업로드
await uploadFile(conn, composeContent, `${remoteDir}/docker-compose.yaml`);
await exec(conn, `cd ${remoteDir} && docker compose -p ${projectName} up -d`);

// After: compose와 키 파일 분리 업로드
await uploadFile(conn, composeYaml, `${remoteDir}/docker-compose.yaml`);
await uploadFile(conn, keysEnvContent, `${remoteDir}/.keys.env`);
await exec(conn, `chmod 600 ${remoteDir}/.keys.env`);
await exec(conn, `cd ${remoteDir} && docker compose --env-file .keys.env -p ${projectName} up -d`);
```

#### 4.1.8 변경 범위 요약

| 파일 | 변경 내용 | 난이도 |
|------|----------|--------|
| `compose-generator.js` | testnet 함수: 키 → `${변수}`, CLI arg → env, 키 파라미터 → 주소 파라미터 | 중 |
| `deployment-engine.js` | 주소 파생 추가, `writeKeysEnvFile()` + `buildKeysEnvContent()` 추가, provision 함수 수정 | 중 |
| `docker-local.js` | `docker compose` 실행 시 `--env-file` 조건부 추가 | 낮 |
| `docker-remote.js` | `.keys.env` 분리 업로드 + `chmod 600` + `--env-file` 추가 | 낮 |
| Rust (`options.rs`) | **변경 없음** (이미 env var fallback 지원) | 없음 |
| `.gitignore` | `.keys.env` 추가 | 낮 |

**변경하지 않는 것**:
- Local (테스트 L1): 하드코딩 테스트 키이므로 변경 불필요 — `generateComposeFile()`은 그대로
- AI Deploy: 이미 `.env` 분리 방식 사용 중 — `ai-prompt-generator.js` 변경 없음
- Rust 코드: env var fallback 이미 지원 — `options.rs`, `deployer.rs` 변경 없음

#### 4.1.9 검증 방법

1. **로컬 Testnet 배포 테스트**:
   - `~/.tokamak/deployments/{id}/docker-compose.yaml`에 프라이빗 키가 없는지 확인
   - `~/.tokamak/deployments/{id}/.keys.env`에 키가 있고 `chmod 600`인지 확인
   - `docker inspect {container}` env에 키가 주입되었는지 확인 (정상 동작)
   - 실제 deployer → l2 node → prover 파이프라인이 정상 동작하는지 확인

2. **원격 Testnet 배포 테스트**:
   - 원격 `/opt/tokamak/{id}/docker-compose.yaml`에 키 없는지 확인
   - 원격 `/opt/tokamak/{id}/.keys.env`에 키 있고 `chmod 600`인지 확인
   - 전체 배포 파이프라인 정상 동작 확인

3. **기존 Local (테스트 L1) 배포 회귀 테스트**:
   - `.keys.env` 없이도 기존처럼 정상 동작하는지 확인

#### 4.1.10 하위 호환성

- 기존 배포(이미 실행 중인 것)는 영향 없음 — compose yaml이 이미 생성된 상태
- 신규 배포부터 적용
- `docker-local.js`의 `--env-file` 조건부 적용으로 기존 Local 배포도 정상 동작

### 4.2 Phase 2: Docker Secrets (Mainnet 준비)

`.keys.env` 파일도 결국 디스크에 평문이다. Docker Compose secrets를 사용하면 compose yaml에서 키를 완전히 분리하고, `docker inspect`에서도 노출되지 않는다.

> **주의**: Docker Compose (비-Swarm)에서 secrets는 **bind mount**로 동작한다. tmpfs가 아니므로 호스트 디스크에 secret 파일이 존재해야 한다. 다만, compose yaml과 `docker inspect` 양쪽 모두에서 키 값이 보이지 않는다는 장점이 있다. 진정한 tmpfs 기반 secrets는 Docker Swarm 모드에서만 가능하다.

```yaml
secrets:
  deployer_key:
    file: ./.secrets/deployer.key
  committer_key:
    file: ./.secrets/committer.key
  proof_coordinator_key:
    file: ./.secrets/proof_coordinator.key
  bridge_owner_key:
    file: ./.secrets/bridge_owner.key

services:
  tokamak-app-deployer:
    secrets:
      - deployer_key
      - bridge_owner_key
    environment:
      - ETHREX_DEPLOYER_L1_PRIVATE_KEY_FILE=/run/secrets/deployer_key
      - ETHREX_BRIDGE_OWNER_PK_FILE=/run/secrets/bridge_owner_key

  tokamak-app-l2:
    secrets:
      - committer_key
      - proof_coordinator_key
    environment:
      - ETHREX_COMMITTER_L1_PRIVATE_KEY_FILE=/run/secrets/committer_key
      - ETHREX_PROOF_COORDINATOR_L1_PRIVATE_KEY_FILE=/run/secrets/proof_coordinator_key
```

**Rust 변경 필요**:
- `cmd/ethrex/l2/options.rs`: `_FILE` suffix 규약 지원 추가
- 환경변수 이름이 `_FILE`로 끝나면 해당 파일에서 키를 읽도록 수정

```rust
// 예시: _FILE env var 지원
fn resolve_private_key(env_name: &str) -> Option<SecretKey> {
    // 1. 직접 값 체크
    if let Ok(val) = std::env::var(env_name) {
        return parse_private_key(&val);
    }
    // 2. _FILE suffix 체크
    if let Ok(path) = std::env::var(format!("{env_name}_FILE")) {
        let content = std::fs::read_to_string(path).ok()?;
        return parse_private_key(content.trim());
    }
    None
}
```

### 4.3 Phase 3: 외부 시크릿 매니저 (Mainnet)

메인넷 배포 시에는 클라우드 네이티브 시크릿 매니저 직접 연동:

- **GCP Secret Manager**: `gcloud secrets versions access`
- **AWS Secrets Manager**: `aws secretsmanager get-secret-value`
- **HashiCorp Vault**: `vault kv get`

AI Deploy에는 이미 GCP/AWS 연동 프롬프트가 구현되어 있으므로, Manual Deploy에도 동일 패턴을 적용한다.

## 5. 개발 계획

### Phase 1: compose 변수 치환 + `.keys.env` 분리 (즉시)

**대상**: Local Testnet + Remote Testnet (Manual Deploy)
**목표**: compose yaml에서 모든 프라이빗 키 값 제거

| 순서 | 작업 | 파일 |
|------|------|------|
| 1 | `generateLocalTestnetComposeFile()` — 키 파라미터 → 주소 파라미터로 변경, 키 값 → `${변수}` 치환, CLI arg에서 키 제거 | `compose-generator.js` |
| 2 | `generateRemoteTestnetComposeFile()` — 동일 적용 | `compose-generator.js` |
| 3 | `buildKeysEnvContent()`, `writeKeysEnvFile()` 함수 추가 | `deployment-engine.js` |
| 4 | `provisionLocalTestnet()` — 주소 파생 + keysEnv 생성 + writeKeysEnvFile | `deployment-engine.js` |
| 5 | `provisionRemoteTestnet()` — 주소 파생 + keysEnv 생성 + 분리 업로드 | `deployment-engine.js` |
| 6 | `docker compose` 실행 시 `--env-file .keys.env` 조건부 추가 | `docker-local.js` |
| 7 | 원격 `.keys.env` 분리 업로드 + chmod 600 + `--env-file` 추가 | `docker-remote.js` |
| 8 | `.keys.env` 패턴 추가 | `.gitignore` |

### Phase 2: Docker Secrets (메인넷 준비)

**대상**: 모든 Testnet + Mainnet
**목표**: `docker inspect`에서도 키 노출 차단

| 순서 | 작업 | 파일 |
|------|------|------|
| 1 | Rust `_FILE` env var 규약 구현 | `cmd/ethrex/l2/options.rs` |
| 2 | compose template에 `secrets:` 섹션 추가 | `compose-generator.js` |
| 3 | deployment-engine에서 `.secrets/` 디렉토리에 개별 키 파일 생성 | `deployment-engine.js` |
| 4 | Remote에서 secrets 파일 분리 업로드 | `docker-remote.js` |

### Phase 3: 외부 시크릿 매니저 (메인넷)

**대상**: Mainnet
**목표**: 키가 로컬/원격 디스크에 절대 존재하지 않음

| 순서 | 작업 | 파일 |
|------|------|------|
| 1 | Rust에 시크릿 매니저 SDK 연동 (GCP/AWS) | `cmd/ethrex/l2/` 새 모듈 |
| 2 | compose template에 시크릿 매니저 env 지원 | `compose-generator.js` |
| 3 | Manager UI에 시크릿 매니저 설정 UI | `public/app.js`, `index.html` |

## 6. File Reference

| 파일 | 역할 | Phase 1 변경 |
|------|------|-------------|
| `compose-generator.js` | compose yaml 생성, 현재 키 평문 삽입 | 키 → `${변수}`, CLI arg → env, 키 파라미터 제거 |
| `deployment-engine.js` | 배포 오케스트레이션, Keychain에서 키 읽기 | 주소 파생, `writeKeysEnvFile()` + `buildKeysEnvContent()` 추가 |
| `docker-local.js` | 로컬 Docker 조작 | `--env-file .keys.env` 조건부 추가 |
| `docker-remote.js` | SSH로 원격 Docker 조작 | `.keys.env` 분리 업로드 + `--env-file` |
| `keychain.js` | macOS Keychain 읽기/쓰기 | 변경 없음 |
| `ai-prompt-generator.js` | AI 프롬프트 생성 (이미 안전) | 변경 없음 |
| `cmd/ethrex/l2/options.rs` | Rust CLI 옵션 (env var fallback 지원) | Phase 1 변경 없음, Phase 2에서 `_FILE` 지원 |
| `cmd/ethrex/l2/deployer.rs` | Rust deployer | 변경 없음 |
