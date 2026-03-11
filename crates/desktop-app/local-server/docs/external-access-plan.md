# External Access (외부 접속) 기능 설계 문서

## 1. 현재 상태 분석

### 1.1 구현된 것

| 항목 | 상태 | 설명 |
|------|------|------|
| DB 스키마 | ✅ | `is_public`, `public_domain`, `public_*_url` 필드 |
| API 라우트 | ✅ | `POST/DELETE /api/deployments/:id/public-access` |
| Docker 포트 바인딩 | ✅ | `isPublic=true` → L2 RPC `0.0.0.0` 바인딩 |
| Manager UI | ✅ | 도메인/IP 입력 모달, Enable/Disable 버튼 |
| Tools URL 자동 생성 | ✅ | `tools-config.js`에서 domain+port로 URL 생성 |

### 1.2 현재 동작 흐름

```
사용자: "Enable Public Access" 클릭
  → 도메인/IP 입력 (예: 203.0.113.50)
  → POST /api/deployments/:id/public-access { publicDomain: "203.0.113.50" }
  → DB 저장: is_public=1, public_domain="203.0.113.50"
  → docker-compose 재생성: L2 RPC 포트 127.0.0.1 → 0.0.0.0
  → docker stop → start (L2 서비스 재시작)
  → tools 재시작 (public URL 반영)
```

### 1.3 포트 노출 현황

| 서비스 | 포트 | isPublic=false | isPublic=true |
|--------|------|----------------|---------------|
| L2 RPC | 1729+ | `127.0.0.1` | `0.0.0.0` ✅ |
| Proof Coordinator | 3900+ | `127.0.0.1` | `127.0.0.1` (항상 내부) |
| Metrics | 3702+ | `127.0.0.1` | `127.0.0.1` (항상 내부) |
| L2 Explorer | tools | localhost | 도메인:port (URL만 변경) |
| L1 Explorer | tools | localhost | 도메인:port (URL만 변경) |
| Dashboard/Bridge | tools | localhost | 도메인:port (URL만 변경) |

### 1.4 문제점

1. **~~L2 RPC만 `0.0.0.0`으로 바인딩~~** ✅ Phase 1에서 수정 완료
2. **~~Tools 포트 항상 `0.0.0.0`~~** ✅ Phase 1에서 수정 — 기본 `127.0.0.1`, public 시 `0.0.0.0`
3. **네트워크 설정은 사용자 몫** — 포트 포워딩, 방화벽, DNS 등 별도 구성 필요
4. **터널링 미지원** — ngrok/cloudflare tunnel 같은 자동 터널 없음
5. **SSL/HTTPS 없음** — HTTP만 지원, 프로덕션 부적합
6. **접속 테스트 없음** — 도메인/IP가 실제 접근 가능한지 검증 안 함

---

## 2. 설계

### 2.1 목표

개인 컴퓨터에서 배포한 L2 앱체인을 **외부에서 접속 가능**하게 만드는 것.

두 가지 시나리오:
- **A. 로컬 배포 + 터널링** — 집/사무실 PC에서 ngrok 등으로 노출
- **B. 테스트넷 배포** — Sepolia L1 기반, 서버에서 0.0.0.0 바인딩

### 2.2 기능 범위

#### Phase 1: Tools 포트 노출 (필수)
현재 L2 RPC만 `0.0.0.0`으로 바꾸는데, Tools(Explorer, Bridge, Dashboard)도 외부 접근 필요.

**변경 파일:**
- `crates/desktop-app/local-server/lib/docker-local.js`
  - `startTools()` / `buildToolsUpArgs()`에서 tools compose의 포트 바인딩도 `0.0.0.0`으로 변경
- `docker-compose-zk-dex-tools.yaml` (또는 tools compose 생성 시)
  - 포트 바인딩: `127.0.0.1:${port}:3000` → `${bindAddr}:${port}:3000`

**노출할 Tools 서비스:**
| 서비스 | 컨테이너 포트 | 외부 포트 변수 |
|--------|---------------|----------------|
| Bridge UI / Dashboard | 3000 | `tools_bridge_ui_port` |
| L2 Explorer (Blockscout) | 4000 | `tools_l2_explorer_port` |
| L1 Explorer (Blockscout) | 4000 | `tools_l1_explorer_port` |
| Proxy (nginx) | 80 | 내부만 |

#### Phase 2: ngrok 터널 자동 설정 (선택)
사용자가 ngrok authtoken만 입력하면 자동으로 터널 생성.

**새 파일:**
- `crates/desktop-app/local-server/lib/tunnel.js`
  - `createTunnel(port, authtoken)` — ngrok TCP 터널 생성
  - `destroyTunnel(port)` — 터널 해제
  - `getTunnelUrl(port)` — 현재 터널 URL 조회

**변경 파일:**
- `crates/desktop-app/local-server/routes/deployments.js`
  - `POST /api/deployments/:id/tunnel` — ngrok 터널 생성
  - `DELETE /api/deployments/:id/tunnel` — 터널 해제
