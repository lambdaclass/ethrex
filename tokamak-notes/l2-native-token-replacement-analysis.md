# L2 커스텀 네이티브 가스 토큰 설계 문서

> **브랜치**: `feat/custom-native-gas-token`
> **목적**: 특정 ERC-20 토큰을 L2의 네이티브 가스 토큰으로 사용하여 ZK 회로 검증 비용 절감

## 1. 개요

### 왜 네이티브 가스 토큰인가?

현재 ethrex의 Fee Token 시스템은 ERC-20 컨트랙트 시뮬레이션(`lockFee`/`payFee`)을
**매 트랜잭션마다** ZK 회로 안에서 실행해야 한다. 이는:

1. **회로 크기 증가**: ERC-20 스토리지 읽기/쓰기가 ZK 프루버의 constraint 수를 대폭 늘림
2. **증명 생성 비용 증가**: Fee Token 시뮬레이션이 게스트 프로그램 실행 시간과 메모리를 소모
3. **검증 비용 증가**: 더 큰 회로 → L1 온체인 검증 가스비 상승

**네이티브 가스 토큰** 방식은 커스텀 토큰을 `AccountInfo.balance` 필드에 직접 매핑하여,
ERC-20 시뮬레이션 없이 `increase_account_balance()`/`decrease_account_balance()`만으로
가스비 처리를 완료한다. 이는 기존 ETH 가스비 처리와 동일한 회로 복잡도를 유지하면서
커스텀 토큰을 가스비로 사용할 수 있게 해준다.

### L1/L2 실행 환경 분리 구조

ethrex는 동일 코드베이스로 L1과 L2를 모두 실행할 수 있다.
**네이티브 가스 토큰 변경은 L2 체인에만 적용되며, L1 실행에는 영향을 주지 않는다.**

```
L1 노드 (이더리움 메인넷)              L2 노드 (ethrex 롤업)
┌─────────────────────────┐         ┌─────────────────────────┐
│ VMType::L1              │         │ VMType::L2(FeeConfig)   │
│ Hook: DefaultHook       │         │ Hook: L2Hook + BackupHook│
│ balance = ETH (불변)     │         │ balance = 커스텀 토큰 (변경)│
│ 별도 노드, 별도 상태 트리  │         │ 별도 노드, 별도 상태 트리  │
└─────────────────────────┘         └─────────────────────────┘
         │                                    │
         └──── CommonBridge (L1 컨트랙트) ─────┘
               ERC-20 lock ↔ L2 balance 매핑
```

**핵심**: L1과 L2는 **완전히 독립된 노드이자 독립된 상태 트리**다.
L2의 `AccountInfo.balance` 의미를 커스텀 토큰으로 바꿔도 L1에는 아무 영향이 없다.
**브릿지에서 L1 ERC-20 토큰 ↔ L2 네이티브 balance를 올바르게 매핑**하기만 하면 된다.

```rust
// crates/vm/levm/src/vm.rs:36-44
pub enum VMType {
    #[default]
    L1,                    // ← L1 노드: ETH 기반, DefaultHook만 사용
    L2(FeeConfig),         // ← L2 노드: 커스텀 토큰 기반, L2Hook 사용
}

// crates/vm/levm/src/hooks/hook.rs:19-35
pub fn get_hooks(vm_type: &VMType) -> Vec<...> {
    match vm_type {
        VMType::L1 => vec![DefaultHook],           // L1 전용
        VMType::L2(fee_config) => vec![L2Hook, BackupHook],  // L2 전용
    }
}
```

L2Hook의 `prepare_execution()`은 일반 트랜잭션에서 `DefaultHook.prepare_execution()`을 호출하지만,
이는 **L2 노드 안에서** 호출되는 것이므로 L2의 balance 의미를 따른다.
L1 노드는 자신만의 독립된 DefaultHook 인스턴스를 사용한다.

### Fee Token vs 네이티브 가스 토큰 비교

| 구분 | Fee Token (현재 구현) | 네이티브 가스 토큰 (이 문서) |
|------|----------------------|---------------------------|
| **msg.value** | ETH | 커스텀 토큰 |
| **address.balance** | ETH | 커스텀 토큰 |
| **가스비 지불** | ERC-20 lockFee/payFee 시뮬레이션 | balance 필드 직접 조작 (ETH와 동일) |
| **ZK 회로 영향** | ERC-20 시뮬레이션 → 회로 크기 증가 | **변경 없음 (ETH와 동일)** |
| **BALANCE opcode** | ETH 잔액 반환 | 커스텀 토큰 잔액 반환 |
| **브릿지 deposit** | ETH 직접 전송 | L1 ERC-20 lock → L2 balance 증가 |
| **브릿지 withdrawal** | ETH 직접 수신 | L2 balance burn → L1 ERC-20 unlock |
| **L1 영향** | 없음 | **없음** (별도 노드/상태) |
| **참고 구현** | ethrex Fee Token | OP Stack Custom Gas Token |

### ZK 회로 비용 절감 효과 (추정)

| 항목 | Fee Token 방식 | 네이티브 가스 토큰 방식 |
|------|---------------|----------------------|
| 트랜잭션당 ERC-20 시뮬레이션 | 2-6회 (lock, pay×N, refund) | 0회 |
| ERC-20 스토리지 접근 (SLOAD/SSTORE) | 트랜잭션당 4-12회 | 0회 |
| 추가 VM 인스턴스 생성 | `simulate_common_bridge_call()` 호출마다 | 0회 |
| 게스트 프로그램 실행 오버헤드 | 상당 (DB clone + VM 생성) | 없음 |

