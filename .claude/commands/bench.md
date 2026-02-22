# Benchmark Runner

`perf_opcode_timings` 기반 벤치마크를 실행하고 결과를 분석한다.

## 실행 순서

1. `cargo build --release --features perf_opcode_timings` 빌드
2. 빌드 성공 확인
3. 벤치마크 실행 (가능한 경우):
   - 테스트넷(Holesky) 블록 실행으로 타이밍 수집
   - 또는 EF 테스트 벡터로 opcode 타이밍 수집
4. `RUST_LOG=info` 환경에서 출력 파싱
5. 결과 분석:
   - 가장 느린 opcode Top 10
   - 이전 실행 대비 회귀(regression) 감지
   - SSTORE/SLOAD/CALL 등 핵심 opcode 타이밍 변화

## 회귀 감지 기준

- 개별 opcode 평균 시간이 이전 대비 20%+ 증가: WARNING
- 개별 opcode 평균 시간이 이전 대비 50%+ 증가: REGRESSION
- 전체 블록 실행 시간이 이전 대비 10%+ 증가: REGRESSION

## 보고 형식

```
[BENCH] {STABLE|WARNING|REGRESSION}
- build: perf_opcode_timings={success|failed}
- top 10 slowest opcodes:
  1. {OPCODE}  {avg_time}  ({call_count} calls)
  ...
- regressions: {none | list with % change}
- total block time: {duration}
```
