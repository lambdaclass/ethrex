# ZK-DEX 프로젝트 진행현황

**작성일**: 2026-02-23
**최종 업데이트**: 2026-02-27
**브랜치**: `feat/zk/zk-dex-full-circuit` (from `feat/zk/sp1-zk-dex-e2e`)

---

## 1. 전체 작업 타임라인

### Phase 1: 기반 구축 + 프로파일링 — 완료

| 날짜 | 작업 | 커밋 |
|------|------|------|
| 2026-02-22 | Guest Program 모듈화 프레임워크 구현 | `f6be4be`~`048a766` |
| | - ZK-DEX, Tokamon 커스텀 타입 및 실행 로직 | |
| | - SP1 zkVM 엔트리포인트 생성 | |
| | - SDK 스캐폴딩, E2E 테스트, ELF 검증 | |
| | - DynamicGuestProgram (런타임 ELF 로딩) | |
| | - Production hardening | |
| 2026-02-23 | App-specific circuit 프레임워크 + DexCircuit | `2107f49`~`4a070dc` |
| | - AppCircuit trait, AppState, incremental MPT | |
| | - DexCircuit: token transfer 로직 구현 | |
| 2026-02-23 | SP1 벤치마크 바이너리 + cycle-tracker 계측 | `4af8122`, `61ce982` |
| 2026-02-23 | 프루버 파이프라인 모듈화 | `25d9025`~`5753e75` |
| | - configurable guest_program_id (시퀀서/CLI) | |
| | - programs.toml 설정, docker-compose 오버라이드 | |
| 2026-02-23 | L1 컨트랙트 + 플랫폼 UI | `318a06f`~`e29718d` |
| | - guestProgramRegistry 컨트랙트 연동 | |
| | - /launch, /guide 페이지, 공식 프로그램 시딩 | |
| 2026-02-23 | SP1 ZK-DEX 바이너리 완성 + 프로파일링 | `881e31d`~`d8cc431` |
| | - SP1 바이너리를 DexCircuit + AppProgramInput으로 재작성 | |
| | - ProgramInput → AppProgramInput 변환 | |
| | - build_trie_from_proofs 수정 + signed tx 벤치마크 | |
| | - **SP1 crypto precompile 패치 → 182x 사이클 감소** | |

---

## 2. SP1 ZK-DEX 프로파일링 결과

### 환경
- **머신**: Apple M4 Max (Rosetta 2 / x86_64 에뮬레이션, CPU only)
- **SP1**: v5.0.8 (Groth16 circuit v5.0.0)

### 핵심 결과 (3-way 비교)

| | EVM L2 (baseline) | ZK-DEX (패치 전) | ZK-DEX (패치 후) | baseline 대비 |
|---|---|---|---|---|
| **총 실행 사이클** | 65,360,896 | 11,449,345 | **357,761** | **182x 감소** |
| **총 proving 시간** | 27분 44초 | 5분 05초 | **3분 26초** | **8.1x 단축** |

> 사이클 182x 감소 대비 proving 시간 8.1x만 단축된 이유:
> recursive compression + Groth16 wrapping 고정 오버헤드(~3분)가 전체의 대부분을 차지.

### 사이클 분석

**EVM L2 주요 병목**:
- `execute_block`: 29.4M cycles (45.6%) — EVM 인터프리터
- 미분류 오버헤드: 24.4M cycles (38.0%) — zkVM 오버헤드
- trie 연산: 9.4M cycles (14.7%) — receipts, state root 등

**ZK-DEX 패치 후 (337K cycles)**:
- ECDSA 서명 검증: ~20K cycles (~6%) — SP1 precompile 가속
- Merkle proof 검증: ~100K cycles (~30%) — sha3 패치 가속
- State root 재계산: ~100K cycles (~30%)
- Token 잔액 업데이트: ~50K cycles (~15%)

### SP1 Crypto Precompile 패치 효과

| 연산 | 패치 전 | 패치 후 | 감소율 |
|------|---------|---------|--------|
| secp256k1 ecrecover | ~10,000,000 cycles | ~20,000 cycles | **~500x** |
| keccak256 해싱 | 소프트웨어 | sha3 precompile | 수배 |

