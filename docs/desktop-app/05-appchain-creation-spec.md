# Appchain Creation & Deployment Specification

## Overview

사용자가 앱체인을 생성하고 배포하는 3가지 경로를 지원한다.

| Mode | L1 | Contract Deploy | Prover | 사용자 준비물 |
|------|----|----------------|--------|-------------|
| Local | ethrex `--dev` (자동) | 자동 | 없음 (exec) | ethrex 바이너리만 |
| Testnet | Sepolia/Holesky RPC | 자동 (테스트 ETH 필요) | 선택 (sp1/risc0/exec) | RPC URL + 테스트 ETH |
| Mainnet | Ethereum RPC | 수동 확인 후 배포 | 필수 (sp1/risc0) | RPC URL + ETH + 개인키 |

---

## 1. ethrex L2 실행 구조 (현재)

### 1.1 `--dev` 모드 (Local)

```bash
ethrex l2 --dev --osaka-activation-time <timestamp>
```

이 한 줄로:
1. L1 ethrex 노드 자동 실행 (localhost:8545)
2. Bridge + OnChainProposer 컨트랙트 자동 배포
3. L2 노드 시작 (localhost:1729)
4. 시퀀서 자동 시작

### 1.2 수동 모드 (Testnet/Mainnet)

**Step 1: L1 준비**
```bash
# L1 RPC가 이미 존재 (Sepolia/Mainnet)
```

**Step 2: 컨트랙트 배포**
```bash
ethrex l2 deploy \
  --eth-rpc-url <L1_RPC_URL> \
  --private-key <DEPLOYER_KEY> \
  --on-chain-proposer-owner <OWNER_ADDR> \
  --bridge-owner <OWNER_ADDR> \
  --bridge-owner-pk <BRIDGE_OWNER_KEY> \
  --genesis-l1-path <L1_GENESIS> \
  --genesis-l2-path <L2_GENESIS>
```

배포 후 `.env` 파일에 컨트랙트 주소 저장:
- `ETHREX_WATCHER_BRIDGE_ADDRESS`
- `ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS`

**Step 3: L2 노드 실행**
```bash
ethrex l2 \
  --network <L2_GENESIS> \
  --http.port 1729 \
  --eth.rpc-url <L1_RPC_URL> \
  --l1.bridge-address <BRIDGE_ADDR> \
  --l1.on-chain-proposer-address <PROPOSER_ADDR> \
  --committer.l1-private-key <COMMITTER_KEY> \
  --proof-coordinator.l1-private-key <PROOF_KEY>
```

**Step 4: 프로버 (선택)**
```bash
ethrex l2 prover \
  --proof-coordinators tcp://127.0.0.1:3900 \
  --backend sp1|risc0|exec
```

### 1.3 주요 변수

| 변수 | 설명 | 기본값 |
|------|------|--------|
| L1_RPC_URL | L1 RPC 엔드포인트 | http://localhost:8545 |
| L2_PORT | L2 RPC 포트 | 1729 |
| L1_PRIVATE_KEY | L1 배포/커밋터 키 | dev key |
| BRIDGE_ADDRESS | 브릿지 컨트랙트 주소 | 배포 후 생성 |
| ON_CHAIN_PROPOSER_ADDRESS | 프로포저 컨트랙트 주소 | 배포 후 생성 |

---

## 2. Desktop App 구현 설계

### 2.1 앱체인 생성 흐름

```
[홈] → STEP 선택 (Local/Testnet/Mainnet)
  ↓
[위저드] → Step 1: 기본 정보 (이름, Chain ID, 아이콘, 설명)
  ↓
[위저드] → Step 2: 네트워크 (L1 RPC, L2 포트, 시퀀서 모드)
  ↓
[위저드] → Step 3: 토큰/프로버 (네이티브 토큰, 프로버 타입)
  ↓
[위저드] → Step 4: 공개 설정 (오픈 앱체인, 해시태그) + 요약
  ↓
[앱체인 생성] 버튼 클릭
  ↓
[셋업 진행 화면] ← 새로 만들 화면
```

### 2.2 셋업 진행 화면 (SetupProgressView)

"앱체인 생성" 버튼 클릭 후 보이는 화면:

```
┌─────────────────────────┐
│ DEX Chain 생성 중...     │
│                         │
│ ✅ 설정 파일 생성        │
│ ✅ L1 노드 시작          │  ← Local에서만
│ ⏳ 컨트랙트 배포 중...   │
│ ⬜ L2 노드 시작          │
│ ⬜ 시퀀서 시작           │
│ ⬜ 프로버 시작           │  ← 설정한 경우만
│                         │
│ [로그 보기 ▼]            │
│                         │
│         [취소]           │
└─────────────────────────┘
```

### 2.3 모드별 셋업 단계

**Local Mode:**
1. 설정 파일 생성 (appchain config JSON)
2. `ethrex l2 --dev` 실행 → L1+배포+L2 자동
3. 완료 대기 (로그에서 "L2 initializer started" 감지)

**Testnet Mode:**
1. 설정 파일 생성
2. L1 RPC 연결 확인
3. 잔액 확인 (배포할 ETH 충분한지)
4. `ethrex l2 deploy` 실행 → 컨트랙트 배포
5. `ethrex l2` 실행 → L2 노드 시작
6. (선택) `ethrex l2 prover` 실행

**Mainnet Mode:**
1. 설정 파일 생성
2. L1 RPC 연결 확인
3. 잔액 확인 + 사용자 확인 (실제 ETH 소모됨 경고)
4. `ethrex l2 deploy` 실행
5. `ethrex l2` 실행
6. `ethrex l2 prover` 실행 (필수 권장)

---

## 3. Tauri Backend 구현

### 3.1 새 IPC Commands

```rust
// 앱체인 설정 저장
#[tauri::command]
async fn create_appchain(config: AppchainConfig) -> Result<String, String>

// 앱체인 셋업 시작 (L1 → 배포 → L2 → 프로버)
#[tauri::command]
async fn start_appchain_setup(id: String) -> Result<(), String>

// 셋업 진행 상태 조회
#[tauri::command]
async fn get_setup_progress(id: String) -> Result<SetupProgress, String>

// 셋업 로그 스트리밍
#[tauri::command]
async fn get_setup_logs(id: String, offset: usize) -> Result<Vec<String>, String>

// 셋업 취소
#[tauri::command]
async fn cancel_setup(id: String) -> Result<(), String>

// 앱체인 목록 조회
#[tauri::command]
async fn list_appchains() -> Result<Vec<AppchainConfig>, String>

// 앱체인 삭제
#[tauri::command]
async fn delete_appchain(id: String) -> Result<(), String>
```

### 3.2 AppchainConfig 구조

```rust
#[derive(Serialize, Deserialize, Clone)]
struct AppchainConfig {
    id: String,
    name: String,
    icon: String,
    chain_id: u64,
    description: String,
    network_mode: NetworkMode, // Local | Testnet | Mainnet

    // Network
    l1_rpc_url: String,
    l2_rpc_port: u16,
    sequencer_mode: String, // standalone | shared

    // Token / Prover
    native_token: String,
    prover_type: String, // sp1 | risc0 | exec | none

    // Keys (Testnet/Mainnet)
    deployer_private_key: Option<String>,
    committer_private_key: Option<String>,

    // Deploy result
    bridge_address: Option<String>,
    on_chain_proposer_address: Option<String>,

    // Public
    is_public: bool,
    hashtags: Vec<String>,

    // Status
    status: AppchainStatus, // Created | Setting Up | Running | Stopped | Error
    created_at: String,
}

#[derive(Serialize, Deserialize)]
enum NetworkMode { Local, Testnet, Mainnet }

#[derive(Serialize, Deserialize)]
enum AppchainStatus { Created, SettingUp, Running, Stopped, Error(String) }
```

### 3.3 SetupProgress 구조

```rust
#[derive(Serialize, Deserialize)]
struct SetupProgress {
    steps: Vec<SetupStep>,
    current_step: usize,
    logs: Vec<String>,
    error: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct SetupStep {
    id: String,
    label: String,
    status: StepStatus, // Pending | InProgress | Done | Error | Skipped
}
```

### 3.4 AppchainManager

```rust
struct AppchainManager {
    appchains: HashMap<String, AppchainConfig>,
    processes: HashMap<String, Vec<Child>>, // L1, L2, Prover processes
    setup_progress: HashMap<String, SetupProgress>,
    config_dir: PathBuf, // ~/.tokamak-appchain/
}
```