### NATIVE_TOKEN_L1 변수

`CommonBridge.sol`에 `NATIVE_TOKEN_L1` 변수가 이미 존재하지만 **deprecated** 상태이며
실제로 사용되지 않는다 (line 107). 이 변수를 부활시켜 L1 측 커스텀 토큰 주소를 지정하는 데
활용할 수 있다.

---

## 2. ETH 하드코딩 위치 전체 맵

```
ethrex/
├── crates/
│   ├── vm/levm/src/
│   │   ├── vm.rs                          ← VMType::L1 / L2 분기
│   │   ├── hooks/
│   │   │   ├── hook.rs                    ← get_hooks(): L1→DefaultHook, L2→L2Hook
│   │   │   ├── default_hook.rs            ← [L2에서도 사용] value transfer/deduct/refund/pay_coinbase
│   │   │   └── l2_hook.rs                ← [L2 전용] Fee Token 분기, privileged tx 민팅
│   │   └── opcode_handlers/
│   │       ├── environment.rs             ← BALANCE (L39), CALLVALUE (L80)
│   │       ├── block.rs                   ← SELFBALANCE (L112)
│   │       └── system.rs                 ← CALL (L25), CREATE (L499), SELFDESTRUCT (L587)
│   │
│   ├── common/types/
│   │   ├── account.rs                     ← AccountInfo.balance (L119-123)
│   │   └── l2/balance_diff.rs             ← BalanceDiff 구조체 (L16-21)
│   │
│   ├── l2/
│   │   ├── contracts/src/
│   │   │   ├── l1/CommonBridge.sol        ← [L1 컨트랙트] deposit/claimWithdrawal
│   │   │   ├── l2/CommonBridgeL2.sol      ← [L2 컨트랙트] mintETH/withdraw
│   │   │   └── l1/OnChainProposer.sol     ← [L1 컨트랙트] commitBatch/verifyBatch
│   │   ├── sequencer/l1_watcher.rs        ← L1 이벤트 감시 → privileged tx 생성
│   │   ├── common/src/messages.rs         ← get_balance_diffs(): 크로스체인 자산 집계
│   │   └── sdk/src/sdk.rs                ← deposit/withdraw API
│   │
│   └── guest-program/src/l2/
│       └── output.rs                      ← ProgramOutput: balance_diffs 인코딩
│
└── (테스트, 벤치마크 등 생략)
```

---

## 3. 레이어별 상세 분석

### 3.1 VM/EVM 레이어

#### 3.1.1 AccountInfo.balance 필드

**파일**: `crates/common/types/account.rs:119-123`

```rust
pub struct AccountInfo {
    pub code_hash: H256,
    pub balance: U256,      // L2에서: 커스텀 토큰 잔액으로 재정의
    pub nonce: u64,
}
```

**접근 방식**: **(A) balance 필드 재정의** (권장)
- L2 노드에서 `balance` 필드가 커스텀 토큰 잔액을 의미하도록 변경
- `AccountInfo` 구조체 자체는 수정 불필요 — **genesis 초기화 시 커스텀 토큰 잔액을 넣으면 됨**
- L1 노드는 자체 상태 트리에서 ETH 잔액을 계속 사용 (영향 없음)
- Opcodes (BALANCE, SELFBALANCE, CALLVALUE)도 수정 불필요 — `balance` 필드를 그대로 읽음

#### 3.1.2 Opcodes: BALANCE / SELFBALANCE / CALLVALUE

**수정 불필요**. 접근 방식 (A)에서는 `balance` 필드의 의미만 바뀌고,
opcode는 기존과 동일하게 `info.balance`를 읽어서 스택에 푸시한다.

- `op_balance()` — `environment.rs:39`: `self.db.get_account(address)?.info.balance`
- `op_selfbalance()` — `block.rs:117`: `self.db.get_account(address)?.info.balance`
- `op_callvalue()` — `environment.rs:80`: `current_call_frame.msg_value`

#### 3.1.3 Value Transfer 핵심 함수들

**파일**: `crates/vm/levm/src/hooks/default_hook.rs`

이 함수들은 L2에서도 L2Hook을 통해 호출된다 (`l2_hook.rs:60`에서 `DefaultHook.prepare_execution()` 위임).
**함수 자체는 수정 불필요** — `increase_account_balance()`/`decrease_account_balance()`가
`AccountInfo.balance`를 조작하는데, L2에서 이 필드가 커스텀 토큰 잔액이면 자연스럽게 동작한다.

| 함수 | 라인 | 역할 | 네이티브 가스 토큰에서 |
|------|------|------|---------------------|
| `validate_sender_balance()` | L490 | 발신자 잔액 검증 | 커스텀 토큰 잔액으로 검증 |
| `deduct_caller()` | L521 | 선차감 | 커스텀 토큰에서 차감 |
| `transfer_value()` | L553 | value 전송 | 커스텀 토큰 전송 |
| `undo_value_transfer()` | L155 | 실패 시 복구 | 커스텀 토큰 복구 |
| `refund_sender()` | L174 | 가스 환불 | 커스텀 토큰으로 환불 |
| `pay_coinbase()` | L240 | 수수료 지급 | 커스텀 토큰으로 지급 |

