# Phase 3: 멀티역할 플랫폼 — Guest Program Store

이 문서는 Guest Program 모듈화를 기반으로 한 멀티역할 플랫폼 아키텍처를 설계한다.
ChatGPT의 GPTs Store처럼, 사용자가 자신의 컨트랙트+서킷을 등록하면 다른 사람들이
가져다 사용할 수 있는 **Guest Program Store** 모델이다.

> **선행 조건**: Phase 2.1-2.4 완료
> **참조**: `zk-loot-box` 프로젝트 (인증 구성)

---

## 1. 컨셉: GPTs Store 모델

```
┌─────────────────────────────────────────────────────────┐
│                  Guest Program Store                     │
│                                                         │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐              │
│  │ EVM-L2   │  │ ZK-DEX   │  │ Tokamon  │  ...         │
│  │ (공식)   │  │ (개발자A) │  │ (개발자B) │              │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘              │
│       │              │              │                    │
│       ▼              ▼              ▼                    │
│  ┌──────────────────────────────────────┐               │
│  │     사용자가 선택하여 L2 생성         │               │
│  └──────────────────────────────────────┘               │
└─────────────────────────────────────────────────────────┘
```

**ChatGPT GPTs 대비 매핑**:
| GPTs Store | Guest Program Store |
|---|---|
| GPT 생성자 | App Registrar (컨트랙트+서킷 등록) |
| GPT 사용자 | L2 User (등록된 프로그램으로 L2 사용) |
| OpenAI | Platform Manager (플랫폼 관리) |
| GPT (모델+도구) | Guest Program (서킷+컨트랙트+ELF) |
| GPTs Store | Guest Program Store (탐색, 사용) |

---

## 2. 역할 정의

> **핵심 원칙**: L2 User와 App Registrar는 별도 역할이 아니다. **모든 로그인 사용자가
> 프로그램을 사용(User)할 수도 있고, 자신의 프로그램을 등록(Creator)할 수도 있다.**
> GPTs에서 누구나 GPT를 사용하면서 동시에 GPT를 만들 수 있는 것과 동일하다.

### 2.1 플랫폼 매니저 (Platform Manager) — 관리자 전용

- Guest Program 등록 승인/거부
- 전체 플랫폼 모니터링, 사용량 관리
- verifier 등록, VK 관리
- 플랫폼 일시 정지 / 재개
- DB의 `role = 'admin'`인 사용자

### 2.2 일반 사용자 (User = Consumer + Creator)

**하나의 계정으로 두 가지 활동 가능**:

**Consumer로서**:
- Store에서 원하는 Guest Program을 탐색/검색
- 선택한 프로그램으로 L2 인스턴스를 사용
- 트랜잭션 제출, 증명 상태 조회, 자산 입출금

**Creator로서**:
- 자신의 컨트랙트 + 서킷(Guest Program)을 만들어 Store에 등록
- 등록 내용: programId, 설명, ELF 바이너리, VK, 카테고리, 아이콘
- 등록 후 다른 사람들이 가져다 사용 가능
- 자신의 프로그램 통계 확인 (사용 횟수, 배치 수 등)

**역할 전환 없이 네비게이션으로 구분**:
- `/store` → Consumer 모드 (프로그램 탐색/사용)
- `/creator` → Creator 모드 (프로그램 등록/관리)

---

## 3. 시스템 아키텍처

