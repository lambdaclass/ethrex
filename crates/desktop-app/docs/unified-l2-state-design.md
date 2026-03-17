# Unified L2 State Layer 설계 문서

> Telegram Bot, AI Messenger, AppchainManager가 동일한 L2 상태를 실시간으로 공유하기 위한 최적화 설계

## 1. 현재 문제 분석

### 1.1 상태 조회 경로가 분산되어 있음

현재 L2 상태를 조회하는 경로가 **3개로 분리**되어 있어, 동일한 시점에 서로 다른 상태를 볼 수 있다.

```
┌─────────────────────────────────────────────────────────────────┐
│                    현재 아키텍처 (Before)                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  [Telegram Bot]                                                 │
│    ├─ build_appchain_context()  ← AppchainManager 직접 접근     │
│    └─ build_deployment_context_live()  ← HTTP로 local-server   │
│         └─ GET /api/deployments                                │
│         └─ GET /api/deployments/:id/status                     │
│         └─ GET /api/deployments/:id/monitoring                 │
│                                                                 │
│  [AI Messenger (Desktop Chat)]                                  │
│    └─ get_chat_context()  ← AppchainManager만 조회             │
│         └─ build_appchain_context()                             │
│         ❌ Docker 배포 상태 없음                                │
│         ❌ RPC 모니터링 없음                                    │
│         ❌ 컨테이너 상태 없음                                   │
│                                                                 │
│  [Frontend (React)]                                             │
│    ├─ invoke('list_appchains')  ← Tauri Command                │
│    └─ fetch('/api/deployments/:id/status')  ← local-server     │
│         └─ 별도 polling                                        │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 1.2 구체적 문제점

| # | 문제 | 영향 |
|---|------|------|
| 1 | **AI Messenger에 배포 상태 누락** | Desktop AI Chat은 AppchainManager의 정적 정보만 알고, Docker 컨테이너 상태/RPC 헬스/계약 주소를 모름 |
| 2 | **Telegram은 매 메시지마다 HTTP 3회 호출** | `build_deployment_context_live()`가 매번 local-server에 3개 API 호출 (목록 + 상태 + 모니터링) → 지연 + 불필요한 부하 |
| 3 | **상태 스냅샷 시점 불일치** | Telegram과 Messenger가 같은 순간에 다른 상태를 보고할 수 있음 |
| 4 | **context 빌드 로직 중복** | `build_appchain_context()`가 telegram_bot.rs에 정의, commands.rs에서 호출 → 로직 분산 |
| 5 | **이벤트 추적 단절** | Desktop에서 수행한 작업이 Telegram의 PilotMemory에 기록되지 않고, 그 반대도 마찬가지 |
| 6 | **프로세스 상태와 논리 상태 분리** | AppchainManager의 `status: Running`이 실제 프로세스 생존과 무관할 수 있음 |

---

## 2. 목표 아키텍처

### 2.1 Unified State Layer

```
┌─────────────────────────────────────────────────────────────────┐
│                    목표 아키텍처 (After)                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│              ┌──────────────────────────┐                       │
│              │   UnifiedL2State (new)   │                       │
│              │   Arc<Mutex<...>>        │                       │
│              ├──────────────────────────┤                       │
│              │ appchains: Vec<L2Info>   │                       │
│              │ deployments: Vec<L2Info> │                       │
│              │ last_updated: Instant    │                       │
│              │ events: EventBus         │                       │
│              └──────────┬───────────────┘                       │
│                         │                                       │
│           ┌─────────────┼─────────────┐                         │
│           │             │             │                          │
│           ▼             ▼             ▼                          │
│    [Telegram Bot] [AI Messenger] [Frontend]                     │
│     get_l2_state() get_l2_state() get_l2_state()               │
│           │             │             │                          │
│           └─────────────┼─────────────┘                         │
│                         │                                       │
│                    동일한 JSON                                  │
│                    동일한 시점                                  │
│                                                                 │
│  ┌────────────────────────────────────────────┐                 │
│  │ Background Refresh (5초 간격)              │                 │
│  │  ├─ AppchainManager.list_appchains()       │                 │
│  │  ├─ ProcessRunner.check_alive()            │                 │
│  │  ├─ local-server /status + /monitoring     │                 │
│  │  └─ 변경 감지 → EventBus broadcast         │                 │
│  └────────────────────────────────────────────┘                 │
│                                                                 │
│  ┌────────────────────────────────────────────┐                 │
│  │ Unified EventBus                           │                 │
│  │  ├─ 상태 변경 이벤트 발행                    │                 │
│  │  ├─ PilotMemory에 자동 기록                 │                 │
│  │  └─ Telegram / Desktop 모두 수신            │                 │
│  └────────────────────────────────────────────┘                 │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 2.2 핵심 원칙

