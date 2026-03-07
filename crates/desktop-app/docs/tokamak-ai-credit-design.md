# Tokamak AI Credit System Design

## 개요

메신저 AI 채팅에서 토카막 AI 사용 시 크레딧 기반 과금 시스템.
TON 메인넷 컨트랙트를 통해 결제하고, 서버에서 크레딧을 관리한다.

## 핵심 원칙

- **구매는 온체인** (TON 메인넷 컨트랙트)
- **사용은 오프체인** (서버 DB, 로그인 세션 기반)
- **지갑 바인딩 불필요** — 결제 시점에만 지갑 사용, 아무 지갑으로나 결제 가능
- **계정 정보 보호** — 서버가 암호화한 토큰만 온체인에 기록

---

## 결제 흐름

```
클라이언트                    서버                     TON 컨트랙트
    │                         │                          │
    ├── "크레딧 구매" 요청 ──▶│                          │
    │                         ├── 서버 비밀키로           │
    │                         │   userId 암호화           │
    │◀── 암호화 토큰 반환 ────┤   → paymentToken 생성     │
    │                         │                          │
    ├── purchaseCredits(packageId, paymentToken) ──────▶│
    │                         │                          ├── TON 수신
    │                         │                          ├── CreditPurchased
    │                         │                          │   이벤트 emit
    │                         │◀── 이벤트 감지 ───────────┤
    │                         ├── paymentToken 복호화     │
    │                         ├── userId 확인             │
    │                         ├── 크레딧 충전             │
    │◀── 충전 완료 알림 ──────┤                          │
    │                         │                          │
```

### 상세 단계

1. 클라이언트가 로그인 세션으로 서버에 결제 요청
2. 서버가 `paymentToken` 생성 (서버 비밀키로 userId 암호화)
3. 클라이언트는 토큰 내용을 알 수 없음 — 그대로 컨트랙트에 전달만 함
4. 컨트랙트가 TON을 수신하고 이벤트에 paymentToken 포함하여 emit
5. 서버가 이벤트를 감지하고 자기 비밀키로 복호화하여 userId 확인
6. 해당 userId에 크레딧 충전

### 보안

- 클라이언트 코드가 오픈이어도 서버 비밀키는 노출되지 않음
- 외부에서 온체인 이벤트를 봐도 누구 계정인지 알 수 없음
- 토큰 조작 불가 — 서버만 유효한 토큰 생성/검증 가능

---

