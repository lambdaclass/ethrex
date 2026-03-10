# 외부 도메인/IP 접속 지원 설계

배포 완료된 L2에 외부 도메인 또는 공인 IP를 설정하여, 외부 사용자가 Dashboard, Bridge, Explorer, L2 RPC 등에 접속할 수 있도록 한다.

## 1. 현재 상태 분석

### 1.1 포트 바인딩

| 서비스 | 로컬 모드 | 리모트 모드 | 테스트넷 모드 |
|--------|-----------|-------------|---------------|
| L1 RPC | `127.0.0.1:${l1Port}:8545` | `0.0.0.0:${l1Port}:8545` | 없음 (외부 L1) |
| L2 RPC | `127.0.0.1:${l2Port}:1729` | `0.0.0.0:${l2Port}:1729` | `127.0.0.1:${l2Port}:1729` |
| L2 Explorer | `0.0.0.0:${port}:8082` (Docker 기본) | 동일 | 동일 |
| L1 Explorer | `0.0.0.0:${port}:8083` (Docker 기본) | 동일 | 없음 |
| Bridge UI | `0.0.0.0:${port}:80` (Docker 기본) | 동일 | 동일 |

**문제**: Tools(Explorer, Bridge, Dashboard)의 Docker 포트는 이미 `0.0.0.0`으로 열려있지만, **내부 URL이 모두 `localhost`로 하드코딩**되어 외부에서 접속하면 동작하지 않음.

### 1.2 localhost 하드코딩 위치

#### `entrypoint.sh` — config.json 생성
```
l2_rpc: "http://localhost:${TOOLS_L2_RPC_PORT:-1729}"
l2_explorer: "http://localhost:${TOOLS_L2_EXPLORER_PORT:-8082}"
metrics_url: "http://localhost:${TOOLS_METRICS_PORT:-3702}/metrics"
```

#### `docker-compose-zk-dex-tools.yaml` — Blockscout 프론트엔드
```
NEXT_PUBLIC_API_HOST: localhost:${TOOLS_L2_EXPLORER_PORT:-8082}
NEXT_PUBLIC_APP_HOST: localhost:${TOOLS_L2_EXPLORER_PORT:-8082}
NEXT_PUBLIC_API_PROTOCOL: http                  # ← 하드코딩
NEXT_PUBLIC_API_WEBSOCKET_PROTOCOL: ws          # ← 하드코딩
NEXT_PUBLIC_API_HOST: localhost:${TOOLS_L1_EXPLORER_PORT:-8083}
NEXT_PUBLIC_APP_HOST: localhost:${TOOLS_L1_EXPLORER_PORT:-8083}
```

#### Nginx proxy — `server_name`
```
server_name localhost;       # 3곳: proxy, proxy-l2-only, bridge-ui
```
외부 도메인/IP로 접속하면 nginx가 Host 헤더 불일치로 거부.

#### HTML 파일들 — 기본 href
- `dashboard.html`: 탐색기/메트릭 링크 `http://localhost:8083`, `http://localhost:8082`
  - **추가**: JS에서 `l1Link.textContent = 'localhost:' + port` 형태로 표시 텍스트도 하드코딩 (line 762, 772, 780, 789, 828)
- `index.html`: 탐색기 링크, RPC 상수 `http://localhost:8545`, `http://localhost:1729`
- `withdraw-status.html`: 탐색기 링크, provider 초기값 `http://localhost:8545/1729`

#### `deployment-engine.js` — 엔드포인트 URL
```javascript
l2Rpc: `http://127.0.0.1:${l2Port}`  // 테스트넷 포함
```

#### `compose-generator.js` — 메인 컴포즈 포트
```javascript
`127.0.0.1:${l2Port}:1729`  // 로컬 + 테스트넷 모드
```
**참고**: `generateComposeFile()`과 `generateTestnetComposeFile()` 모두 `isPublic` 파라미터 미지원.

### 1.3 이미 존재하는 인프라

- DB `schema.sql`: `is_public INTEGER DEFAULT 0` 컬럼 (미사용)
- `entrypoint.sh`: `PUBLIC_BASE_URL` 환경변수 분기 (부분 구현)
- `entrypoint.sh`: nginx L1 RPC 프록시 + rate limiting (public 모드)
- `docker-local.js`: `buildToolsEnv()` 환경변수 파이프라인
- `tools-config.js`: `getExternalL1Config()` 외부 L1 설정 추출

### 1.4 Internal vs External URL 구분 (신규)

컨테이너 간 통신(Internal)과 브라우저 접속(External)을 구분해야 함:

| 구분 | 예시 | 용도 |
|------|------|------|
| **Internal** | `http://host.docker.internal:8545` | Blockscout backend → L1 RPC |
| **Internal** | `http://backend-l2:4000` | Blockscout frontend → backend API |
| **External** | `http://203.0.113.50:8082` | 사용자 브라우저 → L2 Explorer |
| **External** | `http://203.0.113.50:1729` | MetaMask → L2 RPC |

