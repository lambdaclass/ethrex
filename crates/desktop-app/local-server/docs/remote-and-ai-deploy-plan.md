# 원격 배포 & AI 배포 설계 문서

## 1. 현재 상태 분석

### 1.1 배포 모드 비교

| 모드 | L1 | 이미지 | 키 관리 | Tools | 상태 |
|------|-----|--------|---------|-------|------|
| **Local** | Docker 컨테이너 | 소스 빌드 | 하드코딩 dev 키 | ✅ 완전 지원 | ✅ 완성 |
| **Testnet** | 외부 RPC (Sepolia) | 소스 빌드 | Keychain | ✅ 완전 지원 | ✅ 완성 |
| **Remote** | Docker 컨테이너 | 레지스트리 pull | 하드코딩 dev 키 | ❌ 미지원 | ⚠️ 부분 완성 |
| **Manual** | 없음 | 없음 | 없음 | ❌ 없음 | ❌ 스텁만 |

### 1.2 이미 구현된 것

#### Remote 배포 (부분 완성)
- **SSH 연결**: `docker-remote.js` — connect, exec, uploadFile, testConnection
- **호스트 관리**: DB 스키마 + CRUD API + UI
- **원격 실행**: deployRemote, stopRemote, startRemote, destroyRemote, getStatusRemote
- **Compose 생성**: `generateRemoteComposeFile()` — 레지스트리 이미지 pull 방식
- **프로비저닝**: `provisionRemote()` — SSH → compose 업로드 → pull → start

#### 미완성 부분
1. **레지스트리 이미지 없음** — `ETHREX_IMAGE_REGISTRY` 설정은 있지만 실제 push된 이미지 없음
2. **Tools 미지원** — Remote에서 Blockscout, Bridge UI 실행 불가
3. **Testnet + Remote 조합 없음** — Remote는 로컬 L1만 지원
4. **키 관리** — Remote는 dev 키 하드코딩, Keychain 미연동

### 1.3 핵심 파일 구조

```
lib/
├── compose-generator.js    # 3가지 compose 생성 (local/testnet/remote)
├── deployment-engine.js    # 3가지 provision (local/testnet/remote)
├── docker-local.js         # 로컬 Docker 실행
├── docker-remote.js        # SSH 원격 Docker 실행
├── tools-config.js         # Tools 포트/URL 설정
└── keychain.js             # macOS Keychain 연동

routes/
├── deployments.js          # 배포 API
└── hosts.js                # 원격 호스트 API

public/
└── app.js                  # 매니저 UI
```

---

## 2. 목표

### 2.1 원격 서버 배포 (Remote Deploy)
매니저 UI에서 GCP/AWS VM을 선택하고, 클릭 한 번으로 L2 앱체인을 원격 배포.

### 2.2 AI 배포 (AI Deploy)
매니저에서 배포 설정을 선택하면, AI가 실행 가능한 완전한 프롬프트를 생성.
사용자가 Claude Code 등에 붙여넣으면 AI가 자동으로 클라우드 배포 수행.

---

## 3. 원격 서버 배포 설계

### 3.1 현재 Remote 모드의 한계

현재 `provisionRemote()`는:
- 로컬 L1 전용 (외부 L1 미지원)
- dev 키 하드코딩
- Tools(Explorer, Dashboard) 미지원
- 레지스트리 이미지 미준비

### 3.2 개선된 원격 배포 흐름

```
[매니저 UI]
  1. 앱 선택 (zk-dex 등)
  2. 배포 모드: "Remote + Testnet" 선택
  3. 원격 호스트 선택/추가 (SSH 정보)
  4. L1 설정: Sepolia RPC URL
  5. 키 설정: Deployer 키 (Keychain)
  6. Deploy 클릭

[백엔드]
  1. SSH 연결 테스트
  2. Docker 이미지 레지스트리에서 pull (빌드 불필요)
  3. compose 파일 생성 (Remote + Testnet 조합)
  4. 환경변수 + compose 파일 SFTP 업로드
  5. docker compose up
  6. 컨트랙트 배포 모니터링
  7. L2 노드 시작
  8. Prover 시작
  9. Tools(Explorer, Dashboard) 시작
  10. 포트 + 공인 IP로 External Access 자동 설정
```

### 3.3 필요한 코드 변경

#### A. Compose Generator — Remote + Testnet 조합 추가

```javascript
// 새 함수: generateRemoteTestnetComposeFile()
// Remote(pull 방식) + Testnet(외부 L1) 결합
function generateRemoteTestnetComposeFile({
  programSlug, l2Port, proofCoordPort, projectName,
  l1RpcUrl, deployerPrivateKey, committerPk, proofCoordinatorPk, bridgeOwnerPk,
  gpu, l2ChainId
}) {
  // Remote 이미지 (pull) + Testnet 환경변수 (외부 L1 RPC + Keychain 키)
}
```

**변경 파일**: `compose-generator.js`
**예상 변경량**: ~150줄 (기존 remote + testnet 함수를 조합)

