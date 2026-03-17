# Appchain Metadata Submission System

## Overview

앱체인 운영자가 메신저 앱에서 메타데이터를 서명하고, Platform 서버를 중계로 GitHub 메타데이터 저장소에 PR을 제출하는 시스템.

## Architecture

```
Messenger (Tauri + React)
  |
  | 1. L2Config -> metadata JSON 생성
  | 2. deployer key로 서명 (Rust k256, OS Keychain)
  |
  v
Platform Server (Express.js)
  |
  | 3. 서명 검증 (ethers.verifyMessage)
  | 4. 온체인 검증 (Timelock.hasRole(SECURITY_COUNCIL, signer))
  | 5. GitHub API로 PR 생성 (bot token)
  |
  v
Metadata Repository (GitHub)
  |
  | 6. CI: validate-pr.yml -> 스키마/서명/온체인 재검증
  | 7. Merge -> tokamak-appchain-data/{l1ChainId}/tokamak-appchain/{timelockAddress}.json
  |
```

## 인증 방식

- Platform 로그인 **불필요**
- deployer key 서명 + 온체인 SECURITY_COUNCIL 역할 확인 = 인증
- GitHub bot token은 Platform 서버에만 존재 (GITHUB_BOT_TOKEN env var)

## 서명 메시지 형식

```
Tokamak Appchain Registry
L1 Chain ID: {l1ChainId}
L2 Chain ID: {l2ChainId}
Stack: {stackType}
Operation: {register|update}
Contract: {timelockAddress_lowercase}
Timestamp: {unixTimestamp}
```

- EIP-191 personal_sign (`\x19Ethereum Signed Message:\n{len}{message}`)
- 서명 결과: `0x{r}{s}{v}` (130 hex chars, v = recovery_id + 27)
- Timestamp: register=createdAt, update=lastUpdated (Unix seconds)
- 24시간 만료

## 파일 경로

```
tokamak-appchain-data/{l1ChainId}/tokamak-appchain/{timelockAddress}.json
```

예: `tokamak-appchain-data/11155111/tokamak-appchain/0x1234...abcd.json`

Identity contract = Timelock (SECURITY_COUNCIL role 기반 소유권)

---

## 구현 상세

### Phase 1: Tauri Signing Command (Rust)

**파일:** `crates/desktop-app/ui/src-tauri/Cargo.toml`

```toml
k256 = { version = "0.13", features = ["ecdsa-core", "ecdsa"] }
sha3 = "0.10"
```

**파일:** `crates/desktop-app/ui/src-tauri/src/commands.rs`

```rust
#[derive(Deserialize)]
pub struct SignMetadataRequest {
    pub l1_chain_id: u64,
    pub l2_chain_id: u64,
    pub stack_type: String,       // "tokamak-appchain"
    pub operation: String,        // "register" | "update"
    pub identity_contract: String, // Timelock address (0x-prefixed, lowercase)
    pub timestamp: u64,
    pub keychain_key: String,     // "deployer_pk_{id}"
}

#[derive(Serialize)]
pub struct SignMetadataResult {
    pub signature: String,      // 0x-prefixed, 130 hex chars
    pub signer_address: String, // 0x-prefixed
}
```

**구현:**
1. `keyring::Entry` -> deployer PK 로드
2. 서명 메시지 빌드 (위 형식)
3. EIP-191 해시: `keccak256("\x19Ethereum Signed Message:\n" + len + message)`
4. `k256::ecdsa::SigningKey::sign_prehash_recoverable()` 서명
5. public key -> keccak256 -> signer address 도출
6. `{signature, signer_address}` 반환

**등록:** `lib.rs` invoke_handler에 `sign_appchain_metadata` 추가

### Phase 2: Platform Server Registry Endpoint

**새 파일:** `platform/server/lib/github-pr.js`

```javascript
// GitHub API 헬퍼
async function createBranch(owner, repo, branchName, baseSha, token)
async function createOrUpdateFile(owner, repo, path, content, branch, message, token)
async function createPullRequest(owner, repo, title, body, head, base, token)
async function getPullRequest(owner, repo, prNumber, token)
```

**새 파일:** `platform/server/routes/appchain-registry.js`

```
POST /api/appchain-registry/submit     (인증 불필요)
GET  /api/appchain-registry/status/:pr (인증 불필요)
```

#### POST /api/appchain-registry/submit

**Request:**
```json
{
  "metadata": { /* TokamakAppchainMetadata 전체 */ },
  "operation": "register"
}
```

**처리 순서:**
1. JSON 구조 검증 (필수 필드, 서명 형식)
2. `ethers.verifyMessage(message, signature)` -> signer 복원
3. signer === metadata.signedBy 확인
4. Timestamp 24시간 만료 확인
5. L1 RPC 접속 -> `Timelock.hasRole(SECURITY_COUNCIL_ROLE, signer)` 온체인 확인
6. 파일 경로 결정: `tokamak-appchain-data/{l1ChainId}/tokamak-appchain/{timelockAddress}.json`
7. GitHub API: branch 생성 -> 파일 커밋 -> PR 생성
8. PR URL 반환

**Response (성공):**
```json
{
  "success": true,
  "prUrl": "https://github.com/tokamak-network/tokamak-rollup-metadata-repository/pull/42",
  "prNumber": 42,
  "filePath": "tokamak-appchain-data/11155111/tokamak-appchain/0x1234...abcd.json"
}
```