1. **Single Source of Truth**: 모든 컴포넌트가 `UnifiedL2State`에서 동일한 상태를 읽음
2. **Push over Poll**: 상태 변경 시 이벤트 브로드캐스트, 필요 시에만 polling
3. **Background Refresh**: 주기적 갱신으로 최신 상태 유지 (lazy가 아닌 proactive)
4. **Event Convergence**: Desktop/Telegram 어디서 발생한 이벤트든 하나의 이벤트 버스로 통합

---

## 3. 데이터 모델

### 3.1 L2Info (통합 L2 상태)

```rust
/// AppchainManager의 정적 정보 + local-server의 동적 정보를 통합
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L2Info {
    // === 식별 ===
    pub id: String,
    pub name: String,
    pub source: L2Source,           // Appchain | Deployment

    // === 구성 ===
    pub chain_id: u64,
    pub network_mode: String,       // "local" | "testnet" | "mainnet"
    pub native_token: String,       // "TON"

    // === 실시간 상태 ===
    pub status: L2Status,           // Running | Stopped | SettingUp | Error | Created
    pub health: Option<L2Health>,   // RPC 헬스 체크 결과

    // === 엔드포인트 ===
    pub l1_rpc_url: Option<String>,
    pub l2_rpc_url: Option<String>,

    // === 컨트랙트 ===
    pub contracts: Option<L2Contracts>,

    // === Docker (Deployment만) ===
    pub containers: Option<Vec<ContainerInfo>>,
    pub phase: Option<String>,      // 배포 단계

    // === 메타 ===
    pub is_public: bool,
    pub created_at: String,
    pub updated_at: String,         // 마지막 상태 갱신 시점
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum L2Source {
    Appchain,    // AppchainManager가 관리 (ethrex CLI 직접 실행)
    Deployment,  // local-server가 관리 (Docker Compose)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum L2Status {
    Created,
    SettingUp,
    Running,
    Stopped,
    Error,
    Partial,     // 일부 컨테이너만 실행 중 (Deployment)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L2Health {
    pub l1_healthy: bool,
    pub l2_healthy: bool,
    pub l1_block_number: Option<u64>,
    pub l2_block_number: Option<u64>,
    pub l1_chain_id: Option<u64>,
    pub l2_chain_id: Option<u64>,
    pub checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L2Contracts {
    pub bridge: Option<String>,
    pub proposer: Option<String>,
    pub timelock: Option<String>,
    pub sp1_verifier: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInfo {
    pub service: String,
    pub state: String,      // "running" | "exited" | "restarting"
    pub status: String,     // "Up 5 minutes"
}
```

### 3.2 UnifiedL2State

```rust
pub struct UnifiedL2State {
    /// 모든 L2 인스턴스의 통합 상태
    state: Mutex<Vec<L2Info>>,
    /// 마지막 갱신 시점
    last_refreshed: Mutex<Instant>,
    /// 이벤트 발행 채널
    event_tx: broadcast::Sender<L2Event>,
}

#[derive(Debug, Clone, Serialize)]
pub struct L2Event {
    pub event_type: L2EventType,
    pub l2_id: String,
    pub l2_name: String,
    pub detail: String,
    pub source: String,         // "telegram" | "desktop" | "system"
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize)]
pub enum L2EventType {
    StatusChanged,       // Running → Stopped 등
    HealthChanged,       // healthy → unhealthy
    ContainerChanged,    // 컨테이너 상태 변경
    Created,
    Deleted,
    SetupProgress,
}
```

---

## 4. 컴포넌트 설계

