# Platform & Desktop App - 제품 전략 및 코드 분리 설계

---

## 1. 역할 정의

### Platform = 쇼룸 (웹 서비스)

프로그램을 **등록하고, 검증하고, 보여주는** 곳.

- Guest Program Store (마켓플레이스)
- 프로그램 등록/검증/승인
- 오픈 앱체인 쇼룸 (공개된 L2 목록)
- 사용자 인증 (OAuth) / 프로필 / 커뮤니티

### Desktop = 공장 (네이티브 앱)

L2를 **배포하고, 관리하고, 운영하는** 곳.

- L2 배포/실행 (로컬 Docker + 원격 SSH)
- 로그 뷰어, 모니터링, 컨트롤 (시작/중지/재시작)
- 배포 시 브라우저에서 상세 관리 UI 제공 (로컬 웹 서버)
- AI Pilot, 지갑, 시스템 트레이
- 오프라인에서도 기본 EVM-L2 독립 실행 가능

### 왜 이렇게 나누는가?

> Platform은 웹 서비스이므로 사용자의 **로컬 PC에서 프로세스를 띄울 수 없다.**
> 로컬에서 L2를 실행하려면 반드시 로컬에 설치된 에이전트가 필요하고,
> 그 에이전트가 바로 **Desktop App**이다.

---

## 2. 핵심 흐름

```
[배포] Desktop이 모든 배포를 담당
  Desktop --> 로컬 PC (Docker Compose / 직접 프로세스)
  Desktop --> 원격 서버 (SSH)

[관리] Desktop이 로컬 웹 서버를 띄워서 브라우저에서 상세 관리
  Desktop: "빌드" 클릭 --> 브라우저 열림 --> 로그, 모니터링, 컨트롤

[공개] 사용자가 Desktop에서 결정
  Desktop --> "이 L2를 공개할래?" --> Yes --> Platform API로 등록

[발견] 다른 사용자가 Platform에서 탐색
  Platform 쇼룸 --> 오픈 앱체인 발견 --> 자기 Desktop에서 접속
```

### Platform <-> Desktop 연결 포인트 (4개)

```
1. 프로그램 조회:    Desktop --> Platform Store API
2. 오픈 앱체인 등록: Desktop --> Platform에 공개 L2 등록
3. 오픈 앱체인 탐색: Desktop <-- Platform 공개 L2 목록 조회
4. 인증:            Desktop <-- Platform OAuth 계정 재사용
```

### Platform에 추가할 페이지: "L2 만들기 가이드"

Platform은 쇼룸이므로 직접 L2를 배포하지 않는다. 대신 **어떻게 L2를 만드는지 안내**하고, Desktop 앱 설치를 유도한다.

```
Platform 네비게이션에 "Launch Your L2" 메뉴 추가

내용:
  1. L2란 무엇인가 (간단한 소개)
  2. Tokamak Appchain Desktop 앱 다운로드
     - macOS / Windows / Linux 다운로드 링크
     - 설치 가이드 (스크린샷 포함)
  3. 앱에서 L2 만드는 과정 (스크린샷 포함)
     - Store에서 프로그램 선택
     - 배포 설정 (로컬/원격)
     - 빌드 & 실행
  4. 이미 앱이 설치되어 있다면 → Deep Link로 바로 열기
     tokamak://launch
```

---

## 3. 코드 분류

> **"이 코드는 누구를 위한 것인가?"**
> - **Platform**: 프로그램 개발자, 스토어 방문자, 관리자
> - **Desktop**: L2 운영자가 로컬/원격에서 L2를 배포하고 관리

### 서버

| 파일 | 분류 | 설명 |
|------|------|------|
| `routes/auth.js` | Platform | OAuth 로그인, 회원가입 |
| `routes/store.js` | Platform | 프로그램 스토어 조회 |
| `routes/programs.js` | Platform | 프로그램 등록/관리 |
| `routes/admin.js` | Platform | 관리자 기능 |
| `routes/deployments.js` | **혼합** | CRUD=Platform, 실행/로그/모니터링=Desktop |
| `routes/hosts.js` | Desktop | SSH 서버 관리 |
| `routes/fs.js` | Desktop | 로컬 디렉토리 선택 |
| `lib/deployment-engine.js` | Desktop | L2 배포 상태 머신 |
| `lib/compose-generator.js` | Desktop | docker-compose.yaml 생성 |
| `lib/docker-local.js` | Desktop | 로컬 Docker CLI 래퍼 |
| `lib/docker-remote.js` | Desktop | SSH 원격 Docker 배포 |
| `lib/rpc-client.js` | Desktop | L1/L2 RPC 모니터링 |
| `lib/google-auth.js` | Platform | Google OAuth |
| `lib/naver-auth.js` | Platform | Naver OAuth |
| `lib/kakao-auth.js` | Platform | Kakao OAuth |
| `db/users.js`, `sessions.js`, `programs.js` | Platform | 사용자/세션/프로그램 |
| `db/deployments.js`, `hosts.js` | Desktop | 배포/호스트 레코드 |

