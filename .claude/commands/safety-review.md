# Safety Review

Agent 생성 코드의 안전성을 독립적으로 검증한다.
Volkov R7 지적사항: "Agent가 Agent를 리뷰하면 순환 참조"에 대한 대응.

## 핵심 원칙

Agent의 리뷰를 신뢰하지 않는다. 외부 도구의 객관적 결과만 신뢰한다:
- Clippy 결과 (정적 분석)
- 테스트 통과 여부 (실행 검증)
- Differential testing 결과 (합의 검증)
- Miri (메모리 안전성, unsafe 블록 존재 시)

## 실행 순서

1. `git diff --name-only HEAD~1` 로 변경 파일 식별

2. **정적 분석 계층**
   - `cargo clippy --workspace -- -D warnings`
   - 변경 파일에서 `unsafe` 검색 → 있으면 `cargo +nightly miri test` 시도
   - 변경 파일에서 `.unwrap()` 신규 추가 검색

3. **실행 검증 계층**
   - `cargo test --workspace`
   - 변경이 `crates/vm/levm/`에 있으면 → `/diff-test` 실행

4. **합의 검증 계층** (EVM 변경 시에만)
   - EF 테스트 벡터 실행
   - state root 비교

5. **변경 범위 검증**
   - 변경 LOC 확인. 단일 커밋에서 500줄+ 변경이면 WARNING
   - 변경이 여러 crate에 걸쳐 있으면 의존성 영향 분석

## 판정

- SAFE: 모든 계층 통과
- REVIEW: 정적 분석 통과했으나 EVM 변경 포함 — diff-test 필수
- UNSAFE: 테스트 실패 또는 합의 불일치 → 커밋 금지

## 보고 형식

```
[SAFETY] {SAFE|REVIEW|UNSAFE}
- static analysis: {pass|N issues}
- unsafe blocks: {none|N new — miri: pass|fail|skipped}
- test suite: {pass|N failures}
- consensus check: {pass|fail|not applicable}
- change scope: {N files, M lines}
```