### 4.1 UnifiedL2State 구현

```rust
impl UnifiedL2State {
    /// 캐시된 상태 반환 (즉시, lock만)
    pub fn get_all(&self) -> Vec<L2Info> {
        self.state.lock().unwrap().clone()
    }

    /// 특정 L2 조회
    pub fn get_by_id(&self, id: &str) -> Option<L2Info> {
        self.state.lock().unwrap().iter().find(|l| l.id == id).cloned()
    }

    /// JSON context 생성 (AI 프롬프트용)
    pub fn to_context_json(&self) -> serde_json::Value {
        let items = self.get_all();
        serde_json::json!({
            "l2_instances": items,
            "total_count": items.len(),
            "last_refreshed": format!("{:?}", *self.last_refreshed.lock().unwrap()),
        })
    }

    /// 상태 갱신 (Background Refresh에서 호출)
    pub async fn refresh(
        &self,
        am: &AppchainManager,
        runner: &ProcessRunner,
        local_server_port: u16,
    ) {
        let mut new_state = Vec::new();

        // 1. AppchainManager → L2Info 변환
        for chain in am.list_appchains() {
            let actual_running = runner.is_alive(&chain.id);
            let status = reconcile_status(&chain.status, actual_running);
            new_state.push(L2Info::from_appchain(chain, status));
        }

        // 2. local-server → L2Info 변환
        if let Ok(deployments) = fetch_deployments(local_server_port).await {
            for dep in deployments {
                let status_info = fetch_deployment_status(&dep.id, local_server_port).await;
                let monitoring = fetch_deployment_monitoring(&dep.id, local_server_port).await;
                new_state.push(L2Info::from_deployment(dep, status_info, monitoring));
            }
        }

        // 3. 이전 상태와 비교 → 변경 이벤트 발행
        let old_state = self.state.lock().unwrap().clone();
        let events = diff_states(&old_state, &new_state);
        for event in events {
            let _ = self.event_tx.send(event);
        }

        // 4. 상태 교체
        *self.state.lock().unwrap() = new_state;
        *self.last_refreshed.lock().unwrap() = Instant::now();
    }
}
```

### 4.2 Background Refresh Task

```rust
/// lib.rs에서 앱 시작 시 spawn
pub fn spawn_state_refresh(
    state: Arc<UnifiedL2State>,
    am: Arc<AppchainManager>,
    runner: Arc<ProcessRunner>,
    memory: Arc<PilotMemory>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut event_rx = state.subscribe_events();

        // 이벤트 → PilotMemory 기록 태스크
        let memory_clone = memory.clone();
        tokio::spawn(async move {
            while let Ok(event) = event_rx.recv().await {
                memory_clone.append_event(
                    &event.event_type.to_string(),
                    &event.l2_name,
                    &event.l2_id,
                    &event.detail,
                    &event.source,
                );
            }
        });

        // 주기적 갱신 루프
        loop {
            state.refresh(&am, &runner, 5002).await;
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
}
```

### 4.3 각 컴포넌트 변경 사항

#### Telegram Bot (telegram_bot.rs)

```rust
// Before: 매 메시지마다 HTTP 3회 호출
async fn handle_ai_message(&self, chat_id: i64, text: &str) {
    let deployment_context = build_deployment_context_live().await;   // HTTP x3
    let appchain_context = build_appchain_context(&self.appchain_manager);
    // ...
}

// After: 캐시된 상태 즉시 조회
async fn handle_ai_message(&self, chat_id: i64, text: &str) {
    let context = self.unified_state.to_context_json();  // Mutex lock만, HTTP 없음
    let system_prompt = AiProvider::build_telegram_prompt_v2(&context, &pilot_context);
    // ...
}
```

#### AI Messenger (commands.rs + ChatView.tsx)

```rust
// Before: AppchainManager만 조회
#[tauri::command]
pub fn get_chat_context(am: State<Arc<AppchainManager>>) -> serde_json::Value {
    crate::telegram_bot::build_appchain_context(&am)
}

// After: 통합 상태 조회 (Docker 배포 포함)
#[tauri::command]
pub fn get_chat_context(state: State<Arc<UnifiedL2State>>) -> serde_json::Value {
    state.to_context_json()
}
```

