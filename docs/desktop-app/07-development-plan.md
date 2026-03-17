# 개발 계획서: Platform-Desktop 코드 분리 및 통합

> 기반 문서: `06-platform-desktop-strategy.md`
> 브랜치: `feat/desktop-app-with-ai-chat`

---

## Phase 1: 코드 분리 + local-server 구축

### 1.1 local-server 프로젝트 생성

Desktop이 로컬에서 띄우는 웹 서버. Platform에서 배포 관련 코드를 가져온다.

```
crates/desktop-app/local-server/
  ├── package.json
  ├── server.js                    ← Express 서버 (포트 5002)
  ├── routes/
  │   ├── deployments.js           ← 실행/중지/로그/모니터링/도구
  │   ├── hosts.js                 ← SSH 서버 관리
  │   └── fs.js                    ← 디렉토리 선택
  ├── lib/
  │   ├── deployment-engine.js     ← platform/server/lib/ 에서 복사
  │   ├── compose-generator.js
  │   ├── docker-local.js
  │   ├── docker-remote.js
  │   └── rpc-client.js
  ├── db/
  │   ├── db.js                    ← SQLite 초기화 (~/.tokamak-appchain/local.sqlite)
  │   ├── schema.sql               ← deployments, hosts 테이블만
  │   ├── deployments.js
  │   └── hosts.js
  └── client/                      ← Next.js 관리 UI
      ├── package.json
      ├── app/
      │   ├── page.tsx             ← 대시보드 (배포 목록 + 요약)
      │   ├── layout.tsx
      │   ├── deployments/
      │   │   ├── page.tsx         ← 배포 목록
      │   │   └── [id]/page.tsx    ← 배포 상세 (로그, 모니터링, 컨트롤)
      │   ├── launch/
      │   │   └── page.tsx         ← 배포 마법사 (프로그램 선택 → 설정 → 실행)
      │   └── settings/
      │       └── page.tsx         ← SSH 서버 관리
      └── components/
          ├── deployment-progress.tsx
          ├── deployment-status.tsx
          ├── directory-picker.tsx
          └── log-viewer.tsx
```

**작업 내역:**

| # | 작업 | 소스 | 대상 | 비고 |
|---|------|------|------|------|
| 1 | Express 서버 생성 | 신규 | `local-server/server.js` | 포트 5002, CORS localhost |
| 2 | 배포 엔진 복사 | `platform/server/lib/` | `local-server/lib/` | 5개 파일 그대로 |
| 3 | 배포 라우트 분리 | `platform/server/routes/deployments.js` | `local-server/routes/deployments.js` | 실행/로그/모니터링 엔드포인트만 |
| 4 | hosts, fs 라우트 이동 | `platform/server/routes/` | `local-server/routes/` | 그대로 복사 |
| 5 | DB 스키마 분리 | `platform/server/db/schema.sql` | `local-server/db/schema.sql` | deployments, hosts만 |
| 6 | DB 모듈 복사 | `platform/server/db/` | `local-server/db/` | deployments.js, hosts.js |
| 7 | package.json 생성 | 신규 | `local-server/package.json` | express, better-sqlite3, ssh2 |

### 1.2 local-server 클라이언트 생성

| # | 작업 | 소스 | 대상 | 비고 |
|---|------|------|------|------|
| 8 | Next.js 프로젝트 초기화 | 신규 | `local-server/client/` | |
| 9 | 배포 상세 페이지 복사 | `platform/client/app/deployments/` | `local-server/client/app/deployments/` | auth 의존 제거 |
| 10 | launch 페이지 복사 | `platform/client/app/launch/` | `local-server/client/app/launch/` | Step1 프로그램선택 수정 |
| 11 | settings 페이지 복사 | `platform/client/app/settings/` | `local-server/client/app/settings/` | 그대로 |
| 12 | 컴포넌트 복사 | `platform/client/components/` | `local-server/client/components/` | deployment-*, log-viewer, directory-picker |
| 13 | api.ts 분리 | `platform/client/lib/api.ts` | `local-server/client/lib/api.ts` | deploymentsApi, hostsApi만 |
| 14 | types.ts 복사 | `platform/client/lib/types.ts` | `local-server/client/lib/types.ts` | 필요한 타입만 |
| 15 | 대시보드 홈 생성 | 신규 | `local-server/client/app/page.tsx` | 내 배포 요약 + 바로가기 |

### 1.3 Platform 정리

