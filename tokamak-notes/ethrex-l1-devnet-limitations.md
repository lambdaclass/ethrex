# ethrex L1 Devnet 제한사항 분석

## 개요

ethrex를 L1 devnet으로 사용할 때 발견된 JSON-RPC 스펙 미준수 및 기능 제한 사항을 정리한다.
실제 운영 환경에서는 Geth/Reth 등 완전한 노드를 L1으로 사용하므로 직접적인 영향은 없으나,
로컬 개발/테스트 시 혼동을 줄 수 있어 기록해 둔다.

---

## 1. `eth_getBalance` block 파라미터 무시

### 현상

`eth_getBalance(address, blockNumber)`에서 두 번째 파라미터(block number)를 무시하고
항상 **latest 상태의 잔액**만 반환한다.

### 검증 결과

```
User genesis balance: 1,000,000,000 ETH
User balance at block 0:      999,999,994.998367 ETH  (latest와 동일)
User balance at block 100:    999,999,994.998367 ETH  (latest와 동일)
User balance at block 401:    999,999,994.998367 ETH  (latest와 동일, 이 블록에서 10 ETH 예치했음)
User balance at latest:       999,999,994.998367 ETH
```

모든 블록 번호에 대해 동일한 값을 반환한다.
실제로는 block 401에서 10 ETH 예치, block 964에서 5 ETH 클래임이 있었으므로
과거 블록 조회 시 다른 값이 나와야 정상이다.

### 원인 (추정)

ethrex가 **archive node** 모드를 지원하지 않거나, 과거 상태(state trie)를 저장하지 않는 것으로 보인다.
최신 블록의 world state만 유지하고 있어 historical state query가 불가능하다.

### Ethereum JSON-RPC 스펙

```
eth_getBalance(address, blockNumber)
- blockNumber: "latest", "earliest", "pending", 또는 hex block number
- 해당 블록 시점의 잔액을 반환해야 함
```

참고: https://ethereum.org/en/developers/docs/apis/json-rpc/#eth_getbalance

### 영향

- Blockscout 등 블록 탐색기에서 historical balance chart가 부정확할 수 있음
- DApp에서 과거 시점 잔액 조회가 필요한 경우 동작하지 않음
- 디버깅 시 "이 블록에서 잔액이 얼마였나?" 추적 불가

---

## 2. `debug_traceTransaction` 미지원

### 현상

```json
{
  "error": {
    "code": -32603,
    "message": "Internal Error: Exceeded max amount of blocks to re-execute for tracing"
  }
}
```

`debug_traceTransaction`을 호출하면 위 에러가 반환된다.
`callTracer` 옵션 유무와 관계없이 동일하게 실패한다.

### 영향

- **Blockscout에서 internal transaction이 표시되지 않음**
  - 예: `claimWithdrawal` 호출 시 `payable(msg.sender).call{value: 5 ETH}("")`로 발생하는
    내부 ETH 전송이 Blockscout에서 보이지 않음
  - 사용자가 클래임 성공 여부를 시각적으로 확인하기 어려움
- Contract 간 호출 흐름(call trace)을 디버깅할 수 없음
- Gas profiling 불가

### 검증

```bash
# Blockscout internal transactions API — 빈 결과
curl 'http://localhost:8083/api/v2/transactions/{txHash}/internal-transactions'
# → {"items": [], "next_page_params": null}
```

---

## 3. `eth_getLogs` 인덱스 불일치 (추가 조사 필요)

### 현상

Bridge 컨트랙트 주소로 `eth_getLogs`를 조회하면 결과가 0건이지만,
개별 트랜잭션 receipt의 `logs` 필드에는 이벤트가 정상적으로 포함되어 있다.

```bash
# 0건 반환
eth_getLogs({fromBlock: "0x0", toBlock: "latest", address: "0x651c..."})

# 하지만 개별 receipt에는 로그가 있음
eth_getTransactionReceipt("0x361936...") → logs: [WithdrawalClaimed event]
```

### 원인 (추정)

로그 인덱스(bloom filter 또는 log DB)가 제대로 구축되지 않았거나,
devnet 재시작 시 인덱스가 리빌드되지 않는 것으로 추정된다.

### 영향

- Blockscout 이벤트 인덱싱 누락
- L2 watcher가 `eth_getLogs`로 L1 이벤트를 모니터링하는 경우 이벤트 수신 실패 가능
  - 단, 실제 L2 시스템은 정상 동작하고 있으므로 다른 경로로 이벤트를 수신하거나
    이 문제가 특정 조건에서만 발생할 수 있음

---

## 대안 비교

| 기능 | ethrex L1 devnet | Anvil (Foundry) | Geth (dev mode) |
|------|:---:|:---:|:---:|
| 표준 JSON-RPC | 부분 지원 | 완전 지원 | 완전 지원 |
| `eth_getBalance` historical | X | O | O (archive) |
| `debug_traceTransaction` | X | O | O |
| `eth_getLogs` 정확성 | 불확실 | O | O |
| EIP-4844 blob tx | O | O | O |
| Blockscout internal tx | X | O | O |
| 유지보수 | ethrex 팀 | Foundry 커뮤니티 | Ethereum Foundation |

### Anvil 대체 시 장점

- 위 3가지 제한사항이 모두 해결됨
- `--steps-tracing` 옵션으로 상세 트레이싱 가능
- `--state-interval` 옵션으로 state snapshot 저장 가능
- Docker 이미지: `ghcr.io/foundry-rs/foundry:latest`

### Anvil 대체 시 필요 작업

1. Docker compose에서 L1 서비스를 Anvil로 교체
2. L1 genesis 설정을 Anvil 형식으로 변환 (또는 `--init` 스크립트 사용)
3. L1 컨트랙트 (Bridge, OnChainProposer, SP1Verifier 등) 배포 스크립트 호환성 확인
4. EIP-4844 blob 지원 확인 (`--odyssey` 또는 최신 버전)

---

## TODO

- [ ] ethrex 코드에서 `eth_getBalance` historical 미지원 원인 확인 (state trie pruning 여부)
- [ ] `eth_getLogs` 인덱스 누락 조건 재현 및 원인 분석
- [ ] Anvil L1 대체 PoC 테스트
- [ ] ethrex upstream에 이슈 리포트 여부 검토

---

*작성일: 2026-02-27*
*환경: ethrex L1 devnet (Docker), Blockscout v6*
