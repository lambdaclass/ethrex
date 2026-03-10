# Tools (Dashboard/Bridge/Explorer): 로컬/테스트넷/메인넷 + 외부 공개 설계

## 1. 현재 아키텍처

```
[사용자 브라우저 (localhost)]
    ├─ Dashboard  http://localhost:{bridge_ui_port}/
    ├─ Bridge     http://localhost:{bridge_ui_port}/bridge.html
    ├─ L1 Explorer http://localhost:{l1_explorer_port}  (로컬) 또는 etherscan.io (테스트넷)
    ├─ L2 Explorer http://localhost:{l2_explorer_port}
    ├─ L2 RPC     http://localhost:{l2_port}
    └─ Metrics    http://localhost:{metrics_port}/metrics
```

### config.json 흐름
```
Manager → deployment-engine → docker-local (buildToolsEnv)
  → docker-compose (환경변수)
    → entrypoint.sh → config.json
      → dashboard.html / bridge.html / withdraw-status.html
```

---

## 2. 환경별 대응 현황

| 항목 | 로컬넷 | 테스트넷 (Sepolia) | 메인넷 | 외부 공개 |
|------|--------|-------------------|--------|----------|
| L1 노드 | Docker ✅ | 외부 RPC ✅ | 외부 RPC ✅ | 외부 RPC ✅ |
| L1 Explorer | Blockscout ✅ | Etherscan ✅ | Etherscan ✅ | Etherscan ✅ |
| L2 RPC | localhost ✅ | localhost ✅ | localhost ✅ | **❌ 외부 접근 불가** |
| L2 Explorer | localhost ✅ | localhost ✅ | localhost ✅ | **❌ 외부 접근 불가** |
| Dashboard | localhost ✅ | localhost ✅ | localhost ✅ | **❌ 외부 접근 불가** |
| Bridge | localhost ✅ | localhost ✅ | localhost ✅ | **❌ 외부 접근 불가** |
| Test Accounts | 표시 ✅ | 숨김 ✅ | 숨김 ✅ | 숨김 ✅ |
| L1 RPC API키 | 로컬 ✅ | config.json 노출 | config.json 노출 | **❌ 위험** |
| MetaMask Config | 동적 ✅ | 동적 ✅ | 동적 ✅ | **❌ localhost RPC** |
| Etherscan 검증 | 불필요 | API v1 → v2 마이그레이션 필요 | 필요 | 필요 |

---

## 3. 외부 공개 아키텍처 설계

### 3.1 목표
- 단일 도메인(또는 IP)으로 Dashboard, Bridge, L2 Explorer, L2 RPC 접근
- L1 RPC API 키 보호 (서버 사이드 프록시)
- MetaMask에서 외부 L2 RPC URL 사용 가능
- Etherscan에서 L1 배포 컨트랙트 자동 검증

### 3.2 아키텍처
```
[외부 사용자 브라우저]
        |
  https://l2.example.com (nginx reverse proxy + SSL)
        |
   ┌────┼──────────────────────┐
   |    |                      |
   /              → Dashboard/Bridge (bridge-ui 컨테이너, :3014 내부)
   /explorer/*    → L2 Blockscout (:8087 내부)
   /rpc           → L2 JSON-RPC (:1733 내부)
   /api/l1-rpc    → L1 RPC 프록시 (API 키 숨김)
   /metrics       → Prometheus (:3706 내부)
   /ws            → L2 WebSocket (향후)
```

### 3.3 환경변수
```bash
# docker-compose에 추가
PUBLIC_BASE_URL=https://l2.example.com   # 외부 접근 URL (설정 시 공개 모드 활성화)
```

### 3.4 config.json 변화

| 필드 | 로컬 모드 | 공개 모드 |
|------|----------|----------|
| `l1_rpc` | `http://localhost:8545` 또는 외부 RPC | `https://l2.example.com/api/l1-rpc` (프록시) |
| `l1_rpc_internal` | 없음 | 실제 L1 RPC URL (서버 사이드 전용) |
| `l2_rpc` | `http://localhost:1733` | `https://l2.example.com/rpc` |
| `l2_explorer` | `http://localhost:8087` | `https://l2.example.com/explorer` |
| `metrics_url` | `http://localhost:3706/metrics` | `https://l2.example.com/metrics` |
| `is_public` | `false` | `true` |

---

## 4. 컴포넌트별 수정 계획

### 4.1 entrypoint.sh ✅ (완료)
- `PUBLIC_BASE_URL` 환경변수 지원 추가
- 공개 모드 시 URL을 외부 도메인 기반으로 변환
- L1 RPC 프록시 nginx config 자동 생성

### 4.2 Dashboard (dashboard.html)