#### 3.1.4 L2 Hook: 수정이 필요한 부분

**파일**: `crates/vm/levm/src/hooks/l2_hook.rs`

**수정 필요 항목**:

1. **privileged tx의 네이티브 토큰 민팅** (L350-434):
   현재 ETH 민팅은 bridge 주소의 balance 차감을 스킵하는 방식:
   ```rust
   // l2_hook.rs:361-374
   if sender_address != COMMON_BRIDGE_L2_ADDRESS {
       // balance 차감
   }
   // bridge면 차감 스킵 → 무에서 유 (ETH 민팅)
   ```
   → 커스텀 토큰도 동일한 메커니즘 사용 가능. **bridge에서 오는 privileged tx의 value가
   커스텀 토큰 양**이 되면 됨. VM 레벨의 로직 변경 최소화.

2. **Fee Token 시뮬레이션 제거** (L439-657):
   네이티브 가스 토큰 모드에서는 `prepare_execution_fee_token()`,
   `simulate_common_bridge_call()`, `lock_fee_token()`, `pay_fee_token()` 등
   **전체 Fee Token 시뮬레이션 경로가 불필요**해짐.
   → 일반 트랜잭션은 `DefaultHook.prepare_execution()`으로 직행 (이미 그렇게 동작).

3. **finalize_non_privileged_execution에서 fee token 분기 제거** (L96-223):
   `use_fee_token` 분기 대신 항상 balance 기반 처리.

#### 3.1.5 EIP-7708 (Amsterdam) 관련

`default_hook.rs`의 transfer log 발행 (L563-566)은 그대로 동작.
다만 로그의 의미가 "ETH transfer"에서 "네이티브 토큰 transfer"로 바뀜:

```rust
let log = create_eth_transfer_log(from, to, value);
// 함수명은 eth_transfer_log이지만, 실제로는 네이티브 토큰 전송 로그
```

---

### 3.2 Bridge 레이어 (핵심 변경 영역)

브릿지가 **L1의 ERC-20 토큰 ↔ L2의 네이티브 balance**를 올바르게 매핑하는 것이 전체 설계의 핵심이다.

#### 3.2.1 전체 브릿지 흐름 비교

```
[현재: ETH 기반]

L1 → L2 (Deposit):
  사용자 → CommonBridge.deposit(){value: 1 ETH}
    → deposits[ETH_TOKEN][ETH_TOKEN] += 1 ETH
    → privileged tx: mintETH(recipient), value=1 ETH
    → L2 VM: bridge balance 차감 스킵 → recipient.balance += 1 ETH

L2 → L1 (Withdrawal):
  사용자 → CommonBridgeL2.withdraw(receiver){value: 1 ETH}
    → BURN_ADDRESS.call{value: 1 ETH} (ETH 소멸)
    → Messenger.sendMessageToL1(hash(ETH_TOKEN, ETH_TOKEN, receiver, 1 ETH))
    → L1: CommonBridge.claimWithdrawal() → receiver.call{value: 1 ETH}

──────────────────────────────────────────────────────

[변경 후: 커스텀 토큰 기반]

L1 → L2 (Deposit):
  사용자 → CommonBridge.depositNativeToken(l2Recipient, amount)
    → IERC20(NATIVE_TOKEN_L1).safeTransferFrom(sender, bridge, amount)
    → deposits[NATIVE_TOKEN_L1][NATIVE_TOKEN_L1] += amount
    → privileged tx: mintNativeToken(recipient), value=amount
    → L2 VM: bridge balance 차감 스킵 → recipient.balance += amount

L2 → L1 (Withdrawal):
  사용자 → CommonBridgeL2.withdrawNativeToken(receiver){value: amount}
    → BURN_ADDRESS.call{value: amount} (네이티브 토큰 소멸)
    → Messenger.sendMessageToL1(hash(NATIVE_TOKEN_L1, NATIVE_TOKEN_L1, receiver, amount))
    → L1: CommonBridge.claimNativeTokenWithdrawal()
         → IERC20(NATIVE_TOKEN_L1).safeTransfer(receiver, amount)
```

#### 3.2.2 L1 CommonBridge 변경 상세

**파일**: `crates/l2/contracts/src/l1/CommonBridge.sol`

| 현재 함수 | 변경 방향 | 설명 |
|-----------|----------|------|
| `deposit()` (L273) | `depositNativeToken()` | `msg.value` 대신 `IERC20.transferFrom()` |
| `receive()` (L290) | 제거 또는 ETH→WETH 래핑 | ETH 직접 수신 불필요 |
| `claimWithdrawal()` (L533) | `claimNativeTokenWithdrawal()` | `call{value}` 대신 `IERC20.safeTransfer()` |
| `publishL2Messages()` (L461) | ERC-20 전송으로 변경 | `sendETHValue` 대신 `sendERC20Message` |
| `NATIVE_TOKEN_L1` (L107) | **부활**: 커스텀 토큰 L1 주소 | deprecated 해제 |
| `deposits` 매핑 (L79) | `deposits[nativeToken][nativeToken]` | ETH_TOKEN 대신 커스텀 토큰 주소 |