```
┌─────────────────────────────────────────────────────┐
│                    Client (Next.js)                   │
│  ┌───────────┐ ┌───────────┐ ┌───────────────────┐  │
│  │ Store     │ │ Creator   │ │ Dashboard         │  │
│  │ (탐색/    │ │ Console   │ │ (사용자/관리자)    │  │
│  │  사용)    │ │ (등록/    │ │                   │  │
│  │           │ │  관리)    │ │                   │  │
│  └─────┬─────┘ └─────┬─────┘ └─────────┬─────────┘  │
│        │              │                  │            │
│        └──────────────┼──────────────────┘            │
│                       │                               │
│  OAuth: Google, Naver, Kakao                          │
└───────────────────────┼───────────────────────────────┘
                        │ REST API
┌───────────────────────┼───────────────────────────────┐
│                    Server (Express)                    │
│  ┌─────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ │
│  │ Auth    │ │ Store    │ │ Program  │ │ L2       │ │
│  │ Service │ │ API      │ │ Registry │ │ Manager  │ │
│  └────┬────┘ └────┬─────┘ └────┬─────┘ └────┬─────┘ │
│       │           │            │             │        │
│  ┌────┴───────────┴────────────┴─────────────┴────┐  │
│  │                   Database                      │  │
│  │  (SQLite / PostgreSQL)                          │  │
│  └─────────────────────────────────────────────────┘  │
│                       │                               │
└───────────────────────┼───────────────────────────────┘
                        │ On-chain
┌───────────────────────┼───────────────────────────────┐
│                    L1 (Ethereum)                       │
│  ┌──────────────────┐ ┌───────────────────────┐       │
│  │ GuestProgram     │ │ OnChainProposer       │       │
│  │ Registry.sol     │ │ (3D VK mapping)       │       │
│  └──────────────────┘ └───────────────────────┘       │
└───────────────────────────────────────────────────────┘
```

---

## 4. Server (Express.js)

### 4.1 프로젝트 구조

```
platform/
├── server/
│   ├── server.js                 # Express 메인 서버
│   ├── package.json
│   ├── .env                      # 환경 변수
│   ├── middleware/
│   │   └── auth.js               # 인증 미들웨어
│   ├── lib/
│   │   ├── google-auth.js        # Google OAuth
│   │   ├── naver-auth.js         # Naver OAuth
│   │   └── kakao-auth.js         # Kakao OAuth
│   ├── routes/
│   │   ├── auth.js               # /api/auth/*
│   │   ├── store.js              # /api/store/* (프로그램 탐색)
│   │   ├── programs.js           # /api/programs/* (등록/관리)
│   │   ├── admin.js              # /api/admin/* (플랫폼 관리)
│   │   └── users.js              # /api/users/* (사용자 관리)
│   └── db/
│       ├── schema.sql
│       ├── users.js              # 사용자 테이블
│       ├── programs.js           # 프로그램 테이블
│       └── usage.js              # 사용량 테이블
│
├── client/                       # Next.js 앱 (별도 섹션)
└── contracts/                    # Solidity 컨트랙트 (별도 섹션)
```

### 4.2 API 엔드포인트

#### 인증 (zk-loot-box 패턴 참조)

```
POST /api/auth/signup              # 이메일/패스워드 가입
POST /api/auth/login               # 이메일/패스워드 로그인
POST /api/auth/google              # Google OAuth 콜백
POST /api/auth/naver               # Naver OAuth 콜백
POST /api/auth/kakao               # Kakao OAuth 콜백
GET  /api/auth/google-client-id    # Google Client ID 제공
GET  /api/auth/naver-client-id     # Naver Client ID 제공
GET  /api/auth/kakao-client-id     # Kakao Client ID 제공
GET  /api/auth/me                  # 현재 사용자 정보
POST /api/auth/logout              # 로그아웃
```

#### Guest Program Store (공개)

```
GET  /api/store/programs           # 프로그램 목록 (검색, 카테고리, 정렬)
GET  /api/store/programs/:id       # 프로그램 상세 정보
GET  /api/store/programs/:id/stats # 프로그램 사용 통계
GET  /api/store/categories         # 카테고리 목록
GET  /api/store/featured           # 추천 프로그램
```

#### Program Management (Creator — 인증 필요)

```
POST   /api/programs               # 새 프로그램 등록 요청
GET    /api/programs               # 내가 등록한 프로그램 목록
GET    /api/programs/:id           # 프로그램 상세
PUT    /api/programs/:id           # 프로그램 수정
DELETE /api/programs/:id           # 프로그램 삭제/비활성화
POST   /api/programs/:id/elf      # ELF 바이너리 업로드
POST   /api/programs/:id/vk       # VK 업로드
GET    /api/programs/:id/stats    # 사용 통계
```

#### Platform Admin (Manager — 관리자 인증 필요)

