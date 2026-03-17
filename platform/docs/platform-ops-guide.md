# Tokamak Platform 운영 가이드

> 플랫폼 서비스 배포, 운영, 업데이트에 필요한 모든 내용을 다룹니다.

---

## 1. 아키텍처 개요

플랫폼은 두 레이어로 구성됩니다:

| 구성 요소 | 기술 스택 | 포트 | 역할 |
|-----------|-----------|------|------|
| **Express 서버** | Node.js + SQLite | 5001 | 로컬 배포 엔진, Docker 오케스트레이션, API |
| **Next.js 클라이언트** | Next.js 15 + Postgres | 3000 | Vercel 프로덕션 UI + Serverless API |

### 운영 환경별 구조

```
[로컬 개발]
  Client (Next.js :3000) → Express Server (:5001) → SQLite + Docker

[Vercel 프로덕션]
  Client (Vercel Serverless) → Neon Postgres + Upstash Redis
  └─ Cron: 매 시간 metadata-sync (GitHub 폴링)
```

---

## 2. 디렉토리 구조

```
platform/
├── server/                    # Express API 서버
│   ├── server.js              # 진입점 (포트 5001)
│   ├── routes/                # API 라우트 (auth, store, programs, deployments, admin)
│   ├── db/                    # SQLite DB + 스키마 + 쿼리 모듈
│   │   ├── schema.sql         # 전체 테이블 정의
│   │   └── platform.sqlite    # DB 파일 (자동 생성)
│   ├── lib/                   # 배포 엔진, 메타데이터 동기화, 인증
│   ├── middleware/             # 세션 인증
│   ├── uploads/               # ELF 파일 저장소
│   └── tests/                 # 단위 테스트
│
├── client/                    # Next.js 클라이언트
│   ├── app/                   # App Router (페이지 + API Routes)
│   │   └── api/               # Vercel Serverless Functions
│   │       ├── auth/          # 인증 (signup, login, OAuth)
│   │       ├── store/         # 스토어 + 소셜 기능 (appchains, reviews, comments, ...)
│   │       ├── admin/         # 관리자
│   │       ├── ai/            # AI 채팅 프록시
│   │       └── cron/          # Vercel Cron (metadata-sync)
│   ├── lib/                   # 공통 라이브러리
│   │   ├── db.ts              # Postgres 연결 + 스키마 초기화
│   │   ├── auth.ts            # 세션 관리
│   │   ├── wallet-auth.ts     # 지갑 서명 검증 (ethers)
│   │   ├── social-queries.ts  # 소셜 DB 쿼리
│   │   ├── appchain-resolver.ts  # listings → deployments fallback
│   │   └── api.ts             # 프론트엔드 API 클라이언트
│   ├── vercel.json            # Cron 스케줄
│   └── package.json           # 의존성
│
├── Dockerfile                 # Docker 이미지 빌드 (l1, l2, sp1)
├── build-images.sh            # 이미지 빌드 스크립트
├── dev.sh                     # 로컬 개발 실행 스크립트
└── docs/                      # 문서
```

---

## 3. 환경 변수

### Express 서버 (`server/.env`)

| 변수 | 설명 | 기본값 |
|------|------|--------|
| `PORT` | 서버 포트 | 5001 |
| `CORS_ORIGINS` | 허용 CORS 오리진 (쉼표 구분) | `http://localhost:3000` |
| `GOOGLE_CLIENT_ID` | Google OAuth 클라이언트 ID | - |
| `NAVER_CLIENT_ID` | Naver OAuth 클라이언트 ID | - |
| `NAVER_CLIENT_SECRET` | Naver OAuth 시크릿 | - |
| `KAKAO_REST_API_KEY` | Kakao REST API 키 | - |
| `KAKAO_CLIENT_SECRET` | Kakao 시크릿 | - |
| `GITHUB_TOKEN` | GitHub API 토큰 (rate limit 향상) | - |
| `METADATA_REPO_OWNER` | 메타데이터 저장소 소유자 | `tokamak-network` |
| `METADATA_REPO_NAME` | 메타데이터 저장소 이름 | `tokamak-rollup-metadata-repository` |
| `METADATA_SYNC_INTERVAL` | 동기화 간격 (ms) | `300000` (5분) |

