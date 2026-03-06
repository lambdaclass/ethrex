# Fixture Platform Development Plan

## Goal

새로운 개발자가 새로운 앱의 fixture를 자유롭게 추가하고, 오프라인 테스트를 실행할 수 있는
셀프 서비스 플랫폼을 만든다.

**완료**: 어떤 앱이든 `tests/fixtures/<app>/` 디렉토리에 JSON을 넣으면
자동으로 테스트가 돌아가는 구조를 구축했다.

---

## Current State Analysis

### Already Generic (변경 불필요)

| Component | Location | Status |
|-----------|----------|--------|
| Fixture loader | `tests/fixture_types.rs` | `load_all_fixtures(app: &str)` — 어떤 앱이든 로드 가능 |
| Merge script | `tests/merge-fixtures.sh` | 앱 무관하게 동작 |
| Prover fixture dump | `crates/l2/prover/src/prover.rs:174-223` | `program_id`로 디렉토리 자동 결정 |
| Committer fixture dump | `crates/l2/sequencer/l1_committer.rs:1306-1362` | DB에서 program_id 조회 |
| Fixture JSON schema | `tests/fixture_types.rs` | `app` 필드로 앱 구분 |

### Previously Hardcoded (✅ 수정 완료)

| Component | Before | After |
|-----------|--------|-------|
| test_program_output.rs | `load_all_fixtures("zk-dex")` | `discover_all_apps()` 자동 탐색 |
| test_commitment_match.rs | `load_all_fixtures("zk-dex")` | `discover_all_apps()` 자동 탐색 |
| test_state_continuity.rs | `load_all_fixtures("zk-dex")` x3 | `discover_all_apps()` 자동 탐색 |

### Previously Missing (현재 상태)

| Component | Status |
|-----------|--------|
| Auto-discovery tests | ✅ `discover_all_apps()` 구현, 디렉토리만 추가하면 자동 실행 |
| Prover balance_diffs dump | ⚠️ 빈 배열 (인코딩 포맷상 decode 불가, warn 처리로 완화) |
| 새 앱 추가 가이드 | ✅ `adding-new-app-fixtures.md` 작성 |
| `cargo test` 검증 | ✅ feature flag 없이 정상 동작 확인 |
| CI integration | ✅ `.github/workflows/pr_fixture_tests.yml` 추가 |

---

## Development Plan

### Phase 1: Test Auto-Discovery (테스트 자동 탐색)

**목표**: `tests/fixtures/<app>/` 디렉토리만 만들면 테스트가 자동으로 실행되게 한다.

**작업 내용**:

1. **`fixture_types.rs`에 `discover_all_apps()` 함수 추가**
   - `tests/fixtures/` 하위 디렉토리를 스캔하여 앱 목록 반환
   - 빈 디렉토리는 건너뜀

2. **테스트 3개를 앱 자동 탐색 방식으로 변경**

   Before:
   ```rust
   #[test]
   fn encode_matches_all_zk_dex_fixtures() {
       let fixtures = load_all_fixtures("zk-dex");
       // ...
   }
   ```

   After:
   ```rust
   #[test]
   fn encode_matches_all_app_fixtures() {
       let apps = discover_all_apps();
       assert!(!apps.is_empty(), "No fixture apps found");
       for app in &apps {
           let fixtures = load_all_fixtures(app);
           for f in &fixtures {
               // ... test logic ...
           }
       }
   }
   ```

3. **state_continuity 테스트도 앱별 자동 실행**
   - 앱별로 batch_number 순 정렬 → 연속 batch 간 state hash 검증
   - fixture가 1개뿐인 앱은 skip

**수정 파일**:
- `crates/guest-program/tests/fixture_types.rs`
- `crates/guest-program/tests/test_program_output.rs`
- `crates/guest-program/tests/test_commitment_match.rs`
- `crates/guest-program/tests/test_state_continuity.rs`

**검증**: `cargo test -p ethrex-guest-program` — zk-dex fixture로 기존과 동일 결과

---

### Phase 2: New App Onboarding Guide (새 앱 추가 가이드)

**목표**: 새 개발자가 따라할 수 있는 step-by-step 문서 작성.

**문서 내용** (`platform/docs/adding-new-app-fixtures.md`):

1. **앱 등록** — 3곳 수정 필요:
   - `crates/l2/common/src/lib.rs` → `resolve_program_type_id()` 에 새 앱 추가
   - `platform/server/db/db.js` → programs 배열에 추가
   - `crates/l2/prover/src/prover.rs` → GuestProgram registry에 추가