**핵심**: Internal URL은 변경 불필요. `NEXT_PUBLIC_*` 같은 **브라우저에서 사용하는 URL만** 외부 주소로 변경.

## 2. 목표 아키텍처

### 2.1 사용자 시나리오

1. **로컬 배포** → 기본 `localhost` 모드 (현재와 동일)
2. **배포 후 공개 전환** → Manager 상세페이지에서 도메인/IP 설정 → tools 재시작 → 외부 접속 가능
3. **처음부터 공개 배포** → 배포 설정 시 도메인/IP 입력 → 바로 공개 모드로 시작

### 2.2 설정 UI (Manager 상세페이지)

배포 상세페이지 Overview 탭에 "External Access" 섹션 추가:

```
┌─────────────────────────────────────────────────────┐
│ External Access                          [Disabled] │
├─────────────────────────────────────────────────────┤
│                                                     │
│ Public Domain/IP:  [________________________]       │
│                    예: l2.example.com 또는 IP        │
│                                                     │
│ ▸ Advanced URL Settings (선택)                      │
│   L2 RPC URL:      [자동: http://domain:1729  ]     │
│   L2 Explorer URL: [자동: http://domain:8082  ]     │
│   L1 Explorer URL: [자동: http://domain:8083  ]     │
│   Dashboard URL:   [자동: http://domain:3000  ]     │
│                                                     │
│              [Enable Public Access]                  │
│                                                     │
└─────────────────────────────────────────────────────┘
```

공개 모드 활성화 후:
```
┌─────────────────────────────────────────────────────┐
│ External Access                           [Enabled] │
├─────────────────────────────────────────────────────┤
│ Public Domain: l2.example.com                       │
│                                                     │
│ External URLs:                                      │
│  Dashboard:   http://l2.example.com:3000   [Copy]   │
│  Bridge:      http://l2.example.com:3000/… [Copy]   │
│  L2 Explorer: http://l2.example.com:8082   [Copy]   │
│  L2 RPC:      http://l2.example.com:1729   [Copy]   │
│  L1 Explorer: http://l2.example.com:8083   [Copy]   │
│                                                     │
│    [Edit Settings]  [Disable Public Access]         │
└─────────────────────────────────────────────────────┘
```

### 2.3 설정 필드

| 필드 | 설명 | 예시 |
|------|------|------|
| Public Domain/IP | 외부 접속 기본 주소 | `l2.example.com` 또는 `203.0.113.50` |
| L2 RPC URL (외부) | 외부에서 접근 가능한 L2 RPC | `https://l2.example.com/rpc` 또는 `http://203.0.113.50:1729` |
| L2 Explorer URL (외부) | 외부 접근 L2 탐색기 | `https://l2.example.com/explorer` 또는 `http://203.0.113.50:8082` |
| L1 Explorer URL (외부) | 외부 접근 L1 탐색기 (로컬 L1일 때만) | `http://203.0.113.50:8083` |
| Dashboard URL (외부) | 외부 접근 대시보드 | `https://l2.example.com` 또는 `http://203.0.113.50:3000` |

**간소화 옵션**: `Public Domain/IP`만 입력하면 나머지는 자동 계산:
```
Public Base = http://{domain_or_ip}
L2 RPC      = http://{domain_or_ip}:{l2Port}
L2 Explorer = http://{domain_or_ip}:{explorerPort}
L1 Explorer = http://{domain_or_ip}:{l1ExplorerPort}  (로컬 L1일 때만)
Dashboard   = http://{domain_or_ip}:{bridgeUiPort}
Bridge      = http://{domain_or_ip}:{bridgeUiPort}/bridge.html
```

