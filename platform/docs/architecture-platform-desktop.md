# Platform vs Desktop App 역할 분리 아키텍처

> Date: 2026-03-06
> Branch: `feat/desktop-app-with-ai-chat`

## 개요

Tokamak 서비스는 **Platform (Showroom)**과 **Desktop App (Factory)** 두 가지로 분리됩니다.

```
Platform (Showroom)                    Desktop App (Factory)
========================               ========================
공개 카탈로그, 인증, 레지스트리           앱체인 생성/실행, 모니터링, AI

웹 서비스 (Next.js + Express)           네이티브 앱 (Tauri + React)
Firebase Hosting                       macOS / Linux 배포
누구나 접근 가능                         사용자 로컬 머신에서 실행
```

## 역할 구분

### Platform = Showroom (전시장)

**"보여주고, 등록하고, 찾는다"**

| 기능 | 설명 |
|------|------|
| Guest Program Store | 프로그램 카탈로그 조회/등록/승인 |
| Open Appchain Showroom | 공개된 앱체인 목록 열람 |
| 사용자 인증 | Email/Google 로그인, 세션 관리 |
| Creator 관리 | 프로그램 등록, ELF 업로드, VK 관리 |
| Admin | 프로그램 승인/거절, 사용자 관리 |
| Open Appchain 등록 | Desktop에서 생성한 앱체인을 공개 등록 |

**하지 않는 것:**
- Docker 실행/관리
- 프로세스 스폰/라이프사이클
- 로그 스트리밍
- 모니터링
- SSH 원격 배포

### Desktop App = Factory (공장)

**"만들고, 실행하고, 관리한다"**

| 기능 | 설명 |
|------|------|
| 앱체인 생성 | Local/Testnet/Mainnet 원클릭 생성 |
| 프로세스 관리 | ethrex L2 프로세스 스폰/종료 |
| 실시간 로그 | 프로세스 stdout/stderr 실시간 표시 |
| AI Pilot | 앱체인 상태 인식 AI 어시스턴트 |
| Program Store 브라우저 | Platform API로 프로그램 조회 |
| Open Appchain 등록 | Platform API로 내 앱체인 공개 |
| Platform 로그인 | OS Keychain에 토큰 저장 |
| 월렛 | TON 토큰 관리, L1<>L2 브릿지 |
| Local Server | Docker 배포 엔진 (Express + SQLite) |

## 기술 스택

### Platform

```
platform/
├── server/          Express.js API (port 5001)
│   ├── SQLite       platform.sqlite (users, programs, deployments, sessions)
│   ├── routes/      auth, store, programs, deployments, admin
│   └── .env         JWT_SECRET, GOOGLE_CLIENT_ID
└── client/          Next.js 15 (port 3000)
    ├── app/         store, showroom, launch, creator, admin, settings
    └── lib/api.ts   API 클라이언트
```

### Desktop App

```
crates/desktop-app/
├── ui/
│   ├── src/                React + TypeScript
│   │   ├── App.tsx         ViewType 라우팅 (10개 뷰)
│   │   ├── api/
│   │   │   ├── local-server.ts   Local Server API 클라이언트
│   │   │   └── platform.ts       Platform API 클라이언트 (Keychain 인증)
│   │   └── components/     HomeView, MyL2View, ChatView, etc.
│   └── src-tauri/src/      Rust 백엔드
│       ├── runner.rs       tokio::process::Command (ethrex 프로세스)
│       ├── commands.rs     Tauri IPC 커맨드
│       ├── appchain_manager.rs  앱체인 CRUD + JSON 파일 저장
│       ├── ai_provider.rs  멀티 AI (Claude/GPT/Gemini/Tokamak)
│       └── local_server.rs Node.js local-server 프로세스 관리
└── local-server/           Express.js (port 5002, localhost only)
    ├── server.js
    ├── db/                 SQLite (~/.tokamak-appchain/local.sqlite)
    ├── routes/             deployments, hosts, fs
    └── lib/                docker-local, docker-remote, compose-generator
```

## 데이터 흐름

### 1. 앱체인 생성 (로컬)

```
Desktop App
  └→ CreateL2Wizard (React)
      └→ invoke('create_appchain')  [Tauri IPC]
          └→ AppchainManager.create()  [Rust]
              └→ Runner.spawn("ethrex l2 --dev")  [Rust]
                  └→ tokio::process::Command  [OS Process]
```

Platform은 관여하지 않음.

### 2. Program Store 조회

```
Desktop App                          Platform
  └→ ProgramStoreView (React)
      └→ fetch('/api/store/programs')  ──→  Express (port 5001)
                                              └→ SQLite: programs table
                                           ←──  { programs: [...] }
```

인증 불필요. 공개 API.

### 3. Open Appchain 등록

