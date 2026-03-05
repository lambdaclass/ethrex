# Development Plan: Tokamak Desktop App (개발계획서)

## Phase 0: 프로젝트 초기 설정

**목표**: Tauri + React 프로젝트 스캐폴딩 및 빌드 파이프라인 구축

### 작업 목록
- [ ] Tauri 2.x 프로젝트 생성 (`crates/desktop-app/`)
- [ ] React + TypeScript + Vite 프론트엔드 설정
- [ ] Tailwind CSS + shadcn/ui 설치
- [ ] ethrex workspace에 크레이트 추가 (Cargo.toml)
- [ ] macOS/Windows/Linux 빌드 테스트
- [ ] CI/CD 파이프라인 (GitHub Actions)

### 산출물
- 빈 Tauri 앱이 빌드·실행되는 상태
- 기본 창(window) 표시

---

## Phase 1: 기본 UI 쉘 + 노드 제어

**목표**: 카카오톡 스타일 레이아웃 + 노드 시작/중지 기능

### 작업 목록

#### Frontend
- [ ] 사이드바 네비게이션 (홈, 채팅, 노드제어, 설정)
- [ ] 노드 제어 화면: 프로세스 목록, 시작/중지 버튼
- [ ] 실시간 로그 뷰어 (터미널 스타일)
- [ ] 설정 화면: 네트워크, 포트, 경로 설정

#### Backend (Rust)
- [ ] `ProcessManager` 구현
  - ethrex 바이너리 실행/중지
  - 프로세스 상태 모니터링
  - stdout/stderr 로그 캡처
- [ ] Tauri IPC 명령어 (`start_node`, `stop_node`, `get_status`, `get_logs`)
- [ ] 설정 파일 관리 (SQLite 또는 TOML)

### 산출물
- 데스크탑 앱에서 ethrex L1/L2 노드를 시작/중지할 수 있는 상태

---

## Phase 2: 대시보드 WebView 통합

**목표**: 각 레이어의 웹 대시보드를 탭으로 표시

### 작업 목록
- [ ] 탭 바 컴포넌트 (동적 탭 추가/제거)
- [ ] WebView/iframe으로 외부 URL 로딩
- [ ] 기본 대시보드 탭 프리셋 (L1, L2, Explorer, Prover, Metrics)
- [ ] 탭 URL 커스터마이징 (사용자가 직접 URL 추가)
- [ ] 대시보드 연결 상태 표시 (연결됨/끊김)

### 산출물
- 앱 내에서 각 레이어의 웹 대시보드를 확인할 수 있는 상태

---

## Phase 3: AI 채팅 인터페이스

**목표**: AI 로그인 + 카카오톡 스타일 채팅 UI

### 작업 목록

#### AI 로그인/인증
- [ ] AI 프로바이더 선택 화면 (Claude, OpenAI, Gemini)
- [ ] API 키 입력 UI
- [ ] OS Keychain에 API 키 저장 (Tauri keychain plugin)
- [ ] 모델 선택 드롭다운

#### 채팅 UI
- [ ] 카카오톡 스타일 메시지 버블 (나/AI)
- [ ] 스트리밍 응답 표시 (타이핑 효과)
- [ ] 코드 블록, 테이블, 링크 렌더링 (Markdown)
- [ ] 채팅 히스토리 저장/로드 (SQLite)
- [ ] 대화방 목록 (여러 대화 관리)

#### AI Service (Backend)
- [ ] `AiProvider` trait 구현
- [ ] `ClaudeProvider` 구현 (Messages API)
- [ ] `OpenAiProvider` 구현 (Chat Completions API)
- [ ] `GeminiProvider` 구현 (Generate Content API)
- [ ] 스트리밍 SSE/WebSocket 지원

### 산출물
- AI와 일반 대화가 가능한 채팅 인터페이스

---

## Phase 4: AI 도구(Tool Use) + 앱 제어

**목표**: AI가 앱의 기능을 실행할 수 있도록 도구 연결

