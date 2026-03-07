# Tokamak Appchain Platform (Client + API Server)

Tokamak Appchain의 웹 클라이언트와 API 서버를 하나의 Next.js 프로젝트로 통합한 플랫폼입니다.
Vercel 위에 배포되며, Neon Postgres(DB)와 Upstash Redis(토큰 제한)를 사용합니다.

## 프로젝트 구조

```
platform/client/
├── app/                    # Next.js App Router
│   ├── api/                # API 라우트 (Vercel Serverless Functions)
│   │   ├── ai/
│   │   │   ├── chat/       # POST — Tokamak AI 프록시 (토큰 제한 포함)
│   │   │   └── usage/      # GET  — 디바이스별 일일 토큰 사용량 조회
│   │   ├── auth/
│   │   │   ├── signup/     # POST — 이메일/비밀번호 회원가입
│   │   │   ├── login/      # POST — 이메일/비밀번호 로그인
│   │   │   ├── logout/     # POST — 로그아웃 (세션 삭제)
│   │   │   ├── me/         # GET  — 현재 로그인 유저 정보
│   │   │   ├── profile/    # PUT  — 프로필 수정
│   │   │   ├── providers/  # GET  — 활성화된 OAuth 제공자 목록
│   │   │   ├── google/     # GET/POST — Google OAuth
│   │   │   ├── naver/      # GET/POST — Naver OAuth
│   │   │   └── kakao/      # GET/POST — Kakao OAuth
│   │   ├── programs/
│   │   │   ├── route.ts    # POST (생성) / GET (내 프로그램 목록)
│   │   │   └── [id]/       # GET / PUT / DELETE
│   │   ├── deployments/
│   │   │   ├── route.ts    # POST (생성) / GET (내 배포 목록)
│   │   │   └── [id]/       # GET / PUT / DELETE
│   │   │       └── activate/  # POST — 배포 활성화
│   │   ├── store/
│   │   │   ├── programs/   # GET — 공개 프로그램 목록 (검색/카테고리/페이지네이션)
│   │   │   ├── categories/ # GET — 카테고리 목록 + 개수
│   │   │   ├── featured/   # GET — 추천 프로그램
│   │   │   └── appchains/  # GET — 공개 앱체인 목록 (Showroom)
│   │   ├── admin/
│   │   │   ├── programs/   # GET (전체) / [id] (상세) / approve / reject
│   │   │   ├── users/      # GET (전체) / [id]/role / [id]/suspend / [id]/activate
│   │   │   ├── deployments/# GET (전체 배포)
│   │   │   └── stats/      # GET — 플랫폼 통계
│   │   └── health/         # GET — 헬스 체크
│   └── ...                 # UI 페이지
├── lib/
│   ├── db.ts               # Vercel Postgres 연결 + 스키마 초기화
│   ├── auth.ts             # 세션 기반 인증 (createSession, requireAuth, requireAdmin)
│   ├── oauth.ts            # Google/Naver/Kakao OAuth 헬퍼
│   ├── oauth-user.ts       # findOrCreateOAuthUser 공유 함수
│   ├── token-limiter.ts    # AI 토큰 일일 제한 (Vercel KV 기반)
│   └── validate.ts         # 입력값 검증 유틸리티
├── next.config.ts
└── package.json
```

## 환경변수

### 필수 (Vercel Storage)

Vercel 대시보드에서 Postgres/KV를 생성하면 자동으로 연결됩니다.

| 변수 | 설명 |
|------|------|
| `DATABASE_URL` | Neon Postgres 연결 문자열 (Vercel Storage 연동 시 자동 설정) |
| `UPSTASH_REDIS_REST_URL` | Upstash Redis REST URL (Vercel Storage 연동 시 자동 설정) |
| `UPSTASH_REDIS_REST_TOKEN` | Upstash Redis 인증 토큰 (Vercel Storage 연동 시 자동 설정) |

### OAuth (선택 — 사용할 제공자만 설정)

| 변수 | 설명 |
|------|------|
| `GOOGLE_CLIENT_ID` | Google OAuth 클라이언트 ID |
| `NAVER_CLIENT_ID` | Naver OAuth 클라이언트 ID |
| `NAVER_CLIENT_SECRET` | Naver OAuth 클라이언트 시크릿 |
| `KAKAO_REST_API_KEY` | Kakao REST API 키 |
| `KAKAO_CLIENT_SECRET` | Kakao 클라이언트 시크릿 (선택) |

설정하지 않은 OAuth 제공자는 자동으로 비활성화됩니다.
`GET /api/auth/providers`에서 어떤 제공자가 활성화되었는지 확인할 수 있습니다.

## 로컬 개발

```bash
cd platform/client

# 의존성 설치
npm install

# 환경변수 설정 (Vercel CLI 사용)
vercel env pull .env.local

# 개발 서버 실행
npm run dev
```

개발 서버: http://localhost:3000

> Vercel KV가 설정되지 않은 로컬 환경에서는 인메모리 스토어로 대체됩니다.
> Postgres는 반드시 필요합니다 (`vercel env pull`로 가져오거나, `.env.local`에 `POSTGRES_URL`을 직접 설정).

## Vercel 배포

### 1. 초기 설정

```bash
# Vercel CLI 설치
npm i -g vercel

# 로그인
vercel login

# 프로젝트 연결
cd platform/client
vercel link
```

### 2. Storage 생성

```bash
# Postgres 데이터베이스 생성
vercel storage add postgres

# KV (Redis) 생성 — AI 토큰 제한용
vercel storage add kv
```

