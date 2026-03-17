# Tools/Dashboard 테스트넷(Sepolia/Holesky) 대응 작업

> GitHub Issue: #26 (Etherscan contract auto-verification)
> 별도 브랜치에서 작업 필요

## 현상

세폴리아 L1으로 배포 시 Dashboard(Bridge UI)에 다음 문제가 발생:

- L1 Chain ID가 `9`로 표시됨 (실제: 11155111)
- L1 RPC가 `localhost:8545`로 표시됨 (실제: 외부 Sepolia RPC)
- L1 Explorer가 로컬 Blockscout 링크 (실제: Etherscan 사용해야 함)
- "Localnet" 배지가 표시됨
- 로컬 테스트 계정(1B ETH)이 표시됨 (세폴리아에는 없음)
- MetaMask 네트워크 설정이 Chain ID 9, localhost:8545로 표시됨
- L1 Explorer Backend/Frontend 컨테이너가 불필요하게 실행됨

## 원인

`docker-compose-zk-dex-tools.yaml`와 `tooling/bridge/` 파일들이 로컬 전용으로 하드코딩되어 있음.

## 수정 대상 파일

### 1. `crates/l2/tooling/bridge/entrypoint.sh`

config.json 생성 시 테스트넷 환경변수 지원 추가:

```sh
# 테스트넷이면 환경변수에서 읽고, 아니면 기존 localhost 폴백
if [ -n "$L1_RPC_URL" ]; then
  L1_RPC_VALUE="$L1_RPC_URL"
else
  L1_RPC_VALUE="http://localhost:${TOOLS_L1_RPC_PORT:-8545}"
fi
# L1_EXPLORER_URL, L1_CHAIN_ID, L1_NETWORK_NAME, IS_TESTNET 동일 패턴
```

config.json에 추가할 필드:
- `l1_chain_id`: 9 → `${L1_CHAIN_ID:-9}`
- `l1_network_name`: `${L1_NETWORK_NAME:-Local}`
- `is_testnet`: `${IS_TESTNET:-false}`

### 2. `crates/l2/tooling/bridge/dashboard.html`

`loadConfig()` 함수에서 config.json 기반으로 동적 렌더링:

| 영역 | 로컬 모드 | 테스트넷 모드 |
|------|----------|-------------|
| 배지 | "Localnet" | "Testnet (Sepolia)" |
| L1 Chain ID | 9 | config.l1_chain_id |
| L1 Explorer 링크 | localhost:8083 (Blockscout) | sepolia.etherscan.io |
| L1 RPC 표시 | localhost:8545 | 외부 RPC URL |
| Test Accounts 섹션 | 표시 (1B ETH) | 숨김 |
| L1 Blockscout 서비스 카드 | 표시 | Etherscan 링크로 대체 |
| MetaMask L1 설정 | Chain ID 9, localhost | Chain ID 11155111, 외부 RPC |
| Footer | "Localnet Dashboard" | "Testnet Dashboard" |

구현 방법:
```javascript
// loadConfig() 내에서
if (CONFIG.is_testnet) {
  document.querySelector('.badge').textContent = 'Testnet (' + CONFIG.l1_network_name + ')';
  document.querySelector('.chain-stat-title.l1').textContent = 'L1 (' + CONFIG.l1_network_name + ', Chain ID: ' + CONFIG.l1_chain_id + ')';
  document.getElementById('testAccountsSection').style.display = 'none';
  // L1 Blockscout 서비스 카드 → Etherscan으로 대체
  // MetaMask L1 설정 업데이트
}
```

### 3. `crates/l2/tooling/bridge/index.html` (Bridge UI)

Bridge 페이지도 L1 RPC URL을 config.json에서 읽으므로 자동 대응됨.
단, L1 chain ID 검증 로직이 있다면 동적 처리 필요.

### 4. `crates/l2/docker-compose-zk-dex-tools.yaml`

