# Architecture: Tokamak Desktop App (설계서)

## 1. 기술 스택

| 레이어 | 기술 | 선택 이유 |
|--------|------|----------|
| Desktop Framework | **Tauri 2.x** | Rust 기반, 경량, ethrex와 동일 언어 |
| Frontend | **React + TypeScript** | 풍부한 UI 생태계, WebView 호환 |
| UI Library | **Tailwind CSS + shadcn/ui** | 빠른 개발, 카카오톡 스타일 커스터마이징 |
| Backend (Tauri) | **Rust** | ethrex 크레이트 직접 호출 가능 |
| AI Integration | **REST API** | Claude/OpenAI/Gemini 통합 |
| 상태 관리 | **Zustand** | 경량, React와 자연스러운 통합 |
| 로컬 저장소 | **SQLite (rusqlite)** | 설정, 채팅 히스토리, AI 가이드 캐시 |
| 키 관리 | **OS Keychain** (Tauri plugin) | API 키 안전 저장 |

## 2. 시스템 아키텍처

```
┌─────────────────────────────────────────────────────┐
│                   Tauri Desktop App                  │
│                                                      │
│  ┌──────────────────────────────────────────────┐   │
│  │              Frontend (WebView)               │   │
│  │                                               │   │
│  │  ┌─────────┐ ┌─────────┐ ┌───────────────┐  │   │
│  │  │ AI Chat │ │Dashboard│ │  Node Control  │  │   │
│  │  │  (React)│ │ (WebView│ │   (React)      │  │   │
│  │  │         │ │  iframe)│ │                │  │   │
│  │  └────┬────┘ └────┬────┘ └───────┬────────┘  │   │
│  │       │           │              │            │   │
│  │  ┌────┴───────────┴──────────────┴────────┐  │   │
│  │  │         Tauri IPC Bridge               │  │   │
│  │  └────────────────┬───────────────────────┘  │   │
│  └───────────────────┼──────────────────────────┘   │
│                      │                               │
│  ┌───────────────────┼──────────────────────────┐   │
│  │           Rust Backend (Tauri Core)           │   │
│  │                                               │   │
│  │  ┌────────────┐ ┌────────────┐ ┌──────────┐ │   │
│  │  │ AI Service │ │Process Mgr │ │ Config   │ │   │
│  │  │            │ │            │ │ Manager  │ │   │
│  │  │ - Claude   │ │ - ethrex   │ │          │ │   │
│  │  │ - OpenAI   │ │ - prover   │ │ - SQLite │ │   │
│  │  │ - Gemini   │ │ - sequencer│ │ - Keychain│ │   │
│  │  └─────┬──────┘ └─────┬──────┘ └──────────┘ │   │
│  │        │               │                      │   │
│  │  ┌─────┴──────┐ ┌─────┴──────────────────┐  │   │
│  │  │ AI Guide   │ │  ethrex Crate Bindings  │  │   │
│  │  │ Registry   │ │  (직접 Rust 호출)        │  │   │
│  │  └────────────┘ └────────────────────────┘   │   │
│  └──────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────┘
         │                    │
         ▼                    ▼
  ┌──────────────┐   ┌───────────────┐
  │  AI APIs     │   │  ethrex Node  │
  │  - Claude    │   │  - L1 Client  │
  │  - OpenAI    │   │  - L2 Client  │
  │  - Gemini    │   │  - Prover     │
  └──────────────┘   └───────────────┘
```

## 3. 모듈 설계

### 3.1 AI Service (`ai_service`)

```rust
// AI 프로바이더 추상화
pub trait AiProvider: Send + Sync {
    async fn chat(&self, messages: Vec<Message>, tools: Vec<Tool>) -> Result<Response>;
    async fn stream_chat(&self, messages: Vec<Message>, tools: Vec<Tool>) -> Result<Stream>;
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
}

// 구현체
pub struct ClaudeProvider { api_key: String, model: String }
pub struct OpenAiProvider { api_key: String, model: String }
pub struct GeminiProvider { api_key: String, model: String }

// AI에게 제공할 도구 정의
pub struct Tool {
    name: String,
    description: String,
    parameters: serde_json::Value,
    handler: Box<dyn ToolHandler>,
}
```

### 3.2 Process Manager (`process_manager`)

```rust
pub struct ProcessManager {
    processes: HashMap<String, ManagedProcess>,
}

pub struct ManagedProcess {
    name: String,           // "ethrex-l1", "ethrex-l2", "prover"
    command: String,
    args: Vec<String>,
    status: ProcessStatus,
    pid: Option<u32>,
    log_path: PathBuf,
}

pub enum ProcessStatus {
    Stopped,
    Starting,
    Running,
    Error(String),
}

impl ProcessManager {
    pub fn start_process(&mut self, name: &str) -> Result<()>;
    pub fn stop_process(&mut self, name: &str) -> Result<()>;
    pub fn restart_process(&mut self, name: &str) -> Result<()>;
    pub fn get_status(&self, name: &str) -> ProcessStatus;
    pub fn stream_logs(&self, name: &str) -> Result<LogStream>;
}
```

### 3.3 AI Guide Registry (`ai_guide`)

AI가 앱의 기능을 이해하고 실행할 수 있도록 가이드를 제공하는 모듈.

