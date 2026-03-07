# Tokamak Platform (Showroom)

Guest Program Store, Open Appchain 레지스트리, 사용자 인증을 담당하는 웹 서비스.

> 배포/Docker 실행은 Desktop App(Factory)에서 담당합니다. Platform은 CRUD + 카탈로그 역할만 합니다.

## Architecture

```
platform/
├── server/                # Express API Server (port 5001)
│   ├── server.js          # Entry point, middleware, routes
│   ├── routes/
│   │   ├── auth.js        # Email/Google 인증, 세션 관리
│   │   ├── store.js       # Program Store (public 조회)
│   │   ├── programs.js    # Program CRUD (creator 전용)
│   │   ├── deployments.js # Open Appchain 등록/관리
│   │   └── admin.js       # 관리자 (프로그램 승인 등)
│   ├── db/
│   │   ├── db.js          # SQLite 연결 (platform.sqlite)
│   │   ├── schema.sql     # 테이블 정의
│   │   ├── users.js       # 사용자 CRUD
│   │   ├── programs.js    # 프로그램 CRUD
│   │   ├── sessions.js    # 세션 관리
│   │   └── deployments.js # 배포 등록 CRUD
│   └── uploads/           # ELF 파일 업로드 저장소
├── client/                # Next.js 15 웹 클라이언트 (port 3000)
│   ├── app/
│   │   ├── store/         # Program Store 페이지
│   │   ├── auth/, login/, signup/ # 인증 페이지
│   │   ├── creator/       # 프로그램 등록/관리
│   │   ├── deployments/   # Open Appchain 목록
│   │   ├── admin/         # 관리자 페이지
│   │   ├── settings/      # 계정 설정
│   │   └── guide/, launch/ # 가이드, 런치 페이지
│   └── lib/
│       └── api.ts         # API 클라이언트
├── dev.sh                 # 로컬 개발 실행 스크립트
├── firebase.json          # Firebase Hosting 설정
└── tests/                 # API 테스트
```

## Database Tables

| Table | Description |
|-------|-------------|
| `users` | 사용자 (email/Google, role: user/admin) |
| `programs` | Guest Program (ELF, VK, 카테고리, 승인 상태) |
| `program_versions` | ELF 업로드 히스토리 |
| `program_usage` | 프로그램 사용 로그 |
| `deployments` | Open Appchain 등록 (chain_id, rpc_url 등) |
| `sessions` | 인증 세션 토큰 |

## API Endpoints

### Public (인증 불필요)
- `GET /api/health` - Health check
- `GET /api/store` - Program Store 목록
- `GET /api/store/:id` - 프로그램 상세
- `POST /api/auth/login` - 이메일 로그인
- `POST /api/auth/signup` - 회원가입
- `POST /api/auth/google` - Google 로그인

### Authenticated
- `GET /api/auth/me` - 내 정보
- `POST /api/auth/logout` - 로그아웃
- `GET/POST /api/programs` - 내 프로그램 CRUD
- `GET/POST /api/deployments` - Open Appchain 등록/관리
- `PUT /api/deployments/:id/activate` - Appchain 활성화

### Admin
- `GET /api/admin/programs` - 전체 프로그램 (승인 대기 포함)
- `PUT /api/admin/programs/:id/approve` - 프로그램 승인

## Local Development

```bash
# 전체 서비스 한번에 실행
cd platform && ./dev.sh

# 개별 실행
cd platform/server && npm install && npm run dev   # API Server (port 5001)
cd platform/client && npm install && npx next dev  # Client (port 3000)
```

### 서비스 URL
- API Server: http://localhost:5001
- Client: http://localhost:3000
- Health: http://localhost:5001/api/health

### 환경변수 (.env)
```
PORT=5001
JWT_SECRET=your-secret
GOOGLE_CLIENT_ID=your-google-client-id
CORS_ORIGINS=http://localhost:3000,http://localhost:1420
```

## Production Deployment

```bash
# Next.js static export + Firebase Hosting
cd platform/client && npm run build
firebase deploy --only hosting
```

## Desktop App 연동

Desktop App에서 Platform API를 사용하는 기능:
- **Program Store 브라우저**: `GET /api/store` (인증 불필요)
- **Open Appchain 등록**: `POST /api/deployments` (Keychain 토큰 사용)
- **Settings 로그인**: `POST /api/auth/login` (토큰을 OS Keychain에 저장)