```
GET  /api/admin/programs           # 전체 프로그램 목록 (승인 대기 포함)
PUT  /api/admin/programs/:id/approve  # 프로그램 승인
PUT  /api/admin/programs/:id/reject   # 프로그램 거부
GET  /api/admin/users              # 전체 사용자 목록
GET  /api/admin/stats              # 플랫폼 통계
GET  /api/admin/health             # 시스템 상태
```

### 4.3 인증 구성 (zk-loot-box 참조)

zk-loot-box의 인증 패턴을 그대로 채택:

```javascript
// lib/google-auth.js — Google ID Token 검증
async function verifyGoogleIdToken(idToken) {
  const ticket = await client.verifyIdToken({ idToken, audience: GOOGLE_CLIENT_ID });
  const payload = ticket.getPayload();
  return { email: payload.email, name: payload.name, picture: payload.picture };
}

// lib/naver-auth.js — Naver Authorization Code Exchange
async function exchangeNaverCode(code, state) {
  // 1. code → access_token (https://nid.naver.com/oauth2.0/token)
  // 2. access_token → profile (https://openapi.naver.com/v1/nid/me)
  return { email, name, picture };
}

// lib/kakao-auth.js — Kakao Authorization Code Exchange
async function exchangeKakaoCode(code, redirectUri) {
  // 1. code → access_token (https://kauth.kakao.com/oauth/token)
  // 2. access_token → profile (https://kapi.kakao.com/v2/user/me)
  return { email, name, picture };
}
```

세션 관리: 인메모리 세션 (24시간 TTL), `ts_` 접두사 토큰.

### 4.4 데이터베이스 스키마

```sql
-- 사용자
CREATE TABLE users (
  id TEXT PRIMARY KEY,
  email TEXT UNIQUE NOT NULL,
  name TEXT NOT NULL,
  password_hash TEXT,           -- OAuth 사용자는 NULL
  auth_provider TEXT DEFAULT 'email',  -- 'email', 'google', 'naver', 'kakao'
  role TEXT DEFAULT 'user',     -- 'user' (일반, Consumer+Creator), 'admin' (Platform Manager)
  picture TEXT,
  status TEXT DEFAULT 'active', -- 'active', 'suspended', 'pending'
  created_at INTEGER NOT NULL
);

-- Guest Program (Store에 등록되는 프로그램)
CREATE TABLE programs (
  id TEXT PRIMARY KEY,
  program_id TEXT UNIQUE NOT NULL,     -- "zk-dex", "tokamon" 등
  program_type_id INTEGER UNIQUE,      -- L1 매핑용 (자동 할당)
  creator_id TEXT NOT NULL REFERENCES users(id),
  name TEXT NOT NULL,                  -- 표시 이름
  description TEXT,
  category TEXT DEFAULT 'general',
  icon_url TEXT,
  elf_hash TEXT,                       -- SHA-256 of ELF binary
  elf_storage_path TEXT,               -- ELF 파일 경로 (서버 로컬 또는 S3)
  vk_sp1 TEXT,                         -- SP1 verification key (hex)
  vk_risc0 TEXT,                       -- RISC0 verification key (hex)
  status TEXT DEFAULT 'pending',       -- 'pending', 'active', 'rejected', 'disabled'
  use_count INTEGER DEFAULT 0,         -- 사용 횟수
  batch_count INTEGER DEFAULT 0,       -- 처리된 배치 수
  is_official BOOLEAN DEFAULT FALSE,   -- 공식 템플릿 여부
  created_at INTEGER NOT NULL,
  approved_at INTEGER
);

-- 프로그램 사용 기록
CREATE TABLE program_usage (
  id TEXT PRIMARY KEY,
  program_id TEXT NOT NULL REFERENCES programs(id),
  user_id TEXT NOT NULL REFERENCES users(id),
  batch_number INTEGER,
  created_at INTEGER NOT NULL
);
```

---

## 5. Client (Next.js + React)

### 5.1 프로젝트 구조

