# 배포 최적화 종합 분석 및 계획

> 작성일: 2026-03-10
> 브랜치: `feat/deployment-optimization-plan`

---

## 목차

1. [현재 상태 분석](#1-현재-상태-분석)
2. [모드별 배포 흐름 비교](#2-모드별-배포-흐름-비교)
3. [발견된 문제점](#3-발견된-문제점)
4. [최적화 계획](#4-최적화-계획)
5. [UI/UX 개선 계획](#5-uiux-개선-계획)
6. [구현 우선순위](#6-구현-우선순위)

---

## 1. 현재 상태 분석

### 1.1 배포 모드 3가지

| 항목 | Local | Testnet (Sepolia) | Remote |
|------|-------|-------------------|--------|
| L1 노드 | Docker 컨테이너 직접 실행 | 외부 RPC (Infura/Alchemy) | 외부 RPC |
| 이미지 빌드 | **매번 리빌드** (기존 삭제 후) | 기존 이미지 있으면 재사용 | 레지스트리에서 pull |
| 이미지 태그 | `tokamak-appchain:{slug}-{projectName}` | `tokamak-appchain:{slug}` (공유) | `{registry}/{name}` |
| 컨트랙트 배포 | **매번 재배포** | DB에 주소 있으면 스킵 | DB에 주소 있으면 스킵 |
| 지갑 키 | 하드코딩된 개발용 키 | 사용자 제공 or Keychain | 사용자 제공 |
| 가스 비용 | 무료 (로컬 devnet) | **실제 ETH 소모** | **실제 ETH 소모** |
| Tools (Dashboard) | Blockscout + Bridge UI | Bridge UI만 (Etherscan 사용) | 없음 |

### 1.2 4-Role 키 구조

| 역할 | 환경변수 | Local 값 | Testnet 값 | 용도 |
|------|---------|---------|-----------|------|
| Deployer | `ETHREX_DEPLOYER_L1_PRIVATE_KEY` | 하드코딩 `0x385c...` | 사용자 제공 | 컨트랙트 배포 |
| Committer | `--committer.l1-private-key` | 하드코딩 `0x385c...` | 별도 키 or Deployer | 배치 커밋 (60초마다) |
| Proof Coordinator | `--proof-coordinator.l1-private-key` | 하드코딩 `0x3972...` | 별도 키 or Deployer | ZK 증명 제출 |
| Bridge Owner | `ETHREX_BRIDGE_OWNER_PK` | 하드코딩 `0x941e...` | 별도 키 or Deployer | 브릿지 관리 |

### 1.3 배포되는 컨트랙트 목록

| 컨트랙트 | Proxy | 트랜잭션 수 | 설명 |
|----------|-------|-----------|------|
| CommonBridge | ✅ | 2 (impl + proxy) | L1↔L2 자산 브릿지 |
| OnChainProposer | ✅ | 2 (impl + proxy) | 배치 커밋 + 증명 검증 |
| Timelock | ❌ | 1 | 거버넌스 타임락 |
| SP1Verifier | ❌ | 1 (zk-dex만) | ZK 증명 검증 |
| GuestProgramRegistry | ❌ | 1 (zk-dex만) | 게스트 프로그램 등록 |
| SequencerRegistry | ❌ | 1 (based만) | 시퀀서 등록 |

**초기화 트랜잭션**: ~5-8개 추가 (initialize, ownership transfer 등)
**총 트랜잭션**: ~15-20개, 가스 리밋 10M/tx → **총 ~150-200M gas**
**세폴리아 예상 비용**: 2-5 gwei 기준 약 0.3-1 ETH

---

## 2. 모드별 배포 흐름 비교

### 2.1 Local 배포 흐름

```
사용자 → "Provision" 클릭
  ↓
[checking_docker] Docker 설치 확인
  ↓
[building] ⚠️ 기존 이미지 삭제 → docker compose build (매번 리빌드)
  ↓
[l1_starting] L1 컨테이너 시작 → 헬스체크 (8545 응답 대기)
  ↓
[deploying_contracts] deployer 컨테이너 실행 → ⚠️ 매번 재배포
  ↓
[l2_starting] L2 노드 시작 → 헬스체크
  ↓
[starting_prover] 프로버 시작
  ↓
[starting_tools] Blockscout + Bridge UI 시작
  ↓
[running] 완료
```

### 2.2 Testnet 배포 흐름

```
사용자 → "Provision" 클릭
  ↓
[checking_docker] Docker 설치 확인
  ↓
[building] ✅ 기존 이미지 확인 → 있으면 스킵, 없으면 빌드
  ↓
[deploying_contracts]
  ├─ DB에 bridge_address + proposer_address 있으면 → ✅ 스킵
  └─ 없으면 → deployer 실행 (사용자 키로, 실제 ETH 소모)
  ↓
[l2_starting] L2 노드 시작 (외부 L1 RPC 연결)
  ↓
[starting_prover] 프로버 시작
  ↓
[starting_tools] Bridge UI만 시작 (Blockscout 없음, Etherscan 사용)
  ↓
[running] 완료
```

---

## 3. 발견된 문제점

### 3.1 🔴 Critical — 가스비 낭비 위험

#### P1: 테스트넷 배포 전 잔액 확인 미사용
- **현상**: API에 `POST /api/deployments/testnet/check-balance` 엔드포인트가 있지만 **UI에서 호출하지 않음**
- **위험**: 잔액 없이 배포 시작 → 컨트랙트 배포 실패 → 부분 배포 상태로 남음
- **영향**: 사용자가 실패 원인을 파악하기 어렵고, 이미 배포된 일부 컨트랙트의 가스가 낭비됨

#### P2: 배포 전 확인(confirm) 다이얼로그 없음
- **현상**: 위자드에서 "Create" 버튼 클릭 시 즉시 배포 시작, 확인 없음
- **위험**: 테스트넷에서 잘못된 설정으로 바로 배포 → 가스비 낭비
- **영향**: 실수로 배포 트리거 시 0.3-1 ETH 손실 가능

#### P3: 가스비 예상 미표시
- **현상**: 배포에 필요한 예상 가스비를 사용자에게 보여주지 않음
- **영향**: 충분한 ETH 없이 시작하거나, 예상 외의 비용에 놀랄 수 있음

### 3.2 🟡 Important — 빌드 최적화

#### P4: Local 모드 매번 이미지 리빌드
- **현상**: `buildImages()`가 기존 이미지를 삭제한 후 매번 새로 빌드
- **코드**: `docker-local.js` line 64-76 — 기존 이미지 강제 삭제
- **영향**: SP1 빌드는 **30-60분** 소요 → 사소한 설정 변경에도 전체 리빌드
- **개선**: 이미지 재사용 옵션 제공 (testnet처럼), 사용자가 "Force Rebuild" 선택 시에만 삭제

#### P5: 이미지 빌드 진행률 부족
- **현상**: 빌드 중 로그만 표시, 예상 소요 시간이나 진행률(%) 없음
- **영향**: 사용자가 빌드가 멈춘 건지 진행 중인지 판단 어려움

### 3.3 🟡 Important — 지갑/키 관리

#### P6: 위자드에서 지갑 주소 미표시
- **현상**: 배포 설정 시 Keychain에서 키를 가져오지만, **파생된 주소를 사용자에게 보여주지 않음**
- **영향**: 사용자가 어떤 주소로 배포되는지 모른 채 진행
- **개선**: 키 선택 후 즉시 파생 주소를 표시하고, 잔액도 같이 보여줌

#### P7: 4개 역할 키의 주소 확인 불가
- **현상**: 서비스 탭에서 배포된 컨트랙트 주소는 보이지만, 역할별 지갑 주소는 안 보임
- **영향**: Committer, Proof Coordinator 등의 잔액을 확인할 수 없음 (이 주소들도 L1 가스비를 지속 소모)
- **개선**: 역할별 지갑 주소 + 잔액 표시 섹션 추가

### 3.4 🟢 Nice-to-have — UX 개선

#### P8: L1 RPC URL 연결 테스트 없음
- **현상**: 사용자가 입력한 RPC URL을 검증하지 않고 바로 진행
- **영향**: 잘못된 URL로 배포 시작 → Docker 내부에서 실패 → 원인 파악 어려움

#### P9: 컨트랙트 주소 복사 버튼 없음
- **현상**: 컨트랙트 주소가 표시되지만 클립보드 복사 기능 없음
- **영향**: 사용자가 수동으로 선택+복사해야 함

#### P10: 배포 재시도 시 부분 실패 처리
- **현상**: 컨트랙트 배포 중 일부만 성공 시, 로그 파싱으로 주소 복구 시도
- **문제**: 로그 파싱 패턴이 정확하지 않으면 주소를 놓칠 수 있음
- **개선**: deployer.rs에서 각 컨트랙트 배포 후 즉시 stdout에 구조화된 JSON 출력

---

## 4. 최적화 계획

### 4.1 빌드 최적화

#### A. 이미지 재사용 로직 통일

**현재**:
```
Local:   매번 삭제 → 리빌드 (buildImages에서 docker rmi 후 build)
Testnet: findImage() 확인 → 있으면 스킵
```

**개선안**:
```
모든 모드: findImage() 확인
  ├─ 있으면 → "기존 이미지 사용" (기본값)
  ├─ 사용자가 "Force Rebuild" 체크 → 삭제 후 리빌드
  └─ 없으면 → 새로 빌드
```

**변경 파일**:
- `docker-local.js`: `buildImages()` — 이미지 삭제 로직을 옵션으로 변경
- `deployment-engine.js`: `provision()` — testnet과 동일한 findImage 체크 추가
- `compose-generator.js`: Local도 공유 이미지 태그 사용 가능하도록

#### B. 빌드 캐시 활용

Docker 레이어 캐시는 이미 `cargo-chef`로 최적화되어 있음 (Dockerfile의 다단계 빌드).
하지만 이미지를 삭제하면 캐시도 무효화됨.

**개선안**: `docker rmi` 대신 새 태그만 붙이기
```javascript
// Before (현재):
execSync(`docker rmi "${img}"`)
// After (개선):
// 이미지 삭제하지 않고, 새 빌드 시 --cache-from 활용
```

### 4.2 컨트랙트 배포 최적화

#### A. Local 모드도 DB 기반 스킵

**현재**: Local은 항상 deployer 실행 (가스 무료이므로 큰 문제 아님)
**개선안**: Local에서도 DB에 주소가 있으면 스킵 옵션 제공 (provision 속도 개선)

```javascript
// deployment-engine.js provision() 추가
if (existingDep?.bridge_address && existingDep?.proposer_address) {
  const skipDeploy = true // 또는 UI에서 사용자 선택
  if (skipDeploy) {
    emit(id, "phase", { phase: "deploying_contracts", message: "기존 컨트랙트 재사용" })
    // extractEnv or rebuild from DB
    return
  }
}
```

#### B. 배포 비용 사전 계산 API

`POST /api/deployments/testnet/estimate-gas` 엔드포인트 추가:
- L1 현재 가스 가격 조회
- 배포 트랜잭션 수 × 가스 리밋(10M)으로 예상 비용 계산
- 역할별 키 잔액 조회
- 결과: `{ estimatedCostEth, deployerBalance, isBalanceSufficient }`

### 4.3 지갑 관리 최적화

#### A. 키 → 주소 파생 API

`POST /api/deployments/testnet/resolve-keys` 엔드포인트:
```json
// Request
{ "keychainKeyName": "deployer-sepolia", "committerKeychainKey": "committer-sepolia" }

// Response
{
  "deployer": { "address": "0x1234...", "balance": "1.5 ETH" },
  "committer": { "address": "0x5678...", "balance": "0.3 ETH" },
  "proofCoordinator": { "address": "0x9abc...", "balance": "0.1 ETH" },
  "bridgeOwner": { "address": "0xdef0...", "balance": "0.0 ETH" },
  "totalRequired": "0.5 ETH",
  "warnings": ["bridgeOwner 잔액 부족"]
}
```

#### B. 서비스 탭에 역할별 지갑 정보 표시

```
┌─────────────────────────────────────────────┐
│ 역할별 지갑 현황                              │
├──────────────┬──────────────┬────────────────┤
│ 역할          │ 주소         │ L1 잔액        │
├──────────────┼──────────────┼────────────────┤
│ Deployer     │ 0x1234...5678│ 1.2 ETH       │
│ Committer    │ 0x5678...9abc│ 0.3 ETH ⚠️    │
│ ProofCoord   │ 0x9abc...def0│ 0.1 ETH ⚠️    │
│ BridgeOwner  │ 0xdef0...1234│ 0.5 ETH       │
└──────────────┴──────────────┴────────────────┘
```

---

## 5. UI/UX 개선 계획

### 5.1 위자드 개선

#### Step 2 (Network Configuration) 개선

**현재**:
```
L1 RPC URL: [https://rpc.sepolia.org          ]
L2 RPC Port: [8550]
Sequencer Mode: [Standalone ▾]
```

**개선안**:
```
L1 RPC URL: [https://rpc.sepolia.org          ] [🔗 Test Connection]
                                                  ✅ Connected (Chain ID: 11155111, Sepolia)

L2 RPC Port: [8550]
Sequencer Mode: [Standalone ▾]
```

#### Step 2.5 (키 설정 — 테스트넷 전용) 추가

```
┌─ Deployer Key ──────────────────────────────────┐
│ Keychain Name: [deployer-sepolia ▾]             │
│ → Address: 0x4417...fc62                        │
│ → Balance: 1.5 ETH ✅                           │
├─ Committer Key ─────────────────────────────────┤
│ Keychain Name: [deployer-sepolia ▾] (기본=Deployer)│
│ → Address: 0x4417...fc62                        │
│ → Balance: 1.5 ETH ✅                           │
├─ Proof Coordinator Key ─────────────────────────┤
│ Keychain Name: [deployer-sepolia ▾] (기본=Deployer)│
│ → Address: 0x4417...fc62                        │
│ → Balance: 1.5 ETH ✅                           │
├─ Bridge Owner Key ──────────────────────────────┤
│ Keychain Name: [deployer-sepolia ▾] (기본=Deployer)│
│ → Address: 0x4417...fc62                        │
│ → Balance: 1.5 ETH ✅                           │
└─────────────────────────────────────────────────┘

예상 배포 비용: ~0.5 ETH (15 transactions × 10M gas × 3 gwei)
Deployer 잔액: 1.5 ETH ✅ 충분
```

### 5.2 배포 확인 다이얼로그 (테스트넷)

"Create" 버튼 클릭 시 확인 팝업:

```
┌─ ⚠️ 테스트넷 배포 확인 ──────────────────────────┐
│                                                  │
│ 다음 설정으로 Sepolia 테스트넷에 배포합니다:          │
│                                                  │
│ • 앱체인: My ZK DEX                              │
│ • L1 네트워크: Sepolia (Chain ID: 11155111)       │
│ • 예상 가스비: ~0.5 ETH                           │
│ • Deployer: 0x4417...fc62 (잔액: 1.5 ETH)        │
│                                                  │
│ ⚠️ 실제 ETH가 소모됩니다. 진행하시겠습니까?         │
│                                                  │
│              [취소]  [배포 시작]                    │
└──────────────────────────────────────────────────┘
```

### 5.3 빌드 단계 개선

#### 이미지 재사용 옵션

```
┌─ Docker 이미지 ─────────────────────────────────┐
│ 기존 이미지가 감지되었습니다:                       │
│ tokamak-appchain:zk-dex (빌드: 2026-03-09 15:30) │
│                                                  │
│ ○ 기존 이미지 사용 (권장, 즉시 시작)               │
│ ○ 이미지 새로 빌드 (~30-60분 소요)                │
└──────────────────────────────────────────────────┘
```

#### 컨트랙트 재배포 옵션

```
┌─ 컨트랙트 배포 ─────────────────────────────────┐
│ 기존 배포가 감지되었습니다:                         │
│ • CommonBridge: 0xABC...123                      │
│ • OnChainProposer: 0xDEF...456                   │
│ (배포일: 2026-03-08)                              │
│                                                  │
│ ○ 기존 컨트랙트 사용 (권장, 가스비 절약)            │
│ ○ 컨트랙트 새로 배포 (~0.5 ETH 소요)              │
└──────────────────────────────────────────────────┘
```

### 5.4 서비스 상세 탭 개선

#### 역할별 지갑 섹션 추가

현재 "L1 Deployed Contracts" 섹션 아래에 추가:

```
┌─ L1 역할별 지갑 ────────────────────────────────┐
│                                                  │
│ Deployer                                         │
│ 0x4417092b70a3e5f10dc504d0947dd256b965fc62  📋   │
│ Balance: 1.2 ETH                                 │
│                                                  │
│ Committer (배치 커밋)                             │
│ 0x4417092b70a3e5f10dc504d0947dd256b965fc62  📋   │
│ Balance: 0.3 ETH  ⚠️ 잔액 부족 경고              │
│ 예상 소진: ~7일 (60초마다 커밋, ~$0.01/tx)         │
│                                                  │
│ Proof Coordinator (증명 제출)                     │
│ 0x9abc...def0  📋                                │
│ Balance: 0.1 ETH                                 │
│                                                  │
│ Bridge Owner (브릿지 관리)                        │
│ 0xdef0...1234  📋                                │
│ Balance: 0.5 ETH                                 │
└──────────────────────────────────────────────────┘
```

### 5.5 진행 상황 개선

```
[building] Docker 이미지 빌드 중...
  ├─ Stage 1/4: 의존성 준비 (cargo-chef)     ████████░░ 80%
  ├─ Stage 2/4: 컴파일                       ██████░░░░ 60%
  ├─ Stage 3/4: SP1 게스트 프로그램 빌드      ░░░░░░░░░░ 대기
  └─ Stage 4/4: 최종 이미지 생성             ░░░░░░░░░░ 대기

  경과: 12분 | 예상 잔여: ~18분
```

---

## 6. 구현 우선순위

### Phase 1: 안전성 (가스비 보호) — ✅ 완료

| # | 작업 | 파일 | 상태 |
|---|------|------|------|
| 1 | 테스트넷 배포 전 잔액 확인 UI 연동 | `CreateL2Wizard.tsx`, `local-server.ts` | ✅ |
| 2 | 배포 확인 다이얼로그 추가 | `CreateL2Wizard.tsx` | ✅ |
| 3 | 키 → 주소 파생 + 잔액 표시 | `deployments.js` (API), `CreateL2Wizard.tsx` | ✅ |
| 4 | L1 RPC URL 연결 테스트 버튼 | `CreateL2Wizard.tsx`, `deployments.js` | ✅ |

### Phase 2: 빌드 최적화 — ✅ 완료

| # | 작업 | 파일 | 상태 |
|---|------|------|------|
| 5 | Local 이미지 재사용 옵션 | `docker-local.js`, `deployment-engine.js` | ✅ |
| 6 | "Force Rebuild" 체크박스 UI | `CreateL2Wizard.tsx` (publish step) | ✅ |
| 7 | Local 컨트랙트 스킵 옵션 | `deployment-engine.js` | ✅ |
| 8 | 기존 배포 감지 시 재사용 프롬프트 | `CreateL2Wizard.tsx` (기존 이미지 감지 + 안내) | ✅ |

### Phase 3: UX 개선 — ✅ 완료

| # | 작업 | 파일 | 상태 |
|---|------|------|------|
| 9 | 역할별 지갑 키 + 잔액 표시 (서비스 탭) | `L2DetailServicesTab.tsx`, `local-server.ts` | ✅ |
| 10 | 컨트랙트 주소 클립보드 복사 버튼 | `L2DetailServicesTab.tsx` | ✅ |
| 11 | 빌드 진행률 개선 (단계별 표시) | `deployment-engine.js` (SSE progress 이벤트) | ✅ |
| 12 | Committer/ProofCoord 잔액 소진 경고 | `L2DetailServicesTab.tsx`, API | ✅ |

### Phase 4: 안정성 — ✅ 완료

| # | 작업 | 파일 | 상태 |
|---|------|------|------|
| 13 | deployer.rs 구조화된 JSON 출력 | `deployer.rs`, `deployment-engine.js` | ✅ |
| 14 | 부분 배포 복구 UI | `L2DetailServicesTab.tsx`, `L2DetailView.tsx` | ✅ |
| 15 | 배포 비용 사전 계산 API | `deployments.js`, `CreateL2Wizard.tsx` | ✅ |

---

## 부록: 파일 위치 참조

| 파일 | 위치 | 설명 |
|------|------|------|
| deployment-engine.js | `crates/desktop-app/local-server/lib/` | 배포 라이프사이클 관리 |
| docker-local.js | `crates/desktop-app/local-server/lib/` | Docker CLI 래퍼 |
| compose-generator.js | `crates/desktop-app/local-server/lib/` | docker-compose.yaml 생성 |
| deployments.js | `crates/desktop-app/local-server/routes/` | REST API 라우트 |
| keychain.js | `crates/desktop-app/local-server/lib/` | macOS Keychain 연동 |
| CreateL2Wizard.tsx | `crates/desktop-app/ui/src/components/` | 배포 설정 위자드 |
| SetupProgressView.tsx | `crates/desktop-app/ui/src/components/` | 배포 진행률 화면 |
| L2DetailServicesTab.tsx | `crates/desktop-app/ui/src/components/` | 서비스 상세 탭 |
| deployer.rs | `cmd/ethrex/l2/` | Rust 컨트랙트 배포자 |
| Dockerfile.sp1 | 프로젝트 루트 | SP1 이미지 빌드 |
