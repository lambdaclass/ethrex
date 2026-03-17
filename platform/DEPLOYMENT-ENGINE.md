# L2 Deployment Engine

Docker Compose 기반 L2 배포/관리 엔진. 앱스토어에서 앱을 선택하듯이 Guest Program을 선택하고, 로컬 Docker 또는 원격 서버에서 L2 체인을 자동으로 배포한다.

## 아키텍처 개요

```
┌─────────────────────────────────────────────────────┐
│  Frontend (Next.js)                                  │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐             │
│  │ App Store│ │Configure │ │Dashboard │             │
│  │ (Step 1) │ │ (Step 2) │ │ (Step 3) │             │
│  └──────────┘ └──────────┘ └──────────┘             │
│       │              │           │                   │
│       ▼              ▼           ▼                   │
│  ┌──────────────────────────────────────┐           │
│  │         deploymentsApi               │           │
│  │  provision / start / stop / destroy  │           │
│  │  status / monitoring / logs (SSE)    │           │
│  └──────────────────────────────────────┘           │
└─────────────────────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────┐
│  Backend (Express.js)                                │
│  ┌──────────────────────────────────────┐           │
│  │  routes/deployments.js               │           │
│  │  POST /:id/provision                 │           │
│  │  POST /:id/start|stop|destroy        │           │
│  │  GET  /:id/status|monitoring|logs    │           │
│  │  GET  /:id/events (SSE)              │           │
│  └──────────────────────────────────────┘           │
│       │                                              │
│       ▼                                              │
│  ┌──────────────────────────────────────┐           │
│  │  lib/deployment-engine.js            │           │
│  │  State Machine:                      │           │
│  │  configured → building → l1_starting │           │
│  │  → deploying_contracts → l2_starting │           │
│  │  → starting_prover → running         │           │
│  └──────────────────────────────────────┘           │
│       │              │                               │
│       ▼              ▼                               │
│  ┌──────────┐ ┌──────────────────┐                  │
│  │ docker-  │ │compose-generator │                  │
│  │ local.js │ │.js               │                  │
│  └──────────┘ └──────────────────┘                  │
│       │              │                               │
│       ▼              ▼                               │
│  docker compose CLI  ~/.tokamak/deployments/<id>/    │
│       │                                               │
│       ├── Local: docker compose on this machine       │
│       └── Remote: SSH → docker compose on server      │
│                                                       │
│  ┌──────────────────────────────────────┐            │
│  │  lib/docker-remote.js (SSH engine)  │            │
│  │  ssh2: SFTP upload + remote exec    │            │
│  └──────────────────────────────────────┘            │
│       │                                               │
│  ┌──────────────────────────────────────┐            │
│  │  routes/hosts.js                    │            │
│  │  POST /api/hosts (add SSH server)   │            │
│  │  POST /api/hosts/:id/test           │            │
│  └──────────────────────────────────────┘            │
└─────────────────────────────────────────────────────┘
```

## 앱별 배포 차이점

앱 선택에 따라 다른 서킷(circuits)과 검증 컨트랙트(verification contracts)가 배포된다.

| 항목 | EVM L2 | ZK-DEX | Tokamon |
|------|--------|--------|---------|
| Docker 이미지 | `ethrex:main-l2` | `ethrex:sp1` | `ethrex:main-l2` |
| Dockerfile | 기본 | `Dockerfile.sp1` | 기본 |
| Build Features | `l2,l2-sql` | `l2,l2-sql,sp1` | `l2,l2-sql` |
| Guest Programs | — | `evm-l2,zk-dex` | — |
| Genesis 파일 | `l2.json` | `l2-zk-dex.json` | `l2.json` |
| Prover Backend | `exec` (no-op) | `sp1` (실제 ZK 증명) | `exec` |
| 검증 컨트랙트 | 기본 Verifier | SP1 Verifier | 기본 Verifier |
| Programs TOML | 불필요 | `programs-zk-dex.toml` | 불필요 |

### 새 앱 추가 방법

`platform/server/lib/compose-generator.js`의 `APP_PROFILES` 객체에 새 프로필을 추가:

```javascript
"my-app": {
  image: "ethrex:sp1",
  dockerfile: "Dockerfile.sp1",
  buildFeatures: "--features l2,l2-sql,sp1",
  guestPrograms: "evm-l2,my-app",
  genesisFile: "l2-my-app.json",
  proverBackend: "sp1",
  sp1Enabled: true,
  registerGuestPrograms: "my-app",
  programsToml: "programs-my-app.toml",
  description: "My custom app",
}
```

## API 엔드포인트