**Response (실패):**
```json
{
  "success": false,
  "error": "Signer does not have SECURITY_COUNCIL role on Timelock",
  "code": "OWNERSHIP_CHECK_FAILED"
}
```

**에러 코드:**
- `INVALID_METADATA` — 필수 필드 누락/형식 오류
- `INVALID_SIGNATURE` — 서명 검증 실패
- `SIGNATURE_EXPIRED` — 24시간 초과
- `OWNERSHIP_CHECK_FAILED` — SECURITY_COUNCIL 역할 없음
- `GITHUB_API_ERROR` — GitHub API 오류
- `DUPLICATE_SUBMISSION` — 동일 파일에 대한 열린 PR 존재

**환경변수:**
- `GITHUB_BOT_TOKEN` — metadata repo에 `repo` scope 권한
- `METADATA_REPO_OWNER` — `tokamak-network` (기본값)
- `METADATA_REPO_NAME` — `tokamak-rollup-metadata-repository` (기본값)

**PR 제목 형식:**
```
[Appchain] {l1ChainId} - tokamak-appchain - {timelockAddress_short} - {name}
```

**Rate limit:** 5 submissions/hour/IP (기본 100 req/min보다 엄격)

#### GET /api/appchain-registry/status/:pr

**Response:**
```json
{
  "prNumber": 42,
  "state": "open",
  "merged": false,
  "checksStatus": "success",
  "htmlUrl": "https://github.com/.../pull/42"
}
```

**서버 마운트:** `server.js`에 추가
```javascript
app.use("/api/appchain-registry", require("./routes/appchain-registry"));
```

### Phase 3: Messenger UI

**파일:** `crates/desktop-app/ui/src/api/appchain-registry.ts`

```typescript
const BASE_URL = import.meta.env.VITE_PLATFORM_URL || 'https://tokamak-appchain.vercel.app'

export async function submitMetadata(
  metadata: TokamakAppchainMetadata,
  operation: 'register' | 'update'
): Promise<{ prUrl: string; prNumber: number; filePath: string }>

export async function getSubmissionStatus(
  prNumber: number
): Promise<{ state: string; merged: boolean; checksStatus: string; htmlUrl: string }>
```

**파일:** `crates/desktop-app/ui/src/components/L2DetailPublishTab.tsx`

기존 IPFS 메타데이터 섹션을 **메타데이터 저장소 제출**로 교체:

1. 조건: `l2.timelockAddress && l2.proposerAddress` (둘 다 필요)
2. "메타데이터 미리보기" — JSON preview
3. "서명 & 제출" 버튼
4. 제출 후 PR URL 표시 + 상태 추적

---

## L2Config -> TokamakAppchainMetadata 매핑

| Metadata 필드 | L2Config 소스 | 비고 |
|---|---|---|
| `l1ChainId` | `l2.l1ChainId` | 필수 |
| `l2ChainId` | `l2.l2ChainId ?? l2.chainId` | |
| `name` | `l2.name` | |
| `description` | `publishDesc` (UI 입력) | |
| `stackType` | `'tokamak-appchain'` | 고정 |
| `rollupType` | `'zk'` | ethrex = ZK rollup |
| `rpcUrl` | `l2.publicRpcUrl ?? localhost:{rpcPort}` | |
| `nativeToken.type` | `l2.nativeToken` 파싱 | ETH='eth', 기타='erc20' |
| `nativeToken.symbol` | `l2.nativeToken` 파싱 | |
| `nativeToken.decimals` | `18` | |
| `status` | `l2.status === 'running' ? 'active' : 'inactive'` | |
| `createdAt` | `new Date().toISOString()` | register 시 |
| `lastUpdated` | `new Date().toISOString()` | 항상 현재 |
| `l1Contracts.Timelock` | `l2.timelockAddress` | 필수, 파일명 |
| `l1Contracts.OnChainProposer` | `l2.proposerAddress` | 필수 |
| `l1Contracts.CommonBridge` | `l2.bridgeAddress` | |
| `l1Contracts.SP1Verifier` | `l2.sp1VerifierAddress` | |
| `operator.address` | sign 결과의 `signerAddress` | |
| `supportResources` | `socialLinks` (UI 입력) | |
| `metadata.signature` | sign 결과 | |
| `metadata.signedBy` | sign 결과 | |
| `metadata.version` | `'1.0.0'` | |

---

## 보안

1. **서명 = 인증**: Platform 계정 불필요. deployer key 서명 + 온체인 SECURITY_COUNCIL 확인
2. **Private key**: Tauri Rust 프로세스 내에서만 사용, React 프론트엔드에 노출 안 됨
3. **Bot token**: Platform 서버 환경변수에만 존재
4. **Replay 방지**: Timestamp 24시간 만료 + lastUpdated 순차 증가
5. **Rate limit**: IP당 5회/시간
6. **이중 검증**: Platform 서버 + GitHub CI 둘 다 검증

## 구현 순서

1. **Phase 1**: Tauri Rust 서명 커맨드 (`k256` + `sha3`)
2. **Phase 2**: Platform 서버 엔드포인트 (`github-pr.js` + `appchain-registry.js`)
3. **Phase 3**: 메신저 UI (API client + Publish 탭 개선)
4. **Phase 4**: E2E 테스트 (Sepolia 테스트넷)