설정 파일 저장 위치:
```
~/.tokamak-appchain/
  ├── appchains.json          # 앱체인 목록
  ├── chains/
  │   ├── <chain-id>/
  │   │   ├── config.json     # 앱체인 설정
  │   │   ├── .env            # 배포된 컨트랙트 주소
  │   │   ├── genesis-l2.json # L2 제네시스
  │   │   └── data/           # DB 디렉토리
```

---

## 4. Frontend 구현

### 4.1 새 컴포넌트

| 컴포넌트 | 역할 |
|---------|------|
| `SetupProgressView.tsx` | 셋업 진행 화면 (체크리스트 + 로그) |

### 4.2 위저드 → 셋업 연결

1. 위저드 "앱체인 생성" 클릭
2. `invoke('create_appchain', config)` → 설정 저장
3. `invoke('start_appchain_setup', id)` → 셋업 시작
4. `SetupProgressView` 표시
5. 폴링으로 `invoke('get_setup_progress', id)` 조회
6. 완료 시 → L2DetailView로 이동

### 4.3 i18n Keys 추가

```
setup.title: '앱체인 생성 중...' / 'Setting up appchain...'
setup.step.config: '설정 파일 생성' / 'Creating config'
setup.step.l1: 'L1 노드 시작' / 'Starting L1 node'
setup.step.deploy: '컨트랙트 배포' / 'Deploying contracts'
setup.step.l2: 'L2 노드 시작' / 'Starting L2 node'
setup.step.sequencer: '시퀀서 시작' / 'Starting sequencer'
setup.step.prover: '프로버 시작' / 'Starting prover'
setup.step.done: '완료!' / 'Done!'
setup.viewLogs: '로그 보기' / 'View logs'
setup.cancel: '취소' / 'Cancel'
setup.balanceCheck: '잔액 확인' / 'Checking balance'
setup.balanceWarning: '실제 ETH가 사용됩니다' / 'Real ETH will be spent'
```

---

## 5. 개발 순서

### Phase 1A: Local Mode (MVP)
1. `AppchainManager` Rust 구현 (설정 저장/로드)
2. `create_appchain` + `list_appchains` IPC 커맨드
3. `start_appchain_setup` - Local 모드만 (`ethrex l2 --dev` 실행)
4. `SetupProgressView` 프론트엔드
5. MyL2View에서 저장된 앱체인 목록 표시 (하드코딩 제거)
6. 프로세스 관리 (시작/중지/재시작)

### Phase 1B: Testnet Mode
1. L1 RPC 연결 확인 기능
2. 잔액 확인 기능
3. `ethrex l2 deploy` 실행 + 컨트랙트 주소 파싱
4. `ethrex l2` 수동 실행 (배포된 주소 사용)
5. 프로버 선택적 실행

### Phase 1C: Mainnet Mode
1. 사용자 확인 다이얼로그 (ETH 소모 경고)
2. 개인키 안전 관리 (OS Keychain 연동)
3. 트랜잭션 가스 추정 + 표시
4. 배포 전 시뮬레이션

### Phase 1D: 앱체인 관리
1. 앱체인 시작/중지/재시작 (개별 프로세스 제어)
2. 실시간 로그 스트리밍
3. 앱체인 삭제 (DB + 설정 정리)
4. 앱체인 내보내기/가져오기

---

## 6. ethrex 바이너리 관리

Desktop App이 ethrex를 실행하려면 바이너리가 필요하다.

### 옵션 A: 사용자가 직접 빌드/설치 (현재)
- 설정에서 바이너리 경로 지정
- 장점: 간단
- 단점: 진입장벽 높음

### 옵션 B: 앱에서 자동 빌드 (권장)
- 앱이 cargo를 호출해서 ethrex를 빌드
- `cargo build --release --features l2,l2-sql --manifest-path <path> --bin ethrex`
- 장점: 원클릭
- 단점: Rust toolchain 필요, 빌드 시간

### 옵션 C: 미리 빌드된 바이너리 다운로드 (미래)
- GitHub Releases에서 플랫폼별 바이너리 다운로드
- 장점: 가장 쉬움
- 단점: 릴리스 파이프라인 필요

**현재 전략**: 옵션 B 먼저 구현, 옵션 C는 나중에.