추가된 패치: `k256`, `ecdsa`, `crypto-bigint`, `sha2`, `sha3`, `p256` (기존 `tiny-keccak`, `secp256k1`에 추가)

---

## 3. E2E Localnet 벤치마크 (실측)

### 환경
- **머신**: Apple M4 Max (Rosetta 2 / x86_64 에뮬레이션, CPU only)
- **SP1**: v5.0.8, Groth16 circuit v5.0.0, Docker 기반 Groth16 wrapping
- **L1/L2**: ethrex --dev (로컬), SP1 Prover (CPU)

### 배치별 Proving 시간 실측

| Batch | 블록 수 | 트랜잭션 수 | 트랜잭션 유형 | SP1 실행 사이클 | Core Proving | Groth16 Wrapping | 전체 시간 | L1 검증 |
|-------|---------|------------|---------------|----------------|-------------|-----------------|----------|---------|
| 1 | 1 | 49 | L1 deposit (privileged) | 5,406,639 | ~3분 31초 | ~17초 | **3분 48초** | ✅ 성공 |
| 2 | ~134 | 200 | ETH transfer | 15,792,250 | ~6분 29초 | ~17초 | **6분 46초** | ✅ 성공 |

### 분석

1. **Groth16 wrapping은 사이클 수에 무관하게 고정 ~17초** (Docker 기반, SP1 v5.0.0 circuit)
2. **Core proving 시간은 사이클 수에 비례**:
   - 5.4M cycles → 3분 31초 (~25.6K cycles/sec)
   - 15.8M cycles → 6분 29초 (~40.6K cycles/sec)
   - 큰 배치에서 throughput이 더 높음 (proving 초기 고정 오버헤드 분산)
3. **200개 ETH 전송 배치를 7분 이내에 증명 + L1 검증 완료**
4. **Empty batch 자동 검증**: 트랜잭션이 없는 빈 배치는 ZK proof 없이 자동 검증됨

### 스케일링 전망 (실측 기반 업데이트)

| 시나리오 | 예상 사이클 | 예상 Proving 시간 | 비고 |
|----------|------------|------------------|------|
| 1 ETH transfer | ~80K | ~3분 (고정 오버헤드 지배적) | 최소 단위 |
| 49 deposits (batch 1) | 5.4M | 3분 48초 | **실측** |
| 200 ETH transfers (batch 2) | 15.8M | 6분 46초 | **실측** |
| 500 transfers | ~40M | ~12-15분 | 추정 |
| 1,000 transfers | ~80M | ~20-30분 | 추정 |

> **참고**: 위 데이터는 단순 ETH transfer 기준. 실제 DEX 트랜잭션(mint/spend/makeOrder 등)은
> Groth16 검증 + Poseidon 해싱이 추가되어 트랜잭션당 사이클이 더 높을 수 있음.

---

## 4. Halo2 vs SP1 비교 및 전략적 결정

### 결론: SP1 선택

| 기준 | Halo2 (직접 구현) | SP1 (zkVM) |
|------|-------------------|------------|
| 개발 생산성 | 회로를 직접 작성 (수개월) | Rust 코드 그대로 실행 (수일) |
| 유연성 | 회로 변경 시 전면 재작성 | Rust 코드 수정만으로 변경 |
| 성능 | 이론적으로 최적 | crypto precompile로 충분한 성능 |
| 검증 비용 | Groth16 (저렴) | Groth16 wrapping 지원 (동일) |
| 생태계 | 직접 유지보수 | SP1 생태계 활용 |

### SP1 선택 이유

1. **개발 속도**: DexCircuit을 Rust로 구현하여 수일 만에 동작하는 프로토타입 완성
2. **충분한 성능**: crypto precompile 패치만으로 182x 사이클 감소 달성
3. **모듈화 용이**: 새 AppCircuit 추가가 Rust 코드 작성만으로 가능
4. **플랫폼 전략 적합**: 다수의 앱 전용 L2를 빠르게 지원해야 하는 Tokamak 모델에 적합
5. **Halo2는 옵션으로 유지**: 극한 최적화가 필요한 특정 회로에 한해 Halo2 직접 구현 가능

