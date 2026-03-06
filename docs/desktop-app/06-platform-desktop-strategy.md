# Platform & Desktop App - 제품 전략 및 통합 설계 (v2)

> 이 문서는 실제 코드 분석을 기반으로 작성되었습니다.
> Platform: `feat/l2-deployment-engine` 브랜치의 배포 엔진 포함
> Desktop: `feat/app-customized-framework` 브랜치의 최신 상태

---

## 1. 두 제품의 현재 구현 상태

### Platform (웹) - 실제 구현 분석

| 항목 | 내용 |
|------|------|
| **정체성** | Guest Program Store + L2 Deployment Engine |
| **기술 스택** | Next.js + Express + SQLite + Docker Compose |
| **배포 방식** | Docker 기반 (로컬 빌드 / 원격 서버 SSH / 수동 설정) |

**구현된 기능:**

| 모듈 | 파일 | 기능 |
|------|------|------|
| **프로그램 스토어** | `routes/store.js`, `routes/programs.js` | 프로그램 등록/승인/검색, ELF/VK 업로드 |
| **배포 엔진** | `lib/deployment-engine.js` | 10단계 상태 머신 (configured → running), SSE 실시간 진행 |
| **Compose 생성** | `lib/compose-generator.js` | 앱 프로필별 docker-compose.yaml 자동 생성 |
| **로컬 Docker** | `lib/docker-local.js` | `docker compose` CLI 래퍼, 이미지 빌드/서비스 관리 |
| **원격 Docker** | `lib/docker-remote.js` | SSH + SFTP로 원격 서버 배포, 사전 빌드 이미지 사용 |
| **호스트 관리** | `routes/hosts.js`, `db/hosts.js` | SSH 서버 등록/테스트/관리 |
| **모니터링** | `routes/deployments.js` | JSON-RPC 폴링 (블록 높이, 체인 ID, 잔액) |
| **도구 서비스** | Blockscout, Bridge UI, Dashboard | 선택적 도구 (실패해도 메인 배포에 영향 없음) |
| **인증** | OAuth (Google, Naver, Kakao) | 세션 기반 사용자 관리 |

**앱 프로필 (APP_PROFILES):**

| 프로필 | Dockerfile | 빌드 피처 | Genesis | 프로버 |
|--------|-----------|----------|---------|--------|
| evm-l2 | Default | `l2,l2-sql` | `l2.json` | exec (no-op) |
| zk-dex | `Dockerfile.sp1` | `l2,l2-sql,sp1` | `l2-zk-dex.json` | sp1 |
| tokamon | Default | `l2,l2-sql` | `l2.json` | exec |

**배포 흐름 (로컬):**
```
configured → checking_docker → building → l1_starting
→ deploying_contracts → l2_starting → starting_prover
→ starting_tools → running
```

**배포 흐름 (원격):**
```
configured → pulling → l1_starting → deploying_contracts
→ l2_starting → starting_prover → running
```

### Desktop App (네이티브) - 실제 구현 분석

| 항목 | 내용 |
|------|------|
| **정체성** | Tokamak Appchain - 로컬 L2 컨트롤 센터 |
| **기술 스택** | Tauri 2.x + React + Rust + tokio |
| **배포 방식** | 직접 프로세스 스포닝 (`ethrex l2 --dev --no-monitor`) |

**구현된 기능:**

| 모듈 | 파일 | 기능 |
|------|------|------|
| **프로세스 관리** | `runner.rs` | ethrex 바이너리 탐색/빌드, tokio 프로세스 스포닝, stdout 파싱 |
| **앱체인 상태** | `appchain_manager.rs` | JSON 기반 설정 저장 (`~/.tokamak-appchain/`), 설정 단계 추적 |
| **생성 마법사** | `CreateL2Wizard.tsx` | 4단계 (네트워크 → 기본정보 → 네트워크설정 → 토큰/공개) |
| **AI 채팅** | `ai_provider.rs`, `ChatView.tsx` | 4개 프로바이더 (Tokamak/Claude/GPT/Gemini), 키링 저장 |
| **오픈 앱체인** | `OpenL2View.tsx` | 하드코딩된 8개 샘플 L2 목록 |
| **홈 화면** | `HomeView.tsx` | Hero + Journey 3단계 + Quick Links |
| **진행 추적** | `SetupProgressView.tsx` | 1초 폴링, 로그 500줄 유지 |

