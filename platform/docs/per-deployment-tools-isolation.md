# Per-Deployment Tools Isolation Design

> L2 배포별 독립적인 Tools(Dashboard, Bridge, L1 Explorer, L2 Explorer) 실행 및 제어

## 1. Tools 구성요소

| Tool | 설명 | 로컬 L1 | 테스트넷/메인넷 |
|------|------|---------|--------------|
| **Dashboard (Bridge UI)** | 브릿지 입금/출금, 컨트랙트 주소, 네트워크 정보 | 로컬 RPC 사용 | 외부 L1 RPC 사용 |
| **Bridge** | Dashboard 내 브릿지 기능 | 로컬 L1 | 외부 L1 (Sepolia/Mainnet) |
| **L1 Explorer** | Blockscout (L1 블록 탐색기) | Blockscout 컨테이너 | **Etherscan 링크** (컨테이너 불필요) |
| **L2 Explorer** | Blockscout (L2 블록 탐색기) | Blockscout 컨테이너 | Blockscout 컨테이너 |

### 배포 모드별 Tools 컨테이너 구성

```
로컬 L1 배포:
  redis, db, backend-l1, frontend-l1, backend-l2, frontend-l2,
  proxy, selectors, selectors-l2, bridge-ui
  → 12개 컨테이너

테스트넷/메인넷 배포:
  redis, db, backend-l2, frontend-l2,
  proxy-l2-only, selectors-l2, bridge-ui
  → 7개 컨테이너 (L1 Blockscout 제외)
  → L1 Explorer = Etherscan 링크 (https://sepolia.etherscan.io)
```

## 2. 현재 아키텍처 분석

### 2.1 현재 구조

```
┌─────────────────────────────────────────────────┐
│  Manager (Express.js)                           │
│                                                 │
│  Deployment A (tokamak-08cab1ae) — 로컬 L1       │
│  ├─ L1 node  (container: tokamak-08cab1ae-l1)   │  ✅ 격리됨
│  ├─ L2 node  (container: tokamak-08cab1ae-l2)   │  ✅ 격리됨
│  └─ Prover   (container: tokamak-08cab1ae-...)  │  ✅ 격리됨
│                                                 │
│  Deployment B (tokamak-3fa2b1c9) — Sepolia       │
│  ├─ L2 node  (container: tokamak-3fa2b1c9-l2)   │  ✅ 격리됨
│  └─ Prover   (container: tokamak-3fa2b1c9-...)  │  ✅ 격리됨
│                                                 │
│  Tools (SHARED - 전체 배포 공유)                    │
│  ├─ zk-dex-tools-redis         ❌ 하드코딩       │
│  ├─ zk-dex-tools-db            ❌ 하드코딩       │
│  ├─ zk-dex-tools-backend-l1    ❌ 하드코딩       │
│  ├─ zk-dex-tools-frontend-l1   ❌ 하드코딩       │
│  ├─ zk-dex-tools-backend-l2    ❌ 하드코딩       │
│  ├─ zk-dex-tools-frontend-l2   ❌ 하드코딩       │
│  ├─ zk-dex-tools-proxy         ❌ 하드코딩       │
│  ├─ zk-dex-tools-bridge-ui     ❌ 하드코딩       │
│  └─ zk-dex-tools-blockscout-db ❌ 공유 볼륨      │
└─────────────────────────────────────────────────┘
```

### 2.2 문제점

| 문제 | 영향 | 위치 |
|------|------|------|
| **하드코딩된 container_name** | 동시에 하나의 tools 스택만 실행 가능 | `docker-compose-zk-dex-tools.yaml` 전체 |
| **공유 PostgreSQL 볼륨** | Blockscout DB가 모든 L2에서 공유되어 인덱싱 충돌 | `zk-dex-tools-blockscout-db` 볼륨 |
| **Docker Compose 프로젝트 미분리** | `stopTools()`가 모든 배포의 tools를 중단 | `docker-local.js:469-482` |
| **단일 `.zk-dex-deployed.env`** | 마지막 배포의 컨트랙트 주소만 유지 | `docker-local.js:288` |
| **L1 모드 미구분** | 테스트넷에서도 L1 Blockscout를 띄우려 함 | `docker-local.js:startTools()` |

### 2.3 현재 동작 시나리오

```
1. Deployment A가 tools 시작 → ports 8082, 8083, 3000 할당 → 정상 동작
2. Deployment B가 tools 시작 → ports 8084, 8085, 3002 할당
   → .zk-dex-deployed.env 덮어쓰기
   → docker compose up → 기존 컨테이너 재시작
   → A의 explorer/dashboard 접속 불가
3. A의 tools 중지 시도 → B의 tools도 함께 중단
```

