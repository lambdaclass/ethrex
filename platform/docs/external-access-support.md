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
NEXT_PUBLIC_API_HOST: localhost:${TOOLS_L1_EXPLORER_PORT:-8083}
NEXT_PUBLIC_APP_HOST: localhost:${TOOLS_L1_EXPLORER_PORT:-8083}
```

#### Nginx proxy — `server_name`
```
server_name localhost;
```
외부 도메인/IP로 접속하면 nginx가 Host 헤더 불일치로 거부.

#### HTML 파일들 — 기본 href
- `dashboard.html`: 탐색기/메트릭 링크 `http://localhost:8083`, `http://localhost:8082`
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

### 1.3 이미 존재하는 인프라

- DB `schema.sql`: `is_public INTEGER DEFAULT 0` 컬럼 (미사용)
- `entrypoint.sh`: `PUBLIC_BASE_URL` 환경변수 분기 (부분 구현)
- `entrypoint.sh`: nginx L1 RPC 프록시 + rate limiting (public 모드)
- `docker-local.js`: `buildToolsEnv()` 환경변수 파이프라인
- `tools-config.js`: `getExternalL1Config()` 외부 L1 설정 추출

## 2. 목표 아키텍처

### 2.1 사용자 시나리오

1. **로컬 배포** → 기본 `localhost` 모드 (현재와 동일)
2. **배포 후 공개 전환** → Manager UI에서 도메인/IP 설정 → tools 재시작 → 외부 접속 가능
3. **처음부터 공개 배포** → 배포 설정 시 도메인/IP 입력 → 바로 공개 모드로 시작

### 2.2 설정 필드

Manager UI에 "External Access" 섹션 추가:

| 필드 | 설명 | 예시 |
|------|------|------|
| Public Domain/IP | 외부 접속 기본 주소 | `l2.example.com` 또는 `203.0.113.50` |
| L2 RPC URL (외부) | 외부에서 접근 가능한 L2 RPC | `https://l2.example.com/rpc` 또는 `http://203.0.113.50:1729` |
| L2 Explorer URL (외부) | 외부 접근 L2 탐색기 | `https://l2.example.com/explorer` 또는 `http://203.0.113.50:8082` |
| L1 Explorer URL (외부) | 외부 접근 L1 탐색기 (로컬 L1일 때만) | `http://203.0.113.50:8083` |
| Dashboard URL (외부) | 외부 접근 대시보드 | `https://l2.example.com` 또는 `http://203.0.113.50:3000` |
| Bridge URL (외부) | 외부 접근 브릿지 | `https://l2.example.com/bridge.html` |

**간소화 옵션**: `Public Domain/IP`만 입력하면 나머지는 자동 계산:
```
Public Base = http://{domain_or_ip}
L2 RPC     = http://{domain_or_ip}:{l2Port}
L2 Explorer = http://{domain_or_ip}:{explorerPort}
Dashboard  = http://{domain_or_ip}:{bridgeUiPort}
Bridge     = http://{domain_or_ip}:{bridgeUiPort}/bridge.html
```

HTTPS + 리버스 프록시(nginx/Caddy) 사용 시에는 개별 URL 커스텀 입력.

### 2.3 데이터 흐름

```
Manager UI → API → DB (public_domain, public_urls) → restartTools()
                                                        ↓
                                             buildToolsEnv() + 새 환경변수
                                                        ↓
                                             Docker Compose 재시작
                                                        ↓
                                             entrypoint.sh → config.json 재생성
                                             Blockscout frontends → 새 NEXT_PUBLIC_* 값
                                             nginx → 새 server_name
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
  // 1. DB에 저장
  // 2. is_public = 1 설정
  // 3. tools 재시작 (새 환경변수로)
});

// DELETE /api/deployments/:id/public-access
// 공개 모드 해제 → localhost 모드로 복귀
router.delete("/:id/public-access", async (req, res) => {
  // 1. public_* 필드 null로
  // 2. is_public = 0
  // 3. tools 재시작
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
    env.PUBLIC_BASE_URL = toolsPorts.publicBaseUrl;    // http://l2.example.com
    env.PUBLIC_L2_RPC_URL = toolsPorts.publicL2RpcUrl;
    env.PUBLIC_L2_EXPLORER_URL = toolsPorts.publicL2ExplorerUrl;
    env.PUBLIC_L1_EXPLORER_URL = toolsPorts.publicL1ExplorerUrl;
    env.PUBLIC_DASHBOARD_URL = toolsPorts.publicDashboardUrl;
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
  "metrics_url": "${METRICS_PUBLIC}",
  ...기존 필드
}
EOF
```