**deposit 변경 예시**:
```solidity
// 현재
function deposit(address l2Recipient) public payable {
    deposits[ETH_TOKEN][ETH_TOKEN] += msg.value;
    bytes memory callData = abi.encodeCall(ICommonBridgeL2.mintETH, (l2Recipient));
    SendValues memory sv = SendValues({to: L2_BRIDGE, gasLimit: 105000, value: msg.value, data: callData});
    _sendToL2(L2_BRIDGE, sv);
}

// 변경 후
function depositNativeToken(address l2Recipient, uint256 amount) public {
    IERC20(NATIVE_TOKEN_L1).safeTransferFrom(msg.sender, address(this), amount);
    deposits[NATIVE_TOKEN_L1][NATIVE_TOKEN_L1] += amount;
    bytes memory callData = abi.encodeCall(ICommonBridgeL2.mintNativeToken, (l2Recipient));
    SendValues memory sv = SendValues({to: L2_BRIDGE, gasLimit: 105000, value: amount, data: callData});
    _sendToL2(L2_BRIDGE, sv);
}
```

#### 3.2.3 L2 CommonBridgeL2 변경 상세

**파일**: `crates/l2/contracts/src/l2/CommonBridgeL2.sol`

| 현재 함수 | 변경 방향 | 설명 |
|-----------|----------|------|
| `mintETH()` (L45) | `mintNativeToken()` | 동일 메커니즘 (privileged tx value 전달) |
| `withdraw()` (L30) | `withdrawNativeToken()` | 동일 메커니즘 (burn address로 전송) |
| `sendToL2()` (L143) | 네이티브 토큰 burn | `msg.value`가 커스텀 토큰 양 |
| `ETH_TOKEN` 상수 (L17) | `NATIVE_TOKEN` 상수로 변경 | withdrawal 메시지 인코딩용 |

**mintNativeToken 변경 예시**:
```solidity
// 현재: mintETH — privileged tx의 value로 ETH 민팅
function mintETH(address to) external payable onlySelf {
    (bool success, ) = to.call{value: msg.value}("");
    if (!success) { this.withdraw{value: msg.value}(to); }
}

// 변경 후: mintNativeToken — 동일 메커니즘, 다만 "ETH"가 아닌 커스텀 토큰
// VM의 prepare_execution_privileged()가 bridge balance 차감을 스킵하므로
// value만큼의 네이티브 토큰이 무에서 생성됨 (기존 ETH 민팅과 동일)
function mintNativeToken(address to) external payable onlySelf {
    (bool success, ) = to.call{value: msg.value}("");
    if (!success) { this.withdrawNativeToken{value: msg.value}(to); }
}
```

**withdraw 변경 예시**:
```solidity
// 현재: withdraw — ETH를 burn address로 전송
function withdraw(address _receiverOnL1) external payable {
    (bool success, ) = BURN_ADDRESS.call{value: msg.value}("");
    require(success, "Failed to burn");
    IMessenger(L1_MESSENGER).sendMessageToL1(
        keccak256(abi.encodePacked(ETH_TOKEN, ETH_TOKEN, _receiverOnL1, msg.value))
    );
}

// 변경 후: withdrawNativeToken — 동일 메커니즘
function withdrawNativeToken(address _receiverOnL1) external payable {
    (bool success, ) = BURN_ADDRESS.call{value: msg.value}("");
    require(success, "Failed to burn native token");
    IMessenger(L1_MESSENGER).sendMessageToL1(
        keccak256(abi.encodePacked(NATIVE_TOKEN_L1, NATIVE_TOKEN_L1, _receiverOnL1, msg.value))
    );
}
```

> **핵심 인사이트**: L2 측 컨트랙트 (`mintNativeToken`, `withdrawNativeToken`)의 실제 로직은
> 현재 `mintETH`, `withdraw`와 **거의 동일**하다. `msg.value`와 `address.balance`의 의미만
> "ETH"에서 "커스텀 토큰"으로 바뀌기 때문. L1 측 변경이 더 크다.

#### 3.2.4 L1 Watcher 변경

**파일**: `crates/l2/sequencer/l1_watcher.rs`

L1 Watcher는 `PrivilegedTxSent` 이벤트를 감시하여 L2 privileged tx를 생성한다 (L251-317).
현재 흐름:

```
L1 PrivilegedTxSent(from, to, txId, value, gasLimit, data)
  → PrivilegedTransactionData 파싱
  → PrivilegedL2Transaction 생성 (nonce=txId)
  → L2 mempool에 추가
```

**변경사항**: 구조적으로 변경 불필요.
- `PrivilegedTxSent` 이벤트의 `value` 필드에 커스텀 토큰 양이 들어감
- L2에서 이 value가 `msg_value`로 설정되어 `mintNativeToken()`으로 전달됨
- VM의 `prepare_execution_privileged()`가 bridge balance 차감을 스킵 → 민팅 완료

#### 3.2.5 Cross-Chain (L2 → L2) 흐름

**현재** (`CommonBridgeL2.sendToL2()`, L143-174):
```
msg.value > 0 이면:
  → Messenger에 mintETH 메시지 전송
  → BURN_ADDRESS로 ETH burn
```

**변경 후**: 동일 메커니즘. `msg.value`가 커스텀 토큰 양이 되고,
목적지 L2에서는 `mintNativeToken()` 또는 해당 체인의 민팅 함수가 호출됨.

단, 목적지 L2가 **다른 네이티브 토큰**을 사용하는 경우,
`BalanceDiff.value_per_token`의 `AssetDiff`를 통해 ERC-20 레벨에서 처리해야 함.

#### 3.2.6 ETH 처리: WETH 필요성