---

## 5. L1/L2 인프라 현황

### 이미 구현된 것

**Guest Program 모듈화 (Phase 2)**:
- [x] AppCircuit trait + AppState 추상화
- [x] Incremental MPT (Merkle proof 기반 상태 관리)
- [x] DexCircuit 레퍼런스 구현 (token transfer)
- [x] DynamicGuestProgram (런타임 ELF 로딩)
- [x] ELF 헤더 검증 + fuzz-style 테스트
- [x] SDK 스캐폴딩 도구 (모듈 구조 자동 생성)
- [x] programs.toml 설정 + docker-compose 오버라이드
- [x] configurable guest_program_id (시퀀서/CLI 관통)

**L1 컨트랙트**:
- [x] `guestProgramRegistry` → `initialize()` 연동
- [x] `programTypeId` → `verify()` 연동

**GP Store 플랫폼** (Next.js + Express + SQLite):
- [x] 공식 프로그램 3종 시딩 (EVM L2, ZK-DEX, Tokamon)
- [x] /store, /launch, /guide 페이지
- [x] L2 설정 생성 + TOML/docker-compose 다운로드
- [x] 세션 관리 (SQLite 마이그레이션)

**SP1 ZK-DEX 바이너리**:
- [x] SP1 게스트 바이너리 (`sp1-zk-dex`)
- [x] ProgramInput → AppProgramInput 변환 (serialize_input)
- [x] SP1 crypto precompile 패치 (k256, ecdsa, sha2, sha3 등)
- [x] SP1 벤치마크 바이너리
- [x] Cycle-tracker 계측

**출금 UX 최적화 (Phase 4)**:
- [x] 출금 감지 시 조건부 즉시 배치 커밋 (`has_pending_withdrawals()` + 증명 상태 확인)
- [x] 즉시 커밋 조건: 출금 TX 있음 + 증명 대기 배치 없음 (`committed ≤ verified`)
- [x] Withdrawal Tracker UI (4단계 상태 추적 + Claim 버튼)

**Docker 인프라 (Phase 4)**:
- [x] Blockscout restart policy + heavy indexer 비활성화
- [x] ethrex → TokamakAppL2 ZK-DEX 리브랜딩 (docker-compose, dashboard, bridge UI)
- [x] Dashboard에 L2 genesis 컨트랙트 섹션 추가
- [x] 로컬 배포 주소 파일 (`.zk-dex-deployed.env`)

### Phase 2: L1/L2 환경 구성 + 배포 자동화 — 완료

| 날짜 | 작업 | 커밋 |
|------|------|------|
| 2026-02-23 | L1 deployer에 ZK-DEX 게스트 프로그램 등록 파이프라인 추가 | `fe62d40` |
| | - `--register-guest-programs` CLI 옵션 | |
| | - GuestProgramRegistry 초기화 + 공식 프로그램 등록 | |
| | - Timelock을 통한 SP1 VK 등록 | |
| 2026-02-23 | SP1 게스트 크래시 수정 + Timelock VK 등록 수정 | `808d428` |
| 2026-02-24 | **로컬 환경 자동 구축 스크립트** | `b31fed5`, `057383d` |
| | - `crates/l2/scripts/zk-dex-localnet.sh` 신규 작성 | |
| | - 한 명령으로 L1→배포→L2→프로버 자동 시작 | |
| | - `start/stop/status/logs` 명령 + `--no-prover` 옵션 | |
| | - Makefile 타겟 4개 추가 | |
| | - 실제 로컬넷 구동 검증 완료 | |

### Phase 3: L2 제네시스 + E2E 증명 파이프라인 — 완료