HTTPS + 리버스 프록시(nginx/Caddy) 사용 시에는 개별 URL 커스텀 입력.

### 2.4 데이터 흐름

```
Manager UI (상세페이지)
  ↓ POST /api/deployments/:id/public-access
API → DB 저장 (public_domain, public_urls, is_public=1)
  ↓
compose-generator.js → 메인 컴포즈 재생성 (0.0.0.0 바인딩)
  ↓
L2 서비스 재시작 (새 포트 바인딩)
  ↓
buildToolsEnv() → PUBLIC_* 환경변수 생성
  ↓
Docker Compose 재시작 (tools)
  ├─ entrypoint.sh → config.json 재생성 (외부 URL)
  ├─ Blockscout frontends → 새 NEXT_PUBLIC_*_HOST/PROTOCOL 값
  ├─ nginx → server_name _ (모든 Host 허용)
  └─ config.json 캐시 방지 헤더
  ↓
외부 접속 가능
```

## 3. 수정 계획

### Phase 1: DB + API (설정 저장)

#### 3.1 `schema.sql` — 새 컬럼 추가
```sql
-- 기존
is_public INTEGER DEFAULT 0,
-- 추가
public_domain TEXT,          -- 'l2.example.com' 또는 '203.0.113.50'
public_l2_rpc_url TEXT,      -- 커스텀 L2 RPC URL (null이면 자동 계산)
public_l2_explorer_url TEXT, -- 커스텀 L2 Explorer URL
public_l1_explorer_url TEXT, -- 커스텀 L1 Explorer URL (로컬 L1용)
public_dashboard_url TEXT,   -- 커스텀 Dashboard URL
```

#### 3.2 `deployments.js` — 허용 필드 추가
`allowedFields`에 `public_domain`, `public_l2_rpc_url`, `public_l2_explorer_url`, `public_l1_explorer_url`, `public_dashboard_url` 추가.

#### 3.3 `routes/deployments.js` — 새 엔드포인트

```javascript
// POST /api/deployments/:id/public-access
// Body: { publicDomain, l2RpcUrl?, l2ExplorerUrl?, l1ExplorerUrl?, dashboardUrl? }
router.post("/:id/public-access", async (req, res) => {
  const deployment = db.prepare("SELECT * FROM deployments WHERE id = ?").get(req.params.id);
  if (!deployment) return res.status(404).json({ error: "Deployment not found" });
  if (!deployment.docker_project) return res.status(400).json({ error: "Not provisioned yet" });

  const { publicDomain } = req.body;
  if (!publicDomain) return res.status(400).json({ error: "publicDomain is required" });

  // 1. DB에 저장
  updateDeployment(deployment.id, {
    is_public: 1,
    public_domain: publicDomain,
    public_l2_rpc_url: req.body.l2RpcUrl || null,
    public_l2_explorer_url: req.body.l2ExplorerUrl || null,
    public_l1_explorer_url: req.body.l1ExplorerUrl || null,
    public_dashboard_url: req.body.dashboardUrl || null,
  });

  // 2. 메인 컴포즈 재생성 + L2 재시작 + Tools 재시작
  // (비동기 — 30초 이상 소요 가능)
  res.json({ ok: true, message: "Enabling public access..." });
  enablePublicAccess(deployment).catch(e => {
    console.error("Public access enable failed:", e.message);
  });
});

// DELETE /api/deployments/:id/public-access
// 공개 모드 해제 → localhost 모드로 복귀
router.delete("/:id/public-access", async (req, res) => {
  // 1. public_* 필드 null로
  // 2. is_public = 0
  // 3. 컴포즈 재생성 (127.0.0.1) + 재시작
});
```

### Phase 2: 환경변수 파이프라인

#### 3.4 `docker-local.js` — `buildToolsEnv()` 확장

