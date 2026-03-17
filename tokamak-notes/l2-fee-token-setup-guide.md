# L2 Fee Token 설정 가이드: ERC20 토큰으로 가스비 지불하기

## 개요

ethrex L2에서는 ETH가 항상 네이티브 토큰(msg.value)으로 사용된다.
하지만 **Fee Token** 시스템을 통해 특정 ERC-20 토큰으로 가스비를 지불할 수 있다.

이는 OP Stack의 "Custom Gas Token"과는 다른 접근 방식이다:
- OP Stack: 네이티브 통화(msg.value) 자체를 ERC-20으로 교체
- ethrex: ETH는 네이티브 통화로 유지, **별도 트랜잭션 타입**(`FeeTokenTransaction`, type `0x7d`)으로 ERC-20 가스비 지불

> 예를 들어, TON 토큰을 Fee Token으로 등록하면 사용자는 ETH 없이도
> TON만으로 L2 트랜잭션 가스비를 지불할 수 있다.

---

## 아키텍처

```
L1 CommonBridge                          L2 System Contracts
┌──────────────┐                        ┌─────────────────────┐
│ registerNew  │  privileged tx         │ FeeTokenRegistry    │
│ FeeToken()   │ ──────────────────────>│ (0x...fffc)         │
│              │                        │  - isFeeToken()     │
│ setFeeToken  │  privileged tx         ├─────────────────────┤
│ Ratio()      │ ──────────────────────>│ FeeTokenPricer      │
│              │                        │ (0x...fffb)         │
└──────────────┘                        │  - getFeeTokenRatio()│
                                        ├─────────────────────┤
                                        │ CommonBridgeL2      │
                                        │ (0x...ffff)         │
                                        │  - lockFee()        │
                                        │  - payFee()         │
                                        └─────────────────────┘
```

**트랜잭션 흐름:**
1. 사용자가 `FeeTokenTransaction` (type `0x7d`) 전송
2. L2 VM Hook이 `FeeTokenRegistry`에서 토큰 등록 여부 확인
3. `FeeTokenPricer`에서 토큰/ETH 비율 조회
4. `lockFee()`로 가스비 선불 잠금
5. 트랜잭션 실행
6. `payFee()`로 실제 가스비 분배 (coinbase, base fee vault, operator vault 등)
7. 남은 가스비 환불

---

## 전제 조건

- 로컬 환경이 구축되어 있어야 한다 (`local-setup-guide.md` 참조)
- L1 Docker + L2 시퀀서가 실행 중이어야 한다
- Foundry (forge)가 설치되어 있어야 한다

---

## Step 1: Fee Token 컨트랙트 작성

Fee Token은 다음 인터페이스를 구현해야 한다:

### 필수 인터페이스

```solidity
// IFeeToken = IERC20 + IERC20L2 + lockFee + payFee
interface IFeeToken is IERC20, IERC20L2 {
    function lockFee(address payer, uint256 amount) external;
    function payFee(address receiver, uint256 amount) external;
}

// IERC20L2 = IERC20 + crosschain mint/burn
interface IERC20L2 is IERC20 {
    function l1Address() external returns (address);
    function crosschainMint(address to, uint256 amount) external;
    function crosschainBurn(address from, uint256 amount) external;
}
```

### 함수별 역할

| 함수 | 호출자 | 설명 |
|------|--------|------|
| `lockFee(payer, amount)` | Bridge (`0x...ffff`) | 트랜잭션 시작 시 가스비를 payer → bridge로 전송 (선불 잠금) |
| `payFee(receiver, amount)` | Bridge (`0x...ffff`) | 트랜잭션 완료 후 가스비를 bridge → receiver로 분배. receiver가 `address(0)`이면 burn |
| `crosschainMint(to, amount)` | Bridge (`0x...ffff`) | L1→L2 브릿지 시 토큰 민팅 |
| `crosschainBurn(from, amount)` | Bridge (`0x...ffff`) | L2→L1 출금 시 토큰 소각 |
| `l1Address()` | 누구나 | 대응하는 L1 토큰 주소 반환 |

### 예제 컨트랙트

ethrex에 포함된 레퍼런스 구현 (`crates/l2/contracts/src/example/FeeToken.sol`):