| 항목 | 현재 상태 | 수정 필요 |
|------|----------|----------|
| Chain Status (L1/L2) | config.json 기반 ✅ | L1 RPC 프록시 URL 사용 시 CORS 확인 |
| Quick Links | 동적 ✅ | 공개 모드에서 L2 Explorer URL 자동 변환 ✅ |
| Services 상태 체크 | L1/L2 Explorer fetch | 공개 모드에서 프록시 경유 → same-origin이므로 CORS 없음 ✅ |
| L1 Deployed Contracts | Etherscan 링크 ✅ | - |
| Test Accounts | 외부 L1이면 숨김 ✅ | - |
| MetaMask Config | 동적 ✅ | L2 RPC URL이 공개 URL로 자동 변환 ✅ |
| **L1 RPC 표시** | 전체 URL 노출 | **공개 모드: 프록시 URL 표시, API키 미노출** |

### 4.3 Bridge (index.html)

| 항목 | 현재 상태 | 수정 필요 |
|------|----------|----------|
| Deposit (L1→L2) | MetaMask로 L1 tx 전송 | **공개 모드: MetaMask가 L1에 직접 연결하므로 프록시 불필요 ✅** |
| Withdraw (L2→L1) | MetaMask로 L2 tx 전송 | **공개 모드: L2 RPC를 공개 URL로 → MetaMask 네트워크 추가 시 사용** |
| Balance 조회 | ethers.js JsonRpcProvider | 공개 모드: config.json의 l1_rpc(프록시), l2_rpc(공개) 사용 ✅ |
| switchNetwork | wallet_addEthereumChain | **⚠️ L1이 Sepolia/Mainnet이면 이미 MetaMask에 있음 → switchEthereumChain 우선 시도** |
| Nav links | 동적 ✅ | - |
| Contract info | config.json 기반 ✅ | - |

**Bridge 핵심 수정**:
1. `switchNetwork()`: 공개 모드에서 `wallet_switchEthereumChain` 우선 시도 후 실패 시 `wallet_addEthereumChain`
   - 현재는 `addEthereumChain` 우선 → Sepolia/Mainnet은 이미 존재하므로 일부 지갑에서 에러
2. Deposit 시 L1 RPC URL: MetaMask는 자체 Sepolia RPC 사용 → 프록시 불필요 ✅
3. Withdraw 시 L2 RPC URL: `wallet_addEthereumChain`에 공개 L2 RPC URL 전달
4. Balance 조회용 Read provider: config.json의 `l1_rpc` / `l2_rpc` 사용 (프록시 경유)

### 4.4 Withdrawal Tracker (withdraw-status.html)

| 항목 | 현재 상태 | 수정 필요 |
|------|----------|----------|
| L1 이벤트 스캔 | ethers.js로 L1 RPC 직접 호출 | 공개 모드: 프록시 경유 ✅ |
| L2 withdrawal 조회 | ethers.js로 L2 RPC 호출 | 공개 모드: 공개 URL 사용 ✅ |
| Explorer 링크 | 동적 ✅ | - |
| **Rate limit** | 없음 | **L1 RPC 호출 횟수 제한 필요 (이벤트 스캔 시 다량 호출)** |

### 4.5 Docker Compose 수정

```yaml
# docker-compose-zk-dex-tools.yaml에 추가
services:
  bridge-ui:
    environment:
      PUBLIC_BASE_URL: ${PUBLIC_BASE_URL:-}

  # 새 서비스: 외부 공개용 리버스 프록시 (공개 모드 전용)
  public-proxy:
    image: nginx:alpine
    profiles: ["public"]
    ports:
      - "${PUBLIC_PORT:-443}:443"
      - "${PUBLIC_HTTP_PORT:-80}:80"
    volumes:
      - ./nginx-public.conf:/etc/nginx/conf.d/default.conf
      - ${SSL_CERT_PATH:-./certs}:/etc/nginx/certs:ro
    depends_on:
      - bridge-ui
      - proxy-l2-only  # 또는 proxy
```

### 4.6 nginx 리버스 프록시 설정 (public-proxy)

```nginx
server {
    listen 443 ssl;
    server_name l2.example.com;

    ssl_certificate /etc/nginx/certs/cert.pem;
    ssl_certificate_key /etc/nginx/certs/key.pem;

    # Dashboard & Bridge
    location / {
        proxy_pass http://bridge-ui:80;
    }

    # L2 Blockscout Explorer
    location /explorer/ {
        proxy_pass http://proxy-l2-only:80/;  # 또는 직접 frontend-l2
    }

    # L2 JSON-RPC
    location /rpc {
        proxy_pass http://tokamak-app-l2:1729;
        proxy_set_header Content-Type application/json;
    }

    # L1 RPC 프록시 (API 키 보호)
    location /api/l1-rpc {
        # entrypoint.sh에서 생성된 config 사용
        proxy_pass <L1_RPC_INTERNAL>;
        proxy_set_header Content-Type application/json;
        proxy_method POST;
        limit_req zone=l1rpc burst=20 nodelay;
    }

    # Prometheus Metrics
    location /metrics {
        proxy_pass http://tokamak-app-l2:3702/metrics;
    }
}
```

### 4.7 Etherscan 컨트랙트 자동 검증