네이티브 토큰이 커스텀 토큰이 되면, L2에서 **ETH를 직접 다룰 수 없다**.
ETH를 L2에서 사용하려면:

1. L1에서 ETH deposit → L2에서 **WETH (Wrapped ETH) ERC-20**으로 수신
2. 기존 `depositERC20()` 경로 활용 가능
3. L2에 WETH 시스템 컨트랙트 배포 필요

이는 OP Stack의 Custom Gas Token 구현과 동일한 패턴이다.

#### 3.2.7 SDK 변경

**파일**: `crates/l2/sdk/src/sdk.rs`

- `deposit()` → `depositNativeToken()`: ETH 전송 대신 ERC-20 approve + transferFrom
- `withdraw()` → `withdrawNativeToken()`: L2에서 네이티브 토큰 burn
- 새 함수: `depositETH()` → WETH 경로 (depositERC20 활용)

---

### 3.4 Decimal 처리: 18이 아닌 토큰 지원

#### 문제

USDT(6 decimals), USDC(6 decimals), WBTC(8 decimals) 등 주요 토큰 중 상당수가
**18 decimals가 아니다**. 이 토큰들을 L2 네이티브 가스 토큰으로 사용하려면
decimal 차이를 처리해야 한다.

**핵심 충돌**: EVM의 가스 시스템은 `wei` (1e-18) 기반으로 설계되어 있다:
- `gas_price`는 wei 단위 (예: 1 gwei = 1e9 wei)
- `tx.value`는 wei 단위
- `address.balance`는 wei 단위
- `base_fee`는 wei 단위

만약 네이티브 토큰이 USDT(6 decimals)라면:
- `1 USDT = 1e6` 최소 단위
- `1e18`을 사용할 수 없음 → 가스 가격의 정밀도가 1e12배 줄어듦
- `gas_price = 1` wei가 `1e-6 USDT`가 되어 가스비가 과도하게 비쌈

#### 접근 방식 비교

| 옵션 | 설명 | 장점 | 단점 |
|------|------|------|------|
| **(a) 18 decimals만 지원** | 네이티브 토큰 후보를 18 decimals로 제한 | 구현 간단, EVM 완전 호환 | USDT/USDC/WBTC 등 사용 불가 |
| **(b) 브릿지 스케일링** | L1↔L2 브릿지에서 decimal을 18로 스케일 업/다운 | EVM 내부는 항상 18 decimals로 동작, 호환성 유지 | L1과 L2의 토큰 양 표현이 다름, 사용자 혼동 가능 |
| **(c) 가변 decimal** | EVM 가스 시스템 자체를 커스텀 decimal에 맞게 수정 | 완전한 유연성 | 매우 복잡, 모든 tooling과 충돌, 프루버 변경 필요 |

#### 권장: (b) 브릿지 스케일링

**원칙**: L2 VM 내부는 항상 18 decimals로 동작한다.
브릿지가 deposit/withdrawal 시 decimal 변환을 수행한다.

```
예시: USDT (L1: 6 decimals) → L2 네이티브 토큰 (내부: 18 decimals)

Deposit:
  L1에서 100 USDT = 100 × 1e6 = 1e8 (L1 최소단위)
  브릿지 스케일링: 1e8 × 1e12 = 1e20 (L2 내부 단위)
  L2에서 address.balance = 1e20 (= 100 USDT를 18 decimals로 표현)

Withdrawal:
  L2에서 balance = 1e20 (L2 내부 단위)
  브릿지 스케일링: 1e20 / 1e12 = 1e8 (L1 최소단위)
  L1에서 100 USDT 반환

스케일 팩터 = 10^(18 - L1_decimals) = 10^(18-6) = 10^12
```

**구현 위치**:

```solidity
// CommonBridge.sol (L1)
uint256 public immutable NATIVE_TOKEN_SCALE_FACTOR; // = 10^(18 - L1_decimals)

function depositNativeToken(address l2Recipient, uint256 l1Amount) public {
    IERC20(NATIVE_TOKEN_L1).safeTransferFrom(msg.sender, address(this), l1Amount);
    uint256 l2Amount = l1Amount * NATIVE_TOKEN_SCALE_FACTOR;  // 스케일 업
    deposits[NATIVE_TOKEN_L1][NATIVE_TOKEN_L1] += l1Amount;   // L1 단위로 기록
    // privileged tx의 value = l2Amount (L2 내부 단위)
    SendValues memory sv = SendValues({
        to: L2_BRIDGE, gasLimit: 105000, value: l2Amount, data: callData
    });
    _sendToL2(L2_BRIDGE, sv);
}

function claimNativeTokenWithdrawal(..., uint256 l2Amount, ...) public {
    uint256 l1Amount = l2Amount / NATIVE_TOKEN_SCALE_FACTOR;  // 스케일 다운
    IERC20(NATIVE_TOKEN_L1).safeTransfer(receiver, l1Amount);
}
```

**L2 측은 변경 불필요**: L2 VM은 항상 18 decimals 단위로 동작하므로
`AccountInfo.balance`, `msg.value`, `gas_price` 등 모든 것이 기존 EVM과 동일하게 동작한다.

**주의사항**:
1. **스케일 다운 시 정밀도 손실**: `l2Amount % SCALE_FACTOR != 0`이면 소수점 이하 절삭.
   → withdrawal 시 `require(l2Amount % SCALE_FACTOR == 0)` 검증 필요, 또는 dust 허용 정책 결정.
