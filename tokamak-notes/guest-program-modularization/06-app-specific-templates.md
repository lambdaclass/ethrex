# App-Specific Guest Program 템플릿 계획

이 문서는 Guest Program 모듈화를 활용한 앱 특화 L2 템플릿을 정의한다.
EVM-L2 (programTypeId=1) 외에 두 가지 앱 특화 Guest Program을 템플릿으로 등록한다.

---

## 1. 템플릿 목록

| programTypeId | program_id | 이름 | 설명 | 참조 |
|---|---|---|---|---|
| 1 | `evm-l2` | EVM-L2 | 기본 범용 EVM 블록 실행 (기존) | - |
| 2 | `zk-dex` | ZK-DEX | 프라이버시 보존 탈중앙화 거래소 | [tokamak-network/zk-dex](https://github.com/tokamak-network/zk-dex/tree/circom) |
| 3 | `tokamon` | Tokamon | 위치 기반 리워드/스탬프 게임 | [tokamak-network/tokamon](https://github.com/tokamak-network/tokamon/tree/deploy/thanos-sepolia) |

---

## 2. ZK-DEX Guest Program (programTypeId=2)

### 2.1 참조 프로젝트

- **레포**: https://github.com/tokamak-network/zk-dex/tree/circom
- **원본**: Circom 2.1 기반 Groth16 증명, BabyJubJub 키, Poseidon 해시

### 2.2 핵심 상태 전이 (컨트랙트 트랜잭션 기반)

Guest Program 서킷은 아래 컨트랙트 트랜잭션의 상태 전이를 증명한다:

#### A. Note 관리 (ZkDai.sol)

| 트랜잭션 | 입력 | 상태 전이 | 검증 사항 |
|---|---|---|---|
| `mint()` | 토큰 입금 + noteHash | `Invalid → Valid` | 소유권 증명 (EdDSA), 해시 정합성, 입금액 일치 |
| `spend()` | 2개 old notes → 2개 new notes | old: `Valid → Spent`, new: `Invalid → Valid` | 소유권 증명, 금액 보존, 토큰 타입 일관성 |
| `liquidate()` | noteHash + 출금 주소 | `Valid → Spent` | 소유권 증명, 해시 정합성, 출금액 일치 |

#### B. 주문 매칭 (ZkDex.sol)

| 트랜잭션 | 입력 | 상태 전이 | 검증 사항 |
|---|---|---|---|
| `makeOrder()` | maker의 note | note: `Valid → Trading`, order: `Created` | 소유권 증명, 해시 정합성 |
| `takeOrder()` | taker의 note + orderId | taker note: `Valid → Trading`, smart note 생성 | 소유권 증명, 금액 보존, smart note 구조 |
| `settleOrder()` | orderId + price | old notes: `Spent`, 3개 smart notes 생성 | 소유권 증명, 가격 계산 정확성, 금액 분배 |
| `convertNote()` | smart note | smart → regular note 변환 | parent hash 연결, 금액/타입 보존 |

### 2.3 Guest Program 설계

```rust
pub struct ZkDexGuestProgram;

impl GuestProgram for ZkDexGuestProgram {
    fn program_id(&self) -> &str { "zk-dex" }
    fn program_type_id(&self) -> u8 { 2 }
    // ...
}
```

**zkVM 내 검증 대상**:
- Note commitment (Poseidon 해시) 정합성
- 소유권 증명 (EdDSA 서명 검증)
- 금액 보존 (입력 합 == 출력 합)
- 주문 매칭 가격 계산
- 상태 루트 전이 (이전 state root → 새 state root)

**Public Output**:
- `initial_state_root` (이전 배치의 state root)
- `final_state_root` (현재 배치 후 state root)
- 처리된 트랜잭션 수

---

## 3. Tokamon Guest Program (programTypeId=3)

### 3.1 참조 프로젝트

- **레포**: https://github.com/tokamak-network/tokamon/tree/deploy/thanos-sepolia
- **원본**: Solidity 컨트랙트 (UUPS 프록시), 위치 기반 리워드 게임

### 3.2 핵심 상태 전이 (컨트랙트 트랜잭션 기반)

#### A. Spot 관리

| 트랜잭션 | 입력 | 상태 전이 | 검증 사항 |
|---|---|---|---|
| `createSpotSelf()` | ETH 입금, 위치(lat/lng), 보상 설정 | 새 Spot 생성, `remaining += msg.value` | 파라미터 유효성 (좌표, 시간 범위), 입금액 |
| `redepositSelf()` | spotId, ETH 입금 | `remaining += msg.value` | 소유권, 입금액 |
| `updateSpot()` | spotId, 변경 파라미터 | Spot 파라미터 업데이트 | 소유권 |

#### B. 클레임 (리워드 수령)

| 트랜잭션 | 입력 | 상태 전이 | 검증 사항 |
|---|---|---|---|
| `claimToTelegram()` | spotId, telegramHash | `remaining -= reward`, `telegramBalances[hash] += reward`, 스탬프 카운트 증가 | 쿨다운 시간 경과, 시간대 활성 여부, 잔액 충분, 크로스링크 쿨다운 |
| `claimByDevice()` | spotId, deviceHash | `remaining -= reward`, `deviceBalances[hash] += reward`, 스탬프 카운트 증가 | 동일 검증 + claimManager 권한 |
| `claimTelegramToWallet()` | telegramHash | `telegramBalances[hash] → 0`, ETH 전송 | wallet-telegram 링크 검증 |
| `claimDeviceToWallet()` | deviceHash | `deviceBalances[hash] → 0`, ETH 전송 | wallet-device 링크 검증 |

#### C. 스탬프 보너스 메커니즘

`stampCount >= stampGoal`이면 `reward + stampBonus`를 지급하고 카운트 리셋.

### 3.3 Guest Program 설계

```rust
pub struct TokammonGuestProgram;

impl GuestProgram for TokammonGuestProgram {
    fn program_id(&self) -> &str { "tokamon" }
    fn program_type_id(&self) -> u8 { 3 }
    // ...
}
```

**zkVM 내 검증 대상**:
- Spot 잔액 업데이트 정확성 (`remaining` 감소량 == 지급 보상)
- 쿨다운 타이머 검증 (마지막 클레임 시각 + cooldown ≤ 현재 시각)
- 시간대 활성 검증 (UTC offset 적용 후 dailyStartTime/dailyEndTime 범위)
- 스탬프 보너스 계산 (stampCount ≥ stampGoal → bonus 지급)
- 크로스링크 쿨다운 (telegram/device 양쪽 cooldown 동시 검증)
- 상태 루트 전이

**Public Output**:
- `initial_state_root`, `final_state_root`
- 처리된 클레임 수

---

## 4. 구현 로드맵

### Phase 2.4에서 구현할 항목

1. **`ZkDexGuestProgram` 구현** (`crates/guest-program/src/programs/zk_dex.rs`)
   - `GuestProgram` 트레이트 구현
   - 입력 타입 `ZkDexInput` 정의 (노트 트리, 주문 목록, 트랜잭션 배치)
   - 출력 타입 `ZkDexOutput` 정의 (state root 전이)

2. **`TokammonGuestProgram` 구현** (`crates/guest-program/src/programs/tokamon.rs`)
   - `GuestProgram` 트레이트 구현
   - 입력 타입 `TokammonInput` 정의 (Spot 스토리지, 클레임 트랜잭션 배치)
   - 출력 타입 `TokammonOutput` 정의 (state root 전이)

3. **레지스트리 등록** (`crates/l2/prover/src/prover.rs`)
   ```rust
   fn create_default_registry() -> GuestProgramRegistry {
       let mut registry = GuestProgramRegistry::new("evm-l2");
       registry.register(Arc::new(EvmL2GuestProgram));
       registry.register(Arc::new(ZkDexGuestProgram));
       registry.register(Arc::new(TokammonGuestProgram));
       registry
   }
   ```

4. **L1 VK 등록**
   - `upgradeVerificationKey(commitHash, 2, SP1_VERIFIER_ID, zk_dex_vk)` — ZK-DEX
   - `upgradeVerificationKey(commitHash, 3, SP1_VERIFIER_ID, tokamon_vk)` — Tokamon

### 선행 조건

- [x] Phase 2.1: `GuestProgram` 트레이트 ✓
- [x] Phase 2.2: `GuestProgramRegistry`, `ProofData` 프로토콜 ✓
- [x] Phase 2.3: L1 `programTypeId` 지원 ✓ (OnChainProposer VK 3D 매핑)
- [ ] Phase 2.4: 각 프로그램의 zkVM 엔트리포인트 (ELF) 구현

---

## 5. 참고 사항

- ZK-DEX의 원본은 Circom/Groth16을 사용하지만, ethrex Guest Program으로는 SP1/RISC0 zkVM에서 동일한 상태 전이 로직을 Rust로 구현한다.
- Tokamon의 원본은 ZK 증명을 사용하지 않지만, Guest Program으로 구현하면 모든 상태 전이가 zkVM에서 검증되어 trustless한 게임 로직이 된다.
- 두 프로그램 모두 `_getPublicInputsFromCommitment()` 레이아웃이 EVM-L2와 다를 수 있으므로, Phase 2.3에서 추가한 `programTypeId` 기반 디스패치가 필요하다.
