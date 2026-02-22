# Upstream Rebase

ethrex upstream(LambdaClass/ethrex)과 동기화하는 워크플로우.

## 사전 조건

- 현재 브랜치의 모든 변경사항이 커밋되어 있어야 함
- `/quality-gate` PASS 상태여야 함

## 실행 순서

1. `git remote -v`로 upstream 리모트 확인. 없으면 `git remote add upstream https://github.com/lambdaclass/ethrex.git`
2. `git fetch upstream main`
3. `git log --oneline HEAD..upstream/main | head -20`으로 upstream 변경사항 확인
4. 변경사항 분석:
   - `crates/vm/levm/` 변경이 있으면 **HIGH RISK** — LEVM 코어 변경. 충돌 가능성 높음
   - `crates/l2/` 변경이 있으면 **MEDIUM RISK** — Hook 시스템 영향 가능
   - 기타 변경은 **LOW RISK**
5. HIGH RISK인 경우 유저에게 확인 후 진행
6. `git rebase upstream/main` 실행
7. 충돌 발생 시:
   - 충돌 파일 목록 출력
   - 각 충돌을 분석하고 Tokamak 수정사항을 보존하며 해소
   - 해소 후 `git rebase --continue`
8. rebase 완료 후 `/quality-gate` 자동 실행

## EXIT 기준 (Volkov R7)

- rebase 충돌 해소에 1시간 이상 소요되면 중단하고 유저에게 보고
- LEVM 코어(vm.rs, opcodes.rs) 충돌이 3개 이상이면 수동 리뷰 요청

## 보고 형식

```
[REBASE] {SUCCESS|CONFLICT|ABORT}
- upstream commits: {N}
- risk level: {LOW|MEDIUM|HIGH}
- conflicts: {0 | N files}
- quality gate: {PASS|WARN|FAIL}
```