#### Frontend (React)

```typescript
// Before: 앱체인과 배포를 별도 API로 조회
const appchains = await invoke('list_appchains');
const deploymentStatus = await fetch(`/api/deployments/${id}/status`);

// After: 통합 상태 조회 + 이벤트 구독
const allL2 = await invoke('get_all_l2');  // UnifiedL2State.get_all()

// Tauri 이벤트로 실시간 업데이트 수신
listen('l2-state-changed', (event) => {
    updateL2List(event.payload);
});
```

---

## 5. 상태 동기화 흐름

### 5.1 상태 변경 시나리오

```
시나리오: 사용자가 Telegram에서 "앱체인 중지" 요청

1. Telegram Bot → execute_action("stop_appchain", id)
2. ProcessRunner.stop(id)
3. AppchainManager.update_status(id, Stopped)
4. ─── 5초 이내 Background Refresh 실행 ───
5. UnifiedL2State.refresh()
   ├─ AppchainManager: status = Stopped 확인
   ├─ ProcessRunner: is_alive = false 확인
   └─ diff_states() → StatusChanged 이벤트 발행
6. L2Event { StatusChanged, "my-chain", "Running → Stopped", "telegram" }
   ├─→ PilotMemory에 기록
   ├─→ Telegram Bot: 다음 메시지에서 최신 상태 반영
   └─→ Desktop Frontend: Tauri 이벤트로 즉시 UI 갱신
```

### 5.2 즉시 갱신이 필요한 경우

```rust
impl UnifiedL2State {
    /// 액션 직후 즉시 갱신 (5초 대기 없이)
    pub async fn refresh_now(&self, am: &AppchainManager, runner: &ProcessRunner) {
        self.refresh(am, runner, 5002).await;
    }
}

// Telegram Bot에서 액션 실행 후:
async fn execute_action(&self, action: &str, params: &HashMap<String, String>) {
    match action {
        "stop_appchain" => {
            self.runner.stop(&id).await?;
            self.appchain_manager.update_status(&id, AppchainStatus::Stopped);
            self.unified_state.refresh_now(&self.appchain_manager, &self.runner).await;
        }
        // ...
    }
}
```

---

## 6. 이벤트 통합

### 6.1 현재: 이벤트가 분리됨

```
Desktop 작업 → Tauri 이벤트 (Frontend만)
Telegram 작업 → PilotMemory (Telegram만)
Docker 상태 → local-server SSE (Frontend만)
```

### 6.2 목표: 통합 이벤트 버스

```
모든 작업 → L2Event → broadcast::channel
                        ├─→ PilotMemory (영구 기록)
                        ├─→ Telegram (알림 판단)
                        ├─→ Tauri Event (Frontend UI)
                        └─→ 로그 (디버깅)
```

### 6.3 이벤트 구독 구조

```rust
impl UnifiedL2State {
    pub fn subscribe_events(&self) -> broadcast::Receiver<L2Event> {
        self.event_tx.subscribe()
    }
}

// lib.rs - 앱 초기화 시
let unified_state = Arc::new(UnifiedL2State::new());

// Telegram Bot에 이벤트 수신기 연결
let mut telegram_rx = unified_state.subscribe_events();
tokio::spawn(async move {
    while let Ok(event) = telegram_rx.recv().await {
        // 중요한 이벤트만 사용자에게 알림
        if event.source != "telegram" && is_important(&event) {
            telegram_bot.notify_event(&event).await;
        }
    }
});

// Frontend에 Tauri 이벤트 전달
let mut frontend_rx = unified_state.subscribe_events();
let app_handle = app.handle().clone();
tokio::spawn(async move {
    while let Ok(event) = frontend_rx.recv().await {
        app_handle.emit("l2-state-changed", &event).ok();
    }
});
```

---

## 7. 구현 계획

### Phase 1: UnifiedL2State 코어 (1단계)

| 파일 | 작업 | 설명 |
|------|------|------|
| `src-tauri/src/unified_state.rs` | **신규** | UnifiedL2State, L2Info, L2Event 정의 + refresh 로직 |
| `src-tauri/src/lib.rs` | 수정 | UnifiedL2State 초기화 + Background Refresh spawn |
| `src-tauri/src/commands.rs` | 수정 | `get_chat_context` → UnifiedL2State 사용 |