#### 3.7 `docker-compose-zk-dex-tools.yaml` — Blockscout 프론트엔드 동적화

```yaml
frontend-l2:
  environment:
    NEXT_PUBLIC_API_HOST: ${PUBLIC_L2_EXPLORER_HOST:-localhost:${TOOLS_L2_EXPLORER_PORT:-8082}}
    NEXT_PUBLIC_APP_HOST: ${PUBLIC_L2_EXPLORER_HOST:-localhost:${TOOLS_L2_EXPLORER_PORT:-8082}}

frontend-l1:
  environment:
    NEXT_PUBLIC_API_HOST: ${PUBLIC_L1_EXPLORER_HOST:-localhost:${TOOLS_L1_EXPLORER_PORT:-8083}}
    NEXT_PUBLIC_APP_HOST: ${PUBLIC_L1_EXPLORER_HOST:-localhost:${TOOLS_L1_EXPLORER_PORT:-8083}}
```

`buildToolsEnv()`에서 `PUBLIC_L2_EXPLORER_HOST`를 계산:
```javascript
if (toolsPorts.publicL2ExplorerUrl) {
  try {
    const url = new URL(toolsPorts.publicL2ExplorerUrl);
    env.PUBLIC_L2_EXPLORER_HOST = url.host; // "l2.example.com:8082" 또는 "l2.example.com"
  } catch {}
}
```

#### 3.8 Nginx proxy — `server_name` 동적화

`docker-compose-zk-dex-tools.yaml`의 nginx config에서:
```nginx
server_name ${PUBLIC_DOMAIN:-localhost};
```

또는 entrypoint에서 nginx.conf를 envsubst로 생성:
```yaml
proxy:
  command: >
    /bin/sh -c "envsubst '$$PUBLIC_DOMAIN' < /etc/nginx/templates/default.conf.template > /etc/nginx/conf.d/default.conf && nginx -g 'daemon off;'"
  environment:
    PUBLIC_DOMAIN: ${PUBLIC_DOMAIN:-localhost}
```

실제로는 `server_name _` (와일드카드)로 변경하는 것이 가장 간단:
```nginx
server_name _;  # 모든 Host 헤더 허용
```

### Phase 4: 메인 컴포즈 포트 바인딩

#### 3.9 `compose-generator.js` — 공개 모드 포트

L2 RPC를 외부에서 접속하려면 `127.0.0.1` 바인딩을 `0.0.0.0`으로 변경해야 함.

```javascript
// generateTestnetComposeFile() 또는 generateComposeFile()
const bindAddr = opts.isPublic ? '0.0.0.0' : '127.0.0.1';
ports:
  - "${bindAddr}:${l2Port}:1729"
  - "${bindAddr}:${proofCoordPort}:3900"
  - "${bindAddr}:${metricsPort}:3702"
```

**주의**: 공개 모드 전환 시 메인 컴포즈 파일도 재생성 + 서비스 재시작 필요.

#### 3.10 `deployment-engine.js` — 공개 전환 함수

```javascript
async function enablePublicAccess(deployment, publicConfig) {
  const { publicDomain, l2RpcUrl, l2ExplorerUrl, l1ExplorerUrl, dashboardUrl } = publicConfig;

  // 1. DB 업데이트
  updateDeployment(deployment.id, {
    is_public: 1,
    public_domain: publicDomain,
    public_l2_rpc_url: l2RpcUrl || null,
    public_l2_explorer_url: l2ExplorerUrl || null,
    public_l1_explorer_url: l1ExplorerUrl || null,
    public_dashboard_url: dashboardUrl || null,
  });

  // 2. 메인 컴포즈 재생성 (0.0.0.0 바인딩)
  const composeContent = generateComposeFile({ ...existingConfig, isPublic: true });
  writeComposeFile(deployment.id, composeContent);

  // 3. L2 서비스 재시작 (새 포트 바인딩 적용)
  await docker.stop(deployment.docker_project, composeFile);
  await docker.start(deployment.docker_project, composeFile, env);

  // 4. Tools 재시작 (새 환경변수)
  const envVars = await docker.extractEnv(deployment.docker_project, composeFile);
  const toolsPorts = {
    ...기존 포트,
    ...getExternalL1Config(deployment),
    ...getPublicAccessConfig(updatedDeployment),
  };
  await docker.restartTools(envVars, toolsPorts);
}
```