bridge-ui 서비스에 환경변수 추가:
```yaml
bridge-ui:
  environment:
    TOOLS_L1_RPC_PORT: ${TOOLS_L1_RPC_PORT:-8545}
    TOOLS_L2_RPC_PORT: ${TOOLS_L2_RPC_PORT:-1729}
    TOOLS_L1_EXPLORER_PORT: ${TOOLS_L1_EXPLORER_PORT:-8083}
    TOOLS_L2_EXPLORER_PORT: ${TOOLS_L2_EXPLORER_PORT:-8082}
    TOOLS_METRICS_PORT: ${TOOLS_METRICS_PORT:-3702}
    # 테스트넷 전용 (비어있으면 로컬 모드)
    L1_CHAIN_ID: ${L1_CHAIN_ID:-9}
    L1_RPC_URL: ${L1_RPC_URL:-}
    L1_EXPLORER_URL: ${L1_EXPLORER_URL:-}
    L1_NETWORK_NAME: ${L1_NETWORK_NAME:-Local}
    IS_TESTNET: ${IS_TESTNET:-false}
```

proxy(nginx) 서비스:
- `depends_on`에서 `backend-l1`, `frontend-l1` 제거하면 테스트넷에서도 정상 시작
- 또는 테스트넷 전용 nginx 설정에서 L1 upstream 블록 제거
- 가장 단순한 방법: L1 서비스가 없어도 proxy가 시작되게 `depends_on` 조건부 처리 (compose profiles 활용 가능)

### 5. `crates/desktop-app/local-server/lib/docker-local.js`

`startTools()`와 `restartTools()`에서 테스트넷 환경변수 전달:

```javascript
const toolsEnv = {
  TOOLS_L1_EXPLORER_PORT: ...,
  // ... 기존 ...
  // 테스트넷 전용
  L1_CHAIN_ID: String(toolsPorts.l1ChainId || 9),
  L1_RPC_URL: toolsPorts.l1RpcUrl || '',
  L1_EXPLORER_URL: toolsPorts.l1ExplorerUrl || '',
  L1_NETWORK_NAME: toolsPorts.l1NetworkName || 'Local',
  IS_TESTNET: toolsPorts.isTestnet ? 'true' : 'false',
};
```

### 6. `crates/desktop-app/local-server/lib/deployment-engine.js`

`provisionTestnet()` 함수에서 startTools 호출 시 테스트넷 정보 전달:

```javascript
await docker.startTools(freshEnv, {
  // ... 기존 포트 ...
  skipL1Explorer: true,
  l1ChainId: testnetCfg.l1ChainId || 11155111,
  l1RpcUrl: l1RpcUrl,
  l1ExplorerUrl: { sepolia: 'https://sepolia.etherscan.io', holesky: 'https://holesky.etherscan.io' }[testnetCfg.network] || '',
  l1NetworkName: { sepolia: 'Sepolia', holesky: 'Holesky' }[testnetCfg.network] || 'Custom',
  isTestnet: true,
});
```

## 환경변수 흐름

```
매니저 UI (config.testnet.network = "sepolia")
  → deployment-engine.js (provisionTestnet)
    → docker-local.js (startTools, toolsEnv에 L1_CHAIN_ID 등 설정)
      → docker compose up (환경변수 전달)
        → bridge-ui 컨테이너 (entrypoint.sh가 config.json 생성)
          → dashboard.html (config.json 읽어서 동적 렌더링)
```

## 네트워크별 설정값

| 항목 | Local | Sepolia | Holesky |
|------|-------|---------|---------|
| L1 Chain ID | 9 | 11155111 | 17000 |
| L1 RPC | localhost:8545 | 외부 RPC URL | 외부 RPC URL |
| L1 Explorer | localhost:8083 | sepolia.etherscan.io | holesky.etherscan.io |
| L1 Blockscout | 실행 | 스킵 | 스킵 |
| Test Accounts | 표시 | 숨김 | 숨김 |

## 참고

- proxy(nginx)가 `backend-l1`과 `frontend-l1`에 의존하므로, 테스트넷에서 이들을 스킵하면 proxy 시작이 실패할 수 있음
  - 해결: compose profiles 사용하거나, proxy depends_on에서 L1 서비스 제거하고 nginx 설정에서 L1 upstream 에러를 허용
- 한 번 Etherscan에서 검증하면 같은 바이트코드의 다른 배포는 "Similar Match"로 자동 매칭
- macOS solc vs Docker(Linux) solc 바이트코드 차이 (56바이트) → Docker 안에서 검증해야 함