2. **UI 표시**: L2 explorer/wallet에서 잔액 표시 시 SCALE_FACTOR를 나눠서 보여줘야 사용자 혼동 방지.
3. **BalanceDiff**: `value` 필드에 L2 내부 단위(18 decimals) vs L1 단위 중 어떤 것을 사용할지 결정 필요.
   → **L1 단위 권장** (OnChainProposer가 L1에서 실행되므로).
4. **오버플로우**: `l1Amount * SCALE_FACTOR`가 uint256 범위를 넘지 않는지 확인
   (USDT 총 공급량 ~1e17, × 1e12 = 1e29 → uint256 범위 내).

#### Genesis 설정 변경

```toml
# genesis 설정에 추가 필요
[native_token]
l1_address = "0x..."          # L1 ERC-20 주소
l1_decimals = 6               # L1 토큰의 decimals
# scale_factor는 10^(18 - l1_decimals)로 자동 계산
```

---

### 3.3 Prover 레이어

#### 3.3.1 BalanceDiff 구조체

**파일**: `crates/common/types/l2/balance_diff.rs:6-21`

```rust
pub struct BalanceDiff {
    pub chain_id: U256,
    pub value: U256,                        // ← 현재: ETH, 변경 후: 커스텀 토큰
    pub value_per_token: Vec<AssetDiff>,    // ← ERC-20 전송 (ETH→WETH 포함)
    pub message_hashes: Vec<H256>,
}
```

**변경 방향**: `value` 필드가 커스텀 네이티브 토큰의 크로스체인 전송량을 나타내도록 변경.
ETH가 WETH로 브릿지되는 경우 `value_per_token`의 AssetDiff로 처리.

#### 3.3.2 get_balance_diffs()

**파일**: `crates/l2/common/src/messages.rs:169+`

현재:
- `mintETH` selector → `value` 필드에 합산
- `crosschainMintERC20` selector → `value_per_token`에 AssetDiff 추가

변경 후:
- `mintNativeToken` selector → `value` 필드에 합산
- `crosschainMintERC20` selector → `value_per_token`에 AssetDiff 추가 (ETH=WETH 포함)

#### 3.3.3 ProgramOutput ↔ OnChainProposer 동기화

**파일**: `crates/guest-program/src/l2/output.rs` / `OnChainProposer.sol`

인코딩 형식 자체는 변경 불필요할 수 있음.
`value` 필드의 의미만 바뀌고, 바이트 레이아웃은 동일하기 때문.
다만 **함수 selector가 변경**되면 (`mintETH` → `mintNativeToken`),
`get_balance_diffs()`의 selector 매칭 로직을 업데이트해야 함.

---

## 4. 개발 공수 추정

### Fee Token 시뮬레이션의 ZK 비용 상세

현재 `l2_hook.rs`에서 Fee Token 처리 시 호출되는 `simulate_common_bridge_call()` (L661-703):

```rust
fn simulate_common_bridge_call(vm: &VM<'_>, to: Address, data: Bytes)
    -> Result<(ExecutionReport, GeneralizedDatabase), VMError>
{
    let mut db_clone = vm.db.clone();  // ← 전체 DB 클론 (ZK 회로에서도 실행)
    let mut new_vm = VM::new(env_clone, &mut db_clone, &tx, ...)?;
    new_vm.execute()?;  // ← ERC-20 컨트랙트 실행 (SLOAD/SSTORE 다수)
}
```

이 시뮬레이션이 **트랜잭션당 최소 3회** 호출됨 (lockFee 1회 + payFee/refund 2회+).
**네이티브 가스 토큰 방식에서는 완전히 제거**된다.

### 컴포넌트별 공수

| 컴포넌트 | 복잡도 | 추정 기간 | 세부 사항 |
|----------|--------|-----------|-----------|
| **VM (L2Hook 수정)** | 중간 | 2-3주 | Fee Token 시뮬레이션 경로 비활성화, privileged tx 민팅 함수명 변경 |
| **VM (DefaultHook)** | **없음** | 0주 | **수정 불필요** — balance 필드 의미만 바뀌면 자동 적용 |
| **VM (Opcodes)** | **없음** | 0주 | **수정 불필요** — info.balance 그대로 읽음 |
| **L1 CommonBridge** | 높음 | 3-4주 | deposit/claim을 ERC-20 패턴으로, NATIVE_TOKEN_L1 부활 |
| **L2 CommonBridgeL2** | 낮음 | 1-2주 | 함수명 변경 + ETH_TOKEN→NATIVE_TOKEN 상수 변경 (로직 동일) |
| **L1 Watcher** | 낮음 | 0.5-1주 | 구조 변경 없음, selector/상수만 변경 |
| **BalanceDiff / Messages** | 낮음 | 1주 | selector 매칭 변경, value 의미 변경 |
| **ProgramOutput / Guest** | 낮음 | 1주 | 인코딩 자체는 동일 가능, selector만 변경 |
| **OnChainProposer** | 낮음 | 0.5-1주 | balanceDiffs 처리에서 네이티브 토큰 반영 |
| **SDK** | 낮음 | 1주 | deposit/withdraw API 변경 |
| **Decimal 스케일링** | 중간 | 1-2주 | 브릿지 deposit/withdrawal 시 L1↔L2 decimal 변환, 정밀도 검증 |
| **Genesis 설정** | 낮음 | 0.5-1주 | 네이티브 토큰 주소, decimals, scale factor, 초기 잔액 배분 |
| **WETH 컨트랙트** | 중간 | 1-2주 | L2 WETH 시스템 컨트랙트 (ETH 브릿지용) |
| **테스트 & 통합** | 높음 | 4-6주 | E2E, 프루버, 브릿지 회계 검증 |
| **합계** | | **~16-25주** | |

