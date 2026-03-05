# Development Plan: Tokamak Desktop App (개발계획서) v2

## 현재 상태 (완료)

- [x] Phase 0: Tauri 2.x + React + TypeScript + Vite 스캐폴딩
- [x] Phase 0: 사이드바 네비게이션, 기본 뷰 5개 생성
- [x] Phase 0: i18n 시스템 (ko/en/ja)
- [x] Phase 0: 오픈 L2 뷰 (샘플 데이터, 해시태그 필터링)
- [x] Phase 0: Rust 백엔드 (ProcessManager, Tauri IPC 명령어)

---

## Phase 1: 내 L2 관리 + L2 생성

**목표**: 여러 개의 L2를 생성하고 관리하는 핵심 기능

### 1-A. 내 L2 목록 화면

- [ ] `MyL2View` 컴포넌트: 내 L2 리스트
  - L2 이름, Chain ID, 상태(Running/Stopped), 아이콘
  - 선택하면 L2 상세 관리 화면으로 전환
- [ ] 사이드바에 📦 "내 L2" 메뉴 추가
- [ ] Rust: `L2Registry` - 여러 L2 설정을 SQLite에 저장/로드
- [ ] i18n: 내 L2 관련 번역 추가

### 1-B. L2 생성 위자드

- [ ] `CreateL2Wizard` 컴포넌트: 스텝 바이 스텝 생성 UI
  - Step 1: 기본 정보 (이름, Chain ID, 설명)
  - Step 2: 네트워크 설정 (L1 RPC, 포트, 시퀀서 모드)
  - Step 3: 토큰/프로버 설정 (네이티브 토큰, Prover 방식)
  - Step 4: 공개 설정 (오픈 L2 여부, 해시태그)
  - Step 5: 확인 및 생성
- [ ] Rust: `L2Creator` - ethrex L2 설정 파일 생성, 초기화 명령 실행
- [ ] 생성 후 자동으로 내 L2 목록에 추가

### 1-C. L2 상세 관리 화면

- [ ] `L2DetailView` 컴포넌트: 선택된 L2의 노드 제어
  - 시퀀서/프로버/클라이언트 시작·중지·재시작 버튼
  - 실시간 로그 뷰어 (터미널 스타일, ANSI 컬러 지원)
  - 설정 편집 (가스비, 배치 간격, 포트 등)
  - 해당 L2의 대시보드 바로가기
- [ ] Rust: `ProcessManager` 확장 - L2별로 여러 프로세스 관리
  - ethrex 바이너리 실제 실행/중지 구현
  - stdout/stderr 로그 캡처 및 스트리밍
  - 프로세스 상태 주기적 모니터링

### 산출물
- 앱에서 L2를 생성하고, 여러 L2의 노드를 개별적으로 관리할 수 있는 상태

---

## Phase 2: 대시보드 WebView

**목표**: 각 L2의 웹 대시보드를 탭으로 표시

### 작업 목록
- [ ] 탭 바: 동적 탭 추가/제거
- [ ] Tauri WebView 또는 iframe으로 URL 로딩
- [ ] L2별 대시보드 프리셋 (L1, L2, Explorer, Prover, Metrics)
- [ ] 오픈 L2의 대시보드도 탭으로 추가 가능
- [ ] 탭 URL 커스터마이징
- [ ] 연결 상태 표시 (연결됨/끊김/로딩중)

### 산출물
- 선택된 L2의 대시보드를 앱 내에서 확인할 수 있는 상태

---

## Phase 3: AI 채팅 (L2 Pilot)

**목표**: AI 프로바이더 로그인 + 카카오톡 스타일 채팅

### 3-A. AI 로그인/인증

- [ ] 설정 화면에서 AI 프로바이더 선택 (Claude, OpenAI, Gemini)
- [ ] API 키 입력 및 OS Keychain 저장 (tauri-plugin-keychain)
- [ ] 모델 선택 드롭다운 (프로바이더별 모델 목록)
- [ ] 연결 테스트 ("API 키 확인" 버튼)
- [ ] 여러 프로바이더 등록, 채팅 화면에서 전환