| # | 작업 | 설명 |
|---|------|------|
| 16 | `routes/deployments.js`에서 실행 관련 엔드포인트 제거 | CRUD + 공개 배포 조회만 남김 |
| 17 | `lib/` 배포 엔진 파일 제거 | deployment-engine, compose-generator, docker-*, rpc-client |
| 18 | `routes/hosts.js`, `routes/fs.js` 제거 | Desktop으로 이동됨 |
| 19 | `db/hosts.js` 제거, `schema.sql`에서 hosts 테이블 제거 | |
| 20 | 클라이언트 배포 관련 페이지 제거 | deployments/, settings/ 제거 |
| 21 | 클라이언트 배포 관련 컴포넌트 제거 | deployment-*, log-viewer, directory-picker |
| 22 | `launch/page.tsx` 수정 | "Desktop 앱 다운로드" 가이드 페이지로 변경 |
| 23 | `profile/page.tsx` 수정 | "내 배포" 섹션 제거 |
| 24 | `lib/api.ts` 수정 | deploymentsApi, hostsApi 제거 |
| 25 | `server.js`에서 hosts, fs 라우트 제거 | |

### 1.4 Desktop에서 local-server 스포닝

Tauri 앱이 시작할 때 local-server를 자식 프로세스로 띄운다.

**설계:**

```rust
// src-tauri/src/local_server.rs (NEW)

pub struct LocalServer {
    process: Mutex<Option<Child>>,
    server_port: u16,    // 5002
    client_port: u16,    // 3002
}

impl LocalServer {
    pub async fn start(&self) -> Result<(), String> {
        // 1. node local-server/server.js 스포닝 (포트 5002)
        // 2. next dev (또는 next start) 스포닝 (포트 3002)
        // 3. 프로세스를 self.process에 저장
    }

    pub async fn stop(&self) -> Result<(), String> {
        // kill child processes
    }

    pub fn dashboard_url(&self) -> String {
        format!("http://localhost:{}", self.client_port)
    }
}
```

**Tauri 연동:**

```rust
// lib.rs 수정
// 앱 시작 시 local-server 자동 시작
// 앱 종료 시 local-server 자동 종료

.manage(Arc::new(LocalServer::new()))
.setup(|app| {
    let server = app.state::<Arc<LocalServer>>();
    tauri::async_runtime::spawn(async move {
        server.start().await;
    });
    Ok(())
})
```

| # | 작업 | 파일 | 설명 |
|---|------|------|------|
| 26 | `local_server.rs` 생성 | `src-tauri/src/local_server.rs` | Node.js 서버 + Next.js 클라이언트 스포닝 |
| 27 | lib.rs에 등록 | `src-tauri/src/lib.rs` | managed state + setup hook |
| 28 | `open_dashboard` 커맨드 추가 | `src-tauri/src/commands.rs` | 브라우저에서 local-server UI 열기 |
| 29 | Node.js 번들링 방법 결정 | - | 앱 배포 시 node_modules 포함 방법 |

---

## Phase 2: Desktop 배포 기능 통합

### 2.1 Desktop 앱 UI에서 local-server 연동

Desktop 앱(Tauri)의 기존 화면에서 local-server 기능을 호출한다.

**앱체인 생성 마법사 수정 (`CreateL2Wizard.tsx`):**

```
현재 4단계:
  Step 0: 네트워크 선택 → Step 1: 기본정보 → Step 2: 네트워크설정 → Step 3: 토큰/공개

변경 후 5단계:
  Step 0: 프로그램 선택 (NEW)
    ├── "기본 EVM L2" (오프라인)
    ├── "ZK-DEX" (로컬 프로필)
    └── Platform Store에서 선택 (온라인, Phase 3에서 구현)
  Step 1: 네트워크 선택 (Local / Testnet / Mainnet)
  Step 2: 기본정보
  Step 3: 배포 방식 선택 (NEW)
    ├── "직접 실행" (ethrex l2 --dev, 기존 방식)
    └── "Docker로 실행" (local-server 배포 엔진)
  Step 4: 토큰/공개
```

**배포 시작 흐름:**

```
"Docker로 실행" 선택 시:
  1. Desktop이 local-server API 호출: POST /api/deployments (배포 생성)
  2. Desktop이 local-server API 호출: POST /api/deployments/:id/provision
  3. 브라우저 열기: http://localhost:3002/deployments/:id (상세 관리 UI)
  4. 사용자는 브라우저에서 로그/모니터링/컨트롤

"직접 실행" 선택 시:
  기존 방식 유지 (runner.rs → ethrex l2 --dev)
```

