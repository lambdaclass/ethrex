# Deployment Engine Refactoring: Build-Only + 감사 수정사항

> Date: 2026-03-05
> Branch: `feat/l2-deployment-engine`

## 변경 배경

앱마다 서킷(guest program)과 컨트랙트가 다르므로 Docker 이미지도 앱별로 달라야 합니다.
앱이 100개면 미리 이미지를 만들어놓을 수 없으므로 **pull 전략을 제거하고 build-only로 전환**했습니다.
한 컴퓨터에 여러 앱을 설치할 수 있으므로 **이미지 이름에 deployment ID를 포함**시켜 충돌을 방지합니다.

## 변경 요약

### 1. compose-generator.js — pull 제거, build-only, 이미지 이름 개별화

| 항목 | Before | After |
|------|--------|-------|
| 전략 | `strategy` 파라미터 (`"pull"` / `"build"`) | 항상 build |
| 이미지 이름 | 공유 (`ethrex:main`, `ethrex:sp1`) | 고유 (`ethrex:{slug}-{projectName}`) |
| DEPLOY_RICH | 하드코딩 `true` | 프로필 기반 (`evm-l2`: true, `zk-dex`: false) |
| exports | `getRequiredImages`, `IMAGE_REGISTRY` 포함 | 제거 |

**이미지 이름 예시:**
```
# 기존
ethrex:main, ethrex:sp1

# 변경 후
ethrex:l1-tokamak-08cab1ae
ethrex:zk-dex-tokamak-08cab1ae
```

**APP_PROFILES 변경:**
- `image` 필드 제거 (이제 런타임에 생성)
- `deployRich` 필드 추가 (boolean)

### 2. deployment-engine.js — pull 로직 제거, tools lifecycle

**provision() 변경:**
- `strategy` 파싱 제거 → `initialPhase`를 항상 `"building"`으로
- `getRequiredImages` import 제거
- pull 분기(`else` 블록: `imageExists`, `buildAppImages`) 삭제
- `generateComposeFile()` 호출에서 `strategy` 파라미터 제거

**stopDeployment() / destroyDeployment() 변경:**
- `docker.stopTools()` 호출 추가 (non-fatal try/catch)
- Tools가 실행 중이었다면 배포 중지/삭제 시 함께 정리됨

### 3. docker-local.js — 미사용 함수 삭제

삭제된 함수:
- `pullImages()` — pull 전략 미사용
- `imageExists()` — pull 전략에서만 사용
- `buildPlatformImages()` — pull 전략에서만 사용
- `buildAppImages()` — pull 전략에서만 사용
- `dockerBuild()` — 위 함수들의 헬퍼

유지된 함수:
- `buildImages()` — compose build로 대체 (docker compose build)
- `startTools()` / `stopTools()` — 지원 도구 관리

### 4. deployment-status.tsx — 누락 phase 추가

`PHASE_STYLES`와 `isAnimating`에 추가:
- `checking_docker` (yellow, "Checking Docker")
- `starting_tools` (yellow, "Starting Tools")

### 5. deployments/[id]/page.tsx — phase 처리 + tool 엔드포인트

**isDeploying 배열:**
```
["checking_docker", "building", "l1_starting", "deploying_contracts", "l2_starting", "starting_prover", "starting_tools"]
```

**Tool 엔드포인트 카드 (running 상태일 때):**
| Tool | URL |
|------|-----|
| L1 Block Explorer | http://127.0.0.1:8083 |
| L2 Block Explorer | http://127.0.0.1:8082 |
| Bridge UI / Dashboard | http://127.0.0.1:3000 |

Grid: `grid-cols-1 sm:grid-cols-2 lg:grid-cols-3`

### 6. routes/deployments.js — pulling phase 정리

provision in-progress 체크 배열에서 `"pulling"` 제거 (로컬에서 미사용).
Remote provisioning에서는 `provisionRemote()`가 자체적으로 pulling phase를 관리.

## 수정 파일 목록

| # | 파일 | 변경 유형 |
|---|------|----------|
| 1 | `platform/server/lib/compose-generator.js` | 대규모 리팩토링 |
| 2 | `platform/server/lib/deployment-engine.js` | 중규모 리팩토링 |
| 3 | `platform/server/lib/docker-local.js` | 함수 삭제 |
| 4 | `platform/client/components/deployment-status.tsx` | 소규모 추가 |
| 5 | `platform/client/app/deployments/[id]/page.tsx` | 중규모 추가 |
| 6 | `platform/server/routes/deployments.js` | 소규모 수정 |

## 검증 방법

```bash
# 1. 서버 로드 확인
cd platform/server
node -e "require('./lib/deployment-engine'); console.log('OK')"

# 2. Compose 파일 생성 확인
node -e "
const { generateComposeFile } = require('./lib/compose-generator');
const yaml = generateComposeFile({
  programSlug: 'zk-dex',
  l1Port: 8545, l2Port: 1729, proofCoordPort: 3900,
  projectName: 'tokamak-08cab1ae'
});
console.log(yaml);
"

# 3. TypeScript 타입 체크
cd platform/client
npx tsc --noEmit

# 4. 실제 배포 테스트 (Docker 필요)
# Launch → Deploy → 진행 단계 UI 확인
# 완료 후 detail page에서 Blockscout/Bridge UI 링크 확인
```

## 스코프 밖 (향후 작업) — 모두 완료

- ~~Tools compose 포트 동적화~~ — ✅ `TOOLS_*_PORT` 환경변수로 동적 할당
- ~~GPU 감지 및 compose override~~ — ✅ `hasNvidiaGpu()` + NVIDIA device reservation
- ~~Metrics 포트 노출~~ — ✅ `toolsMetricsPort` DB + compose 연동
- ~~Deployer exit code 검증~~ — ✅ bridge/proposer 주소 null 검증 + 에러 throw
- `dumpFixtures` compose 옵션 — ✅ 배포 config에서 fixture 수집 활성화