### 작업 목록

#### AI Guide System
- [ ] AI 시스템 프롬프트 작성 (Tokamak L2 전문가 역할)
- [ ] Tool Use 정의:
  - `start_node(name)` - 노드 시작
  - `stop_node(name)` - 노드 중지
  - `get_node_status(name)` - 노드 상태 조회
  - `get_logs(name, lines)` - 로그 조회
  - `open_dashboard(tab)` - 대시보드 탭 열기
  - `get_balance()` - TON 잔액 조회
  - `update_config(key, value)` - 설정 변경
- [ ] Tool 실행 결과를 AI에게 반환하는 파이프라인
- [ ] 민감한 작업(노드 중지, 설정 변경) 전 사용자 확인 다이얼로그

#### AI Guide 문서
- [ ] ethrex 노드 운영 가이드 (AI가 참조할 컨텍스트)
- [ ] 자주 묻는 질문과 해결 방법
- [ ] 에러 코드별 대응 가이드

### 산출물
- "L2 노드 시작해줘" → AI가 실제로 노드를 시작하는 상태

---

## Phase 5: TON 지갑 & 충전

**목표**: TON 잔액 확인, L1↔L2 입출금, AI 지갑

### 작업 목록
- [ ] 지갑 연결 (MetaMask 등 외부 지갑 또는 내장 지갑)
- [ ] TON 잔액 표시 (L1, L2)
- [ ] Deposit (L1 → L2) UI 및 트랜잭션 실행
- [ ] Withdraw (L2 → L1) UI 및 트랜잭션 실행
- [ ] AI 에이전트 지갑 생성
- [ ] AI 지갑에 TON 충전 기능
- [ ] 트랜잭션 히스토리
- [ ] AI Tool 추가: `deposit()`, `withdraw()`, `get_balance()`

### 산출물
- 앱 내에서 TON을 충전하고 AI가 L2에서 트랜잭션을 실행할 수 있는 상태

---

## Phase 6: L2 마켓플레이스

**목표**: 다른 L2 서비스를 탐색하고 연결

### 작업 목록
- [ ] L2 서비스 레지스트리 (중앙 또는 분산)
- [ ] 서비스 등록 UI (내 L2 기능 공개)
- [ ] 서비스 카탈로그 브라우징 UI
- [ ] 서비스별 AI 가이드 문서 포맷 정의
- [ ] AI가 다른 L2 서비스를 탐색하고 사용하는 기능
- [ ] 서비스 연결 (크로스 L2 호출)
- [ ] 평점/리뷰 시스템

### 산출물
- L2 운영자가 서비스를 공개하고, AI가 이를 탐색·사용할 수 있는 상태

---

## 마일스톤 요약

| Phase | 이름 | 핵심 기능 | 의존성 |
|-------|------|----------|--------|
| 0 | 프로젝트 셋업 | Tauri + React 스캐폴딩 | 없음 |
| 1 | 노드 제어 | 프로세스 시작/중지/로그 | Phase 0 |
| 2 | 대시보드 | WebView 탭, URL 임베드 | Phase 0 |
| 3 | AI 채팅 | AI 로그인, 채팅 UI | Phase 0 |
| 4 | AI 도구 연결 | Tool Use, 앱 제어 | Phase 1 + 3 |
| 5 | TON 지갑 | 충전, 입출금, AI 지갑 | Phase 4 |
| 6 | L2 마켓 | 서비스 공개, 탐색 | Phase 4 + 5 |

```
Phase 0 ──→ Phase 1 ──→ Phase 4 ──→ Phase 5 ──→ Phase 6
  │              │           ↑
  ├──→ Phase 2 ──┘           │
  │                          │
  └──→ Phase 3 ──────────────┘
```

Phase 1, 2, 3은 독립적으로 병렬 개발 가능.
Phase 4부터는 Phase 1(노드 제어)과 Phase 3(AI 채팅)이 합쳐진다.