2. **Guest Program 구현** (ZK proof가 필요한 경우):
   - `crates/guest-program/src/programs/<app>/` 디렉토리 생성
   - `GuestProgram` trait 구현
   - `AppCircuit` trait 구현 (app_execution.rs 사용 시)

3. **Docker Compose 프로필** (플랫폼 배포 시):
   - `platform/server/lib/compose-generator.js` → `APP_PROFILES`에 추가

4. **Fixture 수집 워크플로우**:
   - `ETHREX_DUMP_FIXTURES` 설정 → 배포 → E2E 테스트 → merge → 복사
   - (기존 `fixture-data-collection.md` 참조)

5. **테스트 실행**:
   - `tests/fixtures/<app>/` 에 .json 넣기 → `cargo test` → 자동 실행

**수정 파일**: 신규 문서 1개

---

### Phase 3: Prover Fixture Completeness (Prover dump 완성도) — PARTIAL

**목표**: prover.json이 balance_diffs, l2_in_message_rolling_hashes를 정확히 dump한다.

**현재 상태**:
- Prover fixture dump는 이 필드들을 빈 배열로 저장 (decode 불가)
- **근본 원인**: `ProgramOutput.encode()`가 길이 접두사 없이 가변 필드를 연결하므로,
  바이트만으로 balance_diffs와 l2_in_message_rolling_hashes 경계를 알 수 없음
- `ProgramOutput::decode(bytes)`를 만들려면 인코딩 포맷 변경 필요 (L1 컨트랙트 영향)

**적용된 완화책**:
- `test_commitment_match`: prover 배열이 비어 있고 committer가 데이터가 있으면
  fail 대신 warn 출력. `test_program_output`이 전체 바이트 비교로 이미 커버.
- prover.json의 `encoded_public_values`에는 모든 데이터가 포함되어 있음

**향후 개선 옵션** (인코딩 포맷 변경 시):
1. encode에 길이 접두사 추가 → decode 가능 (L1 컨트랙트 변경 필요)
2. Guest program이 ProgramOutput을 별도로 직렬화하여 두 번째 출력으로 전달

---

### Phase 4: CI Integration (CI 파이프라인)

**목표**: PR마다 fixture 테스트가 자동 실행된다.

**작업 내용**:
1. GitHub Actions workflow에 fixture 테스트 step 추가
2. `cargo test -p ethrex-guest-program --test test_program_output --test test_commitment_match --test test_state_continuity`
3. Feature flag 없이 실행 가능한지 확인 (sp1 의존성 불필요)

**수정 파일**:
- `.github/workflows/` — 기존 CI에 step 추가 또는 새 workflow

**선행 조건**: Phase 1 완료 (테스트가 자동 탐색 방식이어야 새 앱 추가 시 CI 변경 불필요)

---

## Priority & Dependencies

```
Phase 1 (Auto-Discovery)     ✅ 완료
    │
    ├── Phase 2 (Guide)       ✅ 완료
    │
    └── Phase 4 (CI)          ✅ 완료
         │
Phase 3 (Prover dump)        ⚠️ 부분 완료 (decode 불가, warn 처리로 완화)
```

**추천 실행 순서**: Phase 1 → Phase 2 → Phase 4 → Phase 3

---

## New Developer Workflow (완성 후)

```
1. Guest Program 구현 (또는 기존 앱 사용)
2. 앱 등록 (3곳)
3. Docker로 배포 + ETHREX_DUMP_FIXTURES 활성화
4. E2E 테스트로 트랜잭션 생성
5. merge-fixtures.sh로 fixture 병합
6. tests/fixtures/<my-app>/ 에 JSON 복사
7. cargo test → 자동 통과
8. PR → CI 자동 검증
```

---

## File Reference

| File | Role |
|------|------|
| `crates/l2/common/src/lib.rs` | App type ID registry |
| `crates/l2/prover/src/prover.rs` | Guest program registry + fixture dump |
| `crates/l2/sequencer/l1_committer.rs` | Committer fixture dump |
| `crates/guest-program/tests/fixture_types.rs` | Fixture loader (generic) |
| `crates/guest-program/tests/test_*.rs` | Offline tests (auto-discover all apps) |
| `crates/guest-program/tests/merge-fixtures.sh` | Fixture merger |
| `platform/server/lib/compose-generator.js` | Docker compose app profiles |
| `platform/server/db/db.js` | Platform app database |
| `platform/docs/fixture-data-collection.md` | Fixture collection guide |
