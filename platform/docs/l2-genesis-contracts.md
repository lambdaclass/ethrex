# L2 Genesis Contract Addresses

L2 제네시스(`fixtures/genesis/l2-zk-dex.json`)에 배포되는 모든 컨트랙트 주소 목록.

## System Contracts (Guest Program에서 인식)

| Address | Name | 역할 |
|---------|------|------|
| `0x000000000000000000000000000000000000ffff` | **CommonBridgeL2** | L2 브릿지 (입금 수신, **출금 전송**) |
| `0x000000000000000000000000000000000000fffe` | **L2ToL1Messenger** | L2→L1 메시지 전달 (storage slot 0 = lastMessageId) |
| `0x000000000000000000000000000000000000fffc` | **FeeTokenRegistry** | 수수료 토큰 레지스트리 |
| `0x000000000000000000000000000000000000fffb` | **FeeTokenRatio** | 수수료 토큰 비율 관리 |

> Guest program의 `is_system_contract()`가 위 4개 주소를 시스템 컨트랙트로 인식.
> `CommonBridgeL2`(`0x...ffff`)로 보내는 tx는 출금으로 처리됨.

## ZK-DEX Application Contracts

| Address | Name | 역할 |
|---------|------|------|
| `0xDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDE` | **ZkDex (Main)** | DEX 메인 컨트랙트 |
| `0xDE00000000000000000000000000000000000001` | **Verifier 1** | Deposit 검증 |
| `0xDE00000000000000000000000000000000000002` | **Verifier 2** | Withdrawal 검증 |
| `0xDE00000000000000000000000000000000000003` | **Verifier 3** | Mint 검증 |
| `0xDE00000000000000000000000000000000000004` | **Verifier 4** | Spend 검증 |
| `0xDE00000000000000000000000000000000000005` | **Verifier 5** | MakeOrder 검증 |
| `0xDE00000000000000000000000000000000000006` | **Verifier 6** | SettleOrder 검증 |

> Guest program의 `DexCircuit`에서 `DEX_CONTRACT_ADDRESS = 0xDEDE...DEDE`로 향하는 tx를 앱 operation으로 분류.

## EVM Precompile-style Contracts

| Address Range | Count | 역할 |
|---------------|-------|------|
| `0x...efff` ~ `0x...effb` | 5개 | 시스템 유틸리티 |
| `0x...ff00` ~ `0x...fffa` | 251개 | ZK precompile / 시스템 |
| `0x...fffd` | 1개 | 추가 시스템 |

## Utility Contracts

| Address | Name |
|---------|------|
| `0x4e59b44847b379578588920cA78FbF26c0B4956C` | Deterministic Deployer (CREATE2) |
| `0x914d7Fec6aaC8cd542e72Bca78B30650d45643d7` | Create2Deployer |
| `0x13b0D85CcB8bf860b6b79AF3029fCA081AE9beF2` | Permit2 |

---

## 출금 방법

**올바른 출금 주소**: `0x000000000000000000000000000000000000ffff` (CommonBridgeL2)

```bash
cast send 0x000000000000000000000000000000000000ffff \
  "withdraw(address)" <수신자_L1_주소> \
  --value <금액> \
  --rpc-url http://localhost:<L2_PORT> \
  --private-key <개인키>
```

> `0xdead...0001`은 이 프로젝트의 주소가 아닙니다. 출금 시 반드시 `0x...ffff`를 사용하세요.