- `crates/desktop-app/local-server/public/app.js`
  - External Access 모달에 "Use ngrok" 옵션 추가

**동작:**
```
사용자: "Use ngrok" 체크 + authtoken 입력
  → L2 RPC 포트에 ngrok tunnel 생성
  → 반환된 URL을 public_domain으로 자동 설정
  → Tools 포트에도 각각 tunnel 생성
  → Dashboard에 모든 외부 URL 표시
```

**고려사항:**
- ngrok 무료 플랜: TCP 터널 1개, HTTP 터널 1개 제한
- 유료 플랜: 여러 터널 가능
- 대안: cloudflared (Cloudflare Tunnel) — 무료 다중 터널 가능

#### Phase 3: 접속 테스트 + 안내 (UX)

**변경 파일:**
- `crates/desktop-app/local-server/routes/deployments.js`
  - `GET /api/deployments/:id/public-access/test` — 외부 접속 테스트
- `crates/desktop-app/local-server/public/app.js`
  - Enable 후 자동으로 접속 테스트 실행
  - 실패 시 가이드 표시 (포트 포워딩 방법, 방화벽 설정 등)

### 2.3 아키텍처 변경

```
현재:
  Manager UI → API → DB 업데이트 → docker-compose 재생성 (L2 RPC만 0.0.0.0)
                                  → L2 재시작
                                  → Tools 재시작 (URL만 변경, 바인딩 불변)

개선 후:
  Manager UI → API → DB 업데이트 → docker-compose 재생성 (L2 RPC 0.0.0.0)
                                  → L2 재시작
                                  → Tools compose 재생성 (Explorer/Bridge도 0.0.0.0)
                                  → Tools 재시작
                                  → (선택) ngrok 터널 생성
                                  → 접속 테스트
```

### 2.4 DB 스키마 변경

```sql
-- 기존 필드 (변경 없음)
is_public INTEGER DEFAULT 0
public_domain TEXT
public_l2_rpc_url TEXT
public_l2_explorer_url TEXT
public_l1_explorer_url TEXT
public_dashboard_url TEXT

-- 추가 필드 (Phase 2)
tunnel_provider TEXT          -- 'ngrok' | 'cloudflared' | null
tunnel_authtoken TEXT         -- 암호화 저장 (keychain)
tunnel_active INTEGER DEFAULT 0
```

---

## 3. 구현 계획

### Phase 1: Tools 포트 노출 (우선)

| # | 작업 | 파일 | 난이도 |
|---|------|------|--------|
| 1-1 | tools compose 포트 바인딩에 `bindAddr` 변수 적용 | `docker-local.js` | 중 |
| 1-2 | `setPublicAccess()`에서 tools compose도 재생성 | `deployment-engine.js` | 중 |
| 1-3 | tools-config에 `isPublic` 상태 전달 | `tools-config.js` | 하 |
| 1-4 | Manager UI에서 Tools 포트 노출 상태 표시 | `app.js` | 하 |

**예상 변경량:** ~100줄

### Phase 2: ngrok 터널 자동화 (선택)

| # | 작업 | 파일 | 난이도 |
|---|------|------|--------|
| 2-1 | ngrok 라이브러리 연동 (또는 CLI 호출) | `tunnel.js` (신규) | 중 |
| 2-2 | 터널 API 라우트 추가 | `deployments.js` | 중 |
| 2-3 | UI에 ngrok 옵션 추가 | `app.js` | 중 |
| 2-4 | authtoken 안전 저장 (keychain) | `keychain.js` | 하 |
| 2-5 | 서버 재시작 시 터널 복원 | `deployment-engine.js` | 중 |

**예상 변경량:** ~300줄 + 새 파일 1개

### Phase 3: UX 개선 (선택)

| # | 작업 | 파일 | 난이도 |
|---|------|------|--------|
| 3-1 | 외부 접속 테스트 API | `deployments.js` | 하 |
| 3-2 | 접속 실패 시 트러블슈팅 가이드 | `app.js` | 하 |
| 3-3 | 공개 상태 대시보드 위젯 | `app.js` | 하 |

**예상 변경량:** ~100줄

---

## 4. 보안 고려사항

| 위험 | 대응 |
|------|------|
| L2 RPC 무제한 노출 | rate limiting 없음 — 현재는 테스트 용도로 허용 |
| Proof Coordinator 노출 | `127.0.0.1` 고정 — 변경 금지 |
| Metrics 노출 | `127.0.0.1` 고정 — 변경 금지 |
| ngrok authtoken 유출 | keychain에 암호화 저장 |
| 악의적 RPC 요청 | L2 노드 자체 보호에 의존 (ethrex RPC 레이어) |

---

## 5. 우선순위 결정

**Phase 1만 먼저 구현 권장.**

이유:
- ngrok 없이도 서버/VPS에서는 바로 사용 가능
- Tools 포트 노출이 빠져 있는 것이 가장 큰 갭
- ngrok은 npm 의존성 추가 또는 CLI 설치 필요 — 복잡도 높음

Phase 2는 사용자 피드백 후 별도 브랜치에서 진행.