| # | 작업 | 파일 | 설명 |
|---|------|------|------|
| 30 | CreateL2Wizard에 프로그램 선택 단계 추가 | `CreateL2Wizard.tsx` | evm-l2, zk-dex 선택 |
| 31 | CreateL2Wizard에 배포 방식 선택 추가 | `CreateL2Wizard.tsx` | 직접실행 vs Docker |
| 32 | local-server API 호출 유틸 생성 | `src/lib/local-api.ts` | fetch wrapper for localhost:5002 |
| 33 | Docker 배포 시 브라우저 열기 | `commands.rs` | `open_dashboard` 커맨드 호출 |
| 34 | MyL2View에서 Docker 배포 상태 표시 | `MyL2View.tsx` | local-server API로 상태 폴링 |

### 2.2 local-server 클라이언트 커스터마이징

Platform에서 복사한 UI를 Desktop 맥락에 맞게 수정한다.

| # | 작업 | 파일 | 설명 |
|---|------|------|------|
| 35 | auth 의존 제거 | 전체 | Platform OAuth 불필요 (로컬이므로) |
| 36 | 네비게이션 수정 | `layout.tsx` | Desktop 로컬 서버용 심플 네비 |
| 37 | launch 페이지 수정 | `launch/page.tsx` | Step1 프로그램 선택을 로컬 프로필로 변경 |
| 38 | 홈 대시보드 생성 | `page.tsx` | 내 배포 요약, 빠른 시작 버튼 |

### 2.3 테스트

| # | 작업 | 설명 |
|---|------|------|
| 39 | Desktop 앱 시작 → local-server 자동 시작 확인 | |
| 40 | 마법사에서 Docker 배포 선택 → 브라우저에서 진행 확인 | |
| 41 | 로그 뷰어, 모니터링 동작 확인 | |
| 42 | 시작/중지/재시작/파괴 동작 확인 | |
| 43 | SSH 원격 배포 동작 확인 | |
| 44 | 앱 종료 시 local-server 정리 확인 | |

---

## Phase 3: Platform <-> Desktop 연동

### 3.1 Platform Store API 연동

Desktop (또는 local-server)에서 Platform Store를 조회한다.

| # | 작업 | 파일 | 설명 |
|---|------|------|------|
| 45 | Platform Store proxy 추가 | `local-server/routes/store-proxy.js` | Platform API를 프록시 |
| 46 | launch 페이지에 Store 검색 추가 | `local-server/client/app/launch/` | 온라인일 때 Store에서 프로그램 선택 |
| 47 | Desktop 마법사에 Store 프로그램 표시 | `CreateL2Wizard.tsx` | Platform 연결 시 프로그램 목록 |

### 3.2 오픈 앱체인

| # | 작업 | 파일 | 설명 |
|---|------|------|------|
| 48 | 공개 등록 API 추가 | `local-server/routes/deployments.js` | POST /:id/publish → Platform API 호출 |
| 49 | Desktop OpenL2View 실데이터 연동 | `OpenL2View.tsx` | 하드코딩 → Platform API 조회 |
| 50 | 배포 상세에서 "공개하기" 버튼 추가 | `local-server/client` | |

### 3.3 인증 + Deep Link

| # | 작업 | 파일 | 설명 |
|---|------|------|------|
| 51 | Platform OAuth 연동 | `SettingsView.tsx` + keyring | Platform 계정 로그인 |
| 52 | Deep Link 핸들러 | `lib.rs` + tauri plugin | `tokamak://launch?program=zk-dex` |

### 3.4 Platform "L2 만들기 가이드" 페이지

| # | 작업 | 파일 | 설명 |
|---|------|------|------|
| 53 | 가이드 페이지 생성 | `platform/client/app/launch/page.tsx` | Desktop 다운로드 + 설치 안내 |
| 54 | 네비게이션에 "Launch Your L2" 추가 | `platform/client/components/nav.tsx` | |
| 55 | Deep Link 버튼 추가 | 가이드 페이지 | "앱이 설치되어 있다면 바로 열기" |

---

## Phase 4: AI + 고급 기능