## 3. 목표 아키텍처

```
┌──────────────────────────────────────────────────────────┐
│  Manager (Express.js)                                    │
│                                                          │
│  Deployment A (tokamak-08cab1ae) — 로컬 L1                │
│  ├─ L1/L2 nodes, Prover                                  │
│  └─ Tools (project: tokamak-08cab1ae-tools)              │
│     ├─ redis, db              ✅ 격리 (별도 볼륨)          │
│     ├─ blockscout-l1 (4개)    ✅ 로컬 L1이므로 실행        │
│     ├─ blockscout-l2 (4개)    ✅ 격리                     │
│     ├─ proxy                  ✅ 격리                     │
│     └─ bridge-ui              ✅ 격리 (로컬 RPC)           │
│                                                          │
│  Deployment B (tokamak-3fa2b1c9) — Sepolia                │
│  ├─ L2 node, Prover                                      │
│  └─ Tools (project: tokamak-3fa2b1c9-tools)              │
│     ├─ redis, db              ✅ 격리 (별도 볼륨)          │
│     ├─ blockscout-l1          ❌ 미실행 (Etherscan 사용)   │
│     ├─ blockscout-l2 (4개)    ✅ 격리                     │
│     ├─ proxy-l2-only          ✅ L2 전용 프록시             │
│     └─ bridge-ui              ✅ 격리 (외부 L1 RPC)        │
│         L1 Explorer URL = https://sepolia.etherscan.io    │
└──────────────────────────────────────────────────────────┘
```

## 4. 구현 계획

### Phase 1: Tools Compose 템플릿 동적화

**목표**: `container_name` 제거, Docker Compose 프로젝트 격리 활용

#### 4.1 `docker-compose-zk-dex-tools.yaml` 수정

**변경**: 모든 `container_name` 필드 제거, 볼륨명 변경

```yaml
# BEFORE
services:
  redis-db:
    image: 'redis:alpine'
    container_name: zk-dex-tools-redis    # ← 제거
  db:
    container_name: zk-dex-tools-db       # ← 제거

volumes:
  zk-dex-tools-blockscout-db:             # ← 제거

# AFTER
services:
  redis-db:
    image: 'redis:alpine'
    # container_name 없음 → Docker Compose가 {project}-redis-db-1 으로 자동 생성
  db:
    # container_name 없음 → {project}-db-1

volumes:
  blockscout-db:
    # 볼륨명도 {project}_blockscout-db 로 자동 네임스페이스됨
```

**제거할 container_name** (12개):
`zk-dex-tools-redis`, `zk-dex-tools-db-init`, `zk-dex-tools-db`,
`zk-dex-tools-backend-l1`, `zk-dex-tools-frontend-l1`,
`zk-dex-tools-backend-l2`, `zk-dex-tools-frontend-l2`,
`zk-dex-tools-proxy`, `zk-dex-tools-proxy-l2`,
`zk-dex-tools-selectors`, `zk-dex-tools-selectors-l2`,
`zk-dex-tools-bridge-ui`

#### 4.2 env_file 경로 동적화

```yaml
# BEFORE
bridge-ui:
  env_file: .zk-dex-deployed.env

# AFTER
bridge-ui:
  env_file: ${TOOLS_ENV_FILE:-.zk-dex-deployed.env}
```

### Phase 2: Docker Local 함수 격리

**파일**: `crates/desktop-app/local-server/lib/docker-local.js`

모든 tools 함수에 `deploymentId` 파라미터 추가, `-p` 프로젝트 격리 적용.

#### 4.3 `startTools(deploymentId, envVars, toolsPorts, options)` 수정

```javascript
// BEFORE
async function startTools(envVars, toolsPorts) {
  const envFilePath = path.join(repoRoot, 'crates/l2/.zk-dex-deployed.env');
  spawn('docker', ['compose', '-f', toolsCompose, 'up', '-d']);
}

// AFTER
async function startTools(projectName, envVars, toolsPorts = {}) {
  const l2Dir = path.resolve(ETHREX_ROOT, "crates/l2");
  const toolsCompose = path.join(l2Dir, 'docker-compose-zk-dex-tools.yaml');

  // 배포별 .env 파일 (projectName은 이미 `${docker_project}-tools` 형태)
  const envFileName = `.deployed-${projectName}.env`;
  const envFilePath = path.join(l2Dir, envFileName);
  writeEnvFile(envFilePath, envVars);

  const env = {
    ...buildToolsEnv(toolsPorts),
    TOOLS_ENV_FILE: envFilePath,
  };

  // 테스트넷/메인넷: L1 Blockscout 제외, L2 전용 프로필 사용
  const profiles = options.isExternalL1 ? ['external-l1'] : ['default'];

  spawn('docker', [
    'compose',
    '-f', toolsCompose,
    '-p', projectName,
    ...profiles.flatMap(p => ['--profile', p]),
    'up', '-d', '--remove-orphans',
  ], { env: { ...process.env, ...env } });
}
```