**앱체인 설정 구조:**
```
AppchainConfig {
  id, name, icon, chain_id, description,
  network_mode: Local | Testnet | Mainnet,
  l1_rpc_url, l2_rpc_port, sequencer_mode,
  native_token: "TON" (고정),
  prover_type: "sp1" (고정),
  bridge_address, on_chain_proposer_address,
  is_public, hashtags, status, created_at
}
```

**현재 실제로 동작하는 것:**
- Local 모드: `ethrex l2 --dev` 한 줄로 L1+계약배포+L2 한 번에 실행
- Testnet/Mainnet: stub 구현 (미완성)

---

## 2. 핵심 발견: 기능 중복과 차이

### 2.1 기능 중복 매트릭스

| 기능 | Platform (웹) | Desktop (네이티브) | 중복도 |
|------|--------------|-------------------|--------|
| **L2 생성 마법사** | 3단계 (앱선택→설정→배포) | 4단계 (네트워크→정보→설정→토큰) | **높음** |
| **L2 실행** | Docker Compose 기반 | 직접 프로세스 스포닝 | **다름** |
| **실시간 진행** | SSE 이벤트 스트림 | 1초 폴링 | **다름** |
| **로그 뷰어** | 서비스별 필터, 검색, 스트리밍 | 단순 텍스트 (500줄) | Platform 우위 |
| **모니터링** | JSON-RPC 폴링 (블록높이, 잔액) | 없음 | Platform만 |
| **원격 배포** | SSH + 사전빌드 이미지 | 없음 (로컬만) | Platform만 |
| **도구 서비스** | Blockscout, Bridge UI, Dashboard | 없음 | Platform만 |
| **앱 프로필** | evm-l2, zk-dex, tokamon | evm-l2만 (기본) | Platform 우위 |
| **AI 채팅** | 없음 | 4개 프로바이더 지원 | Desktop만 |
| **오픈 앱체인** | Deployment DB (실데이터) | 하드코딩 샘플 8개 | Platform 우위 |
| **지갑** | 없음 | 멀티토큰 지갑 | Desktop만 |
| **시스템 트레이** | 해당 없음 | 백그라운드 실행 | Desktop만 |
| **GPU 감지** | NVIDIA 자동 감지 | 없음 | Platform만 |

### 2.2 아키텍처 차이의 근본 원인

```
Platform의 접근법:                    Desktop의 접근법:
─────────────────                    ─────────────────
Docker Compose로 격리               OS 프로세스 직접 스포닝
서비스별 독립 컨테이너              단일 ethrex 바이너리
이미지 빌드 → 컨테이너 실행         cargo build → 바이너리 실행
포트 자동 할당 (충돌 방지)          고정 포트 (8550)
다중 L2 동시 실행 가능              단일 L2 (포트 충돌 가능)
웹 브라우저에서 관리                네이티브 앱에서 관리
```

### 2.3 현재 단절 지점 (코드 근거)

1. **Platform에서 L2를 Docker로 실행하지만** Desktop은 이 사실을 모름
   - Platform: `deployment-engine.js` → `docker-local.js` → Docker Compose
   - Desktop: `runner.rs` → `tokio::process::Command` → 직접 바이너리
   - 같은 ethrex인데 실행 방식이 완전히 다름

2. **Desktop의 앱체인 마법사에 Guest Program 선택이 없음**
   - Platform: `launch/page.tsx` Step 1에서 Store 프로그램 선택
   - Desktop: `CreateL2Wizard.tsx`에 프로그램 개념 자체가 없음

3. **오픈 앱체인 데이터가 분리됨**
   - Platform: `deployments` 테이블의 실제 배포 데이터
   - Desktop: `OpenL2View.tsx`의 하드코딩된 `SAMPLE_OPEN_L2S` 배열

