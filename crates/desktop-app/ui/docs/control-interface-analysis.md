# 앱체인 제어 인터페이스 분석

## 1. 현재 구조 — 상태 소스 3개

| 컴포넌트 | 상태 소스 | 파일 | 갱신 주기 |
|----------|----------|------|----------|
| **매니저** (local-server) | SQLite `deployments` 테이블 | `local-server/db/deployments.js` | 실시간 (DB write) |
| **메신저** (Tauri/React) | SQLite 읽기 + Docker 라이브 reconciliation | `MyL2View.tsx`, `L2DetailView.tsx` | 5초 폴링 |
| **텔레그램 봇** | AppchainManager(JSON) + Docker 라이브 | `telegram_bot.rs` | 5분 헬스체크 |

### 레거시: `appchains.json` (AppchainManager)
- 경로: `~/.tokamak-appchain/appchains.json`
- 메신저의 `update_appchain_public()`, 텔레그램 봇의 앱체인 상태에서 사용
- **SQLite와 동기화되지 않음** → 불일치 위험

---

## 2. 상태 값 매핑

### SQLite DB (매니저 기준)
| 필드 | 값 | 의미 |
|------|-----|------|
| `status` | configured, active | DB 수준 상태 (거의 사용 안됨) |
| `phase` | configured → checking_docker → building → l1_starting → deploying_contracts → l2_starting → starting_prover → starting_tools → running / stopped / error | 배포 생명주기 |

### 메신저 UI 상태
| UI 상태 | 조건 |
|---------|------|
| `running` | 모든 컨테이너 running |
| `stopped` | 컨테이너 없음 또는 모두 중지 |
| `starting` | phase가 배포 중 |
| `error` | 컨테이너 exited/dead |
| `created` | phase가 configured |

### 텔레그램 봇 상태
| 상태 | 소스 |
|------|------|
| Appchain: Running/Stopped/Error | AppchainManager + ProcessRunner.is_running() |
| Deployment: running/stopped/partial | Docker 라이브 컨테이너 상태 |

---

## 3. 제어 액션 비교

| 액션 | 매니저 | 메신저 | 텔레그램 |
|------|--------|--------|----------|
| **Start** | POST /api/deployments/:id/start | invoke('start_docker_deployment') | [ACTION:start_deployment] |
| **Stop** | POST /api/deployments/:id/stop | invoke('stop_docker_deployment') | [ACTION:stop_deployment] |
| **Destroy** | POST /api/deployments/:id/destroy | invoke('delete_docker_deployment') | [ACTION:delete_deployment] (확인 필요) |
| **Provision** | POST /api/deployments/:id/provision | L2 매니저 열기 (외부 위임) | [ACTION:create_appchain] |
| **개별 서비스 시작/중지** | POST /api/.../service/:svc/start,stop | 없음 | 없음 |
| **Start All / Stop All** | 버튼 (조건부) | 버튼 (조건부) | 자연어 |

---

## 4. Reconciliation 흐름 (수정 후)

```
Docker 컨테이너 (진짜 상태)
    ↓ 5초 폴링 (메신저), 5분 (텔레그램)
    ↓
┌─────────────────────────────────────────┐
│ MyL2View: loadDeployments()             │
│ 1. SQLite 읽기 (deployments 테이블)     │
│ 2. Docker 컨테이너 라이브 조회          │
│ 3. Reconcile:                           │
│    - status, phase, description 동기화  │
│    - sequencerStatus, proverStatus 동기화│
│    - errorMessage 설정/해제             │
└──────────────────┬──────────────────────┘
                   ↓ 클릭
┌──────────────────┴──────────────────────┐
│ L2DetailView: useMemo 실시간 파생       │
│ - l2Prop (스냅샷) + containers (라이브) │
│ - 헤더 health 상태 실시간 반영          │
│ - 컨테이너 변경 시 자동 갱신            │
└──────────────────┬──────────────────────┘
                   ↓
┌──────────────────┴──────────────────────┐
│ L2DetailServicesTab                     │
│ - 개별 서비스: containers 기반 상태     │
│ - 버튼: allStopped → Start All          │
│         anyRunning → Stop All           │
└─────────────────────────────────────────┘
```

---

## 5. 발견된 문제점 및 개선사항

### P0 — 이번에 수정 완료

| # | 문제 | 수정 |
|---|------|------|
| 1 | phase/description이 DB값(stale) 그대로 표시 | reconciliation에서 phase, description, sequencerStatus 모두 동기화 |
| 2 | L2DetailView에서 l2 prop이 클릭 시점 스냅샷 | useMemo로 containers 기반 실시간 상태 파생 |
| 3 | Start All / Stop All 항상 둘 다 표시 | 컨테이너 상태 기반 조건부 표시 |
| 4 | error 상태에서 Start 버튼 없음 | error에서도 Start 버튼 표시 |

### P1 — 향후 개선 필요

| # | 문제 | 영향 | 제안 |
|---|------|------|------|
| 5 | `appchains.json`과 SQLite 이중 상태 | 텔레그램 봇이 JSON 기반 → 매니저/메신저와 불일치 가능 | AppchainManager를 SQLite로 통합 |
| 6 | 컨테이너 조회 실패 시 무시 (catch → return l2) | local-server 꺼지면 stale 상태 유지 | "상태 확인 불가" UI 표시 |
| 7 | partial 상태에서 status='running' + errorMessage 혼재 | UX 혼란 (녹색 dot + 에러 메시지) | 별도 'degraded' 상태 추가 |
| 8 | 동시 Start/Stop/Delete 경쟁 조건 | 같은 deployment에 동시 명령 가능 | per-deployment 뮤텍스 |
| 9 | Start 후 즉시 fetchContainers → 아직 미반영 | UI 깜빡임 | 1-2초 딜레이 후 fetch |
| 10 | 매니저 개별 서비스 제어 있으나 메신저에 없음 | 기능 격차 | 메신저에도 per-service 제어 추가 |

### P2 — 장기 개선

| # | 문제 | 제안 |
|---|------|------|
| 11 | Public toggle 낙관적 업데이트 → 실패 시 롤백 없음 | try/catch에서 setIsPublic 롤백 |
| 12 | Prover 시작 후 헬스체크 없음 | Prover 상태 모니터링 추가 |
| 13 | 텔레그램 헬스체크 5분 → 느림 | 1분으로 단축 또는 이벤트 기반 |