### 3-B. 채팅 UI 고도화

- [ ] 스트리밍 응답 표시 (글자가 타이핑되는 효과)
- [ ] Markdown 렌더링 (코드 블록, 테이블, 링크, 볼드 등)
- [ ] 채팅 히스토리 저장/로드 (SQLite)
- [ ] 대화방 목록: 여러 대화 관리 (L2별 대화방도 가능)
- [ ] 메시지 검색

### 3-C. AI Service 백엔드

- [ ] Rust: `AiProvider` trait
- [ ] `ClaudeProvider` - Anthropic Messages API, 스트리밍
- [ ] `OpenAiProvider` - Chat Completions API, 스트리밍
- [ ] `GeminiProvider` - Generate Content API, 스트리밍
- [ ] 에러 처리 (키 만료, 레이트 리밋, 네트워크 오류)

### 산출물
- 실제 AI와 대화가 가능한 채팅 인터페이스

---

## Phase 4: AI Tool Use (앱 제어 연결)

**목표**: AI가 앱의 기능을 실행할 수 있도록 도구 연결

### 4-A. Tool 정의 및 실행 파이프라인

- [ ] AI 시스템 프롬프트 (L2 Pilot 역할, 도메인 지식)
- [ ] Tool Use 정의:

| Tool | 설명 | 확인 필요 |
|------|------|----------|
| `list_my_l2s()` | 내 L2 목록 조회 | ❌ |
| `create_l2(config)` | 새 L2 생성 | ✅ |
| `start_node(l2_id, name)` | 노드 시작 | ❌ |
| `stop_node(l2_id, name)` | 노드 중지 | ✅ |
| `get_node_status(l2_id)` | 노드 상태 | ❌ |
| `get_logs(l2_id, name, lines)` | 로그 조회 | ❌ |
| `open_dashboard(l2_id, tab)` | 대시보드 열기 | ❌ |
| `get_balance(wallet, l2_id)` | 잔액 조회 | ❌ |
| `deposit(l2_id, amount)` | L2에 입금 | ✅ |
| `withdraw(l2_id, amount)` | L1으로 출금 | ✅ |
| `search_open_l2(query, tag)` | 오픈 L2 검색 | ❌ |
| `get_l2_guide(l2_id)` | L2 AI 가이드 로드 | ❌ |
| `update_config(l2_id, key, value)` | 설정 변경 | ✅ |

- [ ] Tool 실행 결과 → AI에게 반환하는 파이프라인
- [ ] 확인 필요(✅) 작업은 사용자 확인 다이얼로그 표시

### 4-B. AI 가이드 문서

- [ ] ethrex L2 운영 가이드 (AI 컨텍스트용)
- [ ] FAQ 및 에러 코드 대응 가이드
- [ ] 오픈 L2 가이드 표준 포맷 (YAML)

### 산출물
- "DEX Chain 노드 시작해줘" → AI가 실제로 노드를 시작하는 상태

---

## Phase 5: TON 지갑 & L2별 잔액 관리

**목표**: 지갑 등록, L2별 TON 잔액, 입출금, AI 지갑

### 5-A. 지갑 등록 및 관리

- [ ] 지갑 주소 직접 입력 UI
- [ ] MetaMask 등 외부 지갑 연동 (WalletConnect 또는 브라우저 익스텐션)
- [ ] 여러 지갑 등록, 기본 지갑 선택
- [ ] 지갑 주소 OS Keychain 저장

### 5-B. L2별 잔액 표시

- [ ] L1 + 각 L2별 TON 잔액을 트리 구조로 표시
- [ ] 각 L2의 RPC를 통해 잔액 실시간 조회
- [ ] AI 지갑도 L2별 잔액 표시