4. **로그/모니터링 역량 차이**
   - Platform: `log-viewer.tsx` (서비스별, 검색, 스트리밍, 2000줄)
   - Desktop: `SetupProgressView.tsx` (단순 텍스트, 500줄, 폴링)

5. **AI가 Platform을 모름**
   - Desktop `ai_provider.rs`의 시스템 프롬프트에 Platform/Store 정보 없음
   - Function Calling 미구현 (텍스트 응답만)

---

## 3. 통합 전략: 역할 재정의

### 3.1 핵심 질문

> **두 제품이 동일한 L2 배포 기능을 각각 다르게 구현하고 있다.
> 이것은 중복인가, 아니면 각각의 존재 이유가 있는가?**

**답: 중복이 아니라, 각각의 배포 대상이 다르다.**

- Platform은 **웹 서비스**이므로 사용자의 로컬 PC에서 프로세스를 직접 띄울 수 없다.
- 로컬 PC에서 L2를 실행하려면 반드시 **로컬에 설치된 에이전트**가 필요하다.
- 그 에이전트가 바로 **Desktop App**이다.

### 3.2 역할 재정의

```
Platform = "쇼룸" (웹 서비스)
  - 프로그램 등록, 검증, 승인
  - Guest Program Store (마켓플레이스)
  - 오픈 앱체인 쇼룸 (공개된 L2 목록)
  - 사용자 인증/프로필/커뮤니티

Desktop = "공장 + 운전석" (네이티브 앱)
  - L2 배포/실행 (로컬 + 원격 모두)
  - 로그 뷰어, 모니터링, 컨트롤 (시작/중지/재시작)
  - 배포 진행 추적 (실시간)
  - 도구 서비스 관리 (Blockscout, Bridge UI 등)
  - AI 어시스턴트, 지갑
  - 공개 여부 결정 → Platform에 등록
```

### 3.3 배포와 공개의 흐름

```
[배포] Desktop이 모든 배포를 담당
  Desktop ──→ 로컬 PC (Docker Compose / 직접 프로세스)
  Desktop ──→ 원격 서버 (SSH)

[관리] Desktop이 로컬에서 직접 컨트롤
  Desktop: 로그 뷰어, 모니터링, 시작/중지/재시작
  Desktop: Blockscout, Bridge UI 등 도구 관리

[공개] 사용자가 Desktop에서 결정
  Desktop → "이 L2를 공개할래?" → Yes → Platform API로 등록
  Platform: 쇼룸에 오픈 앱체인으로 표시

[발견] 다른 사용자가 Platform에서 탐색
  Platform 쇼룸 → "이 L2에 접속하고 싶어"
  → 자기 Desktop에서 접속 (RPC 연결)
```

### 3.4 Platform에서 Desktop으로 이관할 기능

Platform에 현재 구현되어 있지만, 아키텍처상 Desktop에 있어야 하는 기능들:

| Platform 현재 구현 | Desktop으로 이동 | 참고 코드 |
|---|---|---|
| 배포 엔진 (상태 머신) | Rust 백엔드로 포팅 | `lib/deployment-engine.js` |
| Docker Compose 생성 | Rust로 포팅 | `lib/compose-generator.js` |
| 로컬 Docker 관리 | Rust CLI 래퍼 | `lib/docker-local.js` |
| 원격 Docker (SSH) | Rust SSH 클라이언트 | `lib/docker-remote.js` |
| 배포 진행 UI | React 컴포넌트 포팅 | `deployment-progress.tsx` |
| 로그 뷰어 UI | React 컴포넌트 포팅 | `log-viewer.tsx` |
| 모니터링 (RPC 폴링) | Rust JSON-RPC 클라이언트 | `routes/deployments.js` |
| 배포 상세 대시보드 | React 페이지 포팅 | `deployments/[id]/page.tsx` |
| 호스트 관리 (SSH 서버) | Rust + Desktop UI | `routes/hosts.js` |

### 3.5 통합 원칙