```javascript
function buildToolsEnv(toolsPorts) {
  const env = { /* 기존 포트 변수들 */ };
  // 기존 외부 L1 설정
  if (toolsPorts.l1RpcUrl) env.L1_RPC_URL = toolsPorts.l1RpcUrl;
  // ...

  // 새로 추가: 외부 접속 설정
  if (toolsPorts.publicDomain) {
    env.PUBLIC_DOMAIN = toolsPorts.publicDomain;
    env.PUBLIC_BASE_URL = toolsPorts.publicBaseUrl;    // http://l2.example.com
    env.PUBLIC_L2_RPC_URL = toolsPorts.publicL2RpcUrl;
    env.PUBLIC_L2_EXPLORER_URL = toolsPorts.publicL2ExplorerUrl;
    env.PUBLIC_L1_EXPLORER_URL = toolsPorts.publicL1ExplorerUrl;
    env.PUBLIC_DASHBOARD_URL = toolsPorts.publicDashboardUrl;

    // Blockscout 프론트엔드용 HOST + PROTOCOL 분리
    if (toolsPorts.publicL2ExplorerUrl) {
      try {
        const url = new URL(toolsPorts.publicL2ExplorerUrl);
        env.PUBLIC_L2_EXPLORER_HOST = url.host;
        env.PUBLIC_L2_EXPLORER_PROTOCOL = url.protocol.replace(':', '');  // 'http' or 'https'
        env.PUBLIC_L2_WS_PROTOCOL = url.protocol === 'https:' ? 'wss' : 'ws';
      } catch {}
    }
    if (toolsPorts.publicL1ExplorerUrl) {
      try {
        const url = new URL(toolsPorts.publicL1ExplorerUrl);
        env.PUBLIC_L1_EXPLORER_HOST = url.host;
        env.PUBLIC_L1_EXPLORER_PROTOCOL = url.protocol.replace(':', '');
        env.PUBLIC_L1_WS_PROTOCOL = url.protocol === 'https:' ? 'wss' : 'ws';
      } catch {}
    }
  }
  return env;
}
```

#### 3.5 `tools-config.js` — `getPublicAccessConfig()` 추가

```javascript
function getPublicAccessConfig(deployment) {
  if (!deployment.is_public || !deployment.public_domain) return {};

  const domain = deployment.public_domain;
  return {
    publicDomain: domain,
    publicBaseUrl: `http://${domain}`,
    publicL2RpcUrl: deployment.public_l2_rpc_url || `http://${domain}:${deployment.l2_port}`,
    publicL2ExplorerUrl: deployment.public_l2_explorer_url || `http://${domain}:${deployment.tools_l2_explorer_port}`,
    publicL1ExplorerUrl: deployment.public_l1_explorer_url || (
      deployment.l1_port ? `http://${domain}:${deployment.tools_l1_explorer_port}` : null
    ),
    publicDashboardUrl: deployment.public_dashboard_url || `http://${domain}:${deployment.tools_bridge_ui_port}`,
  };
}
```

### Phase 3: 컨테이너 내부 수정

#### 3.6 `entrypoint.sh` — 외부 URL 지원

```bash
# 기존 PUBLIC_BASE_URL 분기 확장
if [ -n "${PUBLIC_BASE_URL:-}" ]; then
  IS_PUBLIC="true"
  # L2 RPC: 커스텀 URL 또는 자동 계산
  L2_RPC_PUBLIC="${PUBLIC_L2_RPC_URL:-${PUBLIC_BASE_URL}:${TOOLS_L2_RPC_PORT:-1729}}"
  L2_EXPLORER_PUBLIC="${PUBLIC_L2_EXPLORER_URL:-${PUBLIC_BASE_URL}:${TOOLS_L2_EXPLORER_PORT:-8082}}"
  L1_EXPLORER_PUBLIC="${PUBLIC_L1_EXPLORER_URL:-${L1_EXPLORER_RESOLVED}}"
  DASHBOARD_PUBLIC="${PUBLIC_DASHBOARD_URL:-${PUBLIC_BASE_URL}:${TOOLS_BRIDGE_UI_PORT:-3000}}"
  METRICS_PUBLIC="${PUBLIC_BASE_URL}:${TOOLS_METRICS_PORT:-3702}/metrics"
else
  IS_PUBLIC="false"
  L2_RPC_PUBLIC="http://localhost:${TOOLS_L2_RPC_PORT:-1729}"
  L2_EXPLORER_PUBLIC="http://localhost:${TOOLS_L2_EXPLORER_PORT:-8082}"
  L1_EXPLORER_PUBLIC="${L1_EXPLORER_RESOLVED}"
  DASHBOARD_PUBLIC="http://localhost:${TOOLS_BRIDGE_UI_PORT:-3000}"
  METRICS_PUBLIC="http://localhost:${TOOLS_METRICS_PORT:-3702}/metrics"