| 항목 | 현재 상태 | 수정 필요 |
|------|----------|----------|
| Etherscan API | V1 사용 → V2 마이그레이션 경고 | **V2 API URL로 전환** |
| 검증 시점 | Tools 시작 시 | 배포 직후 (deploying_contracts 완료 시) |
| API URL 분기 | 없음 | sepolia → `api-sepolia.etherscan.io/api`, mainnet → `api.etherscan.io/api` |
| Proxy 검증 | 미구현 | CommonBridge, OnChainProposer는 Proxy → `verifyproxycontract` 필요 |
| 솔리디티 컴파일러 | Docker 내 solc | Docker 배포 직후 같은 solc로 검증 → 바이트코드 일치 ✅ |

**Etherscan V2 API URL**:
```
Sepolia: https://api-sepolia.etherscan.io/api
Mainnet: https://api.etherscan.io/api
```

---

## 5. Manager App 수정 사항

### 핵심 원칙: 배포 먼저, 공개 설정은 나중에
```
1. 배포 (Provision)        → 컨트랙트 배포 + L2 시작 (localhost 모드)
2. 정상 동작 확인          → Dashboard/Bridge/Explorer 로컬에서 테스트
3. 공개 설정 (언제든지)    → Manager Detail에서 Public URL 입력
4. Tools 자동 재시작       → config.json 재생성 (외부 URL 반영)
5. 프록시 활성화           → public-proxy 컨테이너 자동 시작
```

**배포는 기존 로직 변경 없음.** 공개 설정은 이미 배포된 L2에 대해 별도로 수행.

### 5.1 DB 스키마 추가
```sql
ALTER TABLE deployments ADD COLUMN public_base_url TEXT;
-- 예: 'https://l2.tokamak.network'
-- NULL이면 로컬 전용 모드
```

### 5.2 Manager Detail UI (공개 설정 섹션)
- Detail → Settings 카드에 "Public Access" 섹션 추가
- `public_base_url` 입력 필드 + "Enable Public Access" 버튼
- 설정 시: DB 업데이트 → Tools 자동 재시작 (config.json 재생성)
- 공개 URL 표시: Dashboard URL, L2 RPC URL, Explorer URL 복사 버튼
- 해제 시: public_base_url = NULL → Tools 재시작 → localhost 모드 복귀

### 5.3 deployment-engine 수정
- `startTools()` / `restartTools()`: deployment에서 `public_base_url` 읽어서 `PUBLIC_BASE_URL` 환경변수로 전달
- 기존 배포 로직은 변경 없음

### 5.4 docker-local.js 수정
- `buildToolsEnv()`: `if (deployment.public_base_url) env.PUBLIC_BASE_URL = deployment.public_base_url;`
- `buildToolsUpArgs()`: 공개 모드일 때 `public-proxy` 서비스 함께 시작

### 5.5 API 엔드포인트 추가
```
PUT /api/deployments/:id/public-access
Body: { "publicBaseUrl": "https://l2.example.com" }  // null이면 해제

→ DB 업데이트
→ Tools 재시작 (restartTools)
→ 응답: { ok: true, publicUrl: "https://l2.example.com" }
```

---

## 6. 구현 우선순위

### Phase 1: 기본 동작 (✅ 완료)
- [x] localhost 하드코딩 제거
- [x] config.json 동적 생성
- [x] 테스트넷 Explorer URL 자동 매핑
- [x] Test accounts 조건부 숨김

### Phase 2: 외부 공개 인프라 (진행 중)
- [x] entrypoint.sh PUBLIC_BASE_URL 지원
- [ ] Docker Compose에 public-proxy 서비스 추가
- [ ] nginx 리버스 프록시 설정 파일 생성
- [ ] L1 RPC 프록시 (API 키 보호)
- [ ] SSL 인증서 설정 (Let's Encrypt 또는 수동)

### Phase 3: Bridge 외부 대응
- [ ] `switchNetwork()`: Sepolia/Mainnet은 `wallet_switchEthereumChain` 우선
- [ ] Balance provider: 공개 모드에서 프록시 URL 사용
- [ ] Withdrawal tracker: rate limit 대응
- [ ] L2 RPC WebSocket 프록시 추가 (실시간 이벤트 구독)

### Phase 4: Etherscan 검증
- [ ] V1 → V2 API URL 마이그레이션
- [ ] 배포 직후 자동 검증 (`deploying_contracts` 완료 시)
- [ ] Proxy 컨트랙트 검증 (`verifyproxycontract`)
- [ ] 검증 상태 UI 표시

### Phase 5: Manager 공개 모드 UI
- [ ] Launch L2 설정에 Public Access 토글
- [ ] PUBLIC_BASE_URL 입력
- [ ] 외부 URL 표시 및 복사
- [ ] 공개 모드 상태 대시보드

---

## 7. 보안 체크리스트

- [ ] L1 RPC API 키가 클라이언트에 노출되지 않는지 확인
- [ ] L1 RPC 프록시에 rate limit 적용
- [ ] CORS 정책: 공개 프록시에서만 허용
- [ ] SSL/TLS 필수 (HTTP → HTTPS 리다이렉트)
- [ ] Bridge UI: MetaMask 트랜잭션만 허용 (서버에 개인키 없음)
- [ ] 관리자 인증: Manager App은 외부 노출 안 함 (로컬 전용)
- [ ] Docker 포트: 외부에 필요한 포트만 공개 (443, 80)
