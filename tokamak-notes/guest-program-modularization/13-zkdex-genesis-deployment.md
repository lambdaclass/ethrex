# ZK-DEX L2 제네시스 배포

**최종 업데이트**: 2026-02-26
**상태**: ✅ 구현 완료 (제네시스 생성 파이프라인 구축)

## 배경

ZkDex 컨트랙트와 Groth16 verifier를 L2 제네시스 블록에 포함시켜,
Block 0부터 DEX 기능을 사용 가능하게 하고 SP1 Prover를 즉시 시작할 수 있도록 한다.

### 해결한 문제

1. **verifier 컨트랙트 부재**: `contracts/verifiers/*.sol`이 gitignore되어 있음.
   → ✅ `IGroth16Verifier.sol` 인터페이스 생성 + Circom 회로에서 verifier 생성 파이프라인 구축
2. **배포 순서 의존성**: ZkDex → verifier 주소 필요 → 배포 후에야 주소 확정.
   → ✅ 제네시스에 고정 주소로 사전 배치하여 의존성 제거
3. **DEX_CONTRACT_ADDRESS 하드코딩**: SP1 게스트 프로그램에
   `H160([0xDE; 20])`로 컴파일 타임 상수. → ✅ 제네시스 주소와 일치
4. **chainId 제한**: `ZkDaiBase` constructor가 `development=true` 시
   chainId 1337/31337만 허용. → ✅ `development=false` + 제네시스 storage 직접 세팅으로 우회

### 제네시스 접근법의 장점

- 컨트랙트 주소가 제네시스에서 확정 → `DEX_CONTRACT_ADDRESS` 하드코딩 가능
- 별도 배포 단계 불필요 → Docker 초기화 단순화
- Prover가 첫 트랜잭션부터 증명 가능 (배포 완료 대기 불필요)
- SP1 ZK-DEX 전용 제네시스 파일로 분리

---

## 컨트랙트 주소 배정

| Contract | Address | Purpose |
|----------|---------|---------|
| MintBurnNoteVerifier | `0xDE00000000000000000000000000000000000001` | mint/liquidate 증명 검증 |
| TransferNoteVerifier | `0xDE00000000000000000000000000000000000002` | spend 증명 검증 |
| ConvertNoteVerifier | `0xDE00000000000000000000000000000000000003` | convertNote 증명 검증 |
| MakeOrderVerifier | `0xDE00000000000000000000000000000000000004` | makeOrder 증명 검증 |
| TakeOrderVerifier | `0xDE00000000000000000000000000000000000005` | takeOrder 증명 검증 |
| SettleOrderVerifier | `0xDE00000000000000000000000000000000000006` | settleOrder 증명 검증 |
| ZkDex | `0xDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDE` | 메인 DEX 컨트랙트 |

> MockDai는 ETH-only 모드이므로 불필요. dai 주소 = 0x0.

---

## 구현 결과

### 생성된 파일

| # | 파일 | 프로젝트 | 설명 |
|---|------|----------|------|
| 1 | `contracts/verifiers/IGroth16Verifier.sol` | zk-dex | 6개 Groth16 verifier 인터페이스 |
| 2 | `foundry.toml` | zk-dex | Forge 빌드 설정 (solc 0.8.20, paris EVM) |
| 3 | `scripts/generate-zk-dex-genesis.sh` | ethrex | 제네시스 JSON 자동 생성 스크립트 |

### 수정된 파일

| # | 파일 | 프로젝트 | 변경 내용 |
|---|------|----------|-----------|
| 1 | `circuits-circom/scripts/generate_verifiers.sh` | zk-dex | pragma 호환성 수정 (snarkjs 0.7.x → `^0.8.0`) |
| 2 | `crates/l2/scripts/zk-dex-localnet.sh` | ethrex | `L2_GENESIS` → `l2-zk-dex.json` + 존재 검증 추가 |
| 3 | `crates/l2/docker-compose-zk-dex.overrides.yaml` | ethrex | L2 제네시스 경로 + 볼륨 매핑 변경 |

### IGroth16Verifier.sol 인터페이스 (public input 배열 크기)

| 인터페이스 | 배열 크기 | 용도 |
|-----------|----------|------|
| `IMintNBurnNoteVerifier` | `uint[4]` | output + noteHash + value + tokenType |
| `ITransferNoteVerifier` | `uint[5]` | output + o0Hash + o1Hash + newHash + changeHash |
| `IConvertNoteVerifier` | `uint[4]` | output + smartHash + originHash + newHash |
| `IMakeOrderVerifier` | `uint[3]` | output + noteHash + tokenType |
| `ITakeOrderVerifier` | `uint[6]` | output + oldNoteHash + oldType + newNoteHash + newParentHash + newType |
| `ISettleOrderVerifier` | `uint[14]` | output + 13 public inputs |

---

## 실행 방법

### 전체 파이프라인 (1회, 오프라인)

