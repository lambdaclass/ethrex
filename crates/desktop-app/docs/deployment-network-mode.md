# Deployment Network Mode Detection

메신저 앱에서 배포 유형(Local/AWS/Testnet/Mainnet)을 판별하는 로직.

## 데이터 소스

```
SQLite DB (~/.tokamak-appchain/local.sqlite)
  └── deployments 테이블
        ├── l1_port        — 로컬 L1 노드 포트 (있으면 Docker 내장 L1)
        ├── l1_chain_id    — L1 체인 ID (NULL일 수 있음)
        ├── host_id        — 원격 호스트 ID (hosts 테이블 FK)
        ├── config         — JSON 문자열 (배포 설정 전체)
        │     ├── mode     — 'testnet' | 'mainnet' | 'ai-deploy'
        │     ├── cloud    — 'aws' (클라우드 배포 시)
        │     ├── l1ChainId — L1 체인 ID (config 내)
        │     └── testnet  — { network, l1ChainId, l1RpcUrl }
        ├── public_l2_rpc_url — 외부 접근 가능한 L2 RPC URL
        └── public_domain     — 퍼블릭 도메인
```

## Network Mode 판별 순서 (우선순위)

```
1. config.mode === 'testnet'  → Testnet  (명시적 설정)
2. config.mode === 'mainnet'  → Mainnet  (명시적 설정)
3. config.cloud === 'aws'     → AWS      (AI Deploy로 AWS 배포)
   또는 host_id 존재         → AWS      (원격 호스트 배포)
4. l1_port 존재               → Local    (Docker 내장 L1 노드 사용)
5. l1_chain_id 존재           → Testnet  (외부 L1에 연결)
6. default                    → Local
```

## L1 Chain ID 결정 순서

```
1. DB: deployments.l1_chain_id      (매니저가 직접 저장)
2. Config: config.l1ChainId         (AI Deploy 프롬프트에서 설정)
3. Config: config.testnet.l1ChainId (테스트넷 설정)
4. NULL
```

## 실제 데이터 예시

### Local Docker 배포 (ZK-DEX L2)
```json
{
  "l1_port": 8545,
  "l2_port": 1729,
  "l1_chain_id": 1185,
  "host_id": null,
  "config": { "mode": "ai-deploy", "l1ChainId": 1185 }
}
```
→ `l1_port=8545` 존재 → **Local** (보라색 배지)

### AWS 배포 (Test test)
```json
{
  "l1_port": null,
  "l2_port": null,
  "l1_chain_id": null,
  "host_id": null,
  "config": { "mode": "ai-deploy", "cloud": "aws", "l1ChainId": 3711, "ec2IP": "54.180.160.159" }
}
```
→ `config.cloud='aws'` → **AWS** (주황색 배지)
→ `l1ChainId`는 `config.l1ChainId=3711`에서 가져옴

### Sepolia 테스트넷 배포
```json
{
  "l1_port": null,
  "l2_port": 1729,
  "l1_chain_id": 11155111,
  "host_id": null,
  "config": { "mode": "testnet", "testnet": { "network": "sepolia", "l1ChainId": 11155111 } }
}
```
→ `config.mode='testnet'` → **Testnet** (노란색 배지)

## UI 배지 색상

| Mode | 배지 | 색상 | 공개 가능 |
|------|------|------|-----------|
| Local | `Local` | 보라색 `#6366f1` | ❌ (외부 접근 불가) |
| AWS | `AWS` | 주황색 `#f97316` | ✅ |
| Testnet | `Testnet` | 노란색 `--color-warning` | ✅ |
| Mainnet | `Mainnet` | 초록색 `#22c55e` | ✅ |

## 공개 가능 조건 (L2DetailPublishTab)

```
cannotPublish = !hasPublicUrl && networkMode === 'local'

hasPublicUrl = publicRpcUrl || testnetL1RpcUrl || hostId
```

Local이어도 `publicRpcUrl`이 설정되어 있으면 공개 가능.

## 관련 코드

- `MyL2View.tsx:deploymentToL2Config()` — DB → L2Config 변환 (networkMode 판별)
- `deployment_db.rs:DeploymentRow` — Rust에서 DB 읽기 (SELECT 쿼리)
- `L2DetailPublishTab.tsx` — 공개 가능 여부 판단 + 토글 UI
- `local-server/db/schema.sql` — DB 스키마