### Next.js 클라이언트 (`client/.env.local`)

| 변수 | 설명 | 기본값 |
|------|------|--------|
| `NEXT_PUBLIC_API_URL` | API 기본 URL | `""` (같은 호스트) |
| `DATABASE_URL` | Postgres 연결 문자열 | - (필수) |
| `GOOGLE_CLIENT_ID` | Google OAuth | - |
| `NAVER_CLIENT_ID` / `_SECRET` | Naver OAuth | - |
| `KAKAO_REST_API_KEY` / `_SECRET` | Kakao OAuth | - |
| `TOKAMAK_AI_PROVIDER` | AI 프로바이더 | `openai` |
| `TOKAMAK_AI_BASE_URL` | AI API URL | - |
| `TOKAMAK_AI_API_KEY` | AI API 키 | - |
| `TOKAMAK_AI_MODEL` | AI 모델 이름 | - |
| `TOKAMAK_AI_DAILY_LIMIT` | 일일 토큰 한도 | 50000 |
| `UPSTASH_REDIS_REST_URL` | Upstash Redis URL | - (선택) |
| `UPSTASH_REDIS_REST_TOKEN` | Upstash Redis 토큰 | - (선택) |
| `CRON_SECRET` | Vercel Cron 인증 토큰 | - (선택) |
| `GITHUB_TOKEN` | GitHub API 토큰 | - (선택) |

---

## 4. 로컬 개발

### 빠른 시작

```bash
cd platform
./dev.sh
```

서버(5001)와 클라이언트(3000)를 동시에 시작합니다.

### 수동 실행

```bash
# 서버
cd platform/server
npm install
npm run dev          # node --watch server.js (파일 변경 시 자동 재시작)

# 클라이언트 (별도 터미널)
cd platform/client
npm install
npm run dev          # next dev (Hot Reload)
```

### 데이터베이스

- **서버**: `platform/server/db/platform.sqlite` — 자동 생성, 스키마 자동 마이그레이션
- **클라이언트**: `DATABASE_URL` 환경 변수로 Postgres 연결 — `ensureSchema()`가 첫 요청 시 테이블 자동 생성

### 테스트

```bash
# 서버 단위 테스트
cd platform/server && npm test

# 클라이언트 빌드 검증
cd platform/client && npm run build
```

---

## 5. Vercel 프로덕션 배포

### 사전 요구사항

- Vercel 계정 (Hobby 플랜 이상)
- Neon Postgres 데이터베이스
- (선택) Upstash Redis (AI 토큰 제한용)

### 초기 설정

```bash
# 1. Vercel CLI 설치 및 로그인
npm i -g vercel
vercel login

# 2. 프로젝트 연결
cd platform/client
vercel link

# 3. 스토리지 생성
vercel storage add postgres   # Neon — DATABASE_URL 자동 설정
vercel storage add kv          # Upstash Redis — KV_REST_* 자동 설정

# 4. 환경 변수 설정
vercel env add GOOGLE_CLIENT_ID
vercel env add NAVER_CLIENT_ID
vercel env add NAVER_CLIENT_SECRET
vercel env add KAKAO_REST_API_KEY
vercel env add KAKAO_CLIENT_SECRET
vercel env add GITHUB_TOKEN           # (선택) GitHub API rate limit 향상
vercel env add CRON_SECRET            # (선택) Cron 엔드포인트 보호

# 5. 배포
vercel --prod
```

### 자동 배포

GitHub 저장소 연결 시 `tokamak-dev` 브랜치에 push하면 자동 배포됩니다.

### 프리뷰 배포

PR 생성 시 자동으로 프리뷰 URL이 생성됩니다.

```bash
vercel           # 수동 프리뷰 배포
```

---

## 6. Vercel Cron (예약 작업)

### 메타데이터 동기화

`vercel.json`:
```json
{
  "crons": [
    { "path": "/api/cron/metadata-sync", "schedule": "0 * * * *" }
  ]
}
```

| 항목 | 값 |
|------|-----|
| 엔드포인트 | `GET /api/cron/metadata-sync` |
| 스케줄 | 매 시간 정각 (`0 * * * *`) |
| 플랜 제한 | Hobby: 하루 1개 cron, 최소 1시간 간격 |
|  | Pro: 10개 cron, 최소 1분 간격 |
| 인증 | `CRON_SECRET` 설정 시 `Authorization: Bearer <secret>` 헤더 필요 |