#### 4.4 `stopTools(deploymentId)` 수정

```javascript
// BEFORE
async function stopTools() {
  spawn('docker', ['compose', '-f', toolsCompose, 'down', '--remove-orphans']);
}

// AFTER
async function stopTools(deploymentId) {
  const projectName = `${deploymentId}-tools`;
  spawn('docker', [
    'compose', '-f', toolsCompose,
    '-p', projectName,
    'down', '--remove-orphans',
  ]);
}
```

#### 4.5 `getToolsStatus(deploymentId)` 수정

```javascript
// BEFORE
async function getToolsStatus() {
  spawn('docker', ['compose', '-f', toolsCompose, 'ps', '--format', 'json']);
}

// AFTER
async function getToolsStatus(deploymentId) {
  const projectName = `${deploymentId}-tools`;
  spawn('docker', [
    'compose', '-f', toolsCompose,
    '-p', projectName,
    'ps', '--format', 'json',
  ]);
}
```

#### 4.6 `restartTools(deploymentId, envVars, toolsPorts, options)` 수정

동일 패턴 — `deploymentId` + `-p` 프로젝트명 + `options.isExternalL1` 지원.

### Phase 3: 배포 엔진 & 라우트 연동

#### 4.7 `deployment-engine.js` 수정

```javascript
// provision() 내 tools 시작 부분

// BEFORE
await docker.startTools(envVars, { toolsL1ExplorerPort, ... });

// AFTER
const isExternalL1 = deployment.l1_mode === 'external';
await docker.startTools(
  deployment.project_name,
  envVars,
  { toolsL1ExplorerPort, toolsL2ExplorerPort, toolsBridgeUIPort, toolsDbPort, toolsMetricsPort },
  { isExternalL1 }
);
```

#### 4.8 `routes/deployments.js` 수정

```javascript
// BEFORE — tools 제어 시 배포 구분 없음
if (TOOLS_SERVICES.has(serviceName)) {
  await docker.stopTools();
}

// AFTER — 배포별 tools 제어
if (TOOLS_SERVICES.has(serviceName)) {
  await docker.stopTools(deployment.project_name);
}

// 상태 조회도 동일
// BEFORE
const toolsContainers = await docker.getToolsStatus();
// AFTER
const toolsContainers = await docker.getToolsStatus(deployment.project_name);
```

### Phase 4: Dashboard/Bridge의 L1 모드 분기

**파일**: `crates/l2/tooling/bridge/entrypoint.sh`, `dashboard.html`, `index.html`

Dashboard와 Bridge는 이미 `IS_EXTERNAL_L1` 환경변수를 지원하지만, 격리와 함께 동작하도록 확인 필요.

#### 4.9 Bridge UI에서 L1 Explorer 링크 분기

```javascript
// dashboard.html 내 config.json 기반 분기 (이미 구현됨)

// config.json 예시 — 로컬 L1:
{
  "l1_explorer_url": "http://localhost:8083",
  "is_external_l1": false
}

// config.json 예시 — Sepolia:
{
  "l1_explorer_url": "https://sepolia.etherscan.io",
  "is_external_l1": true
}
```

#### 4.10 Manager UI의 L1 Explorer 표시 분기