### 배포 생성 (기존)
```
POST /api/deployments
Body: { programId, name, chainId, rpcUrl, config: { mode: "local" } }
→ 201 { deployment }
```

### Docker 라이프사이클 (신규)
```
POST /api/deployments/:id/provision   # 배포 시작 (로컬: 빌드, 원격: 이미지 풀)
  Body: { hostId? }                   # hostId 지정 시 원격 배포
POST /api/deployments/:id/start       # 중지된 배포 재시작
POST /api/deployments/:id/stop        # 배포 중지 (볼륨 유지)
POST /api/deployments/:id/destroy     # 완전 삭제 (볼륨 포함)
```

### 호스트 관리 (원격 배포용)
```
POST   /api/hosts                     # SSH 서버 추가
GET    /api/hosts                     # 서버 목록
GET    /api/hosts/:id                 # 서버 상세
POST   /api/hosts/:id/test            # SSH + Docker 연결 테스트
PUT    /api/hosts/:id                 # 서버 정보 수정
DELETE /api/hosts/:id                 # 서버 삭제
```

### 상태/모니터링 (신규)
```
GET /api/deployments/:id/status       # 컨테이너 상태, 엔드포인트, 컨트랙트 주소
GET /api/deployments/:id/monitoring   # 블록 높이, 체인 ID, 잔액
GET /api/deployments/:id/events       # SSE: 배포 진행 상태 스트림
GET /api/deployments/:id/logs?service=&follow=true&tail=100  # 로그 (SSE 또는 텍스트)
```

## 배포 흐름

### 1. 앱 선택 (Step 1)
- 앱스토어 UI에서 Guest Program 선택
- 각 앱의 특성 표시 (ZK 백엔드, 검증 방식, Genesis 타입)

### 2. 설정 (Step 2)
- L2 이름, Chain ID, 환경(Local/Remote/Manual) 설정
- **Local**: 로컬 Docker에서 소스 빌드 후 배포
- **Remote Server**: 원격 서버에 프리빌트 Docker 이미지 풀 + 배포
- **Manual**: 설정 파일 다운로드 후 수동 실행

### 3. 배포 (Step 3 — Local/Remote 모드)

**Local 모드:**
```
configured
  ↓ generateComposeFile()
building
  ↓ docker compose build
l1_starting → deploying_contracts → l2_starting → starting_prover → running ✓
```

**Remote 모드 (프리빌트 이미지):**
```
configured
  ↓ generateRemoteComposeFile() + SFTP 업로드
pulling
  ↓ docker compose pull (원격 서버에서 이미지 풀)
l1_starting → deploying_contracts → l2_starting → starting_prover → running ✓
```

원격 배포 시 이미지 레지스트리: `ghcr.io/tokamak-network/ethrex:*`

### 에러 처리
- 어떤 단계에서든 실패 시 → `error` 상태 + 컨테이너 자동 정리
- 에러 메시지 DB 저장 + SSE 전송
- "Retry Deploy" 버튼으로 재시도 가능

## 파일 구조

### 신규 파일
```
platform/server/lib/
  docker-local.js        # Docker Compose CLI 래퍼 (로컬)
  docker-remote.js       # SSH 기반 Docker 조작 (원격)
  compose-generator.js   # 앱별 compose 파일 생성기 (로컬 + 원격)
  deployment-engine.js   # 배포 상태 머신 + 오케스트레이션
  rpc-client.js          # JSON-RPC 헬스체크

platform/server/routes/
  hosts.js               # SSH 호스트 관리 API

platform/server/db/
  hosts.js               # 호스트 DB 쿼리

platform/client/components/
  deployment-progress.tsx  # 배포 진행 단계 UI (SSE, local/remote 모드)
  deployment-status.tsx    # 상태 배지 + 컨테이너 카드
  log-viewer.tsx           # 실시간 로그 뷰어 (SSE)

platform/client/app/
  settings/page.tsx        # SSH 호스트 관리 UI
```

### 수정된 파일
```
platform/server/db/schema.sql         # deployments 테이블 확장 + hosts 테이블
platform/server/db/deployments.js     # 새 필드 + getNextAvailablePorts()
platform/server/routes/deployments.js # Docker 라이프사이클 엔드포인트 추가
platform/server/middleware/auth.js    # SSE용 query param 인증 지원
platform/server/server.js             # hosts 라우트 등록

platform/client/lib/types.ts          # Deployment, Host, ContainerStatus 등 타입
platform/client/lib/api.ts            # deploymentsApi + hostsApi
platform/client/app/launch/page.tsx   # 3가지 모드 (Local/Remote/Manual) + 호스트 선택
platform/client/app/deployments/page.tsx        # 상태 배지 개선
platform/client/app/deployments/[id]/page.tsx   # 라이브 대시보드 (Local/Remote/Manual 배지)
platform/client/components/nav.tsx    # Settings 링크 추가
```