### 동작 원리

1. GitHub Git Trees API로 `tokamak-rollup-metadata-repository` 트리 조회
2. SHA 비교로 변경된 파일만 식별
3. 변경 파일을 5개씩 병렬로 다운로드 + `explore_listings` 테이블에 upsert
4. 저장소에서 삭제된 파일은 DB에서도 삭제

### 수동 실행

```bash
# 로컬
curl http://localhost:3000/api/cron/metadata-sync

# 프로덕션 (CRON_SECRET 설정 시)
curl -H "Authorization: Bearer <secret>" https://tokamak-appchain.vercel.app/api/cron/metadata-sync
```

### 응답 형식

```json
{
  "ok": true,
  "synced": 3,
  "deleted": 0,
  "errors": 0,
  "elapsed_ms": 1234
}
```

---

## 7. 데이터베이스 스키마

### Postgres (Vercel 프로덕션)

총 12개 테이블 — `ensureSchema()`가 첫 요청 시 자동 생성:

| 테이블 | 용도 |
|--------|------|
| `users` | 사용자 (email, OAuth, 역할) |
| `programs` | Guest Program (ELF, 검증키, 승인 상태) |
| `program_versions` | 프로그램 버전 이력 |
| `deployments` | 사용자 등록 앱체인 |
| `sessions` | 인증 세션 (`ps_` 접두사, 24시간 TTL) |
| `explore_listings` | GitHub 메타데이터 동기화 리스팅 |
| `reviews` | 리뷰 (1-5점, 지갑당 1개) |
| `comments` | 댓글 (스레드, soft-delete) |
| `reactions` | 좋아요 (리뷰/댓글 대상) |
| `bookmarks` | 북마크 (계정 기반) |
| `announcements` | 공지 (앱체인 오너만, 최대 10개) |
| `ai_usage` | AI 토큰 사용량 |

### 스키마 마이그레이션

- **자동**: `CREATE TABLE IF NOT EXISTS` + `addColumnIfMissing()` 패턴
- **수동 마이그레이션 불필요**: 새 컬럼 추가 시 `ALTER TABLE ADD COLUMN`이 이미 존재하면 무시

---

## 8. API 엔드포인트 전체 목록

### 인증

| 메서드 | 경로 | 설명 |
|--------|------|------|
| POST | `/api/auth/signup` | 이메일 회원가입 |
| POST | `/api/auth/login` | 이메일 로그인 |
| POST | `/api/auth/google` | Google OAuth |
| POST | `/api/auth/naver` | Naver OAuth |
| POST | `/api/auth/kakao` | Kakao OAuth |
| GET | `/api/auth/me` | 현재 사용자 정보 (인증 필요) |
| PUT | `/api/auth/profile` | 프로필 수정 (인증 필요) |
| POST | `/api/auth/logout` | 로그아웃 (인증 필요) |

### 스토어 (공개)

| 메서드 | 경로 | 설명 |
|--------|------|------|
| GET | `/api/store/programs` | 프로그램 목록 (검색, 카테고리) |
| GET | `/api/store/programs/:id` | 프로그램 상세 |
| GET | `/api/store/categories` | 카테고리 목록 |
| GET | `/api/store/featured` | 추천 프로그램 |
| GET | `/api/store/appchains` | 앱체인 목록 (검색, stack_type, l1_chain_id) |
| GET | `/api/store/appchains/:id` | 앱체인 상세 |
| POST | `/api/store/appchains/:id/rpc-proxy` | L2 RPC 프록시 |

### 소셜 (지갑 인증)

| 메서드 | 경로 | 설명 |
|--------|------|------|
| GET | `/api/store/appchains/:id/reviews` | 리뷰 목록 + 반응 카운트 |
| POST | `/api/store/appchains/:id/reviews` | 리뷰 작성/수정 (지갑) |
| DELETE | `/api/store/appchains/:id/reviews/:reviewId` | 리뷰 삭제 (작성자만) |
| GET | `/api/store/appchains/:id/comments` | 댓글 목록 + 반응 카운트 |
| POST | `/api/store/appchains/:id/comments` | 댓글 작성 (지갑) |
| DELETE | `/api/store/appchains/:id/comments/:commentId` | 댓글 삭제 (작성자만, soft-delete) |
| POST | `/api/store/appchains/:id/reactions` | 좋아요 토글 (지갑) |