#### B. Deployment Engine — provisionRemoteTestnet 추가

```javascript
async function provisionRemoteTestnet(deployment, hostId) {
  // 1. SSH 연결
  // 2. Keychain에서 키 로드
  // 3. generateRemoteTestnetComposeFile() 호출
  // 4. SFTP 업로드
  // 5. docker compose pull + up
  // 6. 컨트랙트 배포 추적 (SSH log streaming)
  // 7. L2 + Prover 시작
  // 8. Tools 시작 (tools compose도 업로드)
  // 9. External Access 자동 설정 (host.hostname + ports)
}
```

**변경 파일**: `deployment-engine.js`
**예상 변경량**: ~200줄

#### C. Docker Remote — Tools 지원 추가

```javascript
// tools compose 파일도 업로드 + 실행
async function startToolsRemote(conn, projectName, envVars, toolsPorts, remoteDir) {
  // 1. tools compose 파일 SFTP 업로드
  // 2. docker compose -f tools.yaml up -d
}
```

**변경 파일**: `docker-remote.js`
**예상 변경량**: ~80줄

#### D. 이미지 레지스트리 준비

현재 `platform/build-images.sh` 스크립트가 있지만 실제 push된 이미지가 없음.

**필요 작업**:
1. GitHub Container Registry (ghcr.io) 또는 Docker Hub에 이미지 push
2. CI/CD 파이프라인에서 자동 빌드 + push
3. `ETHREX_IMAGE_REGISTRY` 환경변수 기본값 설정

#### E. UI — Remote + Testnet 모드 결합

현재 Launch L2에서 "Remote"와 "Testnet"은 별개 모드.
→ Remote 선택 시 L1 소스를 "Local L1" / "External L1 (Sepolia)" 중 선택할 수 있게.

**변경 파일**: `app.js`
**예상 변경량**: ~50줄

### 3.4 배포 후 자동 설정

Remote 배포 완료 시:
1. `host.hostname` + 할당된 포트로 External Access 자동 Enable
2. L2 RPC, Explorer, Dashboard URL 자동 생성
3. 매니저 UI에서 바로 확인 가능

---

## 4. AI 배포 설계

### 4.1 개념

Manual 탭을 **AI 배포 프롬프트 생성기**로 교체.
사용자가 설정을 선택하면, AI가 바로 실행 가능한 프롬프트가 생성됨.

### 4.2 프롬프트 구조

AI 프롬프트에 포함되어야 할 것:

```markdown
# Tokamak L2 Appchain 배포

## 환경
- 클라우드: GCP (프로젝트: tokamak-appchain)
- 리전: asia-northeast3 (서울)
- VM 타입: e2-standard-4
- OS: Ubuntu 24.04

## 배포 설정
- 앱: zk-dex
- L1: Sepolia (RPC: https://eth-sepolia.g.alchemy.com/v2/xxx)
- L2 Chain ID: 65998343
- Docker 이미지 레지스트리: ghcr.io/tokamak-network

## 키 정보
- Deployer 개인키: [사용자가 입력 또는 Keychain 참조]
- Committer 개인키: [선택]
- Proof Coordinator 개인키: [선택]
- Bridge Owner 개인키: [선택]

## Docker Compose 파일
```yaml
[전체 compose 내용 — 그대로 실행 가능]
```

## 환경변수 파일 (.env)
```
[모든 ETHREX_* 변수]
```

## Tools Compose 파일
```yaml
[Blockscout + Bridge UI compose]
```

## 배포 순서
1. VM 생성 + Docker 설치
2. 방화벽 포트 오픈: 1729 (L2 RPC), 8082 (Explorer), 3000 (Dashboard)
3. docker compose pull
4. docker compose up -d
5. 컨트랙트 배포 완료 대기
6. L2 노드 정상 확인: curl http://VM_IP:1729 -X POST -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
7. Explorer 확인: http://VM_IP:8082
8. Dashboard 확인: http://VM_IP:3000

## 검증 명령어
```bash
# L2 RPC 확인
curl http://VM_IP:1729 -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}'
# 예상 결과: {"result":"0x3EF0797"} (chain_id: 65998343)

# Explorer 확인
curl -s http://VM_IP:8082/api/v2/stats | jq .total_blocks

# 컨트랙트 주소 확인
docker exec [deployer] cat /env/.env
```
```

### 4.3 구현

#### A. 프롬프트 생성 API

```javascript
// POST /api/deployments/:id/ai-prompt
// 또는 Launch 단계에서 "AI Deploy" 선택 시 생성
router.post("/:id/ai-prompt", async (req, res) => {
  const { cloud, region, vmType } = req.body;  // 'gcp' | 'aws'
  const deployment = getDeploymentById(req.params.id);

  const prompt = generateAIDeployPrompt({
    deployment,
    cloud,
    region,
    vmType,
    composeContent: generateRemoteTestnetComposeFile({...}),
    toolsComposeContent: readToolsCompose(),
    envVars: buildDeployEnvVars(deployment),
  });

  res.json({ prompt });
});
```