> 기존 추정(29-42주)보다 크게 줄어든 이유: `AccountInfo.balance` 재정의 방식은
> VM 코어와 opcode 변경이 불필요하고, L2 컨트랙트 로직도 대부분 동일하기 때문.

---

## 5. 리스크 분석

### 5.1 EVM 호환성 리스크 (중간)

- **L2에 배포되는 컨트랙트 영향**: `address(this).balance`가 커스텀 토큰 잔액 반환.
  ETH 잔액을 기대하는 컨트랙트(WETH, DEX 등)는 L2 전용 버전 필요.
- **msg.value 의미**: L2에서 `call{value: X}`는 커스텀 토큰 전송.
- **L1에는 영향 없음**: 별도 노드, 별도 상태 트리.
- **완화책**: WETH 컨트랙트 제공, 개발자 문서화.

### 5.2 프루버 일관성 리스크 (중간)

- `ProgramOutput.encode()`와 `_getPublicInputsFromCommitment()`의 동기화 필수.
- 인코딩 바이트 레이아웃이 변경되지 않으면 VK 재생성 불필요할 수 있음.
- 함수 selector만 변경되면 게스트 프로그램 리컴파일 + VK 재생성 필요.

### 5.3 브릿지 보안 리스크 (높음)

- **회계 일관성**: `deposits[NATIVE_TOKEN_L1][NATIVE_TOKEN_L1]`가 L1에 lock된 실제 ERC-20 양과 일치해야 함.
- **민팅 권한**: 기존 ETH 민팅과 동일 메커니즘 (bridge balance 차감 스킵) → 보안 모델 유지.
- **출금 검증**: withdrawal merkle proof 로직은 토큰 주소만 바뀌고 구조는 동일.
- **완화책**: 보안 감사, deposit/withdrawal 양 일치 invariant 테스트.

### 5.4 Fee Token 시스템과의 관계

- 네이티브 가스 토큰 모드에서는 **Fee Token 시뮬레이션이 불필요**.
- `l2_hook.rs`의 `fee_token` 분기를 비활성화하면 ZK 회로 비용 절감 달성.
- 추가 Fee Token 지원이 필요하면 선택적으로 유지 가능 (단, 해당 토큰 사용 시 ZK 비용 증가).

### 5.5 Decimal 스케일링 리스크 (중간)

- **정밀도 손실**: L2→L1 withdrawal 시 `l2Amount / SCALE_FACTOR`에서 나머지가 발생할 수 있음.
  예: USDT(6 decimals), scale_factor=1e12일 때, L2에서 1.5e11 단위를 출금하면 L1에서 0 USDT.
  → `require(l2Amount % SCALE_FACTOR == 0)` 또는 최소 출금량 강제 필요.
- **사용자 혼동**: L2에서 `address.balance = 1e20`이지만 실제로는 `100 USDT`.
  wallet/explorer에서 올바른 표시를 위해 decimal 정보 제공 필요.
- **BalanceDiff 단위**: L1 OnChainProposer에서 검증할 때 L1 단위와 L2 단위 혼동 가능.
  → 일관된 단위 사용 정책 필수 (L1 단위 권장).
- **완화책**: 배포 시 `SCALE_FACTOR` immutable로 고정, overflow 체크, 최소 출금량 설정.

### 5.6 Cross-Chain 호환성 (중간)

- 다른 L2가 ETH를 네이티브로 사용하는 경우, 크로스체인 전송 시 `AssetDiff`를 통해 ERC-20 레벨로 처리.
- 양쪽 L2가 다른 네이티브 토큰을 사용하는 경우, `BalanceDiff.value`의 의미가 체인마다 다를 수 있음 → 주의 필요.

---

## 6. OP Stack 비교

| 항목 | OP Stack Custom Gas Token | ethrex 구현 방향 |
|------|--------------------------|-----------------|
| **네이티브 통화** | 커스텀 ERC-20 | 커스텀 ERC-20 |
| **ETH 처리** | WETH 컨트랙트 | WETH 컨트랙트 |
| **msg.value** | 커스텀 토큰 단위 | 커스텀 토큰 단위 |
| **L1 deposit** | ERC-20 lock (transferFrom) | ERC-20 lock (transferFrom) |
| **L2 민팅** | L2StandardBridge | CommonBridgeL2 (privileged tx) |
| **L1 withdrawal** | ERC-20 unlock (transfer) | ERC-20 unlock (safeTransfer) |
| **L1 영향** | 없음 (별도 체인) | **없음** (별도 노드) |
| **프루버 영향** | 없음 (Fault Proof) | 있음 (ZK Proof) — 하지만 이점이 큼 |
| **18 decimal 제약** | 있음 (18 고정) | **없음** (브릿지 스케일링으로 6/8 등 지원) |

### ethrex의 차별점

1. **ZK 비용 절감이 핵심 동기**: OP Stack은 단순히 다른 토큰을 네이티브로 쓰고 싶은 것.
   ethrex는 Fee Token 시뮬레이션 제거로 **회로 크기를 줄이는 것**이 목적.
