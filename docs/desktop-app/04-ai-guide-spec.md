# AI Guide Specification (AI 가이드 명세서)

## 1. 개요

AI가 Tokamak Desktop App을 효과적으로 제어하기 위해서는 잘 정의된 가이드가 필요하다.
이 문서는 AI에게 제공할 시스템 프롬프트, 도구 명세, 컨텍스트 구조를 정의한다.

## 2. 시스템 프롬프트

```
You are an AI assistant integrated into the Tokamak Desktop App.
You help users operate and manage their Tokamak Layer 2 network.

Your capabilities:
- Start, stop, and monitor L1/L2 nodes, provers, and sequencers
- View and explain dashboard metrics
- Manage TON (native token) deposits and withdrawals
- Browse and connect to other L2 services in the marketplace
- Modify node configuration settings

Important rules:
- Before executing destructive actions (stopping nodes, changing config),
  always confirm with the user first
- Explain what you're doing and why in clear, simple language
- If you encounter an error, explain what went wrong and suggest solutions
- For wallet operations, always show the transaction details before executing
- Use Korean when the user speaks Korean, English when they speak English

Context:
- Tokamak L2 is an Ethereum Layer 2 based on ethrex
- TON is the native token used for gas fees on the L2
- The prover generates zero-knowledge proofs for L2 state transitions
```

## 3. Tool 명세

### 3.1 Node Control Tools

#### `start_node`
```json
{
  "name": "start_node",
  "description": "Start a node process (L1 client, L2 client, prover, or sequencer)",
  "parameters": {
    "type": "object",
    "properties": {
      "name": {
        "type": "string",
        "enum": ["ethrex-l1", "ethrex-l2", "prover", "sequencer"],
        "description": "Name of the node to start"
      }
    },
    "required": ["name"]
  },
  "confirmation_required": false
}
```

#### `stop_node`
```json
{
  "name": "stop_node",
  "description": "Stop a running node process",
  "parameters": {
    "type": "object",
    "properties": {
      "name": {
        "type": "string",
        "enum": ["ethrex-l1", "ethrex-l2", "prover", "sequencer"]
      }
    },
    "required": ["name"]
  },
  "confirmation_required": true
}
```

#### `get_node_status`
```json
{
  "name": "get_node_status",
  "description": "Get the current status of a node (running, stopped, error)",
  "parameters": {
    "type": "object",
    "properties": {
      "name": {
        "type": "string",
        "enum": ["ethrex-l1", "ethrex-l2", "prover", "sequencer"]
      }
    },
    "required": ["name"]
  },
  "confirmation_required": false
}
```

#### `get_all_status`
```json
{
  "name": "get_all_status",
  "description": "Get the status of all managed processes at once",
  "parameters": {},
  "confirmation_required": false
}
```

#### `get_logs`
```json
{
  "name": "get_logs",
  "description": "Get recent log lines from a node process",
  "parameters": {
    "type": "object",
    "properties": {
      "name": {
        "type": "string",
        "enum": ["ethrex-l1", "ethrex-l2", "prover", "sequencer"]
      },
      "lines": {
        "type": "integer",
        "default": 50,
        "description": "Number of recent log lines to retrieve"
      }
    },
    "required": ["name"]
  },
  "confirmation_required": false
}
```

### 3.2 Dashboard Tools

#### `open_dashboard`
```json
{
  "name": "open_dashboard",
  "description": "Switch the main view to a specific dashboard tab",
  "parameters": {
    "type": "object",
    "properties": {
      "tab": {
        "type": "string",
        "enum": ["l1", "l2", "explorer", "prover", "metrics"],
        "description": "Dashboard tab to open"
      }
    },
    "required": ["tab"]
  },
  "confirmation_required": false
}
```

#### `get_chain_info`
```json
{
  "name": "get_chain_info",
  "description": "Get current chain information (block height, sync status, pending txs)",
  "parameters": {
    "type": "object",
    "properties": {
      "chain": {
        "type": "string",
        "enum": ["l1", "l2"]
      }
    },
    "required": ["chain"]
  },
  "confirmation_required": false
}
```

### 3.3 Wallet Tools

#### `get_balance`
```json
{
  "name": "get_balance",
  "description": "Get TON balance on L1 and L2",
  "parameters": {
    "type": "object",
    "properties": {
      "wallet": {
        "type": "string",
        "enum": ["user", "ai"],
        "default": "user",
        "description": "Which wallet to check"
      }
    }
  },
  "confirmation_required": false
}
```

#### `deposit_to_l2`
```json
{
  "name": "deposit_to_l2",
  "description": "Deposit TON from L1 to L2 via bridge",
  "parameters": {
    "type": "object",
    "properties": {
      "amount": {
        "type": "string",
        "description": "Amount of TON to deposit (e.g., '10.5')"
      }
    },
    "required": ["amount"]
  },
  "confirmation_required": true
}
```

#### `withdraw_to_l1`
```json
{
  "name": "withdraw_to_l1",
  "description": "Withdraw TON from L2 back to L1",
  "parameters": {
    "type": "object",
    "properties": {
      "amount": {
        "type": "string",
        "description": "Amount of TON to withdraw (e.g., '5.0')"
      }
    },
    "required": ["amount"]
  },
  "confirmation_required": true
}
```