> **Platform은 쇼룸이고, Desktop이 공장이다.**
>
> - 배포/실행/관리: Desktop이 전부 담당 (로컬 + 원격)
> - 프로그램 탐색: Desktop → Platform Store API 조회
> - 공개: Desktop에서 결정 → Platform 쇼룸에 등록
> - 발견: 다른 사용자 → Platform에서 오픈 앱체인 탐색 → Desktop으로 접속
> - 오프라인: Desktop 단독으로 기본 EVM-L2 개발 가능

**연결은 가볍게:**
```
Platform ←──API──→ Desktop

연결 포인트 (4개만):
1. 프로그램 조회:    Desktop → Platform Store API
2. 오픈 앱체인 등록: Desktop → Platform에 공개 L2 등록
3. 오픈 앱체인 탐색: Desktop ← Platform 공개 L2 목록 조회
4. 인증:            Desktop ← Platform OAuth 계정 재사용
```

---

## 4. 구체적 통합 설계

### 4.1 배포 방식 통합

**현재:** 두 가지 독립 배포 파이프라인
```
Platform: Store → Docker Compose → 컨테이너 (로컬/원격)
Desktop:  마법사 → cargo build → 프로세스 (로컬만)
```

**목표:** Desktop이 로컬 배포의 실행 주체, Platform이 관제탑

```
[로컬 배포 - Desktop이 실행]
  1. Desktop 마법사에서 프로그램 선택 (Platform Store API 조회)
  2. Desktop이 로컬에서 Docker Compose 또는 직접 프로세스로 L2 실행
  3. Desktop이 Platform에 배포 상태를 보고 (heartbeat, 로그, 메트릭)
  4. Platform 대시보드에서 로컬 L2의 로그/모니터링/컨트롤 가능

[원격 배포 - Platform이 직접 실행]
  1. Platform 웹에서 프로그램 선택 + 원격 서버 지정
  2. Platform이 SSH로 원격 서버에 Docker Compose 배포
  3. Platform 대시보드에서 직접 모니터링/컨트롤

[오프라인 - Desktop 단독]
  1. Desktop 마법사에서 기본 EVM-L2 선택 (Store 없이)
  2. ethrex l2 --dev 직접 프로세스 실행 (기존 방식)
  3. Platform 연결 없이 독립 동작
```

**Desktop에 필요한 추가 기능:**
- Platform의 `compose-generator.js` 로직을 Rust로 포팅 (앱 프로필별 Docker Compose 생성)
- Platform의 `docker-local.js` 로직을 Rust로 포팅 (docker compose CLI 래퍼)
- Platform과의 상태 동기화 모듈 (WebSocket 또는 polling)
- Platform에 로그/메트릭 전송 모듈

### 4.2 Desktop → Platform API 연동

```rust
// platform_client.rs (NEW)

// 읽기 (인증 불필요)
pub async fn fetch_store_programs(query: &str, category: &str) -> Vec<Program>
pub async fn fetch_program_detail(id: &str) -> Program
pub async fn fetch_categories() -> Vec<Category>

// 쓰기 (Platform 인증 필요)
pub async fn create_deployment(config: DeploymentConfig) -> Deployment
pub async fn provision_deployment(id: &str, host_id: Option<&str>) -> ()
pub async fn get_deployment_status(id: &str) -> DeploymentStatus
pub async fn get_deployment_events(id: &str) -> EventStream  // SSE
pub async fn stop_deployment(id: &str) -> ()
pub async fn destroy_deployment(id: &str) -> ()
pub async fn get_deployment_logs(id: &str, service: &str) -> String
pub async fn get_monitoring(id: &str) -> MonitoringData
```

### 4.3 앱체인 생성 마법사 확장

현재 4단계 → 5단계로 확장:

