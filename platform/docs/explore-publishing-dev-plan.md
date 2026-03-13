# Explore Publishing — Development Plan

## Repositories

| Repo | Branch | Role |
|------|--------|------|
| `tokamak-rollup-metadata-repository` | `feat/tokamak-appchain-data` | Metadata schema, validators, CI |
| `ethrex` | `feat/explore-publishing` | Platform server sync, Explore page UI |

## Dependency Order

```
metadata-repo Step 1~5  →  ethrex Step 6~9  →  ethrex Step 10
   (schema/validators/CI/docs)   (server sync/API/indexer)   (frontend + Messenger 연동)
```

metadata-repo가 먼저 완성되어야 ethrex에서 sync할 대상이 생김.

---

## Step 1: Directory & Schema (metadata-repo)

- [ ] 1-1. `tokamak-appchain-data/` 디렉토리 생성 + `.gitkeep`
- [ ] 1-2. `schemas/tokamak-appchain-metadata.ts` — 새 타입 정의 작성
  - `TokamakAppchainMetadata` interface
  - `StackType` union type (`tokamak-appchain`, `tokamak-private-app-channel`, `thanos`, `py-ethclient`)
  - Stack별 L1 contracts interface (`TokamakAppchainL1Contracts`, `ThanosL1Contracts`, `GenericL1Contracts`)
- [ ] 1-3. `schemas/example-tokamak-appchain.json` — tokamak-appchain 예제
- [ ] 1-4. `schemas/example-thanos-appchain.json` — thanos 예제 (새 디렉토리 형식)

## Step 2: RPC & Constants (metadata-repo)

- [ ] 2-1. `src/utils/rpc-config.ts` — 임의 L1 Chain ID 지원 (`L1_RPC_{chainId}` 환경변수 폴백)
- [ ] 2-2. `validators/constants.ts` — OnChainProposer ABI, Timelock ABI 추가

## Step 3: Validators (metadata-repo)

- [ ] 3-1. `validators/appchain-schema-validator.ts` — 새 스키마 JSON 검증
  - 필수 필드: l1ChainId, l2ChainId, name, stackType, rpcUrl, status, nativeToken, l1Contracts, operator, metadata
  - stackType별 l1Contracts 필수 필드 분기
- [ ] 3-2. `validators/appchain-contract-validator.ts` — 스택별 온체인 검증
  - tokamak-appchain: `OnChainProposer.owner()` → `Timelock.admin()` 또는 deployer
  - thanos: `SystemConfig.unsafeBlockSigner()`
  - 기타: 컨트랙트 존재 확인 + L2 chainId 매칭
- [ ] 3-3. `validators/signature-validator.ts` 수정 — 새 메시지 포맷 지원
  - "Tokamak Appchain Registry" 프리픽스
  - `Stack:` 필드 추가, `Contract:` 일반화
  - 경로 기반 자동 감지 (data/ vs tokamak-appchain-data/)
- [ ] 3-4. `validators/network-validator.ts` 수정 — 숫자 체인ID 폴더 + 스택 폴더 검증
- [ ] 3-5. `validators/file-validator.ts` 수정 — 새 경로 패턴, 스택별 immutable 필드
- [ ] 3-6. `validators/rollup-validator.ts` 수정 — 경로 기반 라우팅 (legacy vs appchain)

## Step 4: CI & Scripts (metadata-repo)

- [ ] 4-1. `scripts/validate-appchain.ts` — 새 CLI 검증 스크립트
- [ ] 4-2. `package.json` — `validate:appchain` 스크립트 추가
- [ ] 4-3. `.github/workflows/validate-rollup-metadata.yml` 수정
  - `tokamak-appchain-data/**/*.json` 경로 트리거 추가
  - 메타데이터 타입 자동 감지 로직
  - PR 타이틀 `[Appchain]` / `[Update]` 포맷 지원
  - 자동 머지 로직 확장
- [ ] 4-4. 테스트 추가 — 새 검증기 단위 테스트

## Step 5: Documentation (metadata-repo)

- [ ] 5-1. `docs/appchain-registration-guide.md` — 등록 가이드
- [ ] 5-2. `README.md` 업데이트 — 새 디렉토리, 스택 타입, 등록 플로우
- [ ] 5-3. `docs/file-naming.md` 업데이트 — 새 경로 규칙
- [ ] 5-4. `.github/pull_request_template.md` 업데이트

---

## Step 6: DB & Sync (ethrex platform server)

- [ ] 6-1. `platform/server/db/schema.sql` — `explore_listings` 테이블 생성
- [ ] 6-2. `platform/server/db/db.js` — 마이그레이션 추가
- [ ] 6-3. `platform/server/db/listings.js` — CRUD 함수 (신규 파일)
  - `upsertListing()`, `getListings()`, `getListingById()`
  - `updateListingVisibility()`, `deleteListing()`