| # | 작업 | 파일 | 설명 |
|---|------|------|------|
| 56 | AI Function Calling 구현 | `ai_provider.rs` | Tool 정의 + 실행 |
| 57 | Store 검색 도구 | AI tools | `search_store(query)` |
| 58 | 배포 도구 | AI tools | `deploy_l2(program, config)` |
| 59 | 상태 조회 도구 | AI tools | `get_status(id)` |
| 60 | 크리에이터 도구 | Desktop + Platform | ELF 빌드 → Platform 업로드 |

---

## 추가 고려사항

### 문제 1: 데이터 저장소 이원화

현재 Desktop에는 두 개의 데이터 저장소가 생긴다:

```
appchain_manager.rs → ~/.tokamak-appchain/appchains.json  (Rust, JSON)
local-server DB    → ~/.tokamak-appchain/local.sqlite      (Node.js, SQLite)
```

"직접 실행"한 L2는 JSON에, "Docker 실행"한 L2는 SQLite에 저장된다.
MyL2View에서 두 소스를 합쳐서 보여줘야 한다.

**해결 방안:**

```
방법 A: appchain_manager를 SQLite로 마이그레이션 (권장)
  - Rust에서 rusqlite로 local-server와 같은 SQLite 사용
  - 하나의 deployments 테이블에서 모든 L2 관리
  - deploy_method 컬럼 추가 (direct | docker | remote)
  - 장점: 단일 소스, 일관된 데이터 모델
  - Phase 1에서 함께 진행

방법 B: local-server가 appchain_manager API를 호출
  - local-server → Tauri IPC → appchain_manager
  - 장점: 기존 코드 유지
  - 단점: 복잡한 통신 구조
```

### 문제 2: Platform Store "Use This Program" 버튼 변경

현재 `store/[id]/page.tsx`에서 "Use This Program" 클릭 시:
```javascript
// 현재: Platform에서 직접 deployment 생성
await deploymentsApi.create({ programId: program.id, ... });
router.push("/deployments");
```

분리 후 이 플로우가 깨진다. 변경 필요:

```
변경 후:
  "Use This Program" 클릭 →
    Desktop 앱 설치됨? → Deep Link: tokamak://launch?program={id}
    미설치?           → "Desktop 앱 다운로드" 가이드 페이지로 이동
```

| # | 작업 | 파일 |
|---|------|------|
| B1 | "Use This Program" 버튼을 Deep Link + 다운로드 안내로 변경 | `store/[id]/page.tsx` |
| B2 | 배포 생성 모달 제거 | `store/[id]/page.tsx` |

### 문제 3: 오픈 앱체인 온라인 상태 확인

Desktop에서 L2를 공개하면 Platform 쇼룸에 등록된다.
하지만 그 L2가 **지금 돌고 있는지** 어떻게 확인하나?

```
방법 A: Platform이 RPC URL에 직접 ping (권장)
  - Platform 서버가 주기적으로 등록된 RPC URL에 eth_blockNumber 호출
  - 응답 없으면 offline 표시
  - 장점: Desktop이 꺼져도 독립 동작
  - 단점: 방화벽/NAT 뒤에 있으면 접근 불가

방법 B: Desktop이 heartbeat 전송
  - Desktop이 주기적으로 Platform API에 "나 살아있다" 전송
  - 일정 시간 heartbeat 없으면 offline
  - 장점: NAT 뒤에서도 동작
  - 단점: Desktop이 꺼지면 바로 감지 못함

방법 C: 둘 다 (A + B)
  - 공개 RPC → ping으로 확인
  - 비공개(로컬) → heartbeat로 확인
```

| # | 작업 |
|---|------|
| C1 | Platform에 health check 서비스 추가 (주기적 RPC ping) |
| C2 | Desktop에 heartbeat 전송 모듈 추가 |
| C3 | Platform 오픈 앱체인 목록에 online/offline 배지 |

### 문제 4: 프로그램 사용 통계

프로그램 개발자가 "내 프로그램을 몇 명이 사용하는지" 알 수 있어야 한다.
현재 `program_usage` 테이블이 Platform에 있지만, 배포가 Desktop에서 일어나므로 통계가 끊긴다.

```
Desktop에서 L2 배포 시:
  → Platform API: POST /api/programs/{id}/usage (프로그램 사용 기록)
  → Platform Store에서 use_count 증가
  → 프로그램 개발자가 통계 확인 가능
```

| # | 작업 |
|---|------|
| D1 | Desktop 배포 시 Platform에 사용 기록 전송 |
| D2 | Platform Store에서 실시간 사용 수 표시 |