```
Step 0 (NEW): 앱 유형 선택
  ├── "기본 EVM L2" (오프라인, 기존 방식)
  ├── Platform Store에서 선택 (온라인)
  │   ├── 카테고리 필터 (DeFi, Gaming, NFT, ...)
  │   ├── Platform의 GET /api/store/programs 호출
  │   └── 프로그램 상세 (설명, 크리에이터, 사용 수)
  └── "ZK-DEX" (로컬 프로필, evm-l2/zk-dex/tokamon)

Step 1: 네트워크 선택 (Local / Testnet / Mainnet)
Step 2: 기본 정보 (이름, 아이콘, 설명)
Step 3: 네트워크 설정 (L1 RPC, L2 포트)
Step 4: 배포 방식 선택 (NEW for Testnet/Mainnet)
  ├── "내 PC에서 실행" (직접 프로세스)
  ├── "Docker로 실행" (Platform 배포 엔진)
  └── "원격 서버에 배포" (Platform SSH 배포)
```

### 4.4 오픈 앱체인 통합

**현재:** `OpenL2View.tsx`의 하드코딩 → Platform API로 교체

```typescript
// Before (하드코딩)
const SAMPLE_OPEN_L2S = [
  { name: "Tokamak DEX", operator: "tokamak-team", ... },
  ...
];

// After (Platform API)
const [openL2s, setOpenL2s] = useState([]);
useEffect(() => {
  invoke('platform_fetch_deployments', { status: 'running', isPublic: true })
    .then(setOpenL2s);
}, []);
```

### 4.5 로그/모니터링 통합

Desktop의 약한 로그 뷰어를 강화하는 두 가지 방법:

```
방법 A: Platform의 LogViewer를 Desktop에 포팅
  - React 컴포넌트이므로 거의 그대로 사용 가능
  - 서비스별 필터, 검색, 스트리밍 지원

방법 B: Platform 배포 시 Platform 웹 대시보드 링크 제공
  - Desktop에서 "상세 대시보드 열기" → 브라우저에서 Platform 열기
  - Desktop은 요약 정보만, 상세는 Platform에 위임
```

### 4.6 AI Pilot 강화

**현재 시스템 프롬프트에 추가할 내용:**

```
도구(Tools):
  // 기존
  - create_appchain(config) → 로컬 앱체인 생성
  - start_appchain(id) → 앱체인 시작
  - stop_appchain(id) → 앱체인 중지

  // Platform 연동 (NEW)
  - search_store(query, category) → Guest Program Store 검색
  - get_program_detail(id) → 프로그램 상세
  - deploy_with_program(program_slug, config) → 프로그램으로 L2 배포
  - get_deployment_status(id) → 배포 상태 조회
  - get_deployment_logs(id, service) → 서비스 로그 조회

컨텍스트:
  - Platform Store에는 다양한 Guest Program이 등록되어 있음
  - 앱 프로필: evm-l2 (기본 EVM), zk-dex (ZK-DEX), tokamon (토카몬)
  - 사용자가 "DEX L2"를 원하면 zk-dex 프로필을 추천
  - 로컬 개발은 직접 실행, 프로덕션은 Platform 배포 엔진 사용
```

### 4.7 계정 통합

```
Desktop "설정" → "Platform 연결"
  → 브라우저에서 Platform OAuth 로그인
  → 콜백으로 세션 토큰 수신
  → OS Keychain에 저장 (기존 ai_provider.rs의 keyring 패턴 재사용)
  → 이후 Platform API 호출 시 Authorization 헤더에 토큰 포함
```

### 4.8 Deep Link

```
Platform 웹 → "Desktop에서 열기" 버튼:
  tokamak://launch?program=zk-dex&name=ZK-DEX

Tauri Deep Link 핸들러:
  → 앱체인 마법사를 프로그램 미리 선택된 상태로 열기
  → Step 0 건너뛰고 Step 1부터 시작
```

---

## 5. 기능별 소유권 정리