- [ ] 6-4. `platform/server/lib/metadata-sync.js` — GitHub API 폴링 (신규 파일)
  - `tokamak-appchain-data/{l1ChainId}/{stackType}/` 순회
  - JSON 파싱 → `explore_listings` upsert
  - 삭제 감지 (repo에서 제거된 파일)
  - 5분 간격 폴링

## Step 7: API 전환 (ethrex platform server)

- [ ] 7-1. `platform/server/routes/store.js` — `GET /api/store/appchains` 수정
  - `getActiveDeployments()` → `getListings()` 로 데이터 소스 전환
  - social stats enrichment 유지
- [ ] 7-2. `platform/server/routes/store.js` — `GET /api/store/appchains/:id` 수정
  - `getActiveDeploymentById()` → `getListingById()` 전환
- [ ] 7-3. Social 테이블 참조 변경 — `deployment_id` → `listing_id`
  - `platform/server/db/social.js`
  - `platform/server/db/announcements.js`
  - `platform/server/db/bookmarks.js`
- [ ] 7-4. 기존 `deployments` 데이터 → `explore_listings` 마이그레이션 스크립트

## Step 8: L1 Indexer 연동 (ethrex platform server)

- [ ] 8-1. `platform/server/lib/l1-indexer.js` — listings 기반 감시로 전환
  - `explore_listings`의 proposer_address 목록 사용
  - IPFS 메타데이터 (screenshots 등) enrichment 유지

---

## Step 9: Explore 프론트엔드 (ethrex platform client)

- [ ] 9-1. `platform/client/lib/api.ts` — API 응답 타입 업데이트 (새 필드 반영)
- [ ] 9-2. `platform/client/app/explore/page.tsx` — 리스트 페이지 업데이트
  - 새 필터: Stack Type, L1 Network, Rollup Type, Native Token
  - 카드 레이아웃: 스택 배지, 네트워크 배지, 토큰 표시
- [ ] 9-3. `platform/client/app/explore/[id]/page.tsx` — 상세 페이지 업데이트
  - 전체 메타데이터 표시 (L1/L2 컨트랙트, 브릿지, 익스플로러, 네트워크 설정)
  - 스택 타입별 UI 분기

## Step 10: Desktop Messenger 공개 토글 연동 (ethrex desktop app)

등록 폼은 별도로 필요 없음. 메타데이터 레포에 PR을 올리면 CI 자동 검증 → 자동 머지 → Platform 자동 동기화로 Explore 페이지에 노출됨.
Desktop 메신저(Messenger)의 공개 토글이 이 흐름의 진입점.

- [ ] 10-1. `L2DetailPublishTab.tsx` — "공개(Public)" 토글 UI
  - 토글 ON → 메타데이터 JSON 빌드 + 서명 → metadata-repo에 PR 생성
  - 토글 OFF → metadata-repo에서 해당 JSON 삭제 PR 생성
  - 배포 활성화(activate) ≠ Explore 공개 (별개 동작)
- [ ] 10-2. `lib/metadata-repo-pr.ts` — GitHub API로 metadata-repo PR 자동 생성 (신규)
  - `tokamak-appchain-data/{l1ChainId}/{stackType}/{identityContract}.json` 파일 생성/삭제
  - PR 타이틀: `[Appchain] Register {name}` / `[Appchain] Remove {name}`
  - sequencer/deployer 키로 서명 포함
- [ ] 10-3. `routes/deployments.js` (platform server) — `activateDeployment()` 에서 auto-publish 제거

---

## Milestone Summary

| Milestone | Steps | Repo | 결과물 |
|-----------|-------|------|--------|
| **M1: Schema Ready** | 1~2 | metadata-repo | 스키마 정의, 예제 파일, RPC 설정 |
| **M2: Validators Ready** | 3~4 | metadata-repo | CI 자동 검증 + 자동 머지 가능 |
| **M3: Docs Ready** | 5 | metadata-repo | 외부 개발자가 등록 가능 |
| **M4: Server Sync** | 6~8 | ethrex | 메타데이터 레포 → Platform DB 동기화 |
| **M5: Frontend** | 9 | ethrex | Explore 페이지에서 모든 앱체인 표시 |
| **M6: Messenger 연동** | 10 | ethrex | 데스크톱 메신저 공개 토글 → 자동 등록 |

---

## Notes

- M1~M3 (metadata-repo) 완료 후 M4~M6 (ethrex) 시작 가능
- M4 완료 시점에 기존 Explore 기능은 정상 동작 (데이터 소스만 전환)
- M6는 편의 기능이므로 M5까지 완료되면 운영 가능 (직접 PR로 등록)
- 웹 등록 폼은 불필요 — metadata-repo에 PR을 올리면 자동 등록됨
- 등록 경로: (1) 직접 metadata-repo PR, (2) Desktop 메신저 공개 토글
- 기존 `data/` 디렉토리와 검증 로직은 일절 변경하지 않음