### 클라이언트

| 페이지/컴포넌트 | 분류 | 설명 |
|----------------|------|------|
| `page.tsx` (홈), `store/`, `admin/` | Platform | 스토어, 관리자 |
| `auth/`, `login/`, `signup/`, `creator/`, `guide/` | Platform | 인증, 개발자 |
| `auth-provider.tsx`, `social-login-buttons.tsx` | Platform | 로그인 UI |
| `deployments/`, `settings/` | **Desktop** | 배포 관리, SSH 설정 |
| `deployment-progress.tsx`, `log-viewer.tsx` | **Desktop** | 진행 UI, 로그 |
| `deployment-status.tsx`, `directory-picker.tsx` | **Desktop** | 상태, 디렉토리 |
| `launch/page.tsx` | **혼합** | Step1(프로그램선택)=Platform, Step2~3(배포)=Desktop |
| `profile/page.tsx` | **혼합** | 프로필=Platform, 내 배포=Desktop |
| `lib/api.ts` | **혼합** | auth/store/admin=Platform, deployments/hosts=Desktop |

### `routes/deployments.js` 분리

```
Platform에 남김:
  POST /              배포 레코드 생성
  GET  /              내 배포 목록
  GET  /:id           배포 상세
  PUT  /:id           배포 수정
  DELETE /:id         배포 삭제

Desktop으로 이동:
  POST /:id/provision 배포 실행 (Docker)
  POST /:id/start     재시작
  POST /:id/stop      중지
  POST /:id/destroy   파괴
  GET  /:id/status    컨테이너 상태
  GET  /:id/events    SSE 진행 스트림
  GET  /:id/logs      서비스 로그
  GET  /:id/monitoring RPC 폴링
  GET  /docker/status Docker 상태
  POST /:id/*-tools   도구 관리
```

---

## 4. 분리 후 디렉토리 구조

```
platform/                              <-- 클라우드 서비스 (쇼룸)
  ├── server/
  │   ├── routes/  auth, store, programs, admin, deployments(CRUD만)
  │   ├── lib/     google-auth, naver-auth, kakao-auth, validate
  │   └── db/      users, sessions, programs, deployments(공개 레코드)
  └── client/
      └── app/     홈, store, admin, creator, auth, login, guide, profile

crates/desktop-app/
  ├── local-server/                    <-- Desktop이 로컬에서 띄우는 웹 서버
  │   ├── server/
  │   │   ├── routes/  deployments(실행/로그/모니터링), hosts, fs
  │   │   ├── lib/     deployment-engine, compose-generator,
  │   │   │            docker-local, docker-remote, rpc-client
  │   │   └── db/      deployments, hosts
  │   └── client/
  │       └── app/     deployments, launch(배포 설정), settings
  │       └── components/  deployment-progress, log-viewer,
  │                        deployment-status, directory-picker
  └── ui/                              <-- 기존 Tauri 데스크탑 앱
      ├── src-tauri/                   Rust 백엔드
      └── src/                         React 프론트엔드
```

---

## 5. 구현 우선순위

### Phase 1: 코드 분리
- Platform에서 배포 실행 코드 분리 → `crates/desktop-app/local-server/`로 이동
- 혼합 파일 분리 (`deployments.js`, `launch/page.tsx`, `api.ts`)
- Desktop에서 local-server를 자식 프로세스로 스포닝

### Phase 2: Desktop 배포 기능 통합
- Desktop "빌드" 클릭 → 브라우저에서 local-server UI 열기
- 배포 마법사, 로그 뷰어, 모니터링 동작 확인
- SSH 원격 배포 동작 확인

### Phase 3: Platform <-> Desktop 연동
- Desktop → Platform Store API 조회 (프로그램 선택)
- 오픈 앱체인 등록 (Desktop → Platform)
- Platform 계정 연동, Deep Link

### Phase 4: AI + 고급 기능
- AI Function Calling (Store 검색 + 배포 자동화)
- 크리에이터 도구 (ELF 빌드 → Platform 업로드)

---

## 6. 요약

| | Platform (쇼룸) | Desktop (공장) |
|-|:---:|:---:|
| **역할** | 프로그램을 보여주는 곳 | L2를 만들고 운영하는 곳 |
| **배포** | - | 로컬 + 원격 모두 |
| **관리** | - | 로그, 모니터링, 컨트롤 |
| **프로그램** | 등록/검증/전시 | 선택/다운로드/실행 |
| **공개** | 쇼룸에 게시 | 공개 여부 결정 |

> **Platform은 갤러리이고, Desktop은 작업실이다.**
> 갤러리에서 마음에 드는 작품(프로그램)을 고르고,
> 작업실(Desktop)에서 직접 만들고 운영한다.
> 잘 만든 작품은 다시 갤러리에 전시(공개)할 수 있다.
> 작업실에는 AI 조수가 있어서 "DEX 만들어줘"하면 알아서 한다.
