# GP Store를 통한 L2 런칭 플랫폼

## 개요

GP Store는 Guest Program 마켓플레이스이자 **L2 설정 생성 도구**이다.
사용자는 스토어에서 Guest Program을 선택하고, L2 체인 설정을 구성한 뒤,
Docker 기반으로 로컬에서 실행할 수 있는 설정 파일을 다운로드받는다.

**핵심 플로우:**
```
Browse Store → Select Program → Configure L2 → Download Config → docker-compose up
```

## 빌트인 프로그램

스토어에는 3개의 공식(Official) 프로그램이 기본 제공된다:

| Program | Type ID | Category | 설명 |
|---------|---------|----------|------|
| **EVM L2** | 1 | defi | 기본 Ethereum 실행 환경. 범용 L2 체인을 위한 완전한 EVM 호환성. |
| **ZK-DEX** | 2 | defi | 온체인 주문 매칭 및 정산에 최적화된 탈중앙 거래소 회로. |
| **Tokamon** | 3 | gaming | 검증 가능한 게임 상태 전이와 온체인 게이밍을 위한 회로. |

이 프로그램들은 서버 시작 시 DB에 자동으로 시드된다 (`seedOfficialPrograms`).
`creator_id`는 `'system'`이며, `is_official = 1`, `status = 'active'`로 등록된다.

## 사용자 플로우

### 1단계: 스토어 탐색 (`/store`)

스토어 페이지에서 사용 가능한 Guest Program을 탐색한다.
카테고리 필터와 검색 기능을 통해 원하는 프로그램을 찾을 수 있다.

### 2단계: 프로그램 선택 → L2 런치 (`/launch`)

2단계 마법사 UI:
- **Step 1**: 프로그램 선택 (카드 형태, 검색/카테고리 필터)
  - `?program=<id>` 쿼리 파라미터로 딥링크 지원
- **Step 2**: L2 설정 구성
  - L2 이름 (필수, 기본값: `"{program.name} L2"`)
  - Chain ID (선택)
  - L1 RPC URL (선택)
  - "Launch L2" 버튼 클릭 → Deployment 생성 → 상세 페이지로 이동

### 3단계: 설정 파일 다운로드 (`/deployments/{id}`)

생성된 L2 상세 페이지에서 두 가지 설정 파일을 다운로드:

1. **ethrex-l2-config.toml** — L2 노드 구성 (네트워크, 프루버, 시퀀서 설정)
2. **docker-compose.yml** — Docker 컨테이너 실행 설정

### 4단계: 로컬 실행

```bash
mkdir my-l2 && cd my-l2
# 다운로드한 파일들을 이 디렉토리에 배치
docker-compose up -d
docker-compose logs -f
```

## Docker 기반 배포 아키텍처

```yaml
version: "3.8"
services:
  ethrex-l2:
    image: ghcr.io/tokamak-network/ethrex:latest
    ports:
      - "8546:8546"
    volumes:
      - ./ethrex-l2-config.toml:/etc/ethrex/config.toml
    environment:
      - GUEST_PROGRAM_ID={program_slug}
      - GUEST_PROGRAM_URL={API_URL}/uploads/{program_id}/elf
```

- **이미지**: `ghcr.io/tokamak-network/ethrex:latest` — ethrex 공식 이미지
- **포트**: `8546` — L2 RPC 엔드포인트
- **볼륨**: TOML 설정 파일을 컨테이너 내부에 마운트
- **환경변수**:
  - `GUEST_PROGRAM_ID`: Guest Program의 slug (예: `evm-l2`)
  - `GUEST_PROGRAM_URL`: ELF 바이너리 다운로드 URL

ethrex 노드는 시작 시 `GUEST_PROGRAM_URL`에서 ELF 바이너리를 동적으로 로딩하여
`DynamicGuestProgram`으로 실행한다.

## 플랫폼 아키텍처

```
┌─────────────────────────────────────────────┐
│              Next.js Client (3000)           │
│                                             │
│  /           홈페이지 (Hero + How It Works) │
│  /store      Guest Program 스토어           │
│  /store/:id  프로그램 상세                  │
│  /launch     L2 런치 마법사                 │
│  /deployments  My L2s 목록                  │
│  /deployments/:id  L2 상세 + 설정 다운로드  │
│  /creator    내 프로그램 관리               │
│  /admin      관리자 대시보드                │
│  /profile    프로필                         │
└────────────────┬────────────────────────────┘
                 │ REST API
┌────────────────▼────────────────────────────┐
│            Express Server (5001)            │
│                                             │
│  /api/store/*      공개 스토어 API          │
│  /api/programs/*   프로그램 CRUD (인증)     │
│  /api/deployments/* 배포 CRUD (인증)        │
│  /api/auth/*       인증 (이메일/OAuth)      │
│  /api/admin/*      관리자 API               │
└────────────────┬────────────────────────────┘
                 │
┌────────────────▼────────────────────────────┐
│           SQLite (better-sqlite3)           │
│                                             │
│  users, programs, deployments,              │
│  program_versions, program_usage, sessions  │
└─────────────────────────────────────────────┘
```

## "My L2s"의 범위

현재 구현에서 "My L2s"(`/deployments`)는 **설정 기록 관리 도구**이다:

- L2 설정(이름, Chain ID, RPC URL) 저장 및 수정
- 설정 파일(TOML, docker-compose.yml) 생성 및 재다운로드
- L2 생성 기록 목록 관리

사이트에서 실제 L2 인스턴스를 시작/중지/모니터링하는 기능은 제공하지 않는다.
실행은 사용자가 로컬 Docker 환경에서 직접 수행한다.

## 네비게이션 구조

```
GP Store | Store | Launch L2* | My Programs* | My L2s* | Admin**
```
- `*` = 인증 필요
- `**` = 관리자 전용
