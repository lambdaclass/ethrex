# Differential Testing

ethrex와 Geth의 실행 결과를 비교하여 불일치를 탐지한다.
Continuous Benchmarking(Tier S #10)의 핵심 검증 메커니즘이자
Agent 생성 코드의 최종 안전장치.

## 목적

- Agent가 수정한 EVM 코드가 합의를 위반하지 않는지 검증
- Geth/Reth와의 state root 불일치 탐지
- Agent ↔ Agent 리뷰의 순환 참조 방지 (외부 기준점으로 Geth 사용)

## 실행 순서

1. `crates/vm/levm/` 하위 파일이 변경되었는지 확인
   - 변경 없으면 "EVM 미변경 — diff test 생략" 출력 후 종료
2. `cargo build --release` (ethrex 빌드)
3. Ethereum execution-spec-tests 또는 Hive 테스트 중 subset 실행:
   - `cargo test -p levm` — LEVM 유닛 테스트
   - EF 테스트 벡터가 있으면 실행하여 state root 비교
4. 결과 비교:
   - state root 일치: PASS
   - state root 불일치: FAIL — 불일치 트랜잭션/블록 식별

## 불일치 발견 시

1. 불일치 트랜잭션의 opcode trace 비교
2. 어디서 분기하는지 식별 (opcode 단위)
3. 원인 분석: Tokamak 수정 vs upstream 버그 vs 테스트 오류
4. upstream 버그 발견 시 → 이슈 리포트 준비 (Sahil의 R4 전략)

## 보고 형식

```
[DIFF TEST] {PASS|FAIL|SKIP}
- EVM changed: {yes|no}
- tests run: {N}
- state root matches: {N/N}
- mismatches: {0 | details}
```