```
client/
├── app/
│   ├── page.tsx                    # 홈 — Store 메인
│   ├── layout.tsx                  # 루트 레이아웃
│   ├── globals.css
│   ├── store/
│   │   ├── page.tsx                # 프로그램 Store (탐색/검색)
│   │   └── [id]/page.tsx           # 프로그램 상세
│   ├── creator/
│   │   ├── page.tsx                # Creator Console (내 프로그램 관리)
│   │   ├── new/page.tsx            # 새 프로그램 등록
│   │   └── [id]/page.tsx           # 프로그램 편집
│   ├── admin/
│   │   └── page.tsx                # Platform Manager 대시보드
│   ├── signup/page.tsx             # 가입
│   ├── login/page.tsx              # 로그인
│   └── auth/callback/
│       ├── naver/page.tsx          # Naver OAuth 콜백
│       └── kakao/page.tsx          # Kakao OAuth 콜백
├── components/
│   ├── auth-provider.tsx           # AuthContext (세션, OAuth)
│   ├── social-login-buttons.tsx    # Google/Naver/Kakao 버튼
│   ├── google-oauth-wrapper.tsx    # Google Identity Services
│   ├── nav.tsx                     # 네비게이션
│   ├── program-card.tsx            # 프로그램 카드 (Store용)
│   └── program-form.tsx            # 프로그램 등록/편집 폼
├── lib/
│   ├── api.ts                      # API 클라이언트
│   ├── types.ts                    # TypeScript 타입
│   └── constants.ts
├── package.json
├── next.config.ts
├── tsconfig.json
└── tailwind.config.ts
```

### 5.2 주요 페이지

#### Store 메인 (`/store`)
```
┌─────────────────────────────────────────────┐
│  Guest Program Store                         │
│  [검색창____________________] [카테고리 v]    │
│                                              │
│  ⭐ 추천 프로그램                              │
│  ┌──────┐ ┌──────┐ ┌──────┐                │
│  │EVM-L2│ │ZK-DEX│ │Tokamon│               │
│  │공식   │ │@devA │ │@devB  │               │
│  │⬇ 1.2k│ │⬇ 340 │ │⬇ 89  │               │
│  └──────┘ └──────┘ └──────┘                │
│                                              │
│  📂 카테고리별                                │
│  DeFi | Gaming | NFT | Payments | ...        │
└─────────────────────────────────────────────┘
```

#### Creator Console (`/creator`)
```
┌─────────────────────────────────────────────┐
│  My Programs                 [+ 새 프로그램]  │
│                                              │
│  ┌─────────────────────────────────────┐    │
│  │ ZK-DEX     ● Active    사용: 340회   │    │
│  │ type_id=2  배치: 1,200              │    │
│  │ [편집] [통계] [비활성화]              │    │
│  └─────────────────────────────────────┘    │
│                                              │
│  ┌─────────────────────────────────────┐    │
│  │ My-Custom  ◌ Pending   승인 대기     │    │
│  │ [편집] [취소]                        │    │
│  └─────────────────────────────────────┘    │
└─────────────────────────────────────────────┘
```

#### 프로그램 등록 (`/creator/new`)
```
┌─────────────────────────────────────────────┐
│  새 Guest Program 등록                       │
│                                              │
│  프로그램 ID:  [my-payment-app________]      │
│  이름:        [My Payment App_________]      │
│  카테고리:    [Payments v]                    │
│  설명:        [간단한 설명...           ]     │
│  아이콘:      [파일 선택]                     │
│                                              │
│  ELF 바이너리:                               │
│  SP1:    [ELF 파일 업로드]                    │
│  RISC0:  [ELF 파일 업로드] (선택)             │
│                                              │
│  Verification Key:                           │
│  SP1 VK:   [0x..._____________________]     │
│  RISC0 VK: [0x..._____________________]     │
│                                              │
│  [등록 요청]                                  │
└─────────────────────────────────────────────┘
```

### 5.3 인증 UI (zk-loot-box 참조)

로그인/가입 페이지:
```
┌─────────────────────────────────────────────┐
│            Guest Program Store               │
│                                              │
│  [Google로 로그인]          ← @react-oauth   │
│  [네이버로 로그인]          ← OAuth redirect  │
│  [카카오로 로그인]          ← OAuth redirect  │
│                                              │
│  ──────── 또는 ────────                      │
│                                              │
│  이메일:  [________________]                  │
│  비밀번호: [________________]                 │
│                                              │
│  [로그인]        계정이 없나요? [가입하기]      │
└─────────────────────────────────────────────┘
```