2. **VM 변경 최소화**: L2의 `balance` 필드 의미만 바꾸면 opcode/hook 대부분 무변경.
3. **브릿지가 핵심 변경점**: L1 ERC-20 ↔ L2 balance 매핑이 가장 중요.
4. **임의 decimal 지원**: OP Stack은 18 decimals만 허용하지만, ethrex는 브릿지 스케일링으로
   USDT(6), USDC(6), WBTC(8) 등 **어떤 decimal의 토큰이든** 네이티브 가스 토큰으로 사용 가능.

---

## 7. 권장 접근 방식: 단계별 로드맵

### Phase 1: 브릿지 + Genesis (5-7주)

**목표**: L1 ERC-20 ↔ L2 네이티브 balance 매핑 구현 (decimal 스케일링 포함)

- [ ] `CommonBridge.sol`: `NATIVE_TOKEN_L1` 부활, `NATIVE_TOKEN_SCALE_FACTOR` 추가
- [ ] `CommonBridge.sol`: `depositNativeToken()` — L1 amount × scale_factor → L2 value
- [ ] `CommonBridge.sol`: `claimNativeTokenWithdrawal()` — L2 value ÷ scale_factor → L1 amount
- [ ] `CommonBridgeL2.sol`: `mintNativeToken()`, `withdrawNativeToken()` 구현 (기존 로직과 거의 동일)
- [ ] Genesis 설정: 네이티브 토큰 L1 주소, decimals, scale factor, 초기 balance 배분
- [ ] L1 Watcher: selector/상수 업데이트
- [ ] WETH 시스템 컨트랙트 배포 (ETH를 L2에서 ERC-20으로 사용)
- [ ] Decimal 정밀도 테스트: 스케일 업/다운 라운드트립, dust 처리 정책

### Phase 2: L2Hook + Fee Token 정리 (2-3주)

**목표**: Fee Token 시뮬레이션 제거로 ZK 비용 절감

- [ ] `l2_hook.rs`: 네이티브 가스 토큰 모드에서 fee_token 분기 비활성화
- [ ] 일반 트랜잭션: `DefaultHook.prepare_execution()` 직행 (balance = 커스텀 토큰)
- [ ] privileged tx: 기존 민팅 메커니즘 유지 (함수명/selector만 변경)
- [ ] finalize에서 fee token 관련 코드 정리

### Phase 3: 프루버 동기화 (1-2주)

**목표**: 게스트 프로그램 ↔ L1 온체인 검증 일관성

- [ ] `get_balance_diffs()`: 새 selector 매칭
- [ ] `ProgramOutput.encode()` / `_getPublicInputsFromCommitment()` 동기화 확인
- [ ] VK 재생성 (필요시)

### Phase 4: SDK + 테스트 (4-6주)

**목표**: 전체 시스템 E2E 검증

- [ ] SDK: deposit/withdraw API 업데이트
- [ ] E2E 테스트: deposit → L2 사용 → withdraw 전체 흐름
- [ ] ZK 프로파일링: Fee Token 시뮬레이션 제거 효과 측정
- [ ] 브릿지 회계 invariant 테스트

---

## 8. 핵심 설계 결정 사항 (TBD)

| # | 결정 사항 | 옵션 | 권장 |
|---|-----------|------|------|
| 1 | balance 필드 접근 방식 | (A) balance 재정의 vs (B) 가상 잔액 레이어 | **(A)** — VM 변경 최소화, ZK 최적 |
| 2 | ETH 처리 방식 | WETH 시스템 컨트랙트 vs ETH 완전 제거 | **WETH** — OP Stack 검증된 패턴 |
| 3 | Fee Token 시스템 | 완전 제거 vs 선택적 유지 | **우선 비활성화** — 향후 선택적 활성화 가능 |
| 4 | 네이티브 토큰 decimal | (a) 18 고정 vs (b) 브릿지 스케일링 vs (c) 가변 decimal | **(b) 브릿지 스케일링** — 섹션 3.4 참조 |
| 5 | 배포 방식 | genesis부터 시작 | **genesis** — 마이그레이션 불필요 |
| 6 | 네이티브 토큰 L1 주소 | `NATIVE_TOKEN_L1` 부활 vs 새 변수 | **부활** — 이미 존재하는 인프라 활용 |
| 7 | 기존 ETH deposit 유지 | ETH deposit → WETH vs 제거 | **WETH 경로 유지** — 사용자 편의 |

---

## 참고 자료

- [Fee Token 설정 가이드](./l2-fee-token-setup-guide.md)
- [Fee Token 공식 문서](../docs/l2/fundamentals/fee_token.md)
- [OP Stack Custom Gas Token](https://docs.optimism.io/builders/chain-operators/features/custom-gas-token)
- VM 핵심 파일: `crates/vm/levm/src/hooks/default_hook.rs`, `l2_hook.rs`
- VM 타입 분기: `crates/vm/levm/src/vm.rs` (VMType), `hooks/hook.rs` (get_hooks)
- 브릿지 컨트랙트: `crates/l2/contracts/src/l1/CommonBridge.sol`, `l2/CommonBridgeL2.sol`
- L1 Watcher: `crates/l2/sequencer/l1_watcher.rs`
- 프루버 출력: `crates/guest-program/src/l2/output.rs`
- Fee Token ZK 비용 핵심: `l2_hook.rs:661-703` (`simulate_common_bridge_call`)