### 5-C. 입출금

- [ ] Deposit: L1 → 특정 L2 선택 → 금액 입력 → 트랜잭션 실행
- [ ] Withdraw: 특정 L2 → L1 → 금액 입력 → 트랜잭션 실행
- [ ] AI 지갑 충전: 내 지갑 → AI 지갑 (특정 L2)
- [ ] 트랜잭션 히스토리 (L2별 필터링)

### 산출물
- 지갑을 등록하고 여러 L2의 TON 잔액을 관리할 수 있는 상태

---

## Phase 6: 오픈 L2 생태계

**목표**: 오픈 L2 레지스트리, AI 가이드 연동, AI간 상호작용

### 6-A. 오픈 L2 레지스트리

- [ ] 레지스트리 백엔드 (중앙 서버 또는 온체인 레지스트리)
- [ ] 내 L2를 오픈 L2로 등록하는 API
- [ ] 오픈 L2 목록 실시간 조회 (현재 샘플 데이터 → 실제 데이터)
- [ ] 해시태그 인덱싱 및 검색

### 6-B. AI 가이드 연동

- [ ] 각 오픈 L2가 AI 가이드 YAML을 제공
- [ ] L2 Pilot이 가이드를 로드하여 해당 L2 기능 수행
- [ ] "이 L2에서 스왑하는 법" → AI가 가이드 읽고 실행

### 6-C. AI간 상호작용

- [ ] AI 에이전트가 오픈 L2를 탐색하고 연결
- [ ] AI가 다른 L2의 API를 가이드에 따라 호출
- [ ] 크로스 L2 작업 (Bridge 등)

### 6-D. 커뮤니티 기능

- [ ] 오픈 L2 평점/리뷰
- [ ] 인기순/최신순/카테고리별 정렬
- [ ] L2 운영자 프로필

### 산출물
- 실제 오픈 L2 생태계가 작동하고, AI가 서로의 L2를 활용하는 상태

---

## 마일스톤 요약

| Phase | 이름 | 핵심 | 의존성 | 상태 |
|-------|------|------|--------|------|
| 0 | 프로젝트 셋업 | Tauri + React 스캐폴딩, i18n, 기본 UI | 없음 | ✅ 완료 |
| 1 | 내 L2 관리 | L2 생성, 멀티 L2, 노드 제어 | Phase 0 | 🔜 다음 |
| 2 | 대시보드 | WebView 탭, L2별 대시보드 | Phase 1 | |
| 3 | AI 채팅 | AI 로그인, 스트리밍, 채팅 고도화 | Phase 0 | 병렬 가능 |
| 4 | AI Tool Use | AI로 앱 제어, 도구 정의 | Phase 1 + 3 | |
| 5 | TON 지갑 | 지갑 등록, L2별 잔액, 입출금 | Phase 1 | 병렬 가능 |
| 6 | 오픈 L2 생태계 | 레지스트리, AI 가이드, AI 상호작용 | Phase 4 + 5 | |

### 의존성 그래프

```
Phase 0 (완료)
    │
    ├──→ Phase 1 (내 L2) ──→ Phase 2 (대시보드)
    │         │                     │
    │         ├─────────────────────┼──→ Phase 4 (AI Tool Use) ──→ Phase 6 (오픈 L2 생태계)
    │         │                     │          ↑                         ↑
    │         └──→ Phase 5 (지갑) ──┘          │                         │
    │                                          │                         │
    └──→ Phase 3 (AI 채팅) ────────────────────┘                         │
                                                                         │
                                               Phase 5 ──────────────────┘
```

### 병렬 개발 가능 조합
- Phase 1 (내 L2) + Phase 3 (AI 채팅): 독립적으로 동시 개발
- Phase 2 (대시보드) + Phase 5 (지갑): Phase 1 완료 후 동시 개발
- Phase 4 (AI Tool Use): Phase 1 + 3 완료 필요 (합류 지점)
