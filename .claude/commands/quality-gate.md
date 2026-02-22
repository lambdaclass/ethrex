# Quality Gate

Agent 생성 코드의 품질을 검증하는 게이트. 모든 코드 변경 후 실행 필수.

## 실행 순서

1. `cargo clippy --workspace -- -D warnings` 실행. warning 0개가 목표
2. `cargo test --workspace` 실행. 실패 테스트 0개가 목표
3. `cargo build --workspace` 빌드 성공 확인
4. `git diff --stat` 로 변경 범위 확인 — 의도하지 않은 파일 변경 감지
5. 변경된 파일에 `unsafe` 블록이 있으면 경고 출력 + 안전성 분석 수행
6. 변경된 파일에 `unwrap()` 이 새로 추가되었으면 경고 출력

## 결과 판정

- PASS: 위 6개 항목 모두 통과
- WARN: clippy warning 또는 unwrap 존재하지만 빌드/테스트 통과
- FAIL: 빌드 실패 또는 테스트 실패

FAIL 시 커밋 금지. WARN 시 사유를 명시한 후 커밋 가능.

## 보고 형식

```
[QUALITY GATE] {PASS|WARN|FAIL}
- clippy: {0 warnings | N warnings}
- tests: {all passed | N failed}
- build: {success | failed}
- unsafe blocks: {none | N new}
- unwrap additions: {none | N new}
- changed files: {list}
```