```solidity
// SPDX-License-Identifier: MIT
pragma solidity =0.8.31;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import "../l2/interfaces/IFeeToken.sol";

contract FeeToken is ERC20, IFeeToken {
    uint256 public constant DEFAULT_MINT = 1_000_000 * (10 ** 18);
    address public immutable L1_TOKEN;
    address public constant BRIDGE = 0x000000000000000000000000000000000000FFff;

    modifier onlyBridge() {
        require(msg.sender == BRIDGE, "FeeToken: not authorized");
        _;
    }

    constructor(address l1Token) ERC20("FeeToken", "FEE") {
        L1_TOKEN = l1Token;
        _mint(msg.sender, DEFAULT_MINT);
    }

    function freeMint() public {
        _mint(msg.sender, DEFAULT_MINT);
    }

    function l1Address() external view override(IERC20L2) returns (address) {
        return L1_TOKEN;
    }

    function crosschainMint(address destination, uint256 amount)
        external override(IERC20L2) onlyBridge {
        _mint(destination, amount);
    }

    function crosschainBurn(address from, uint256 value)
        external override(IERC20L2) onlyBridge {
        _burn(from, value);
    }

    function lockFee(address payer, uint256 amount)
        external override(IFeeToken) onlyBridge {
        _transfer(payer, BRIDGE, amount);
    }

    function payFee(address receiver, uint256 amount)
        external override(IFeeToken) onlyBridge {
        if (receiver == address(0)) {
            _burn(BRIDGE, amount);
        } else {
            _transfer(BRIDGE, receiver, amount);
        }
    }
}
```

> **기존 ERC-20 토큰을 Fee Token으로 사용하려면** 위의 `IFeeToken` 인터페이스를
> 추가로 구현하거나, 래퍼 컨트랙트를 만들어야 한다.

---

## Step 2: L2에 Fee Token 컨트랙트 배포

Fee Token 컨트랙트를 L2에 배포한다.

```bash
# L2 RPC: http://localhost:1729
# 배포자 개인키: .env 파일에서 확인

cd crates/l2

# Foundry로 L2에 배포 (예시)
# L1_TOKEN_ADDRESS는 L1에 있는 대응 토큰 주소 (없으면 address(0))
forge create \
  --rpc-url http://localhost:1729 \
  --private-key <DEPLOYER_PRIVATE_KEY> \
  contracts/src/example/FeeToken.sol:FeeToken \
  --constructor-args <L1_TOKEN_ADDRESS>
```

배포된 Fee Token 컨트랙트 주소를 기록해둔다 (이후 등록에 필요).

---

## Step 3: Fee Token 등록 (L1 CommonBridge에서)

Fee Token 등록은 **L1 CommonBridge의 owner만** 할 수 있으며,
privileged transaction을 통해 L2 `FeeTokenRegistry`에 전파된다.

### 방법 A: 배포 시 자동 등록 (환경변수)

L1 컨트랙트 배포 단계에서 Fee Token을 자동으로 등록할 수 있다:

```bash
# deploy 시 Fee Token 자동 등록
ETHREX_DEPLOYER_INITIAL_FEE_TOKEN=<L2_FEE_TOKEN_ADDRESS> make deploy-l1

# SP1 백엔드의 경우
ETHREX_DEPLOYER_INITIAL_FEE_TOKEN=<L2_FEE_TOKEN_ADDRESS> make deploy-l1-sp1
```

> 이 방법은 L1 컨트랙트 배포와 동시에 등록되므로 가장 간편하다.
> 단, L1을 처음 배포할 때만 사용 가능하다.

### 방법 B: 배포 후 수동 등록 (cast 사용)

이미 L1이 배포된 후에 Fee Token을 추가로 등록하려면:

```bash
# L1 CommonBridge에서 registerNewFeeToken 호출
# BRIDGE_ADDRESS: cmd/.env에서 ETHREX_L2_COMMON_BRIDGE 확인
# OWNER_PRIVATE_KEY: L1 CommonBridge owner의 개인키

cast send <BRIDGE_ADDRESS> \
  "registerNewFeeToken(address)" \
  <L2_FEE_TOKEN_ADDRESS> \
  --rpc-url http://localhost:8545 \
  --private-key <OWNER_PRIVATE_KEY>
```

등록이 성공하면 L2 시퀀서가 privileged transaction을 가져와
`FeeTokenRegistry`에 토큰을 등록한다.

---

## Step 4: Fee Token 비율(Ratio) 설정

Fee Token으로 가스비를 계산하려면 **토큰/ETH 가격 비율**이 설정되어야 한다.
이것도 L1 CommonBridge를 통해 설정한다.

```bash
# ratio = 토큰 1개당 ETH 가치 (예: 1 TON = 0.001 ETH → ratio = 1000000000000000 = 1e15)
# ratio의 단위는 wei 기준이다.
# 예시: 1 FEE = 1 ETH 이면 ratio = 1000000000000000000 (1e18)

cast send <BRIDGE_ADDRESS> \
  "setFeeTokenRatio(address,uint256)" \
  <L2_FEE_TOKEN_ADDRESS> \
  <RATIO> \
  --rpc-url http://localhost:8545 \
  --private-key <OWNER_PRIVATE_KEY>
```

---

## Step 5: Fee Token 트랜잭션 전송