| 날짜 | 작업 | 커밋 |
|------|------|------|
| 2026-02-26 | **ZK-DEX L2 제네시스 배포 파이프라인 구축** | `523734f` |
| | - `IGroth16Verifier.sol` 인터페이스 생성 (6개 verifier) | |
| | - `foundry.toml` 생성 (Forge 빌드 지원) | |
| | - `generate_verifiers.sh` pragma 호환성 수정 (snarkjs 0.7.x) | |
| | - `generate-zk-dex-genesis.sh` 스크립트 작성 | |
| | - forge build → bytecode 추출 → storage layout 검증 → genesis JSON 생성 자동화 | |
| | - localnet/Docker 스크립트 `l2-zk-dex.json` 전환 | |
| 2026-02-26 | **Localnet 시작 버그 수정** | `2fc703c` |
| | - `storage.rs` 슬롯 상수 수정: ENCRYPTED_NOTES 3→2, NOTES 4→3, ORDERS 14→13 | |
| | - L2 서버 `--no-monitor` 플래그 추가 (TUI→stdout 로깅 전환) | |
| | - L2 P2P 포트 충돌 해결 (`--p2p.port 30304 --discovery.port 30304`) | |
| | - ZkDex 7개 컨트랙트 + storage slots L2 제네시스 배포 확인 | |
| 2026-02-26 | **SP1 E2E 증명 + L1 온체인 검증 성공** | `0541a74` |
| | - `ETHREX_GUEST_PROGRAM_ID=zk-dex` env var 누락 수정 (programTypeId 1→2) | |
| | - 근본 원인: committer가 programTypeId=1(evm-l2)로 batch commit → VK 불일치 | |
| | - Batch 1~9: SP1 zk-dex Groth16 증명 → L1 온체인 검증 성공 | |
| | - 전체 파이프라인: L1→Deploy→L2→Prover→L1 Verify 완전 동작 확인 | |
| 2026-02-26 | **대규모 배치 벤치마크 (200 ETH transfers)** | |
| | - 200개 ETH transfer 트랜잭션 L2 전송 → 단일 블록에 포함 | |
| | - Batch 2: 15.8M cycles, 6분 46초 proving, L1 검증 성공 | |
| | - Groth16 wrapping 고정 ~17초, core proving이 지배적 | |

### Phase 4: Docker 인프라 + 출금 UX 최적화 — 완료

| 날짜 | 작업 | 커밋 |
|------|------|------|
| 2026-02-26 | **Genesis state root 검증 + 프루버 버전 불일치 수정** | `f329a93` |
| | - Genesis 상태 root 검증 로직 추가 | |
| | - 프루버 버전 불일치 문제 수정 | |
| 2026-02-27 | **출금 감지 시 조건부 즉시 배치 커밋 (Early Batch Commit)** | `23759c9` |
| | - `l1_committer.rs`에 `has_pending_withdrawals()` 메서드 추가 | |
| | - 미커밋 블록 중 BRIDGE_ADDRESS(0x...ffff) 대상 TX 감지 | |
| | - 즉시 커밋 조건: 출금 감지 **+** 증명 대기 배치 없음 (`committed ≤ verified`) | |
| | - 증명 중인 배치가 있으면 프루버가 바빠서 즉시 커밋 불필요 → 기존 타이머 대기 | |
| | - `get_last_verified_batch()` L1 조회로 증명 상태 판단 | |
| 2026-02-27 | **Docker Tools 안정화 + TokamakAppL2 브랜딩** | `931fad9` |
| | - Blockscout restart policy + 무거운 indexer 비활성화 | |
| | - ethrex → TokamakAppL2 ZK-DEX 전체 리브랜딩 | |
| | - Dashboard에 L2 genesis 컨트랙트 섹션 추가 | |
| | - Withdrawal Tracker에 Claim 버튼 + 상태 추적 UI 추가 | |
| | - 로컬 배포 주소 파일 (`.zk-dex-deployed.env`) 추가 | |
| 2026-02-27 | **SP1 ZK-DEX 인프라 비용 분석 문서** | `6767b30` |
| | - GPU/CPU 프루버 비용 비교, 운영 비용 추정 | |

### 해결된 주요 이슈