OAuth 콜백 처리:
- Google: 클라이언트에서 ID Token 직접 발급 → 서버에서 검증
- Naver: `https://nid.naver.com/oauth2.0/authorize` → `/auth/callback/naver`
- Kakao: `https://kauth.kakao.com/oauth/authorize` → `/auth/callback/kakao`

### 5.4 의존성

```json
{
  "dependencies": {
    "@react-oauth/google": "^0.12.1",
    "next": "^15.1.0",
    "react": "^19.0.0",
    "react-dom": "^19.0.0"
  },
  "devDependencies": {
    "@tailwindcss/postcss": "^4.0.0",
    "tailwindcss": "^4.0.0",
    "typescript": "^5.7.0"
  }
}
```

---

## 6. 온체인 컨트랙트

### 6.1 GuestProgramRegistry.sol (신규)

```solidity
contract GuestProgramRegistry is Ownable2StepUpgradeable, UUPSUpgradeable {

    struct ProgramInfo {
        string  programId;
        address registrar;
        uint8   programTypeId;
        bool    active;
        bytes32 elfHash;
        uint256 registeredAt;
    }

    uint8 public nextProgramTypeId;  // 10부터 시작 (1-9 예약)
    mapping(uint8 => ProgramInfo) public programs;
    mapping(string => uint8) public programIdToTypeId;

    event ProgramRegistered(uint8 indexed programTypeId, string programId, address registrar);
    event ProgramActivated(uint8 indexed programTypeId);
    event ProgramDeactivated(uint8 indexed programTypeId);

    function registerProgram(string calldata programId, bytes32 elfHash)
        external returns (uint8 programTypeId);

    function activateProgram(uint8 programTypeId) external onlyOwner;
    function deactivateProgram(uint8 programTypeId) external;
    function isActive(uint8 programTypeId) external view returns (bool);
}
```

### 6.2 OnChainProposer 연동

`commitBatch()` 시 등록된 프로그램인지 확인:

```solidity
if (address(guestProgramRegistry) != address(0) &&
    programTypeId != DEFAULT_PROGRAM_TYPE_ID) {
    require(
        guestProgramRegistry.isActive(programTypeId),
        "OnChainProposer: program not active"
    );
}
```

### 6.3 programTypeId 할당

| 범위 | 용도 |
|---|---|
| 0 | 예약 (→ DEFAULT=1) |
| 1 | EVM-L2 (공식) |
| 2-9 | 공식 템플릿 (ZK-DEX=2, Tokamon=3, ...) |
| 10-255 | Store 등록 프로그램 (자동 증가) |

---

## 7. 등록 → 사용 플로우

```
Creator (App Registrar)
│
├─ 1. 로그인 (Google/Naver/Kakao/Email)
├─ 2. /creator/new 에서 프로그램 등록 요청
│     - programId, 설명, 카테고리
│     - ELF 업로드 (SP1/RISC0)
│     - VK 제출
│
├─ 3. 서버: DB에 status='pending'으로 저장
│
├─ 4. Platform Manager가 /admin에서 검토
│     - ELF hash 무결성 확인
│     - 보안 감사
│     - 승인 → status='active', L1 registerProgram() 호출
│
├─ 5. L1: GuestProgramRegistry에 등록, programTypeId 자동 할당
├─ 6. L1: upgradeVerificationKey(commitHash, typeId, verifierId, vk) 호출
│
└─ 7. Store에 프로그램 공개!

User (L2 User)
│
├─ 1. /store에서 프로그램 탐색
├─ 2. 원하는 프로그램 선택
├─ 3. 해당 프로그램의 L2에서 트랜잭션 실행
├─ 4. Proof Coordinator가 배치에 program_id 할당
├─ 5. Prover가 해당 ELF로 증명 생성
└─ 6. L1에서 programTypeId 기반 VK로 검증
```

---

## 8. 구현 계획

### Phase 3.1: Server 기초 + 인증