fi

# config.json에 public URL 반영
cat > /usr/share/nginx/html/config.json << EOF
{
  "l1_rpc": "${L1_RPC_PUBLIC}",
  "l2_rpc": "${L2_RPC_PUBLIC}",
  "l1_explorer": "${L1_EXPLORER_PUBLIC}",
  "l2_explorer": "${L2_EXPLORER_PUBLIC}",
  "dashboard_url": "${DASHBOARD_PUBLIC}",
  "is_public": ${IS_PUBLIC},
  "public_domain": "${PUBLIC_DOMAIN:-}",
  "metrics_url": "${METRICS_PUBLIC}",
  ...기존 필드
}
EOF
```

#### 3.7 `entrypoint.sh` — config.json 캐시 방지 (신규)

nginx 설정에 추가:
```nginx
location = /config.json {
    expires -1;
    add_header Cache-Control "no-cache, no-store, must-revalidate, max-age=0";
}
```
**이유**: 공개 모드 전환 후 브라우저가 이전 config.json(localhost URL)을 캐싱하면 동작하지 않음.

#### 3.8 `docker-compose-zk-dex-tools.yaml` — Blockscout 프론트엔드 동적화

```yaml
frontend-l2:
  environment:
    NEXT_PUBLIC_API_HOST: ${PUBLIC_L2_EXPLORER_HOST:-localhost:${TOOLS_L2_EXPLORER_PORT:-8082}}
    NEXT_PUBLIC_APP_HOST: ${PUBLIC_L2_EXPLORER_HOST:-localhost:${TOOLS_L2_EXPLORER_PORT:-8082}}
    NEXT_PUBLIC_API_PROTOCOL: ${PUBLIC_L2_EXPLORER_PROTOCOL:-http}
    NEXT_PUBLIC_API_WEBSOCKET_PROTOCOL: ${PUBLIC_L2_WS_PROTOCOL:-ws}

frontend-l1:
  environment:
    NEXT_PUBLIC_API_HOST: ${PUBLIC_L1_EXPLORER_HOST:-localhost:${TOOLS_L1_EXPLORER_PORT:-8083}}
    NEXT_PUBLIC_APP_HOST: ${PUBLIC_L1_EXPLORER_HOST:-localhost:${TOOLS_L1_EXPLORER_PORT:-8083}}
    NEXT_PUBLIC_API_PROTOCOL: ${PUBLIC_L1_EXPLORER_PROTOCOL:-http}
    NEXT_PUBLIC_API_WEBSOCKET_PROTOCOL: ${PUBLIC_L1_WS_PROTOCOL:-ws}
```

**핵심**: `NEXT_PUBLIC_API_PROTOCOL`과 `NEXT_PUBLIC_API_WEBSOCKET_PROTOCOL`도 동적으로 설정해야 mixed content 문제 방지.

#### 3.9 Nginx proxy — `server_name` 변경

모든 nginx `server_name localhost;`를 `server_name _;`로 변경:
```nginx
server_name _;  # 모든 Host 헤더 허용
```
**적용 위치**: `proxy`, `proxy-l2-only`, `bridge-ui` (3곳)

**이유**: 와일드카드가 가장 단순하고, 앞단 리버스 프록시에서 Host 필터링을 하므로 보안 문제 없음.

### Phase 4: 메인 컴포즈 포트 바인딩

#### 3.10 `compose-generator.js` — 공개 모드 포트

`generateComposeFile()`과 `generateTestnetComposeFile()` 모두에 `isPublic` 파라미터 추가:

```javascript
// generateComposeFile(opts) 및 generateTestnetComposeFile(opts)
const bindAddr = opts.isPublic ? '0.0.0.0' : '127.0.0.1';
ports:
  - "${bindAddr}:${l2Port}:1729"
  - "${bindAddr}:${proofCoordPort}:3900"
  - "${bindAddr}:${metricsPort}:3702"
