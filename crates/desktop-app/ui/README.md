# Tokamak Appchain Desktop App

Tauri 2.x + React + TypeScript 기반의 앱체인 관리 데스크톱 앱.

## Architecture

```
ui/
├── src/                    # React frontend
│   ├── App.tsx            # Root - routing, theme/lang context
│   ├── api/
│   │   ├── local-server.ts   # Local Docker deployment API client
│   │   └── platform.ts       # Platform API client (Keychain auth)
│   ├── components/
│   │   ├── HomeView.tsx       # Home - quick start, journey, quick links
│   │   ├── MyL2View.tsx       # Appchain list + create
│   │   ├── L2DetailView.tsx   # Appchain detail (control/logs/config/dashboard)
│   │   ├── CreateL2Wizard.tsx # Appchain creation wizard (local/testnet/mainnet)
│   │   ├── SetupProgressView.tsx # Appchain setup progress
│   │   ├── ChatView.tsx       # AI Pilot with context + action buttons
│   │   ├── ProgramStoreView.tsx  # Platform Program Store browser
│   │   ├── OpenL2View.tsx     # Open Appchain explorer
│   │   ├── WalletView.tsx     # TON wallet + bridge
│   │   ├── DashboardView.tsx  # Node monitoring
│   │   ├── NodeControlView.tsx
│   │   ├── SettingsView.tsx   # AI, Platform account, node config
│   │   └── Sidebar.tsx        # Navigation sidebar
│   └── i18n.ts            # Korean/English translations
├── src-tauri/src/          # Rust backend
│   ├── lib.rs             # App setup, tray, managed state
│   ├── commands.rs        # All Tauri commands
│   ├── appchain_manager.rs # Appchain CRUD + setup progress
│   ├── runner.rs          # ethrex process spawning & lifecycle
│   ├── ai_provider.rs     # Multi-provider AI (Claude/GPT/Gemini/Tokamak)
│   ├── local_server.rs    # Node.js local-server process management
│   └── process_manager.rs # Legacy node state
└── local-server/           # Node.js Express (Docker deployment engine)
    ├── server.js           # Express app, port 5002, localhost only
    ├── db/                 # SQLite (~/.tokamak-appchain/local.sqlite)
    ├── routes/             # deployments, hosts, fs
    ├── lib/                # docker-local, docker-remote, compose-generator
    └── public/             # Web UI for deployment management
```

## Views (ViewType)

| View | Key | Description |
|------|-----|-------------|
| Home | `home` | Quick start, appchain journey, quick links |
| My Appchains | `myl2` | Create/manage L2 appchains |
| AI Pilot | `chat` | AI assistant with context-aware actions |
| Nodes | `nodes` | Node control panel |
| Dashboard | `dashboard` | Monitoring dashboards |
| Open Appchain | `openl2` | Browse public appchains |
| Wallet | `wallet` | TON token management, L1↔L2 bridge |
| Program Store | `store` | Browse Platform programs |
| Settings | `settings` | Theme, language, AI, Platform account |

## Key Features

### Appchain Management
- **Local mode**: `ethrex l2 --dev` one-click setup
- **Testnet/Mainnet**: Sepolia/Ethereum L1 connection
- Native token: TON (TOKAMAK), Prover: SP1

### AI Pilot
- Multi-provider: Claude, GPT, Gemini, Tokamak AI
- Real-time appchain state injection via `get_chat_context`
- Clickable action buttons: `[ACTION:navigate:view=store]`
- API key stored in OS Keychain

### Platform Integration
- Program Store browsing (public, no auth)
- Open Appchain registration (auth required)
- Email/password login, token in OS Keychain
- API: `https://platform.tokamak.network`

### Local Server
- Node.js Express on port 5002 (localhost only)
- Docker deployment lifecycle (provision/start/stop/destroy)
- Remote SSH deployment via docker-remote
- SSE for real-time logs
- Auto-started by Tauri on app launch

## Local Development

### Desktop App 실행

```bash
# Local Server + Tauri 앱을 한번에 실행
cd crates/desktop-app && ./dev.sh
```

Desktop App만 띄우면 앱체인 생성/관리/AI Pilot 등 핵심 기능을 모두 사용할 수 있습니다.

### Platform 서비스 (필요 시)

Program Store 브라우저, Open Appchain 등록, Platform 계정 로그인 등을 테스트할 때만 별도 터미널에서 실행합니다.

```bash
cd platform && ./dev.sh
```

자세한 내용은 [platform/README.md](../../../platform/README.md) 참고.

### 개별 실행

```bash
cd ui && pnpm tauri dev          # Tauri dev (Frontend + Rust backend)
cd ui && pnpm dev                # Frontend만 (Vite dev server)
cd local-server && npm start     # Local Server만
cd ui && pnpm tauri build        # Build (배포용)
```

### 서비스 포트
| Service | Port | Description |
|---------|------|-------------|
| Tauri Dev (Vite) | 1420 | Frontend dev server |
| Local Server | 5002 | Docker deployment engine |
| Platform API | 5001 | Program Store / Auth API (필요 시) |
| Platform Client | 3000 | Next.js web client (필요 시) |

### 종료
`Ctrl+C` — dev.sh가 자식 프로세스를 자동 정리합니다.

## Business Rules

- Native token is always TON (TOKAMAK) — non-editable
- Prover type is always SP1 — non-editable
- Local mode: standalone sequencer only, cannot publish as Open Appchain
- Testnet/Mainnet: can publish to Open Appchain registry via Platform API