### Rust SDK 사용

```rust
use ethrex_l2_sdk::{build_generic_tx, send_generic_transaction, wait_for_transaction_receipt};
use ethrex_rpc::clients::eth::EthClient;
use ethrex_rpc::clients::Overrides;
use ethrex_common::types::TxType;
use ethrex_common::{Address, Bytes, U256};

// Fee Token 트랜잭션 빌드
let fee_token: Address = "<L2_FEE_TOKEN_ADDRESS>".parse()?;
let recipient: Address = "<RECIPIENT>".parse()?;

let tx = build_generic_tx(
    &l2_client,
    TxType::FeeToken,      // type 0x7d
    recipient,
    signer.address(),
    Bytes::default(),
    Overrides {
        fee_token: Some(fee_token),
        value: Some(U256::from(0)),  // ETH 전송량 (Fee Token과 별개)
        ..Default::default()
    },
).await?;

let tx_hash = send_generic_transaction(&l2_client, tx, &signer).await?;
wait_for_transaction_receipt(tx_hash, &l2_client, 100).await?;
```

### cast 사용 (직접 RPC 호출)

Fee Token 트랜잭션은 custom type `0x7d`이므로 표준 `cast send`로는 보낼 수 없다.
Rust SDK를 사용하거나, 직접 RLP 인코딩하여 `eth_sendRawTransaction`을 호출해야 한다.

---

## 전체 플로우 요약

```
1. Fee Token 컨트랙트 작성 (IFeeToken 구현)
   ↓
2. L2에 Fee Token 배포 (forge create)
   ↓
3. L1 CommonBridge에서 registerNewFeeToken() 호출
   → privileged tx → L2 FeeTokenRegistry에 등록
   ↓
4. L1 CommonBridge에서 setFeeTokenRatio() 호출
   → privileged tx → L2 FeeTokenPricer에 비율 설정
   ↓
5. 사용자가 FeeTokenTransaction (type 0x7d) 전송
   → lockFee() → 트랜잭션 실행 → payFee() → 환불
```

---

## 시스템 컨트랙트 주소

| 컨트랙트 | 주소 | 역할 |
|----------|------|------|
| CommonBridgeL2 | `0x000...ffff` | Fee lock/pay 실행, 토큰 mint/burn |
| L2Messenger | `0x000...fffe` | L2→L1 메시지 전달 |
| FeeTokenRegistry | `0x000...fffc` | 등록된 Fee Token 목록 관리 |
| FeeTokenPricer | `0x000...fffb` | 토큰/ETH 가격 비율 저장 |

---

## 환경변수

| 변수 | 설명 |
|------|------|
| `ETHREX_DEPLOYER_INITIAL_FEE_TOKEN` | 배포 시 자동 등록할 Fee Token의 L2 주소 |

CLI 플래그: `--initial-fee-token <ADDRESS>`

---

## 주의 사항

1. **Fee Token ≠ Native Token**: ETH는 여전히 L2의 네이티브 통화(msg.value)이다.
   Fee Token은 가스비 지불 수단일 뿐이다.

2. **등록 권한**: `registerNewFeeToken()`과 `setFeeTokenRatio()`는
   L1 CommonBridge의 **owner만** 호출할 수 있다.

3. **비율 업데이트**: 토큰 가격이 변동하면 `setFeeTokenRatio()`를 주기적으로
   호출하여 비율을 업데이트해야 한다. 그렇지 않으면 가스비가 과대/과소 청구될 수 있다.

4. **NATIVE_TOKEN_L1**: L1 CommonBridge에 `NATIVE_TOKEN_L1` 변수가 있지만
   **deprecated**되어 사용되지 않는다. 네이티브 통화 교체 기능은 현재 미구현이다.

5. **토큰 잔액 필요**: Fee Token으로 트랜잭션을 보내려면 해당 토큰의 잔액이 충분해야 한다.
   ETH 잔액이 0이어도 Fee Token 잔액만 있으면 트랜잭션을 보낼 수 있다.

---

## 참고 자료

- [Fee Token 공식 문서](../../docs/l2/fundamentals/fee_token.md)
- [Fee Token 배포 가이드 (WIP)](../../docs/l2/deployment/fee_token.md)
- [예제 컨트랙트](../../crates/l2/contracts/src/example/FeeToken.sol)
- [IFeeToken 인터페이스](../../crates/l2/contracts/src/l2/interfaces/IFeeToken.sol)
- [IERC20L2 인터페이스](../../crates/l2/contracts/src/l2/interfaces/IERC20L2.sol)
- [L2 VM Hook (fee token 로직)](../../crates/vm/levm/src/hooks/l2_hook.rs)
- [SDK register_fee_token](../../crates/l2/sdk/src/sdk.rs)