**새 파일**: `lib/ai-prompt-generator.js`

#### B. 프롬프트 생성 함수

```javascript
function generateAIDeployPrompt({ deployment, cloud, region, vmType, composeContent, toolsComposeContent, envVars }) {
  // 1. 클라우드별 VM 생성 명령어 (GCP/AWS)
  // 2. Docker 설치 스크립트
  // 3. compose 파일 전체 내용 (그대로 복붙 가능)
  // 4. 환경변수 전체
  // 5. 배포 순서 (step by step)
  // 6. 검증 명령어
  // 7. 트러블슈팅 가이드
  return markdownPrompt;
}
```

**예상 변경량**: ~300줄 (새 파일)

#### C. UI — AI Deploy 탭

Manual 탭을 교체:
1. 클라우드 선택 (GCP / AWS)
2. 리전 선택
3. VM 스펙 선택
4. "프롬프트 생성" 클릭
5. 생성된 프롬프트가 코드 블록으로 표시
6. "복사" 버튼으로 클립보드 복사

**변경 파일**: `app.js` (Manual Setup 섹션 교체)
**예상 변경량**: ~100줄

---

## 5. 구현 계획

### Phase 1: 이미지 레지스트리 준비 (선행 작업)

| # | 작업 | 파일 | 난이도 |
|---|------|------|--------|
| 1-1 | ethrex Docker 이미지 빌드 + ghcr.io push | `platform/build-images.sh` | 중 |
| 1-2 | CI에서 자동 빌드/push 파이프라인 | `.github/workflows/` | 중 |
| 1-3 | `ETHREX_IMAGE_REGISTRY` 기본값 설정 | `compose-generator.js` | 하 |

### Phase 2: AI 배포 프롬프트 (빠르게 가치 제공)

| # | 작업 | 파일 | 난이도 |
|---|------|------|--------|
| 2-1 | `ai-prompt-generator.js` 작성 | 신규 | 중 |
| 2-2 | GCP 배포 프롬프트 템플릿 | `ai-prompt-generator.js` | 중 |
| 2-3 | AWS 배포 프롬프트 템플릿 | `ai-prompt-generator.js` | 중 |
| 2-4 | API 라우트 추가 | `deployments.js` | 하 |
| 2-5 | UI — Manual 탭을 AI Deploy로 교체 | `app.js` | 중 |

### Phase 3: 원격 서버 배포 (완전 자동화)

| # | 작업 | 파일 | 난이도 |
|---|------|------|--------|
| 3-1 | Remote + Testnet compose 생성 | `compose-generator.js` | 중 |
| 3-2 | `provisionRemoteTestnet()` 구현 | `deployment-engine.js` | 상 |
| 3-3 | Remote Tools 지원 | `docker-remote.js` | 중 |
| 3-4 | 배포 후 External Access 자동 설정 | `deployment-engine.js` | 하 |
| 3-5 | UI — Remote 모드에 L1 소스 선택 | `app.js` | 중 |

---

## 6. 우선순위

**Phase 2 (AI 배포) 먼저 권장.**

이유:
1. Phase 1(이미지 레지스트리)이 준비되기 전에도 AI 프롬프트에 빌드 명령 포함 가능
2. 코드 변경 최소 — 새 파일 1개 + UI 교체
3. 사용자 가치 즉시 제공 — 프롬프트 복사 → AI에 붙여넣기 → 배포 완료
4. Phase 3(원격 자동화)는 Phase 1 + Phase 2 경험 후 진행

---

## 7. AI 프롬프트 품질 기준

AI가 **실제로 실행 가능**하려면:

1. **모든 파일 내용 포함** — compose.yaml, .env, programs.toml 등 그대로 붙여넣기 가능
2. **명령어 순서 명확** — 1번부터 끝까지 순서대로 실행하면 완료
3. **검증 단계 포함** — 각 단계 후 성공 확인 방법
4. **에러 대응 포함** — 흔한 에러와 해결 방법
5. **비밀키 안전 처리** — 프롬프트에 키를 직접 넣지 않고, 환경변수나 파일 참조
6. **멱등성** — 같은 프롬프트를 다시 실행해도 안전 (이미 존재하면 skip)
7. **클라우드 CLI 명령어 정확** — gcloud/aws CLI 문법 오류 없이

---

## 8. 보안 고려사항

| 위험 | 대응 |
|------|------|
| 프롬프트에 개인키 노출 | 환경변수 파일로 분리, 프롬프트에는 `$DEPLOYER_PRIVATE_KEY` 변수 참조 |
| SSH 키 평문 저장 | 향후 Keychain 연동 (현재는 DB 평문) |
| 클라우드 자격증명 | 프롬프트에 포함 안 함 — 사용자가 gcloud/aws CLI 인증 상태에서 실행 |
| 포트 무제한 노출 | 방화벽 규칙에서 필요 포트만 오픈 (1729, 8082, 3000) |