### 런타임 생성 파일
```
~/.tokamak/deployments/<deployment-id>/
  docker-compose.yaml    # 자동 생성된 compose 파일
```

## DB 스키마 변경

`deployments` 테이블에 추가된 컬럼:

| 컬럼 | 타입 | 설명 |
|------|------|------|
| `docker_project` | TEXT | Docker Compose 프로젝트명 |
| `l1_port` | INTEGER | L1 RPC 포트 |
| `l2_port` | INTEGER | L2 RPC 포트 |
| `host_id` | TEXT | 원격 배포 시 호스트 ID (NULL이면 로컬) |
| `phase` | TEXT | 배포 단계 (configured/building/pulling/.../running/error) |
| `bridge_address` | TEXT | 배포된 Bridge 컨트랙트 주소 |
| `proposer_address` | TEXT | 배포된 OnChainProposer 주소 |
| `error_message` | TEXT | 에러 메시지 |

## 대시보드 기능

### Overview 탭
- 컨테이너 상태 카드 (L1, L2, Prover, Deployer)
- RPC 엔드포인트 (L1, L2)
- 배포된 컨트랙트 주소 (Bridge, Proposer)
- 체인 정보 (블록 높이, Chain ID) — 10초 폴링
- 설정 편집 (이름, Chain ID, RPC URL)
- 액션 버튼 (Deploy, Start, Stop, Destroy)

### Logs 탭
- 서비스 선택 (All, L1, L2, Prover, Deployer)
- SSE 기반 실시간 스트리밍
- 검색/필터
- 자동 스크롤

### Config 탭
- 앱별 설정 정보 (ZK 백엔드, Genesis 등)
- programs.toml 다운로드
- 수동 설정 가이드

## 개발/테스트

### 서버 시작
```bash
cd platform/server
npm install
npm run dev
```

### 클라이언트 시작
```bash
cd platform/client
npm install
npm run dev
```

### 배포 테스트
1. http://localhost:3000/launch 접속
2. 앱 선택 (EVM L2, ZK-DEX 등)
3. Local 모드 → "Deploy L2" 클릭
4. 진행 상태 실시간 확인
5. 완료 후 대시보드에서:
   ```bash
   curl http://127.0.0.1:<l2_port> -X POST \
     -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
   ```

### `hosts` 테이블 (신규)

| 컬럼 | 타입 | 설명 |
|------|------|------|
| `id` | TEXT PK | UUID |
| `user_id` | TEXT | 소유자 |
| `name` | TEXT | 서버 이름 |
| `hostname` | TEXT | IP 또는 도메인 |
| `port` | INTEGER | SSH 포트 (기본 22) |
| `username` | TEXT | SSH 사용자명 |
| `auth_method` | TEXT | 인증 방식 (key) |
| `private_key` | TEXT | SSH 개인키 |
| `status` | TEXT | untested/active/error/no_docker |
| `last_tested` | INTEGER | 마지막 테스트 시각 |

## 원격 배포 (Remote Deployment)

### 개요
프리빌트 Docker 이미지를 원격 서버에서 풀 받아 실행하는 방식. 소스 빌드가 필요 없어 빠르다.

### 이미지 레지스트리
```
ghcr.io/tokamak-network/ethrex:l1          # L1 노드
ghcr.io/tokamak-network/ethrex:main-l2     # EVM L2 (기본)
ghcr.io/tokamak-network/ethrex:sp1         # SP1 ZK 프로바 (zk-dex용)
ghcr.io/tokamak-network/ethrex:deployer    # 컨트랙트 배포기
```

### 설정 방법
1. **Settings** 페이지에서 SSH 서버 추가
2. SSH 키 업로드 또는 붙여넣기
3. "Test" 버튼으로 SSH + Docker 연결 확인
4. Launch 페이지에서 "Remote Server" 선택 후 서버 지정

### 원격 배포 흐름
1. SFTP로 `docker-compose.yaml` + 설정 파일 업로드
2. `docker compose pull` — 레지스트리에서 이미지 풀
3. `docker compose up -d` — 서비스 시작
4. SSH 터널 통해 헬스체크 + 컨트랙트 주소 추출
5. 완료 후 원격 서버의 포트로 직접 접근

## 의존성

서버 의존성:
- `ssh2` — 원격 SSH 연결 + SFTP (원격 배포 시)

Docker 필수 요구사항:
- Docker Engine 20.10+
- Docker Compose V2 (`docker compose` CLI)