| 작업 | 파일 |
|------|------|
| Express 서버 셋업 | `platform/server/server.js` |
| Google OAuth | `platform/server/lib/google-auth.js` |
| Naver OAuth | `platform/server/lib/naver-auth.js` |
| Kakao OAuth | `platform/server/lib/kakao-auth.js` |
| Auth 미들웨어 | `platform/server/middleware/auth.js` |
| DB 스키마 + users 테이블 | `platform/server/db/` |
| Auth API 라우트 | `platform/server/routes/auth.js` |

### Phase 3.2: Client 기초 + 인증 UI

| 작업 | 파일 |
|------|------|
| Next.js 프로젝트 생성 | `platform/client/` |
| Auth Provider | `platform/client/components/auth-provider.tsx` |
| Social Login 버튼 | `platform/client/components/social-login-buttons.tsx` |
| 로그인/가입 페이지 | `platform/client/app/login/`, `signup/` |
| OAuth 콜백 페이지 | `platform/client/app/auth/callback/` |

### Phase 3.3: Guest Program Store

| 작업 | 파일 |
|------|------|
| Programs DB 테이블 | `platform/server/db/programs.js` |
| Store API (공개 목록) | `platform/server/routes/store.js` |
| Program 등록 API | `platform/server/routes/programs.js` |
| Store 페이지 | `platform/client/app/store/` |
| Program Card 컴포넌트 | `platform/client/components/program-card.tsx` |

### Phase 3.4: Creator Console

| 작업 | 파일 |
|------|------|
| Creator 대시보드 | `platform/client/app/creator/page.tsx` |
| 프로그램 등록 폼 | `platform/client/app/creator/new/page.tsx` |
| ELF 업로드 API | `platform/server/routes/programs.js` |
| VK 등록 API | `platform/server/routes/programs.js` |

### Phase 3.5: Admin & 온체인 연동

| 작업 | 파일 |
|------|------|
| Admin 대시보드 | `platform/client/app/admin/page.tsx` |
| 프로그램 승인/거부 API | `platform/server/routes/admin.js` |
| GuestProgramRegistry.sol | `crates/l2/contracts/src/l1/` |
| OnChainProposer 연동 | `crates/l2/contracts/src/l1/OnChainProposer.sol` |
| Deployer 수정 | `cmd/ethrex/l2/deployer.rs` |

### Phase 3.6: 동적 프로그램 할당

| 작업 | 파일 |
|------|------|
| Proof Coordinator 동적 할당 | `crates/l2/sequencer/proof_coordinator.rs` |
| L1 Committer 동적 typeId | `crates/l2/sequencer/l1_committer.rs` |
| ELF 동적 로드 | `crates/l2/prover/src/prover.rs` |

---

## 9. 환경 변수

```env
# Server
PORT=5001
DATABASE_URL=./db/platform.sqlite

# OAuth (zk-loot-box 패턴)
GOOGLE_CLIENT_ID=...
NAVER_CLIENT_ID=...
NAVER_CLIENT_SECRET=...
KAKAO_REST_API_KEY=...
KAKAO_CLIENT_SECRET=...

# Blockchain
L1_RPC_URL=http://localhost:8545
ON_CHAIN_PROPOSER_ADDRESS=0x...
GUEST_PROGRAM_REGISTRY_ADDRESS=0x...
DEPLOYER_PRIVATE_KEY=0x...

# Client
NEXT_PUBLIC_API_URL=http://localhost:5001
NEXT_PUBLIC_GOOGLE_CLIENT_ID=...
NEXT_PUBLIC_NAVER_CLIENT_ID=...
NEXT_PUBLIC_KAKAO_CLIENT_ID=...
```

---

## 10. 참고 사항

- 인증 구성은 `zk-loot-box` 프로젝트의 패턴을 그대로 채택 (Google ID Token 검증, Naver/Kakao authorization code exchange)
- ELF 바이너리는 온체인에 저장하지 않음 (가스 비용). 서버 파일시스템 또는 S3에 저장, elfHash만 온체인 기록
- 보증금(collateral) 메커니즘은 Phase 3.1에서는 미구현 (향후 추가 가능)
- 기존 ethrex L2 기능과 병행하여 동작 — Store는 위에 올라가는 관리 레이어