### 문제 5: 각 타겟 사용자별 누락 기능

#### 프로그램 개발자 (Creator)

| 누락 기능 | 설명 | Phase |
|-----------|------|-------|
| Desktop에서 프로그램 테스트 | ELF 빌드 → 로컬 L2에 적용 → 테스트 | 4 |
| 사용 통계 대시보드 | 내 프로그램 사용 수, 배포 수 | 3 |
| 프로그램 업데이트 알림 | 새 버전 등록 시 운영자에게 알림 | 4 |

#### L2 운영자 (Operator)

| 누락 기능 | 설명 | Phase |
|-----------|------|-------|
| Docker 미설치 감지/안내 | "Docker가 필요합니다" + 설치 가이드 | 2 |
| 다중 L2 포트 관리 | 자동 포트 할당, 충돌 방지 | 2 |
| L2 프로그램 업그레이드 | 실행 중 L2의 Guest Program 버전 업 | 4 |
| 백업/복원 | Docker volume 백업, 상태 스냅샷 | 4 |

#### 일반 사용자 (End User)

| 누락 기능 | 설명 | Phase |
|-----------|------|-------|
| MetaMask 네트워크 추가 가이드 | 오픈 앱체인 RPC로 지갑 연결 방법 | 3 |
| Desktop 지갑 → 앱체인 자동 연결 | 오픈 앱체인 선택 시 지갑에 네트워크 추가 | 3 |
| 앱체인 실시간 상태 | online/offline, 블록 높이, 사용자 수 | 3 |

#### Platform 방문자

| 누락 기능 | 설명 | Phase |
|-----------|------|-------|
| 랜딩 페이지 개선 | "Tokamak이 뭔지" 한눈에 이해 | 3 |
| 데모 영상/스크린샷 | Desktop 앱 사용 과정 시각화 | 3 |

### 문제 6: 오프라인 모드 범위

```
인터넷 없이 가능한 것:
  ✓ 기본 EVM-L2 직접 실행 (ethrex l2 --dev)
  ✓ zk-dex, tokamon 등 로컬 프로필 Docker 실행
    (APP_PROFILES가 local-server에 포함)
  ✓ 로그 뷰어, 모니터링
  ✓ AI 채팅 (Tokamak AI 제외, 로컬 모델 미지원)

인터넷 필요한 것:
  ✗ Platform Store 프로그램 조회
  ✗ 오픈 앱체인 등록/탐색
  ✗ Platform 계정 로그인
  ✗ Docker 이미지 최초 다운로드 (이후 캐시)
  ✗ ELF 파일 다운로드 (Store에서)
```

ELF 파일은 한번 다운로드하면 로컬에 캐시해야 한다:
```
~/.tokamak-appchain/cache/
  ├── elf/
  │   ├── zk-dex-v1.elf
  │   └── tokamon-v1.elf
  └── programs.json  ← 캐시된 프로그램 메타데이터
```

### 문제 7: 보안

```
local-server 보안:
  - localhost만 바인딩 (외부 접근 차단)
  - CORS: localhost:3002만 허용
  - SSH private key가 SQLite에 평문 저장됨 (hosts 테이블)
    → OS Keychain으로 이동하거나 암호화 필요

Platform 토큰 보안:
  - OS Keychain에 저장 (기존 ai_provider.rs 패턴)
  - 토큰 만료 + 리프레시 메커니즘 필요
  - HTTPS 필수 (프로덕션)
```

---

## 인증 설계

### 원칙: Platform이 인증을 소유하고, Desktop이 재사용한다

```
Platform (인증 주체)
  - OAuth 로그인 (Google/Naver/Kakao)
  - 사용자 DB (users 테이블)
  - 세션 관리 (sessions 테이블)
  - API 토큰 발급

Desktop (토큰 재사용)
  - Platform 로그인 페이지를 시스템 브라우저로 열기
  - 콜백으로 토큰 수신 → OS Keychain 저장
  - Platform API 호출 시 토큰 사용
  - local-server는 로컬이므로 인증 불필요
```

### 왜 필요한가?

- 프로그램 등록자가 누구인지 (Platform Store)
- 오픈 앱체인을 올린 사람이 누구인지 (쇼룸)
- 내 앱체인/프로그램을 구별하기 위해
- Platform과 Desktop에서 **같은 사용자**로 식별되어야 함