```

**주의**: 공개 모드 전환 시 메인 컴포즈 파일도 재생성 + L2 서비스 재시작 필요.

#### 3.11 `deployment-engine.js` — 공개 전환 함수

```javascript
async function enablePublicAccess(deployment) {
  const updated = db.prepare("SELECT * FROM deployments WHERE id = ?").get(deployment.id);

  // 1. 메인 컴포즈 재생성 (0.0.0.0 바인딩)
  const composeContent = generateComposeFile({ ...existingConfig, isPublic: true });
  writeComposeFile(deployment.id, composeContent);

  // 2. L2 서비스 재시작 (새 포트 바인딩 적용)
  await docker.stop(deployment.docker_project, composeFile);
  await docker.start(deployment.docker_project, composeFile, env);

  // 3. Tools 재시작 (새 환경변수)
  const envVars = await docker.extractEnv(deployment.docker_project, composeFile);
  const toolsPorts = {
    ...기존 포트,
    ...getExternalL1Config(updated),
    ...getPublicAccessConfig(updated),
  };
  await docker.restartTools(`${deployment.docker_project}-tools`, envVars, toolsPorts);

  // 4. phase 업데이트
  updateDeployment(deployment.id, { phase: "running" });
}

async function disablePublicAccess(deployment) {
  // 1. DB 초기화
  updateDeployment(deployment.id, {
    is_public: 0,
    public_domain: null,
    public_l2_rpc_url: null,
    public_l2_explorer_url: null,
    public_l1_explorer_url: null,
    public_dashboard_url: null,
  });

  // 2. 컴포즈 재생성 (127.0.0.1 바인딩)
  // 3. L2 + Tools 재시작
  // (enablePublicAccess와 동일 흐름, isPublic: false)
}
```

### Phase 5: 프론트엔드 수정

#### 3.12 HTML 파일의 하드코딩 제거

`dashboard.html`, `index.html`, `withdraw-status.html`의 기본 href를 `#`으로 변경하고 config.json 로드 후 동적으로 설정:

```html
<!-- Before -->
<a href="http://localhost:8083" target="_blank" id="navL1Explorer">L1 Explorer</a>

<!-- After -->
<a href="#" target="_blank" id="navL1Explorer">L1 Explorer</a>
```

JavaScript에서 config 로드 후:
```javascript
if (CONFIG.l1_explorer) document.getElementById('navL1Explorer').href = CONFIG.l1_explorer;
if (CONFIG.l2_explorer) document.getElementById('navL2Explorer').href = CONFIG.l2_explorer;
```

이미 부분적으로 구현되어 있으므로, 기본 HTML `href`만 `#`으로 변경하면 됨.

#### 3.13 Dashboard 표시 텍스트 동적화

`dashboard.html`에서 서비스 카드 링크 텍스트도 config 기반으로:
```javascript
// Before (line 762, 772 등)
l1Link.textContent = 'localhost:' + new URL(l1Exp).port;

// After
const l1Url = new URL(CONFIG.l1_explorer);
l1Link.textContent = l1Url.hostname + (l1Url.port ? ':' + l1Url.port : '');
```

#### 3.14 MetaMask "Add Network" 동적화

Dashboard의 MetaMask 설정 섹션도 config 기반 URL 사용:
```javascript
// wallet_addEthereumChain의 rpcUrls 파라미터
rpcUrls: [CONFIG.l2_rpc],  // localhost 대신 외부 URL
blockExplorerUrls: [CONFIG.l2_explorer],
```

#### 3.15 Manager UI — External Access 섹션

`public/app.js`에 External Access 섹션 렌더링:
- Overview 탭에 "External Access" 카드 추가
- 도메인/IP 입력 필드 + Enable/Disable 버튼
- 활성화 후 외부 URL 목록 + Copy 버튼
- API 호출: `POST/DELETE /api/deployments/:id/public-access`

## 4. 수정 파일 목록

| 우선순위 | 파일 | 변경 내용 |
|---------|------|----------|
| P0 | `db/schema.sql` | `public_domain`, `public_l2_rpc_url` 등 컬럼 추가 |
| P0 | `db/db.js` | 기존 DB 마이그레이션 (ALTER TABLE) |
| P0 | `db/deployments.js` | allowedFields에 새 필드 추가 |
| P0 | `routes/deployments.js` | `POST /:id/public-access`, `DELETE /:id/public-access` |
| P0 | `lib/tools-config.js` | `getPublicAccessConfig()` 함수 추가 |
| P0 | `lib/docker-local.js` | `buildToolsEnv()`에 public 환경변수 + HOST/PROTOCOL 분리 |
| P0 | `lib/deployment-engine.js` | `enablePublicAccess()` / `disablePublicAccess()` |
| P0 | `tooling/bridge/entrypoint.sh` | PUBLIC_* 환경변수로 config.json 생성, 캐시 방지 헤더 |
| P1 | `docker-compose-zk-dex-tools.yaml` | Blockscout `NEXT_PUBLIC_*` 동적화, nginx `server_name _`, CORS |
| P1 | `lib/compose-generator.js` | `isPublic` 옵션으로 `0.0.0.0` 바인딩 (local + testnet 모두) |
| P2 | `tooling/bridge/dashboard.html` | href `#`, 표시 텍스트 동적화, MetaMask URL |
| P2 | `tooling/bridge/index.html` | href `#`, config 기반 URL |
| P2 | `tooling/bridge/withdraw-status.html` | href `#`, config 기반 URL |
| P2 | `public/app.js` | Manager UI "External Access" 섹션 |
| P2 | `public/index.html` | External Access 설정 입력 UI |