```rust
pub struct AiGuideRegistry {
    tools: Vec<AiTool>,
    system_prompt: String,
    context: AppContext,
}

pub struct AiTool {
    name: String,              // "start_l2_node"
    description: String,       // "L2 노드를 시작합니다"
    parameters: JsonSchema,    // 파라미터 스키마
    category: ToolCategory,    // NodeControl, Dashboard, Wallet, Marketplace
}

pub enum ToolCategory {
    NodeControl,    // 노드 시작/중지 등
    Dashboard,      // 대시보드 조회
    Wallet,         // TON 충전/전송
    Marketplace,    // L2 마켓 탐색
    Config,         // 설정 변경
}
```

### 3.4 Dashboard Manager (`dashboard`)

```rust
pub struct DashboardManager {
    tabs: Vec<DashboardTab>,
}

pub struct DashboardTab {
    id: String,
    name: String,
    url: String,
    icon: String,
    is_active: bool,
}

impl DashboardManager {
    pub fn add_tab(&mut self, tab: DashboardTab);
    pub fn remove_tab(&mut self, id: &str);
    pub fn get_active_url(&self) -> Option<&str>;
}
```

### 3.5 Wallet & TON Manager (`wallet`)

```rust
pub struct WalletManager {
    user_wallet: Option<Wallet>,
    ai_wallet: Option<Wallet>,    // AI 에이전트 전용 지갑
}

pub struct Wallet {
    address: Address,
    balance_l1: U256,
    balance_l2: U256,
}

impl WalletManager {
    pub async fn get_balance(&self) -> Result<Balance>;
    pub async fn deposit_to_l2(&self, amount: U256) -> Result<TxHash>;
    pub async fn withdraw_to_l1(&self, amount: U256) -> Result<TxHash>;
    pub async fn fund_ai_wallet(&self, amount: U256) -> Result<TxHash>;
}
```

## 4. Tauri IPC 명령어

Frontend ↔ Backend 통신은 Tauri의 `invoke` 시스템을 사용한다.

```rust
// Node Control
#[tauri::command] fn start_node(name: String) -> Result<()>;
#[tauri::command] fn stop_node(name: String) -> Result<()>;
#[tauri::command] fn get_node_status(name: String) -> Result<NodeStatus>;
#[tauri::command] fn get_logs(name: String, lines: usize) -> Result<Vec<String>>;

// AI Chat
#[tauri::command] fn send_message(message: String) -> Result<AiResponse>;
#[tauri::command] fn set_ai_provider(provider: String, api_key: String) -> Result<()>;
#[tauri::command] fn get_chat_history() -> Result<Vec<ChatMessage>>;

// Wallet
#[tauri::command] fn get_balance() -> Result<Balance>;
#[tauri::command] fn deposit(amount: String) -> Result<TxHash>;
#[tauri::command] fn withdraw(amount: String) -> Result<TxHash>;

// Marketplace
#[tauri::command] fn list_l2_services() -> Result<Vec<L2Service>>;
#[tauri::command] fn get_service_guide(service_id: String) -> Result<ServiceGuide>;

// Config
#[tauri::command] fn get_config() -> Result<AppConfig>;
#[tauri::command] fn update_config(config: AppConfig) -> Result<()>;
```

## 5. 디렉토리 구조

```
ethrex/
├── crates/
│   └── desktop-app/              # Tauri 앱 크레이트
│       ├── Cargo.toml
│       ├── tauri.conf.json       # Tauri 설정
│       ├── src/                  # Rust Backend
│       │   ├── main.rs
│       │   ├── ai_service/
│       │   │   ├── mod.rs
│       │   │   ├── claude.rs
│       │   │   ├── openai.rs
│       │   │   └── gemini.rs
│       │   ├── process_manager/
│       │   │   └── mod.rs
│       │   ├── ai_guide/
│       │   │   ├── mod.rs
│       │   │   ├── tools.rs      # AI가 사용할 도구 정의
│       │   │   └── prompts.rs    # 시스템 프롬프트
│       │   ├── dashboard/
│       │   │   └── mod.rs
│       │   ├── wallet/
│       │   │   └── mod.rs
│       │   └── commands.rs       # Tauri IPC 명령어
│       └── ui/                   # React Frontend
│           ├── package.json
│           ├── src/
│           │   ├── App.tsx
│           │   ├── components/
│           │   │   ├── Sidebar.tsx
│           │   │   ├── ChatView.tsx
│           │   │   ├── DashboardView.tsx
│           │   │   ├── NodeControlView.tsx
│           │   │   ├── MarketplaceView.tsx
│           │   │   ├── WalletView.tsx
│           │   │   └── SettingsView.tsx
│           │   ├── stores/
│           │   │   ├── chatStore.ts
│           │   │   ├── nodeStore.ts
│           │   │   └── walletStore.ts
│           │   └── styles/
│           │       └── globals.css
│           └── index.html
```

## 6. 보안 설계

| 위험 | 대응 |
|------|------|
| API 키 노출 | OS Keychain에 저장, 메모리에서 즉시 해제 |
| 프라이빗 키 | 앱이 직접 관리하지 않음, 외부 지갑 연동 |
| IPC 인젝션 | Tauri의 allowlist로 허용된 명령만 실행 |
| AI 프롬프트 인젝션 | 시스템 프롬프트에서 민감 작업 전 사용자 확인 요구 |
| 프로세스 제어 | 허용된 바이너리(ethrex, prover)만 실행 가능 |