### Phase 2: Telegram Bot 통합 (2단계)

| 파일 | 작업 | 설명 |
|------|------|------|
| `src-tauri/src/telegram_bot.rs` | 수정 | `build_deployment_context_live()` 제거, UnifiedL2State 사용 |
| `src-tauri/src/telegram_bot.rs` | 수정 | 이벤트 구독 → Desktop 변경 알림 수신 |
| `src-tauri/src/ai_provider.rs` | 수정 | `build_telegram_prompt_v2()` 통합 context 사용 |

### Phase 3: Frontend 통합 (3단계)

| 파일 | 작업 | 설명 |
|------|------|------|
| `src-tauri/src/commands.rs` | 수정 | `get_all_l2` 명령어 추가 |
| `ui/src/App.tsx` | 수정 | `l2-state-changed` 이벤트 리스너 |
| `ui/src/components/MyL2View.tsx` | 수정 | 통합 상태로 목록 렌더링 |
| `ui/src/components/ChatView.tsx` | 수정 | context 전달 방식 변경 |

### Phase 4: 이벤트 통합 (4단계)

| 파일 | 작업 | 설명 |
|------|------|------|
| `src-tauri/src/unified_state.rs` | 수정 | diff_states + EventBus 구현 |
| `src-tauri/src/pilot_memory.rs` | 수정 | EventBus 자동 기록 연결 |
| `src-tauri/src/telegram_bot.rs` | 수정 | health_monitor를 UnifiedL2State 기반으로 전환 |

---

## 8. 삭제 대상 코드

통합 후 제거할 수 있는 중복 코드:

| 위치 | 함수/코드 | 사유 |
|------|-----------|------|
| `telegram_bot.rs` | `build_appchain_context()` | → `UnifiedL2State::to_context_json()` |
| `telegram_bot.rs` | `build_deployment_context_live()` | → Background Refresh가 대체 |
| `telegram_bot.rs` | `health_monitor()` 내 HTTP 호출 | → 이벤트 구독으로 대체 |
| `commands.rs` | 기존 `get_chat_context()` 구현 | → UnifiedL2State 위임 |

---

## 9. 기대 효과

| 항목 | Before | After |
|------|--------|-------|
| **Telegram 메시지 응답 시간** | ~500ms (HTTP 3회) | ~1ms (Mutex lock) |
| **AI Messenger 상태 정보** | AppchainManager만 | AppChain + Docker + RPC 모두 |
| **상태 일관성** | 조회 시점마다 다름 | 5초 이내 동기화 보장 |
| **이벤트 기록** | Telegram/Desktop 분리 | 통합 기록 |
| **코드 중복** | context 빌드 2곳 | 1곳 (UnifiedL2State) |
| **Desktop ↔ Telegram 인지** | 서로 모름 | 양방향 이벤트 알림 |

---

## 10. 리스크 및 고려사항

### 10.1 Mutex 경합
- Background Refresh가 5초마다 `state` lock 획득
- 읽기가 빈번하므로 `RwLock`을 사용하는 것이 더 적합할 수 있음
- 결정: `RwLock<Vec<L2Info>>` 사용 (read 다수, write 5초 1회)

### 10.2 local-server 미실행 시
- Background Refresh에서 HTTP 실패 → Deployment 정보 없이 Appchain만 반환
- 이전 상태 캐시 유지 + `stale: true` 플래그 추가

### 10.3 하위 호환성
- `list_appchains`, `get_appchain` 등 기존 Tauri 명령어는 유지
- `get_chat_context`의 반환 형식이 변경되므로 Frontend ChatView 동시 수정 필요
- `get_all_l2`는 신규 명령어로 추가 (기존 코드 영향 없음)

### 10.4 테스트 전략
- `UnifiedL2State` 단위 테스트: mock AppchainManager + mock HTTP
- `diff_states` 테스트: 이전/이후 상태 비교 → 이벤트 생성 검증
- 통합 테스트: Telegram 메시지 → 상태 조회 → context JSON 검증