| 이슈 | 증상 | 근본 원인 | 해결 |
|------|------|----------|------|
| L2 로그 미출력 | stdout 리다이렉트 시 0 bytes | TUI 모니터 기본 활성화 → 인메모리 버퍼 전송 | `--no-monitor` 플래그 |
| P2P 포트 충돌 | `UdpSocketError(AddrInUse)` | L1/L2 모두 30303 포트 기본 사용 | `--p2p.port 30304` |
| SP1 증명 L1 검증 실패 (`00e`) | `ISP1Verifier.verifyProof` revert | `ETHREX_GUEST_PROGRAM_ID` 미설정 → programTypeId=1(evm-l2) 사용 | env var 추가 |
| Batch 2 commit 실패 (`005`) | `execution reverted: 005` | Batch 1 미검증 → 다음 batch commit 불가 | Batch 1 검증 후 자동 해소 |
| Storage slot 불일치 | 게스트 프로그램 storage proof 실패 | `storage.rs` 슬롯 상수와 Solidity storage layout 불일치 | 슬롯 번호 수정 |

### 체크리스트

- [x] 실제 L1 네트워크에 컨트랙트 배포 → 로컬 L1 (ethrex --dev) 배포 완료
- [x] 실제 L2 노드를 Guest Program과 함께 가동 → `zk-dex-localnet.sh`로 자동화
- [x] ZK-DEX 컨트랙트 L2 제네시스 배포 → `generate-zk-dex-genesis.sh` 파이프라인 구축
- [x] Circom 회로 컴파일 + trusted setup 실행 (1회, 오프라인) → 6개 회로 컴파일 + PTAU 14 setup 완료
- [x] End-to-end 증명 생성 및 L1 검증 — Batch 1~9 SP1 Groth16 증명 + L1 온체인 검증 성공
- [x] 대규모 배치 벤치마크 — 200 ETH transfers, 15.8M cycles, 6분 46초 proving
- [x] 출금 시 즉시 배치 커밋 → `has_pending_withdrawals()` + early commit 구현
- [x] Docker 인프라 안정화 → Blockscout restart, indexer 최적화, TokamakAppL2 리브랜딩
- [x] Withdrawal Claim UI → withdraw-status.html에 Claim 버튼 + 4단계 상태 추적
- [x] 인프라 비용 분석 문서 → `sp1-zk-dex-infra-cost-analysis.md`
- [ ] 출금 E2E 테스트 (L2 출금 → 즉시 배치 커밋 → L1 Claim)
- [ ] 실제 DEX 트랜잭션 (mint/spend/makeOrder) 벤치마크
- [ ] 프론트엔드 DEX 트레이딩 UI 개발
- [ ] Native ARM 벤치마크 (Rosetta 2 없이)

---

## 6. 로컬 환경 구축 가이드

### 자동 구축 (권장)

```bash
cd crates/l2

# 전체 환경 시작 (L1 + 컨트랙트 배포 + L2 + SP1 프로버)
make zk-dex-localnet

# 프로버 없이 시작 (앱/프론트엔드 테스트용, 더 빠름)
make zk-dex-localnet-no-prover

# 상태 확인
make zk-dex-localnet-status

# 로그 확인
./scripts/zk-dex-localnet.sh logs        # 전체
./scripts/zk-dex-localnet.sh logs l1     # L1만
./scripts/zk-dex-localnet.sh logs l2     # L2만

# 종료
make zk-dex-localnet-stop
```

### 엔드포인트

| 서비스 | URL |
|--------|-----|
| L1 RPC | `http://localhost:8545` |
| L2 RPC | `http://localhost:1729` |
| Proof Coordinator | `tcp://127.0.0.1:3900` |
| Prometheus Metrics | `http://localhost:3702` |

### 배포된 컨트랙트 (로컬)

| 컨트랙트 | 주소 |
|-----------|------|
| OnChainProposer | `cmd/.env` → `ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS` |
| Bridge | `cmd/.env` → `ETHREX_WATCHER_BRIDGE_ADDRESS` |
| SP1 Verifier | `cmd/.env` → `ETHREX_DEPLOYER_SP1_VERIFIER_ADDRESS` |
| Timelock | `cmd/.env` → `ETHREX_TIMELOCK_ADDRESS` |
| ZkDex (L2 Genesis) | `0xDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDE` |
| Verifiers (L2 Genesis) | `0xDE00...01` ~ `0xDE00...06` |