### 북마크 (계정 인증)

| 메서드 | 경로 | 설명 |
|--------|------|------|
| POST | `/api/store/appchains/:id/bookmark` | 북마크 토글 |
| GET | `/api/store/bookmarks` | 내 북마크 목록 |

### 공지 (오너 지갑)

| 메서드 | 경로 | 설명 |
|--------|------|------|
| GET | `/api/store/appchains/:id/announcements` | 공지 목록 (공개) |
| POST | `/api/store/appchains/:id/announcements` | 공지 작성 (오너만) |
| PUT | `/api/store/appchains/:id/announcements/:announcementId` | 공지 수정 (오너만) |
| DELETE | `/api/store/appchains/:id/announcements/:announcementId` | 공지 삭제 (오너만) |

### 관리자 (admin 역할)

| 메서드 | 경로 | 설명 |
|--------|------|------|
| GET | `/api/admin/programs` | 전체 프로그램 (승인/거부 필터) |
| PUT | `/api/admin/programs/:id/approve` | 프로그램 승인 |
| PUT | `/api/admin/programs/:id/reject` | 프로그램 거부 |
| GET | `/api/admin/users` | 전체 사용자 |
| PUT | `/api/admin/users/:id/role` | 역할 변경 |
| PUT | `/api/admin/users/:id/suspend` | 계정 정지 |
| PUT | `/api/admin/users/:id/activate` | 계정 활성화 |
| GET | `/api/admin/stats` | 플랫폼 통계 |

---

## 9. 인증 체계

### 세션 토큰 (`ps_` 접두사)

```
Authorization: Bearer ps_a1b2c3d4e5f6...
```

- 회원가입/로그인 시 발급
- 24시간 TTL
- `sessions` 테이블에 저장

### 지갑 서명 (소셜 기능)

```
x-wallet-address: 0x1234...
x-wallet-signature: 0xabcd...
```

- EIP-191 서명 (ethers.verifyMessage)
- 고정 챌린지 메시지: `"Sign in to Tokamak Appchain Showroom\n\n..."`
- LRU 캐시 (최대 1000개) — 반복 검증 방지

### 권한 매트릭스

| 기능 | 비로그인 | 계정 로그인 | 지갑 서명 | 오너 지갑 | 관리자 |
|------|----------|------------|----------|----------|--------|
| 앱체인 목록/상세 | ✅ | ✅ | ✅ | ✅ | ✅ |
| 리뷰/댓글 읽기 | ✅ | ✅ | ✅ | ✅ | ✅ |
| 리뷰/댓글 쓰기 | ❌ | ❌ | ✅ | ✅ | ✅ |
| 좋아요 | ❌ | ❌ | ✅ | ✅ | ✅ |
| 북마크 | ❌ | ✅ | ❌ | ✅ | ✅ |
| 공지 관리 | ❌ | ❌ | ❌ | ✅ | ❌ |
| 프로그램 관리 | ❌ | ✅ | ❌ | ❌ | ✅ |

---

## 10. Docker 이미지 빌드

### 이미지 목록

| 이미지 | 태그 | 용도 |
|--------|------|------|
| `tokamak-appchain` | `l1` | L1 로컬넷 |
| `tokamak-appchain` | `l2` | 표준 L2 |
| `tokamak-appchain` | `sp1` | ZK-DEX (SP1 프루버) |

### 빌드

```bash
cd platform
./build-images.sh           # 로컬 빌드
./build-images.sh --push    # 빌드 + 레지스트리 push
```

---

## 11. 운영 체크리스트

### 배포 전 확인

- [ ] `npm run build` 타입 에러 없이 통과
- [ ] 환경 변수 설정 완료 (DATABASE_URL 필수)
- [ ] OAuth 자격증명 설정 (최소 1개 프로바이더)
- [ ] Postgres 연결 확인

### 배포 후 확인

