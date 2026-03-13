# Showroom Social Infrastructure Guide

> 작성일: 2026-03-13
> 브랜치: feat/showroom-social-ui

---

## 개요

Showroom 소셜 기능(리뷰, 댓글, 좋아요)의 백엔드 인프라를 Platform DB(SQLite)에 구축한다.
기존 Nostr 기반 코드는 유지하되, Platform DB를 primary backend으로 사용한다.

### 왜 Nostr 대신 Platform DB인가?

| 항목 | Nostr Relay | Platform DB |
|------|------------|-------------|
| 외부 인프라 | `wss://relay.tokamak.network` 서버 필요 | 불필요 (기존 SQLite) |
| 즉시 동작 | Relay 배포 후 | 코드 머지 즉시 |
| 데이터 제어 | Relay 운영자 | 우리 서버 |
| 스팸 방지 | Relay 설정 의존 | DB 레벨 제약 + API 검증 |
| 인증 | Nostr keypair | EVM 지갑 서명 (동일) |

---

## 아키텍처

```
┌─────────────────────────────────────────────┐
│          Showroom Frontend (Next.js)         │
│  /showroom (목록) + /showroom/[id] (상세)     │
└─────┬───────────────────────────┬────────────┘
      │ REST API                  │ Wallet Sign
      ▼                          ▼
┌─────────────────┐    ┌──────────────────────┐
│ Platform Server  │    │ MetaMask / EVM Wallet │
│ (Express.js)     │    └──────────────────────┘
│                  │
│ wallet-auth.js   │ ← ethers.verifyMessage()
│ routes/store.js  │ ← Social API endpoints
│ db/social.js     │ ← CRUD queries
└─────┬────────────┘
      │
      ▼
┌─────────────────┐
│ SQLite (platform │
│  .sqlite)        │
│                  │
│ reviews          │
│ comments         │
│ reactions        │
└──────────────────┘
```

---

## 인증 방식: EVM 지갑 서명

### 플로우
```
1. 유저가 "Sign in with Wallet" 클릭
2. MetaMask에서 고정 메시지 서명 요청
   → "Sign in to Tokamak Appchain Showroom\n\nDomain: platform.tokamak.network\nPurpose: Social interaction authentication\n\nThis signature proves you own this wallet."
3. 서명(signature) + 지갑주소(address) → API 요청 헤더에 포함
4. 서버: ethers.verifyMessage(message, signature) → 주소 복원 → 대조
5. 검증 통과 시 wallet_address를 작성자 ID로 사용
```

### 특징
- **Stateless**: 세션/토큰 없음. 매 요청마다 서명 포함
- **Deterministic**: 같은 지갑 = 항상 같은 identity
- **Platform 계정 불필요**: 지갑만 있으면 리뷰/댓글 가능

---

## DB 스키마

### reviews 테이블
```sql
CREATE TABLE IF NOT EXISTS reviews (
  id TEXT PRIMARY KEY,
  deployment_id TEXT NOT NULL REFERENCES deployments(id) ON DELETE CASCADE,
  wallet_address TEXT NOT NULL,        -- 소문자 hex (0x...)
  rating INTEGER NOT NULL CHECK(rating >= 1 AND rating <= 5),
  content TEXT NOT NULL,
  created_at INTEGER NOT NULL          -- Date.now() (ms)
);
-- 배포당 지갑당 1개 리뷰 (업데이트 가능)
CREATE UNIQUE INDEX IF NOT EXISTS idx_reviews_unique ON reviews(deployment_id, wallet_address);
```

### comments 테이블
```sql
CREATE TABLE IF NOT EXISTS comments (
  id TEXT PRIMARY KEY,
  deployment_id TEXT NOT NULL REFERENCES deployments(id) ON DELETE CASCADE,
  wallet_address TEXT NOT NULL,
  content TEXT NOT NULL,
  parent_id TEXT REFERENCES comments(id) ON DELETE CASCADE,  -- 대댓글 지원
  created_at INTEGER NOT NULL
);
```

### reactions 테이블
```sql
CREATE TABLE IF NOT EXISTS reactions (
  id TEXT PRIMARY KEY,
  target_type TEXT NOT NULL CHECK(target_type IN ('review', 'comment')),
  target_id TEXT NOT NULL,
  wallet_address TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
-- 대상당 지갑당 1개 좋아요 (토글)
CREATE UNIQUE INDEX IF NOT EXISTS idx_reactions_unique ON reactions(target_type, target_id, wallet_address);
```

---

## API 엔드포인트

| Method | Path | Auth | 설명 |
|--------|------|------|------|
| GET | `/api/store/appchains/:id/reviews` | 불필요 | 리뷰 목록 + 좋아요 수 |
| POST | `/api/store/appchains/:id/reviews` | 지갑 | 리뷰 작성 (1인 1리뷰) |
| DELETE | `/api/store/appchains/:id/reviews/:reviewId` | 지갑 | 본인 리뷰 삭제 |
| GET | `/api/store/appchains/:id/comments` | 불필요 | 댓글 목록 + 좋아요 수 |
| POST | `/api/store/appchains/:id/comments` | 지갑 | 댓글 작성 |
| DELETE | `/api/store/appchains/:id/comments/:commentId` | 지갑 | 본인 댓글 삭제 |
| POST | `/api/store/appchains/:id/reactions` | 지갑 | 좋아요 토글 |

### 지갑 인증 헤더
```
x-wallet-address: 0x1234...5678
x-wallet-signature: 0xabcd...ef01
```

---

## 구현 파일 목록

### 서버 (신규)
| 파일 | 설명 |
|------|------|
| `platform/server/db/social.js` | 소셜 DB 쿼리 (CRUD + 집계) |
| `platform/server/lib/wallet-auth.js` | EVM 서명 검증 미들웨어 |

### 서버 (수정)
| 파일 | 변경 |
|------|------|
| `platform/server/db/schema.sql` | reviews, comments, reactions 테이블 추가 |
| `platform/server/routes/store.js` | 소셜 API 엔드포인트 추가 |
| `platform/server/package.json` | `ethers` 의존성 추가 |

### 클라이언트 (수정)
| 파일 | 변경 |
|------|------|
| `platform/client/lib/api.ts` | `socialApi` 객체 추가 |
| `platform/client/lib/nostr.ts` | `connectWalletForApi()` 함수 추가 |
| `platform/client/app/showroom/[id]/page.tsx` | Community 섹션을 API 기반으로 전환 |
| `platform/client/app/showroom/page.tsx` | 카드에 평균 별점, 리뷰/댓글 수 표시 |

---

## 셋업 순서

```bash
# 1. ethers 설치 (서명 검증용)
cd platform/server && npm install ethers

# 2. 서버 재시작 (schema.sql 자동 마이그레이션)
npm run dev

# 3. 클라이언트 (추가 의존성 없음)
cd ../client && npm run dev
```

CORS 설정은 기존 `cors()` 미들웨어가 custom headers를 허용하므로 변경 불필요.
(`x-wallet-address`, `x-wallet-signature`는 simple request가 아니므로 preflight 발생 → cors 미들웨어가 자동 처리)

---

## 스팸 방지

1. **DB 레벨**: `UNIQUE INDEX` — 배포당 1리뷰, 대상당 1좋아요
2. **API 레벨**: content 길이 제한 (500자), rating 범위 (1-5)
3. **서버 레벨**: 기존 rate limiter (IP당 100 req/min)
4. **지갑 레벨**: 댓글 rate limit (지갑당 10 comments/min)