## 5. 보안 고려사항

### 5.1 L1 RPC API 키 보호
이미 `entrypoint.sh`에 nginx L1 RPC 프록시가 구현됨. 공개 모드 시 `/api/l1-rpc`로 프록시하여 API 키를 서버에만 보관. config.json의 `l1_rpc`에는 프록시 URL만 노출.

### 5.2 L2 RPC Rate Limiting (신규)
공개 모드 시 L2 RPC를 nginx proxy를 통해 노출하고 rate limiting 적용:
```nginx
limit_req_zone $binary_remote_addr zone=l2rpc:10m rate=10r/s;
location /rpc {
    limit_req zone=l2rpc burst=20 nodelay;
    proxy_pass http://host.docker.internal:${L2_RPC_PORT};
    # CORS 헤더
    add_header Access-Control-Allow-Origin "*" always;
    add_header Access-Control-Allow-Methods "POST, GET, OPTIONS" always;
    add_header Access-Control-Allow-Headers "Content-Type" always;
    if ($request_method = 'OPTIONS') { return 204; }
}
```

### 5.3 CORS 헤더 (신규)
L2 RPC를 외부에 노출할 때 CORS 헤더 필수:
- MetaMask, 지갑 앱, dApp에서 브라우저 기반 JSON-RPC 호출 시 필요
- `Access-Control-Allow-Origin: *` (공개 RPC이므로)
- ethrex L2 노드 자체 CORS 지원 여부 확인 필요 → 미지원 시 nginx proxy에서 추가