| 기능 | Platform (쇼룸) | Desktop (공장) | 비고 |
|------|:---:|:---:|:---|
| 프로그램 등록/검증/승인 | **O** | - | 쇼룸 핵심 역할 |
| Guest Program Store | **O** | - | Desktop에서 API 조회 |
| 오픈 앱체인 쇼룸 | **O** | - | Desktop에서 조회+등록 |
| 사용자 인증 (OAuth) | **O** | - | Desktop에서 재사용 |
| 커뮤니티/프로필 | **O** | - | - |
| **L2 배포 (로컬)** | - | **O** | Docker/프로세스 |
| **L2 배포 (원격)** | - | **O** | SSH 배포 |
| **로그 뷰어** | - | **O** | 서비스별, 검색, 스트리밍 |
| **모니터링/컨트롤** | - | **O** | 시작/중지/재시작, RPC 폴링 |
| **배포 진행 추적** | - | **O** | 실시간 상태 머신 |
| **도구 관리** | - | **O** | Blockscout, Bridge UI 등 |
| **앱체인 생성 마법사** | - | **O** | Store에서 프로그램 선택 포함 |
| **공개 여부 결정** | - | **O** | Platform에 등록 여부 선택 |
| AI 채팅 (Pilot) | - | **O** | Platform Store 검색 연동 |
| 지갑 | - | **O** | - |
| 시스템 트레이 | - | **O** | 백그라운드 에이전트 |
| 오프라인 개발 | - | **O** | Platform 없이 독립 동작 |

---

## 6. 구현 우선순위

### Phase 1: 연결 기반 구축 (읽기 연동)

**Desktop에서 Platform 데이터 조회 가능하게:**

| 작업 | 파일 | 난이도 |
|------|------|--------|
| `platform_client.rs` 생성 | Desktop backend (NEW) | 중 |
| Store 프로그램 조회 커맨드 | `commands.rs` | 하 |
| 오픈 앱체인 화면 실데이터 교체 | `OpenL2View.tsx` | 중 |
| 앱체인 마법사에 프로그램 선택 추가 | `CreateL2Wizard.tsx` | 중 |

### Phase 2: 인증 + 쓰기 연동

| 작업 | 파일 | 난이도 |
|------|------|--------|
| Platform OAuth 로그인 | Desktop settings + keyring | 중 |
| 배포 엔진 API 연동 | `platform_client.rs` | 중 |
| SSE 이벤트 수신 (배포 진행) | Rust reqwest + eventsource | 상 |
| 배포 상태/모니터링 대시보드 | Desktop UI (NEW) | 중 |

### Phase 3: AI + Deep Link

| 작업 | 파일 | 난이도 |
|------|------|--------|
| AI Function Calling 구현 | `ai_provider.rs` | 상 |
| Platform Store 검색 도구 | AI system prompt + tools | 중 |
| Deep Link 핸들러 | Tauri plugin-deep-link | 중 |
| Platform 웹에 "Desktop에서 열기" 버튼 | `store/[id]/page.tsx` | 하 |

### Phase 4: 고급 기능

| 작업 | 파일 | 난이도 |
|------|------|--------|
| Desktop Docker 지원 (로컬) | `runner.rs` 확장 | 상 |
| 크리에이터 도구 (ELF 빌드+업로드) | Desktop + Platform | 상 |
| 앱체인 간 브릿지 UI | Desktop wallet 확장 | 상 |

---

## 7. 요약

### 최종 역할 정리

| | Platform (쇼룸) | Desktop (공장) |
|-|:---:|:---:|
| **핵심 역할** | 프로그램을 보여주는 곳 | L2를 만들고 운영하는 곳 |
| **비유** | 갤러리/전시장 | 작업실/공장 |
| **배포** | - | 로컬 + 원격 모두 |
| **관리** | - | 로그, 모니터링, 컨트롤 |
| **프로그램** | 등록/검증/전시 | 선택/다운로드/실행 |
| **공개** | 쇼룸에 게시 | 공개 여부 결정 |
| **사용자 행동** | 탐색, 발견 | 실행, 관리, 접속 |
| **AI** | - | Pilot이 Store 검색 + 실행 |

> **Platform은 갤러리이고, Desktop은 작업실이다.**
> 갤러리에서 마음에 드는 작품(프로그램)을 고르고,
> 작업실(Desktop)에서 직접 만들고 운영한다.
> 잘 만든 작품은 다시 갤러리에 전시(공개)할 수 있다.
> 작업실에는 AI 조수가 있어서 "DEX 만들어줘"하면 알아서 한다.