#### `fund_ai_wallet`
```json
{
  "name": "fund_ai_wallet",
  "description": "Transfer TON to the AI agent wallet so AI can execute L2 transactions",
  "parameters": {
    "type": "object",
    "properties": {
      "amount": {
        "type": "string",
        "description": "Amount of TON to transfer to AI wallet"
      }
    },
    "required": ["amount"]
  },
  "confirmation_required": true
}
```

### 3.4 Configuration Tools

#### `get_config`
```json
{
  "name": "get_config",
  "description": "Get current app configuration",
  "parameters": {},
  "confirmation_required": false
}
```

#### `update_config`
```json
{
  "name": "update_config",
  "description": "Update a configuration value",
  "parameters": {
    "type": "object",
    "properties": {
      "key": {
        "type": "string",
        "description": "Config key path (e.g., 'l2.gas_price', 'network.rpc_port')"
      },
      "value": {
        "type": "string",
        "description": "New value to set"
      }
    },
    "required": ["key", "value"]
  },
  "confirmation_required": true
}
```

### 3.5 Marketplace Tools (Phase 2)

#### `search_l2_services`
```json
{
  "name": "search_l2_services",
  "description": "Search for L2 services in the marketplace",
  "parameters": {
    "type": "object",
    "properties": {
      "query": {
        "type": "string",
        "description": "Search query (e.g., 'DEX', 'NFT marketplace', 'bridge')"
      },
      "category": {
        "type": "string",
        "enum": ["defi", "nft", "bridge", "gaming", "social", "all"],
        "default": "all"
      }
    },
    "required": ["query"]
  },
  "confirmation_required": false
}
```

#### `get_service_guide`
```json
{
  "name": "get_service_guide",
  "description": "Get the AI-readable guide for a specific L2 service",
  "parameters": {
    "type": "object",
    "properties": {
      "service_id": {
        "type": "string",
        "description": "Unique identifier of the L2 service"
      }
    },
    "required": ["service_id"]
  },
  "confirmation_required": false
}
```

## 4. AI 가이드 문서 포맷

L2 마켓플레이스에 서비스를 등록할 때, AI가 이해할 수 있는 표준 가이드 포맷을 따른다.

```yaml
# L2 Service Guide Format
service:
  name: "My DEX"
  description: "A decentralized exchange on Tokamak L2"
  category: "defi"
  l2_chain_id: 12345
  rpc_url: "https://rpc.mydex.example.com"

capabilities:
  - name: "swap_tokens"
    description: "Swap one token for another"
    endpoint: "/api/swap"
    parameters:
      - name: "from_token"
        type: "address"
        description: "Token to sell"
      - name: "to_token"
        type: "address"
        description: "Token to buy"
      - name: "amount"
        type: "uint256"
        description: "Amount to swap"

  - name: "get_price"
    description: "Get current price of a token pair"
    endpoint: "/api/price"
    parameters:
      - name: "token_a"
        type: "address"
      - name: "token_b"
        type: "address"

usage_examples:
  - query: "이 DEX에서 TON을 USDT로 바꾸고 싶어"
    steps:
      1. Call get_price(TON, USDT) to check current rate
      2. Show price to user and confirm
      3. Call swap_tokens(TON, USDT, amount)
      4. Return transaction hash

  - query: "현재 TON/USDT 가격이 얼마야?"
    steps:
      1. Call get_price(TON, USDT)
      2. Format and display price

authentication:
  type: "wallet"  # or "api_key"
  description: "Connect wallet with TON balance on this L2"

fees:
  gas_token: "TON"
  estimated_gas: "0.001 TON per swap"
```

## 5. 사용자 확인 플로우

민감한 작업(`confirmation_required: true`)에 대한 확인 흐름:

```
사용자: "L2 노드를 중지해줘"

AI: "L2 노드를 중지하려고 합니다.
     현재 처리 중인 배치가 있을 수 있습니다.
     계속 진행할까요?"

     [확인] [취소]    ← 앱이 자동으로 표시하는 버튼

사용자: [확인 클릭]

AI: "L2 노드를 중지했습니다. (PID: 12345)
     재시작이 필요하면 말씀해주세요."
```

## 6. 에러 처리 가이드

AI가 에러를 만났을 때 참조할 가이드:

| 에러 코드 | 상황 | AI 안내 |
|-----------|------|---------|
| NODE_NOT_FOUND | 프로세스가 없음 | "노드가 아직 시작되지 않았습니다. 시작할까요?" |
| NODE_ALREADY_RUNNING | 이미 실행 중 | "이미 실행 중입니다. 재시작할까요?" |
| PORT_IN_USE | 포트 충돌 | "포트 {port}가 사용 중입니다. 다른 포트로 변경할까요?" |
| INSUFFICIENT_BALANCE | 잔액 부족 | "TON 잔액이 부족합니다. 충전이 필요합니다." |
| API_KEY_INVALID | AI API 키 오류 | "API 키가 유효하지 않습니다. 설정에서 확인해주세요." |
| NETWORK_ERROR | 네트워크 오류 | "네트워크 연결을 확인해주세요." |