### 5.4 HTTPS / Mixed Content (신규)
- 도메인 사용 시 HTTPS 권장. 옵션:
  - Caddy 자동 HTTPS (Let's Encrypt)
  - Cloudflare 프록시
  - 사용자가 별도 리버스 프록시 관리
- **Mixed Content 문제**: Dashboard가 HTTPS로 서빙되는데 config.json의 RPC/Explorer URL이 HTTP이면 브라우저가 차단
  - **해결**: 사용자가 Advanced URL에 `https://` URL을 입력하면 자동으로 `NEXT_PUBLIC_API_PROTOCOL: https`, `NEXT_PUBLIC_API_WEBSOCKET_PROTOCOL: wss` 설정
- 현재 스코프에서는 HTTP 기본 지원, HTTPS는 사용자가 앞단 리버스 프록시로 처리하도록 안내

### 5.5 WebSocket 프로토콜 (신규)
Blockscout 프론트엔드의 `NEXT_PUBLIC_API_WEBSOCKET_PROTOCOL`:
- HTTP 환경: `ws` (기본)
- HTTPS 환경: `wss` (필수 — mixed content 차단)
- `buildToolsEnv()`에서 URL 프로토콜 기반 자동 결정

### 5.6 메트릭 엔드포인트 보호 (신규)
공개 모드에서 `/metrics` (Prometheus) 엔드포인트 노출 시:
- 내부 체인 상태, 블록 번호, 가스 가격, 프로버 성능 등 노출 위험
- **옵션 A**: 공개 모드 시 `/metrics` 경로를 nginx에서 차단 (`return 403`)
- **옵션 B**: Basic auth 추가
- **권장**: 옵션 A (메트릭은 운영자 전용, 공개 불필요)

### 5.7 방화벽 포트 가이드 (신규)
공개 모드 시 열어야 할 포트 목록:

| 포트 | 서비스 | 필수 여부 |
|------|--------|----------|
| `${l2Port}` (기본 1729) | L2 RPC | 필수 |
| `${toolsL2ExplorerPort}` (기본 8082) | L2 Explorer | 필수 |
| `${toolsBridgeUIPort}` (기본 3000) | Dashboard/Bridge UI | 필수 |
| `${toolsL1ExplorerPort}` (기본 8083) | L1 Explorer | 로컬 L1일 때만 |
| `${metricsPort}` (기본 3702) | Metrics | 선택 (보안 주의) |

## 6. 배포 후 공개 전환 플로우

```
[Manager UI - 배포 상세페이지 (Overview 탭)]
  ↓
"External Access" 섹션에서 도메인/IP 입력
  ↓
"Enable Public Access" 클릭
  ↓
API: POST /api/deployments/:id/public-access
  { publicDomain: "l2.example.com" }
  ↓
[Backend 처리 — 비동기]
1. DB 저장 (is_public=1, public_domain, auto-calculated URLs)
2. 컴포즈 재생성 (0.0.0.0 바인딩)
3. L2 서비스 재시작 (포트 바인딩 변경)
4. Tools 재시작 (PUBLIC_* 환경변수)
   ├─ entrypoint.sh → config.json 재생성
   ├─ Blockscout → NEXT_PUBLIC_*_HOST 변경
   └─ nginx → server_name _ 적용
  ↓
[UI 업데이트]
외부 URL 표시 + Copy 버튼 + 방화벽 포트 안내
```

### 공개 해제 플로우
```
"Disable Public Access" 클릭
  ↓
API: DELETE /api/deployments/:id/public-access
  ↓
1. DB 초기화 (is_public=0, public_* = null)
2. 컴포즈 재생성 (127.0.0.1 바인딩)
3. L2 + Tools 재시작 (localhost 모드 복귀)
```

### 실패 시 롤백 (신규)
공개 전환 중 실패 시:
- DB는 이미 업데이트 → `is_public=1` 상태
- 컴포즈 재생성 실패 → DB를 `is_public=0`으로 롤백
- L2 재시작 실패 → 이전 컴포즈 파일 복원 + DB 롤백
- Tools 재시작 실패 → L2는 이미 공개 모드, Tools만 재시도

```javascript
async function enablePublicAccess(deployment) {
  const previousCompose = readComposeFile(deployment.id);
  try {
    // ... 컴포즈 재생성 + 재시작
  } catch (e) {
    // 롤백
    writeComposeFile(deployment.id, previousCompose);
    updateDeployment(deployment.id, { is_public: 0, public_domain: null, ... });
    await docker.stop(deployment.docker_project, composeFile);
    await docker.start(deployment.docker_project, composeFile, env);
    throw e;
  }
}
```

## 7. 테스트 계획

1. **로컬 모드 기본 동작** — 공개 설정 없이 기존처럼 localhost로 동작하는지
2. **IP 모드** — `public_domain=192.168.1.100` 설정 후 해당 IP로 Dashboard/Bridge/Explorer 접속
3. **도메인 모드** — `public_domain=l2.example.com` 설정 후 도메인으로 접속
4. **공개 전환** — 배포 완료 후 공개 모드 전환 시 서비스 정상 재시작
5. **공개 해제** — 공개 모드에서 localhost 모드로 복귀 확인
6. **테스트넷 + 공개** — 외부 L1(Sepolia) + 공개 모드 조합 동작
7. **Blockscout 외부 접속** — 외부 IP/도메인으로 Blockscout 프론트엔드 정상 로딩
8. **MetaMask 연동** — 외부 URL로 "Add Network" 시 L2 체인 정상 추가
9. **Mixed Content** — HTTPS 리버스 프록시 뒤에서 config.json URL이 https://로 설정되는지
10. **WebSocket** — Blockscout 실시간 업데이트가 외부 접속 시에도 동작하는지 (ws/wss)
11. **CORS** — 외부 브라우저에서 L2 RPC JSON-RPC 호출 가능한지
12. **config.json 캐싱** — 모드 전환 후 새로고침 시 새 config 로드되는지
13. **메트릭 차단** — 공개 모드에서 `/metrics` 접근 차단되는지
14. **롤백** — 전환 실패 시 이전 상태로 복구되는지
15. **동시 전환** — 여러 배포의 공개 모드를 동시에 전환해도 포트 충돌 없는지