생성 후 환경변수(`POSTGRES_URL`, `KV_REST_API_URL`, `KV_REST_API_TOKEN`)는 자동으로 프로젝트에 연결됩니다.

### 3. 시크릿 설정

```bash
# OAuth 시크릿 (사용할 제공자만)
vercel env add GOOGLE_CLIENT_ID
vercel env add NAVER_CLIENT_ID
vercel env add NAVER_CLIENT_SECRET
vercel env add KAKAO_REST_API_KEY
vercel env add KAKAO_CLIENT_SECRET
```

### 4. 배포

```bash
# 프리뷰 배포 (테스트용)
vercel

# 프로덕션 배포
vercel --prod
```

### 5. GitHub 자동 배포 (선택)

Vercel 대시보드에서 GitHub 레포를 연결하면:
- `tokamak-dev` 브랜치 push → 프로덕션 자동 배포
- PR 생성 시 → 프리뷰 URL 자동 생성

## DB 스키마

첫 번째 API 요청 시 자동으로 테이블이 생성됩니다 (`ensureSchema()`).

### users
| 컬럼 | 타입 | 설명 |
|------|------|------|
| id | TEXT (PK) | UUID |
| email | TEXT (UNIQUE) | 이메일 |
| name | TEXT | 이름 |
| password_hash | TEXT | bcrypt 해시 (이메일 가입 시) |
| auth_provider | TEXT | email / google / naver / kakao |
| role | TEXT | user / admin |
| status | TEXT | active / suspended |
| created_at | BIGINT | Unix timestamp (ms) |

### programs
| 컬럼 | 타입 | 설명 |
|------|------|------|
| id | TEXT (PK) | UUID |
| program_id | TEXT (UNIQUE) | 슬러그 (예: evm-l2, zk-dex) |
| program_type_id | INTEGER (UNIQUE) | 1=EVM L2, 2=ZK-DEX, 3=Tokamon |
| creator_id | TEXT (FK→users) | 생성자 |
| name | TEXT | 프로그램 이름 |
| description | TEXT | 설명 |
| category | TEXT | general / defi / gaming / social / infrastructure |
| status | TEXT | pending / active / rejected / disabled |
| is_official | INTEGER | 1=공식 프로그램 |
| use_count | INTEGER | 배포 횟수 |

### deployments
| 컬럼 | 타입 | 설명 |
|------|------|------|
| id | TEXT (PK) | UUID |
| user_id | TEXT (FK→users) | 소유자 |
| program_id | TEXT (FK→programs) | 프로그램 |
| name | TEXT | 배포 이름 |
| chain_id | INTEGER | L2 체인 ID |
| rpc_url | TEXT | RPC URL |
| status | TEXT | configured / active |
| phase | TEXT | 배포 단계 |
| bridge_address | TEXT | 브릿지 컨트랙트 주소 |
| proposer_address | TEXT | 프로포저 주소 |
| config | TEXT (JSON) | 배포 설정 |

### sessions
| 컬럼 | 타입 | 설명 |
|------|------|------|
| token | TEXT (PK) | `ps_` 접두어 세션 토큰 |
| user_id | TEXT (FK→users) | 유저 ID |
| created_at | BIGINT | 생성 시각 (24시간 후 만료) |

## AI 프록시 (Tokamak AI)

데스크탑 앱(Tokamak Appchain Messenger)에서 사용하는 AI 채팅 프록시입니다.

### 동작 방식

```
데스크탑 앱 → POST /api/ai/chat → Vercel Proxy → https://api.ai.tokamak.network
                  │                      │
                  │ X-Device-Id 헤더     │ 토큰 사용량 기록 (Vercel KV)
                  │                      │
                  └─ 응답 + _tokamak_usage (used/limit/remaining)
```

1. 앱이 `X-Device-Id` 헤더와 함께 채팅 요청
2. 프록시가 KV에서 오늘 사용량 확인 (일일 50,000 토큰 제한)
3. 제한 내이면 `api.ai.tokamak.network`로 요청 전달
4. 응답의 `usage.total_tokens`를 KV에 기록
5. 앱에 응답 + `_tokamak_usage` 정보 반환

### 제한 초과 시
- HTTP 429 응답 + `{ error: "daily_limit_exceeded", usage: {...} }`
- KV 키에 48시간 TTL이 설정되어 자동 정리됨

### 토큰 사용량 조회
```
GET /api/ai/usage?deviceId=xxx
→ { used: 12000, limit: 50000, remaining: 38000 }
```

## 인증

### 세션 기반
- 로그인/회원가입 시 `ps_` 접두어 세션 토큰 발급
- `Authorization: Bearer ps_xxx` 헤더로 인증
- 세션 유효기간: 24시간

### OAuth
- Google: ID Token 검증 (tokeninfo 엔드포인트 사용, SDK 불필요)
- Naver: Authorization Code → Access Token → 프로필 조회
- Kakao: Authorization Code → Access Token → 프로필 조회

### 권한
- `user`: 일반 사용자 (프로그램 등록, 배포 관리)
- `admin`: 관리자 (프로그램 승인/거부, 유저 관리, 전체 통계)

## 초기 데이터

DB 초기화 시 자동으로 생성되는 데이터:

- **system 유저**: 공식 프로그램의 소유자 (`id: 'system'`, `role: 'admin'`)
- **공식 프로그램** 3개:
  - `evm-l2` (EVM L2) — program_type_id: 1
  - `zk-dex` (ZK-DEX) — program_type_id: 2
  - `tokamon` (Tokamon) — program_type_id: 3