```javascript
// app.js — tools 상태 표시 시

// 로컬 L1 배포: L1 Explorer = Blockscout URL (http://localhost:{port})
// 테스트넷 배포: L1 Explorer = Etherscan 링크 (외부 URL, 컨테이너 없음)

function renderToolsStatus(deployment) {
  const isExternal = deployment.l1_mode === 'external';

  // L1 Explorer
  if (isExternal) {
    // Etherscan 링크 표시 (컨테이너 상태 무관)
    renderExternalLink('L1 Explorer', deployment.l1_explorer_url);
  } else {
    // Blockscout 컨테이너 상태 표시
    renderToolService('L1 Explorer', `http://localhost:${deployment.tools_l1_explorer_port}`);
  }

  // L2 Explorer — 항상 Blockscout
  renderToolService('L2 Explorer', `http://localhost:${deployment.tools_l2_explorer_port}`);

  // Dashboard/Bridge — 항상 bridge-ui 컨테이너
  renderToolService('Dashboard', `http://localhost:${deployment.tools_bridge_ui_port}`);
}
```

### Phase 5: Docker Compose Profile 활용

`docker-compose-zk-dex-tools.yaml`에서 L1 Blockscout 서비스를 profile로 분리하여, 테스트넷 배포 시 L1 관련 컨테이너를 아예 띄우지 않음.

```yaml
services:
  # L1 Blockscout — 로컬 L1에서만 실행
  backend-l1:
    profiles: ["local-l1"]    # ← profile 추가
    image: ghcr.io/lambdaclass/blockscout-private:9.2.2.commit.763c41da  # pinned version
    ...

  frontend-l1:
    profiles: ["local-l1"]    # ← profile 추가
    ...

  # L2 Blockscout — 항상 실행
  backend-l2:
    # profile 없음 → 항상 실행
    ...

  # proxy (L1+L2) — 로컬 L1에서만
  proxy:
    profiles: ["local-l1"]
    ...

  # proxy-l2-only — 테스트넷에서만
  proxy-l2-only:
    profiles: ["external-l1"]
    ...
```

**docker-local.js에서 profile 선택**:
```javascript
const profiles = isExternalL1
  ? ['--profile', 'external-l1']
  : ['--profile', 'local-l1'];

spawn('docker', [
  'compose', '-f', toolsCompose, '-p', projectName,
  ...profiles,
  'up', '-d',
]);
```

## 5. 수정 파일 목록

| 파일 | 변경 내용 | 난이도 |
|------|----------|--------|
| `crates/l2/docker-compose-zk-dex-tools.yaml` | container_name 12개 제거, 볼륨명 변경, env_file 동적화, profile 추가 | 중간 |
| `crates/desktop-app/local-server/lib/docker-local.js` | 4개 함수에 `deploymentId` + `-p` + profile 지원 | 중간 |
| `crates/desktop-app/local-server/lib/deployment-engine.js` | tools 호출 시 `deployment.project_name` + `isExternalL1` 전달 | 쉬움 |
| `crates/desktop-app/local-server/routes/deployments.js` | tools 라우트에서 `deployment.project_name` 전달 | 쉬움 |
| `crates/desktop-app/local-server/public/app.js` | L1 Explorer 표시 분기 (외부 L1이면 Etherscan 링크) | 쉬움 |

## 6. 리소스 고려사항

### 배포당 Tools 메모리 사용량

| 구성 | 메모리 |
|------|--------|
| L1+L2 Blockscout + Bridge UI (로컬) | ~1.6GB |
| L2 Blockscout + Bridge UI (테스트넷) | ~800MB |
| Bridge UI만 | ~50MB |

### 동시 실행 가능 개수 (16GB RAM 기준)

| 시나리오 | 배포 수 |
|----------|---------|
| Tools 전체 (로컬 L1) | 3~4개 |
| Tools (테스트넷, L2 Blockscout만) | 5~6개 |
| Tools 없이 | 8~10개 |

## 7. 마이그레이션 전략

### 하위 호환성
- 기존 배포는 tools 재시작 시 자동으로 새 격리 모드로 전환
- 기존 `.zk-dex-deployed.env` → 배포별 `.deployed-{id}.env`로 복사 (첫 tools 시작 시)
- 기존 `zk-dex-tools-*` 컨테이너는 수동 정리 안내

### 테스트 계획
1. 로컬 L1 단일 배포 → tools 정상 동작 (L1+L2 Explorer, Bridge)
2. 테스트넷 단일 배포 → L2 Explorer + Bridge만 실행, L1 = Etherscan 링크
3. 로컬 + 테스트넷 동시 → 각각 독립 tools 확인
4. 한 배포 tools stop → 다른 배포 tools 영향 없음 확인
5. tools restart → 해당 배포만 재시작 확인

## 8. 구현 순서

```
Phase 1 (30분): docker-compose-zk-dex-tools.yaml — container_name 제거, profile 추가
Phase 2 (1시간): docker-local.js — 4개 함수 격리 + profile 지원
Phase 3 (30분): deployment-engine.js, routes/deployments.js 연동
Phase 4 (30분): app.js — L1 Explorer 분기, Dashboard/Bridge 확인
Phase 5 (15분): profile 적용 및 테스트넷 모드 검증
테스트 (30분): 2개 배포 동시 tools 실행 검증
```

**총 예상: ~3시간**