```
Desktop App                          Platform
  └→ L2DetailView: 공개 토글 ON
      ├→ platformAPI.registerDeployment()  ──→  POST /api/deployments
      │   (Authorization: Bearer <keychain-token>)     └→ SQLite: deployments
      ├→ platformAPI.activateDeployment()  ──→  POST /api/deployments/:id/activate
      └→ invoke('update_appchain_public')  [Tauri IPC, 로컬 상태 업데이트]
```

인증 필요. Desktop이 Keychain에서 토큰 읽어서 전송.

### 4. Showroom 조회

```
브라우저 (또는 Desktop)              Platform
  └→ /showroom 페이지
      └→ fetch('/api/store/appchains')  ──→  Express
                                              └→ SQLite: deployments (status='active')
                                           ←──  { appchains: [...] }
```

인증 불필요. 공개 API.

### 5. AI Pilot

```
Desktop App (전부 로컬)
  └→ ChatView
      ├→ invoke('get_chat_context')     → AppchainManager → 앱체인 상태 JSON
      └→ invoke('send_chat_message')    → AiProvider
          ├→ system prompt + context 주입
          └→ Claude/GPT/Gemini API 호출 (외부)
              └→ 응답에 [ACTION:navigate:view=store] 포함 시
                  → ChatView가 파싱 → 클릭 가능 버튼 렌더링
```

Platform은 관여하지 않음. AI API 키는 OS Keychain에 저장.

## 인증 체계

### Platform (웹)
- Email/Password → bcrypt 해시 → session token → localStorage
- Google OAuth → ID Token → 서버 검증 → session token
- 세션: SQLite `sessions` 테이블, 만료 자동 정리

### Desktop App → Platform
- Settings에서 Email/Password로 Platform 로그인
- Token을 **OS Keychain** (`keyring` crate)에 저장
- Platform API 호출 시 `Authorization: Bearer <token>` 헤더 첨부
- 앱 재시작 시 Keychain에서 토큰 복원

## 저장소

| 위치 | 용도 | 형식 |
|------|------|------|
| `~/.tokamak-appchain/appchains.json` | 앱체인 목록/설정 | JSON (Rust) |
| `~/.tokamak-appchain/local.sqlite` | Docker 배포 정보 | SQLite (local-server) |
| `platform/server/db/platform.sqlite` | 사용자/프로그램/세션 | SQLite (Platform) |
| OS Keychain | AI API 키, Platform 토큰 | 암호화 (OS 관리) |

## 포트 사용

| Port | Service | 위치 |
|------|---------|------|
| 1420 | Tauri Dev (Vite) | Desktop |
| 3000 | Platform Client (Next.js) | Platform |
| 5001 | Platform API Server | Platform |
| 5002 | Local Server (Docker 엔진) | Desktop |

## 코드 이전 히스토리

Phase 1에서 Platform → Desktop으로 이전된 코드:

| 이전된 코드 | From (Platform) | To (Desktop local-server) |
|-------------|-----------------|---------------------------|
| Docker lifecycle | `lib/deployment-engine.js` | `local-server/lib/` |
| Docker compose | `lib/compose-generator.js` | `local-server/lib/` |
| Docker local/remote | `lib/docker-local.js`, `docker-remote.js` | `local-server/lib/` |
| RPC health check | `lib/rpc-client.js` | `local-server/lib/` |
| Host management | `routes/hosts.js`, `db/hosts.js` | `local-server/routes/`, `db/` |
| File system browse | `routes/fs.js` | `local-server/routes/` |

Platform에서 삭제된 항목 (~2,900줄):
- `lib/deployment-engine.js`, `docker-local.js`, `docker-remote.js`, `compose-generator.js`, `rpc-client.js`
- `routes/hosts.js`, `routes/fs.js`, `db/hosts.js`
- `db/schema.sql`에서 `hosts` 테이블, deployments의 Docker 컬럼들
- Client: `deployment-progress.tsx`, `directory-picker.tsx`, `log-viewer.tsx`

## 비즈니스 룰

| 규칙 | 적용 위치 |
|------|----------|
| Native Token = TON (TOKAMAK), 변경 불가 | Desktop |
| Prover = SP1, 변경 불가 | Desktop |
| Local 모드: Open Appchain 등록 불가 | Desktop |
| Testnet/Mainnet만 Open Appchain 등록 가능 | Desktop → Platform |
| Program 등록: admin 승인 필요 | Platform |
| Program Store 조회: 인증 불필요 | Platform (공개) |
| Showroom 조회: 인증 불필요 | Platform (공개) |

## 향후 확장

- Platform을 Firebase Hosting에 static export 배포 (production)
- Desktop App을 GitHub Releases에 바이너리 배포
- Platform API를 `https://platform.tokamak.network`으로 운영
- Desktop은 환경변수/설정으로 Platform API URL을 전환 (local ↔ production)