- [ ] `GET /api/health` → `{ status: "ok" }` 응답
- [ ] Explore 페이지에 앱체인 리스팅 표시
- [ ] 로그인/회원가입 정상 동작
- [ ] Cron 로그 확인 (Vercel 대시보드 → Functions → Cron)

### 정기 점검

- [ ] GitHub API rate limit 확인 (GITHUB_TOKEN 없으면 60회/시간)
- [ ] Postgres 연결 풀 상태 (max: 10)
- [ ] AI 토큰 사용량 모니터링
- [ ] Cron 실행 이력 확인

---

## 12. 트러블슈팅

### Explore 페이지가 비어있음

1. `/api/cron/metadata-sync` 수동 호출하여 동기화 확인
2. `GITHUB_TOKEN` 미설정 시 rate limit (60회/시간) 초과 가능
3. `explore_listings` 테이블에 데이터 있는지 확인

### OAuth 로그인 실패

1. 환경 변수 (CLIENT_ID, SECRET) 확인
2. OAuth 콜백 URL이 Vercel 도메인과 일치하는지 확인
3. Google: GCP 콘솔에서 승인된 리디렉션 URI 확인
4. Naver/Kakao: 개발자 센터에서 콜백 URL 확인

### 지갑 인증 실패

1. 프론트엔드에서 올바른 챌린지 메시지로 서명하는지 확인
2. `x-wallet-address`와 `x-wallet-signature` 헤더 확인
3. ethers 패키지 설치 확인: `npm ls ethers`

### Cron 실행 안됨

1. `vercel.json` 파일이 `platform/client/` 루트에 있는지 확인
2. Hobby 플랜: 하루 1개 cron, 최소 1시간 간격 제한
3. `CRON_SECRET` 설정 시 Vercel가 자동으로 헤더 추가하는지 확인
4. Vercel 대시보드 → Settings → Cron Jobs에서 등록 상태 확인

### DB 스키마 불일치

- `ensureSchema()`가 `CREATE TABLE IF NOT EXISTS` + `addColumnIfMissing()` 사용
- 새 컬럼은 자동 추가되지만, 기존 컬럼 타입 변경은 수동 마이그레이션 필요
- Neon 콘솔에서 직접 SQL 실행 가능

---

## 13. 업데이트 절차

### 코드 업데이트

```bash
# 1. 기능 브랜치에서 작업
git checkout -b feat/my-feature

# 2. 변경 후 빌드 확인
cd platform/client && npm run build

# 3. PR 생성 → 프리뷰 배포 자동 생성
git push -u origin feat/my-feature
gh pr create --base tokamak-dev

# 4. 리뷰 후 머지 → 프로덕션 자동 배포
```

### DB 스키마 업데이트

1. `lib/db.ts`의 `ensureSchema()`에 새 테이블/컬럼 추가
2. `addColumnIfMissing()` 패턴으로 안전하게 마이그레이션
3. 배포 후 첫 API 요청 시 자동 적용

### 환경 변수 추가

```bash
vercel env add NEW_VARIABLE            # 대화형
vercel env add NEW_VARIABLE production  # 프로덕션 전용
```

추가 후 재배포 필요:
```bash
vercel --prod
```

### 의존성 업데이트

```bash
cd platform/client
npm update           # 마이너/패치 업데이트
npm run build        # 빌드 확인
# PR로 배포
```

---

## 14. 보안 고려사항

| 항목 | 구현 방식 |
|------|----------|
| SQL 인젝션 | 파라미터화된 쿼리 (`$1, $2, ...`) |
| SSRF | RPC 프록시에서 사설 IP 차단 (10.x, 192.168.x, 172.16-31.x, localhost) |
| XSS | React 기본 이스케이프 + Content-Type 헤더 |
| 비밀번호 | bcryptjs 해싱 |
| 세션 | 암호학적 랜덤 토큰 (`crypto.getRandomValues`) |
| Rate Limit | 서버: 100 req/min per IP (in-memory) |
| CORS | 허용 오리진 명시 (`CORS_ORIGINS`) |
| Cron 보호 | `CRON_SECRET` 헤더 검증 |
| RPC 프록시 | 허용 메서드 화이트리스트 (6개만 허용) |
| 지갑 인증 | EIP-191 서명 검증 + LRU 캐시 |