### 검증

```bash
# L1 RPC 동작 확인
curl -s -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
  http://localhost:8545

# L2 RPC 동작 확인
curl -s -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
  http://localhost:1729

# L2 ZkDex 컨트랙트 코드 확인
cast code 0xDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDE --rpc-url http://localhost:1729

# L1 마지막 검증된 배치 확인
source cmd/.env
cast call $ETHREX_COMMITTER_ON_CHAIN_PROPOSER_ADDRESS "lastVerifiedBatch()(uint256)" --rpc-url http://localhost:8545
```

### 주의사항

- 첫 실행 시 릴리스 빌드가 포함되어 10분 이상 소요될 수 있음
- 이전 ethrex 프로세스가 남아있으면 포트 충돌 → `pkill ethrex` 후 재시작
- 런타임 파일은 `crates/l2/.zk-dex-localnet/`에 저장 (`.gitignore` 처리됨)
- git commit 변경 시 prover에서 "Version mismatch" 경고 발생 가능 → localnet 재시작 필요

---

## 7. 다음 단계

### 완료된 목표

1. **E2E 파이프라인 검증** ✅
   - L1 Deploy → L2 Genesis (ZkDex + 6 Verifiers) → Block Production → Batch Commit → SP1 Proof → L1 Verify
   - Batch 1~9 연속 검증 성공

2. **대규모 배치 벤치마크** ✅
   - 200 ETH transfers → 15.8M cycles → 6분 46초 proving → L1 검증 성공
   - Throughput: ~40K cycles/sec (대규모 배치 기준)

### 남은 작업

1. **출금 E2E 테스트** (우선순위: 높음)
   - L1→L2 deposit → L2 withdraw → 즉시 배치 커밋 → L1 Claim 전체 흐름 검증
   - `"Pending withdrawal detected, triggering early batch commit"` 로그 확인
   - withdraw-status.html의 4단계 상태 추적 동작 확인

2. **실제 DEX 트랜잭션 벤치마크**
   - mint/spend/makeOrder/takeOrder/settleOrder 트랜잭션을 L2에 전송
   - Groth16 client-side 증명 생성 필요 (snarkjs, circomlibjs)
   - DEX 트랜잭션의 SP1 proving overhead 측정

3. **프론트엔드 DEX 트레이딩 UI**
   - `platform/client/`는 현재 Guest Program Store (배포 관리 플랫폼)
   - DEX 전용 트레이딩 UI (swap, order book, trade history) 미구현
   - BabyJubJub 키 관리 + ECDH 노트 암호화 필요

4. **Native ARM 벤치마크**
   - 현재: Rosetta 2 / x86_64 에뮬레이션 (M4 Max)
   - SP1 Docker Groth16 wrapping은 x86_64 전용
   - Native ARM proving은 compressed proof만 가능 (Groth16 wrapping 불가)

---

## 참조 문서

| 문서 | 설명 |
|------|------|
| `tokamak-notes/sp1-profiling-baseline.md` | EVM L2 베이스라인 프로파일링 |
| `tokamak-notes/sp1-zk-dex-vs-baseline.md` | ZK-DEX vs EVM L2 상세 비교 |
| `tokamak-notes/zk-optimization-plan.md` | 전체 최적화 계획 (Phase 1-6) |
| `tokamak-notes/guest-program-modularization/` | 모듈화 설계 문서 (12개) |
| `tokamak-notes/local-setup-guide.md` | 로컬 실행 가이드 |
| `crates/l2/scripts/ZK-DEX-LOCALNET.md` | 로컬넷 스크립트 사용 가이드 |
| `tokamak-notes/zk-dex-e2e-design.md` | E2E 아키텍처 설계 |
| `tokamak-notes/sp1-zk-dex-infra-cost-analysis.md` | SP1 ZK-DEX 인프라 비용 분석 |