```bash
# 1. Circom 회로 컴파일 + trusted setup + Solidity verifier 생성
cd /Users/zena/tokamak-projects/zk-dex/circuits-circom
npm install
./scripts/compile.sh                    # 6개 회로 → r1cs + wasm
PTAU_SIZE=18 ./scripts/setup.sh         # trusted setup (pot18)
./scripts/generate_verifiers.sh         # Solidity verifier 생성

# 2. Forge로 전체 컨트랙트 컴파일 + 제네시스 JSON 생성
cd /Users/zena/tokamak-projects/ethrex
./scripts/generate-zk-dex-genesis.sh    # 자동: forge build + bytecode 추출 + storage 검증 + genesis 생성
```

### generate-zk-dex-genesis.sh 스크립트가 하는 일

1. zk-dex 프로젝트에서 `forge build --force` 실행
2. 7개 컨트랙트의 `deployedBytecode` 추출 (6 verifiers + ZkDex)
3. `forge inspect ZkDex storage-layout`으로 슬롯 번호 자동 검증
4. `jq`로 기존 `l2.json`에 7개 컨트랙트 alloc 추가
5. ZkDex storage 슬롯 설정 (development=false, verifier 주소 등)
6. `fixtures/genesis/l2-zk-dex.json`으로 출력
7. 출력 JSON 유효성 검증

### 커스텀 zk-dex 경로 지정

```bash
./scripts/generate-zk-dex-genesis.sh --zk-dex-dir /path/to/custom/zk-dex
```

### 로컬넷 시작

```bash
cd crates/l2
make zk-dex-localnet              # l2-zk-dex.json 제네시스 사용
make zk-dex-localnet-no-prover    # 프로버 없이 (앱 테스트용)
```

---

## ZkDex Storage Layout (제네시스 세팅)

ZkDex 상속 체인: `ZkDaiBase → MintNotes → SpendNotes → LiquidateNotes → ZkDai → ZkDex`

| Slot | 변수 | 값 |
|------|------|-----|
| 0 | `development` (bool) + `dai` (address) | `0x0` (development=false, dai=0x0) |
| 1 | `requestVerifier` | MintBurnNoteVerifier (`0xDE...01`) |
| 2 | `encryptedNotes` mapping | 비어있음 (mapping base) |
| 3 | `notes` mapping | 비어있음 (mapping base) |
| 4 | `requestedNoteProofs` mapping | 비어있음 |
| 5 | `verifiedProofs` mapping | 비어있음 |
| 6 | `mintNoteVerifier` | MintBurnNoteVerifier (`0xDE...01`) |
| 7 | `spendNoteVerifier` | TransferNoteVerifier (`0xDE...02`) |
| 8 | `liquidateNoteVerifier` | MintBurnNoteVerifier (`0xDE...01`) |
| 9 | `convertNoteVerifier` | ConvertNoteVerifier (`0xDE...03`) |
| 10 | `makeOrderVerifier` | MakeOrderVerifier (`0xDE...04`) |
| 11 | `takeOrderVerifier` | TakeOrderVerifier (`0xDE...05`) |
| 12 | `settleOrderVerifier` | SettleOrderVerifier (`0xDE...06`) |
| 13 | `orders.length` | 0 |

> `forge inspect ZkDex storage-layout`으로 슬롯 번호 검증 — generate-zk-dex-genesis.sh가 자동 수행.

---

## 검증 방법

1. **Storage layout 검증**: 스크립트가 자동으로 `forge inspect ZkDex storage-layout`과 비교
2. **Genesis JSON 유효성**: 스크립트가 `jq empty`로 검증 + alloc 항목 수 비교
3. **L2 코드 존재 확인**: L2 시작 후 `eth_getCode` RPC로 ZkDex 주소에 코드 확인
4. **E2E 테스트**: localnet에서 mint → spend → batch commit → SP1 proof

```bash
# L2 시작 후 ZkDex 코드 확인
curl -s -X POST -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_getCode","params":["0xDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDEDE","latest"],"id":1}' \
  http://localhost:1729 | jq '.result | length'
# → 코드 길이가 4 이상이면 성공 (0x + bytecode)
```

---

## 참고 파일

### ethrex 프로젝트
- 제네시스 생성 스크립트: `scripts/generate-zk-dex-genesis.sh`
- 생성될 제네시스: `fixtures/genesis/l2-zk-dex.json`
- 베이스 제네시스: `fixtures/genesis/l2.json`
- 로컬넷 스크립트: `crates/l2/scripts/zk-dex-localnet.sh`
- Docker 오버라이드: `crates/l2/docker-compose-zk-dex.overrides.yaml`
- SP1 게스트 프로그램: `crates/guest-program/src/programs/zk_dex/`

### zk-dex 프로젝트
- Verifier 인터페이스: `contracts/verifiers/IGroth16Verifier.sol`
- Forge 설정: `foundry.toml`
- Circom 회로: `circuits-circom/main/`
- 빌드 스크립트: `circuits-circom/scripts/{compile,setup,generate_verifiers}.sh`
- 도구: `circom 2.1.9`, `snarkjs 0.7.6`, `forge`