### 로그인 흐름

```
Desktop "설정" → "Platform 계정 연결"
  1. 시스템 브라우저 열기: https://platform.tokamak.network/auth/desktop
  2. 사용자가 OAuth 로그인 (Google/Naver/Kakao)
  3. Platform이 일회용 코드 생성
  4. 콜백 URL로 Desktop에 코드 전달
     → tokamak://auth?code=xxx (Deep Link)
     또는 localhost:5002/auth/callback?code=xxx (local-server)
  5. Desktop이 코드로 토큰 교환: POST /api/auth/token
  6. 토큰을 OS Keychain에 저장 (기존 ai_provider.rs의 keyring 패턴)
  7. 이후 Platform API 호출 시 Authorization: Bearer {token}
```

### 토큰 사용처

| 동작 | 토큰 필요 | 설명 |
|------|:---------:|------|
| 로컬 L2 배포/관리 | X | local-server는 로컬이므로 |
| Platform Store 조회 | X | 공개 API |
| 오픈 앱체인 등록 (공개) | **O** | 누가 올렸는지 식별 |
| 내 프로그램/배포 목록 | **O** | 사용자별 필터 |
| 프로그램 등록 (개발자) | **O** | 크리에이터 식별 |

### Platform 서버 수정 사항

| # | 작업 | 파일 | 설명 |
|---|------|------|------|
| A1 | Desktop용 토큰 교환 엔드포인트 | `routes/auth.js` | POST /api/auth/token (code → token) |
| A2 | Desktop용 인증 페이지 | `client/app/auth/desktop/` | OAuth 후 Deep Link 콜백 |
| A3 | 토큰 검증 미들웨어 | `middleware/auth.js` | Bearer 토큰 지원 추가 |

---

## 기술 결정 사항

### Node.js 번들링

Desktop 앱 배포 시 local-server의 Node.js 코드를 어떻게 포함할지:

```
방법 A: Node.js 런타임 번들링 (pkg, nexe)
  - local-server를 단일 바이너리로 컴파일
  - 장점: Node.js 설치 불필요
  - 단점: 바이너리 크기 증가 (~50MB)

방법 B: 시스템 Node.js 사용
  - 사용자 PC에 설치된 Node.js 사용
  - 장점: 간단
  - 단점: Node.js 설치 필요 (사용자 부담)

방법 C: Tauri sidecar로 Node.js 포함
  - Tauri의 sidecar 기능으로 node 바이너리 포함
  - 장점: 앱과 함께 배포
  - 단점: 플랫폼별 바이너리 필요

권장: Phase 1에서는 방법 B (시스템 Node.js),
     Phase 2에서 방법 A로 전환 (사용자 편의)
```

### local-server 포트

```
Express API 서버: 5002 (Platform 서비스 5001과 충돌 방지)
Next.js 클라이언트: 3002 (Platform 클라이언트 3000과 충돌 방지)
```

### DB 위치

```
local-server DB: ~/.tokamak-appchain/local.sqlite
  - 기존 Desktop의 appchain_manager가 사용하는 ~/.tokamak-appchain/ 디렉토리에 함께 저장
  - Desktop 앱의 JSON 설정과 local-server의 SQLite가 공존
```

---

## 작업 요약

| Phase | 작업 | 핵심 산출물 |
|-------|------|------------|
| **Phase 1** | #1~#29 + A(데이터 통합) | local-server 프로젝트, Platform 정리, Tauri 스포닝, SQLite 통합 |
| **Phase 2** | #30~#44 + B(Store버튼) | 마법사 확장, Docker 배포 연동, Docker 감지, 테스트 |
| **Phase 3** | #45~#55 + C(상태확인) + D(통계) | Store 연동, 오픈 앱체인, 인증, 가이드, 지갑 연결 |
| **Phase 4** | #56~#60 | AI Function Calling, 크리에이터 도구, 업그레이드, 백업 |

### 결정이 필요한 사항

| 결정 | 선택지 | 권장 |
|------|--------|------|
| 데이터 저장소 | JSON 유지 vs SQLite 통합 | SQLite 통합 |
| 오픈 앱체인 상태 확인 | Platform ping vs Desktop heartbeat | 둘 다 |
| Node.js 번들링 | 시스템 Node vs pkg 번들 vs sidecar | Phase 1: 시스템, Phase 2: pkg |
| SSH key 저장 | SQLite 평문 vs Keychain | Keychain |