## 스마트 컨트랙트

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract TokamakAICredit {
    address public owner;
    address public treasury;

    struct Package {
        uint256 credits;
        uint256 priceTON;  // wei 단위
        bool active;
    }

    mapping(uint256 => Package) public packages;
    uint256 public packageCount;

    event CreditPurchased(
        address indexed buyer,
        uint256 indexed packageId,
        uint256 credits,
        uint256 amount,
        bytes paymentToken    // 서버가 발급한 암호화된 userId
    );

    modifier onlyOwner() {
        require(msg.sender == owner, "Not owner");
        _;
    }

    constructor(address _treasury) {
        owner = msg.sender;
        treasury = _treasury;
    }

    function addPackage(uint256 credits, uint256 priceTON) external onlyOwner {
        packages[packageCount] = Package(credits, priceTON, true);
        packageCount++;
    }

    function purchaseCredits(uint256 packageId, bytes calldata paymentToken) external payable {
        Package memory pkg = packages[packageId];
        require(pkg.active, "Package not active");
        require(msg.value == pkg.priceTON, "Wrong amount");
        require(paymentToken.length > 0, "Invalid token");

        payable(treasury).transfer(msg.value);

        emit CreditPurchased(
            msg.sender,
            packageId,
            pkg.credits,
            msg.value,
            paymentToken
        );
    }

    function updateTreasury(address _treasury) external onlyOwner {
        treasury = _treasury;
    }

    function togglePackage(uint256 packageId, bool active) external onlyOwner {
        packages[packageId].active = active;
    }
}
```

---

## 서버 API

### 크레딧 관련

| Method | Endpoint | 설명 |
|--------|----------|------|
| GET | `/api/credits/balance` | 잔액 조회 (로그인 필요) |
| POST | `/api/credits/request-payment` | paymentToken 발급 (packageId 전달) |
| GET | `/api/credits/history` | 거래 내역 조회 |

### AI 채팅

| Method | Endpoint | 설명 |
|--------|----------|------|
| POST | `/api/ai/chat` | AI 채팅 요청 (크레딧 확인 → 호출 → 차감) |

### 내부 (이벤트 리스너)

- 서버가 TON 메인넷 `CreditPurchased` 이벤트를 상시 모니터링
- 이벤트 감지 시 paymentToken 복호화 → userId → DB 크레딧 충전

---

## DB 스키마

```sql
-- 크레딧 잔액
CREATE TABLE credit_balances (
    user_id     TEXT PRIMARY KEY,
    balance     INTEGER NOT NULL DEFAULT 0,
    updated_at  DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- 거래 내역
CREATE TABLE credit_transactions (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL,
    type        TEXT NOT NULL,          -- 'purchase' | 'usage' | 'refund'
    amount      INTEGER NOT NULL,       -- +충전, -사용
    description TEXT,                   -- 'AI chat', '500 credit pack' 등
    tx_hash     TEXT,                   -- 온체인 트랜잭션 해시 (purchase 시)
    created_at  DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (user_id) REFERENCES credit_balances(user_id)
);
```

---

## paymentToken 구조

```
paymentToken = AES-256-GCM(serverKey, payload)

payload = {
    userId:    "user-abc-123",
    packageId: 1,
    nonce:     "random-uuid",        // 재사용 방지
    expiresAt: 1709856000            // 유효시간 (10분)
}
```

- **serverKey**: 서버만 보유한 비밀키
- **nonce**: 동일 토큰 재사용(리플레이 공격) 방지
- **expiresAt**: 토큰 유효 시간 제한
- 서버는 사용된 nonce를 기록하여 중복 충전 방지

---

## 크레딧 패키지 (예시)

| 패키지 | 크레딧 | 가격 (TON) | 할인 |
|--------|--------|-----------|------|
| Basic | 100 | 10 | - |
| Standard | 500 | 45 | 10% |
| Premium | 1,000 | 80 | 20% |

---

## 크레딧 소진 기준 (예시)

| AI 기능 | 크레딧/요청 |
|---------|-----------|
| 일반 채팅 | 1~5 |
| 코드 분석 | 5~20 |
| 컨트랙트 감사 | 20~50 |
| 이미지 생성 | 10~30 |

실제 소진량은 입력+출력 토큰 수에 비례하여 동적 계산.

---

## 메신저 UI

### AI 채팅 헤더
```
+-----------------------------------------+
|  Tokamak AI                    150 C    |
+-----------------------------------------+
```

### 크레딧 부족 시
```
+-----------------------------------------+
|  크레딧이 부족합니다.                      |
|                                         |
|  [100C / 10 TON]                        |
|  [500C / 45 TON]        인기             |
|  [1,000C / 80 TON]      최고 할인         |
|                                         |
|  지갑을 연결하여 충전하세요.                 |
|          [ 충전하기 ]                     |
+-----------------------------------------+
```

### 충전 과정
```
+-----------------------------------------+
|  크레딧 충전                              |
|                                         |
|  선택: 500 Credits (45 TON)              |
|                                         |
|  지갑 연결 중...                          |
|  [MetaMask] [WalletConnect]             |
|                                         |
|  → 트랜잭션 서명 요청                      |
|  → 확인 대기 중...                        |
|  → 충전 완료! 잔액: 650 C                 |
+-----------------------------------------+
```

---

## 구현 우선순위

| Phase | 내용 | 비고 |
|-------|------|------|
| Phase 1 | DB + 서버 크레딧 관리 API | 수동 충전으로 테스트 |
| Phase 2 | 메신저 UI (잔액 표시, 부족 알림) | 기존 AI 채팅에 연동 |
| Phase 3 | paymentToken 발급/검증 로직 | AES-256-GCM + nonce |
| Phase 4 | TON 메인넷 컨트랙트 배포 | 테스트넷 먼저 |
| Phase 5 | 이벤트 리스너 + 자동 충전 | 서버 상시 모니터링 |
| Phase 6 | 지갑 연결 UI (MetaMask/WalletConnect) | 결제 시점에만 연결 |