### Phase 5: 프론트엔드 HTML 수정

#### 3.11 HTML 파일의 하드코딩 제거

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

## 4. 수정 파일 목록

| 우선순위 | 파일 | 변경 내용 |
|---------|------|----------|
| P0 | `db/schema.sql` | `public_domain`, `public_l2_rpc_url` 등 컬럼 추가 |
| P0 | `db/deployments.js` | allowedFields에 새 필드 추가 |
| P0 | `routes/deployments.js` | `POST /:id/public-access`, `DELETE /:id/public-access` |
| P0 | `lib/tools-config.js` | `getPublicAccessConfig()` 함수 추가 |
| P0 | `lib/docker-local.js` | `buildToolsEnv()`에 public 환경변수 추가 |
| P0 | `tooling/bridge/entrypoint.sh` | PUBLIC_* 환경변수로 config.json 생성 |
| P1 | `docker-compose-zk-dex-tools.yaml` | Blockscout `NEXT_PUBLIC_*_HOST` 동적화, nginx `server_name _` |
| P1 | `lib/compose-generator.js` | `isPublic` 옵션으로 `0.0.0.0` 바인딩 |
| P1 | `lib/deployment-engine.js` | `enablePublicAccess()` / `disablePublicAccess()` |
| P2 | `tooling/bridge/dashboard.html` | 기본 href `#`으로, config 기반 동적 설정 |
| P2 | `tooling/bridge/index.html` | 동일 |
| P2 | `tooling/bridge/withdraw-status.html` | 동일 |
| P2 | `public/app.js` | Manager UI에 "External Access" 설정 섹션 추가 |
| P2 | `public/index.html` | External Access 설정 입력 UI |

## 5. 보안 고려사항

### 5.1 L1 RPC API 키 보호
이미 `entrypoint.sh`에 nginx L1 RPC 프록시가 구현됨. 공개 모드 시 `/api/l1-rpc`로 프록시하여 API 키를 서버에만 보관.

### 5.2 Rate Limiting
공개 모드 시 L2 RPC에 rate limiting 필요. Nginx proxy에 추가:
```nginx
limit_req_zone $binary_remote_addr zone=l2rpc:10m rate=10r/s;
location /rpc {
    limit_req zone=l2rpc burst=20;
    proxy_pass http://host.docker.internal:${L2_RPC_PORT};
}
```

### 5.3 HTTPS
도메인 사용 시 HTTPS 필수. 옵션:
- Caddy 자동 HTTPS (Let's Encrypt)
- Cloudflare 프록시
- 사용자가 별도 리버스 프록시 관리

현재 스코프에서는 HTTP만 지원하고, HTTPS는 사용자가 앞단 리버스 프록시로 처리하도록 안내.

### 5.4 MetaMask CORS
L2 RPC를 외부에 노출할 때 CORS 헤더 필요. ethrex L2 노드 자체가 `--http.cors *`를 지원하는지 확인 필요.

## 6. 배포 후 공개 전환 플로우

```
[Manager UI - Deployment Detail]
  ↓
"External Access" 버튼 클릭
  ↓
모달/섹션 표시:
  - Public Domain/IP: [l2.example.com]
  - (선택) Advanced URLs 토글:
    - L2 RPC URL: [자동계산]
    - L2 Explorer URL: [자동계산]
    - Dashboard URL: [자동계산]
  ↓
"Enable Public Access" 클릭
  ↓
API: POST /api/deployments/:id/public-access
  ↓
1. DB 저장 (is_public=1, public_domain, ...)
2. 컴포즈 재생성 (0.0.0.0 바인딩)
3. L2 서비스 재시작
4. Tools 재시작 (새 환경변수)
  ↓
완료: 외부 URL 표시 + 복사 버튼
```

## 7. 테스트 계획

1. **로컬 모드 기본 동작** — 공개 설정 없이 기존처럼 localhost로 동작하는지
2. **IP 모드** — `public_domain=192.168.1.100` 설정 후 해당 IP로 Dashboard/Bridge/Explorer 접속
3. **도메인 모드** — `public_domain=l2.example.com` 설정 후 도메인으로 접속
4. **공개 전환** — 배포 완료 후 공개 모드 전환 시 서비스 정상 재시작
5. **공개 해제** — 공개 모드에서 localhost 모드로 복귀
6. **테스트넷 + 공개** — 외부 L1(Sepolia) + 공개 모드 조합 동작
7. **Blockscout 외부 접속** — 외부 IP/도메인으로 Blockscout 프론트엔드 정상 로딩
