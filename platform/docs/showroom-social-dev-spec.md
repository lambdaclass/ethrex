# Open Appchain Showroom — 개발 스펙

> 작성일: 2026-03-12
> 브랜치: feat/showroom-social-features
> 방향 설계: [showroom-social-design.md](showroom-social-design.md)

---

## Phase 1: Showroom 기반 완성

Phase 1에서는 Platform DB를 임시 저장소로 활용한다.
Phase 2에서 온체인 + IPFS로 전환 시 Platform DB는 캐시/인덱서 역할로 변경된다.

---

### 1.1 DB 스키마 변경

**파일:** `platform/server/db/schema.sql`

```sql
-- deployments 테이블에 컬럼 추가
ALTER TABLE deployments ADD COLUMN description TEXT;
ALTER TABLE deployments ADD COLUMN screenshots TEXT;        -- JSON 배열: ["ipfs://Qm...", ...]
ALTER TABLE deployments ADD COLUMN explorer_url TEXT;
ALTER TABLE deployments ADD COLUMN dashboard_url TEXT;
ALTER TABLE deployments ADD COLUMN social_links TEXT;        -- JSON: {"twitter":"...", "discord":"..."}
ALTER TABLE deployments ADD COLUMN l1_chain_id INTEGER;
ALTER TABLE deployments ADD COLUMN network_mode TEXT;        -- "local" | "testnet" | "mainnet"
ALTER TABLE deployments ADD COLUMN stack_name TEXT DEFAULT 'tokamak-appchain';
ALTER TABLE deployments ADD COLUMN zk_proof_system TEXT DEFAULT 'sp1';
```

**파일:** `platform/server/db/deployments.js`

`updateDeployment` 함수의 allowedFields에 추가:
```javascript
const allowedFields = [
  "name", "chain_id", "rpc_url", "status", "config", "phase",
  "bridge_address", "proposer_address",
  // 신규 필드
  "description", "screenshots", "explorer_url", "dashboard_url",
  "social_links", "l1_chain_id", "network_mode", "stack_name", "zk_proof_system"
];
```

`getActiveDeployments` 함수의 SELECT에 신규 필드 추가:
```javascript
function getActiveDeployments({ limit = 50, offset = 0, search } = {}) {
  let sql = `
    SELECT d.id, d.name, d.chain_id, d.rpc_url, d.status, d.phase,
           d.bridge_address, d.proposer_address, d.created_at,
           d.description, d.screenshots, d.explorer_url, d.dashboard_url,
           d.social_links, d.l1_chain_id, d.network_mode,
           p.name as program_name, p.program_id as program_slug, p.category,
           u.name as owner_name
    FROM deployments d
    JOIN programs p ON d.program_id = p.id
    JOIN users u ON d.user_id = u.id
    WHERE d.status = 'active'
  `;
  // ... 나머지 동일
}
```

신규 함수 추가:
```javascript
function getActiveDeploymentById(id) {
  return db.prepare(`
    SELECT d.*,
           p.name as program_name, p.program_id as program_slug, p.category,
           u.name as owner_name, u.picture as owner_picture
    FROM deployments d
    JOIN programs p ON d.program_id = p.id
    JOIN users u ON d.user_id = u.id
    WHERE d.id = ? AND d.status = 'active'
  `).get(id);
}
```

---

### 1.2 Platform API 변경

#### 1.2.1 신규: 공개 앱체인 상세 조회 (인증 불필요)

**파일:** `platform/server/routes/store.js`

```javascript
// GET /api/store/appchains/:id — 공개 앱체인 상세 (인증 불필요)
router.get("/appchains/:id", (req, res) => {
  try {
    const appchain = getActiveDeploymentById(req.params.id);
    if (!appchain) {
      return res.status(404).json({ error: "Appchain not found" });
    }
    // screenshots, social_links는 JSON 파싱해서 반환
    res.json({
      appchain: {
        ...appchain,
        screenshots: appchain.screenshots ? JSON.parse(appchain.screenshots) : [],
        social_links: appchain.social_links ? JSON.parse(appchain.social_links) : {},
      }
    });
  } catch (e) {
    res.status(500).json({ error: e.message });
  }
});
```

**응답 형식:**
```json
{
  "appchain": {
    "id": "uuid",
    "name": "My Appchain",
    "description": "ZK-proven appchain for DeFi",
    "chain_id": 49152,
    "rpc_url": "https://rpc.my-appchain.com",
    "status": "active",
    "phase": "live",
    "bridge_address": "0x...",
    "proposer_address": "0x...",
    "explorer_url": "https://explorer.my-appchain.com",
    "dashboard_url": "https://bridge.my-appchain.com",
    "screenshots": ["ipfs://Qm1...", "ipfs://Qm2..."],
    "social_links": { "twitter": "...", "discord": "..." },
    "l1_chain_id": 11155111,
    "network_mode": "testnet",
    "program_name": "Tokamak Appchain",
    "program_slug": "tokamak-appchain",
    "category": "general",
    "owner_name": "Alice",
    "owner_picture": "https://...",
    "created_at": 1710201600
  }
}
```

#### 1.2.2 기존: 앱체인 목록에 신규 필드 포함

`GET /api/store/appchains` 응답에 `description`, `screenshots`, `l1_chain_id`, `network_mode` 추가 (1.1에서 SELECT 변경으로 자동 반영).

#### 1.2.3 기존: 배포 업데이트 API 확장

`PUT /api/deployments/:id`는 이미 동적 필드 업데이트를 지원하므로, allowedFields 추가만으로 신규 필드 저장 가능.

---

### 1.3 Desktop 앱 공개 설정 완성

#### 1.3.1 Platform API 클라이언트 확장

**파일:** `crates/desktop-app/ui/src/api/platform.ts`

```typescript
// PlatformAPI 클래스에 메서드 추가

async updateDeployment(id: string, data: {
  description?: string
  screenshots?: string[]
  explorer_url?: string
  dashboard_url?: string
  social_links?: Record<string, string>
  l1_chain_id?: number
  network_mode?: string
}) {
  return this.fetchWithAuth(`/api/deployments/${id}`, {
    method: 'PUT',
    body: JSON.stringify({
      ...data,
      screenshots: data.screenshots ? JSON.stringify(data.screenshots) : undefined,
      social_links: data.social_links ? JSON.stringify(data.social_links) : undefined,
    }),
  })
}
```

#### 1.3.2 L2DetailPublishTab 수정

**파일:** `crates/desktop-app/ui/src/components/L2DetailPublishTab.tsx`

현재 문제:
- `publishDesc`와 `publishScreenshots`가 `useState`로만 관리 → 저장 안 됨
- 스크린샷은 더미 placeholder

변경사항:

```typescript
// 1. 공개 토글 ON 시 registerDeployment에 추가 정보 포함
const r = await platformAPI.registerDeployment({
  programId: 'tokamak-appchain',    // 'ethrex-appchain'에서 변경
  name: l2.name,
  chainId: l2.chainId,
  rpcUrl: l2.networkMode === 'local'
    ? `http://localhost:${l2.rpcPort}`
    : l2.testnetL1RpcUrl || `http://localhost:${l2.rpcPort}`,
})
await platformAPI.activateDeployment(r.deployment.id)

// 2. 추가 메타데이터 저장 (activateDeployment 후)
await platformAPI.updateDeployment(r.deployment.id, {
  description: publishDesc,
  screenshots: publishScreenshots,  // IPFS CID 배열
  explorer_url: l2.toolsL2ExplorerPort
    ? `http://localhost:${l2.toolsL2ExplorerPort}` : undefined,
  dashboard_url: l2.toolsBridgeUIPort
    ? `http://localhost:${l2.toolsBridgeUIPort}` : undefined,
  l1_chain_id: l2.networkMode === 'testnet' ? 11155111 : 1,  // 네트워크에 따라
  network_mode: l2.networkMode,
})

// 3. 소개글 저장 버튼 추가 (이미 공개 중일 때)
const handleSaveDescription = async () => {
  if (!platformDeploymentId) return
  await platformAPI.updateDeployment(platformDeploymentId, {
    description: publishDesc,
  })
}

// 4. 로컬 DB에 platformDeploymentId 저장 (나중에 업데이트 위해)
await invoke('update_appchain_public', {
  id: l2.id,
  isPublic: true,
  platformDeploymentId: r.deployment.id,
})
```

#### 1.3.3 Desktop 로컬 DB 변경

**파일:** `crates/desktop-app/ui/src-tauri/src/deployment_db.rs`

`DeploymentRow`에 필드 추가:
```rust
pub platform_deployment_id: Option<String>,  // Platform 서버의 deployment ID
```

---

### 1.4 Showroom 상세 페이지

#### 1.4.1 라우트 생성

**파일:** `platform/client/app/showroom/[id]/page.tsx` (신규)

#### 1.4.2 페이지 구성

```
┌─────────────────────────────────────────────┐
│ ← Back to Showroom                          │
├─────────────────────────────────────────────┤
│                                             │
│  [Logo]  My ZK Appchain                     │
│          by Alice                           │
│          Testnet (Sepolia) · Chain ID 49152 │
│          ● Online (블록: 1,234 / 배치: 56)  │
│                                             │
├─────────────────────────────────────────────┤
│                                             │
│  소개                                       │
│  ─────                                      │
│  ZK-proven appchain for DeFi applications   │
│  with low transaction fees and fast...      │
│                                             │
├─────────────────────────────────────────────┤
│                                             │
│  스크린샷                                    │
│  ─────────                                  │
│  [img1] [img2] [img3]                       │
│                                             │
├─────────────────────────────────────────────┤
│                                             │
│  서비스                                      │
│  ─────                                      │
│  L2 Explorer    https://explorer...    [↗]  │
│  Bridge         https://bridge...      [↗]  │
│  RPC URL        https://rpc...        [📋]  │
│                                             │
├─────────────────────────────────────────────┤
│                                             │
│  L1 컨트랙트 (Sepolia)                       │
│  ─────────────────────                      │
│  OnChainProposer  0xABCD...1234       [📋]  │
│  CommonBridge     0x5678...9012       [📋]  │
│                                             │
├─────────────────────────────────────────────┤
│                                             │
│  라이브 상태 (L2 RPC 직접 조회)               │
│  ───────────────────────────                │
│  최신 블록       #1,234                      │
│  최신 배치       #56                         │
│  가스 가격       0.001 Gwei                  │
│  (조회 실패 시: "노드에 연결할 수 없습니다")     │
│                                             │
├─────────────────────────────────────────────┤
│                                             │
│  링크                                       │
│  ────                                       │
│  🐦 Twitter  💬 Discord  📱 Telegram        │
│                                             │
└─────────────────────────────────────────────┘
```

#### 1.4.3 L2 RPC 직접 조회 — CORS 해결

브라우저에서 외부 L2 RPC URL에 직접 요청하면 CORS 에러 발생.

**해결 방안: Platform 서버에 프록시 엔드포인트 추가**

**파일:** `platform/server/routes/store.js`

```javascript
// POST /api/store/appchains/:id/rpc-proxy — L2 RPC 프록시
router.post("/appchains/:id/rpc-proxy", async (req, res) => {
  try {
    const appchain = getActiveDeploymentById(req.params.id);
    if (!appchain || !appchain.rpc_url) {
      return res.status(404).json({ error: "Appchain not found or no RPC URL" });
    }

    // 허용된 메서드만 프록시 (보안)
    const allowedMethods = [
      "eth_blockNumber", "eth_chainId", "eth_gasPrice",
      "ethrex_batchNumber", "net_version"
    ];

    const { method, params } = req.body;
    if (!allowedMethods.includes(method)) {
      return res.status(400).json({ error: "Method not allowed" });
    }

    const response = await fetch(appchain.rpc_url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params: params || [] }),
      signal: AbortSignal.timeout(5000),  // 5초 타임아웃
    });

    const data = await response.json();
    res.json(data);
  } catch (e) {
    // 노드 오프라인 또는 타임아웃
    res.status(502).json({ error: "L2 node unreachable", detail: e.message });
  }
});
```

**프론트엔드 호출:**
```typescript
// platform/client/lib/api.ts에 추가
export const storeApi = {
  // ... 기존
  appchainRpc: async (id: string, method: string, params: unknown[] = []) => {
    const res = await fetch(`${API_BASE}/api/store/appchains/${id}/rpc-proxy`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ method, params }),
    });
    if (!res.ok) return null;
    const data = await res.json();
    return data.result ?? null;
  },
};
```

#### 1.4.4 Showroom 카드에 링크 추가

**파일:** `platform/client/app/showroom/page.tsx`

기존 카드를 `<Link href={/showroom/${chain.id}}>` 로 래핑:

```tsx
import Link from 'next/link';

// 카드 부분 변경
<Link href={`/showroom/${chain.id}`} key={chain.id}>
  <div className="bg-white rounded-xl border p-6 hover:shadow-md transition-shadow">
    {/* 기존 카드 내용 */}
    {/* + description 미리보기 추가 */}
    {chain.description && (
      <p className="text-sm text-gray-600 mt-2 line-clamp-2">{chain.description}</p>
    )}
  </div>
</Link>
```

#### 1.4.5 Platform Client 타입 확장

**파일:** `platform/client/lib/types.ts`

```typescript
export interface AppchainDetail {
  id: string;
  name: string;
  description: string | null;
  chain_id: number | null;
  rpc_url: string | null;
  status: string;
  phase: string;
  bridge_address: string | null;
  proposer_address: string | null;
  explorer_url: string | null;
  dashboard_url: string | null;
  screenshots: string[];
  social_links: Record<string, string>;
  l1_chain_id: number | null;
  network_mode: string | null;
  program_name: string;
  program_slug: string;
  category: string;
  owner_name: string;
  owner_picture: string | null;
  created_at: number;
}
```

---

### 1.5 스크린샷 업로드 (IPFS)

Phase 1에서는 Pinata 무료 티어 사용 (1GB, 충분).

#### 1.5.1 Desktop 앱에서 IPFS 업로드

**파일:** `crates/desktop-app/ui/src/api/platform.ts` (또는 별도 `ipfs.ts`)

```typescript
const PINATA_API = 'https://api.pinata.cloud';

export async function uploadToIPFS(file: File): Promise<string> {
  const formData = new FormData();
  formData.append('file', file);

  const res = await fetch(`${PINATA_API}/pinning/pinFileToIPFS`, {
    method: 'POST',
    headers: {
      'Authorization': `Bearer ${await getPinataJWT()}`,
    },
    body: formData,
  });

  const data = await res.json();
  return `ipfs://${data.IpfsHash}`;
}

// Pinata JWT는 OS Keychain에 저장 (기존 AI API 키와 동일 패턴)
async function getPinataJWT(): Promise<string> {
  return invoke('get_keychain_value', { key: 'pinata_jwt' });
}
```

#### 1.5.2 IPFS Gateway (읽기)

Showroom에서 스크린샷 표시 시 IPFS CID를 HTTP URL로 변환:

```typescript
function ipfsToHttp(uri: string): string {
  if (uri.startsWith('ipfs://')) {
    const cid = uri.replace('ipfs://', '');
    return `https://gateway.pinata.cloud/ipfs/${cid}`;
  }
  return uri;
}
```

#### 1.5.3 Pinata API 키 설정 UI

Desktop SettingsView에 Pinata JWT 입력 필드 추가 (기존 AI API 키 설정과 동일 패턴).

---

### 1.6 오프라인 노드 처리

L2 RPC 호출이 실패하는 경우 (노드 다운, 네트워크 불가):

```tsx
// Showroom 상세 페이지에서
const [liveStatus, setLiveStatus] = useState<{
  blockNumber: number | null;
  batchNumber: number | null;
  gasPrice: string | null;
  online: boolean;
  error: string | null;
}>({ blockNumber: null, batchNumber: null, gasPrice: null, online: false, error: null });

useEffect(() => {
  async function fetchLiveStatus() {
    try {
      const [block, batch, gas] = await Promise.all([
        storeApi.appchainRpc(id, 'eth_blockNumber'),
        storeApi.appchainRpc(id, 'ethrex_batchNumber'),
        storeApi.appchainRpc(id, 'eth_gasPrice'),
      ]);
      setLiveStatus({
        blockNumber: block ? parseInt(block, 16) : null,
        batchNumber: batch ? parseInt(batch, 16) : null,
        gasPrice: gas,
        online: true,
        error: null,
      });
    } catch {
      setLiveStatus(prev => ({ ...prev, online: false, error: 'Node unreachable' }));
    }
  }
  fetchLiveStatus();
  const interval = setInterval(fetchLiveStatus, 30000); // 30초마다 갱신
  return () => clearInterval(interval);
}, [id]);

// UI 표시
{liveStatus.online ? (
  <span className="text-green-600">● Online</span>
) : (
  <span className="text-gray-400">● Offline — 노드에 연결할 수 없습니다</span>
)}
```

---

## Phase 2: 온체인 메타데이터

### 2.1 OnChainProposer 컨트랙트 확장

**파일:** `crates/l2/contracts/src/l1/OnChainProposer.sol`

기존 패턴: `Ownable2StepUpgradeable` + `UUPSUpgradeable`

```solidity
// 상태 변수 추가 (기존 상태 변수 뒤에)
string public metadataURI;

// 이벤트
event MetadataURIUpdated(string newURI);

// setter (onlyOwner)
function setMetadataURI(string calldata _metadataURI) external onlyOwner {
    metadataURI = _metadataURI;
    emit MetadataURIUpdated(_metadataURI);
}
```

**파일:** `crates/l2/contracts/src/l1/interfaces/IOnChainProposer.sol`

```solidity
function setMetadataURI(string calldata _metadataURI) external;
event MetadataURIUpdated(string newURI);
```

Based 버전도 동일하게 수정: `crates/l2/contracts/src/l1/based/interfaces/IOnChainProposer.sol`

### 2.2 Deployer 변경

**파일:** `cmd/ethrex/l2/deployer.rs`

배포 후 metadataURI 설정 옵션 추가:

```rust
// ContractAddresses 구조체는 변경 불필요
// 배포 완료 후 선택적으로 setMetadataURI 호출

pub async fn set_metadata_uri(
    on_chain_proposer_address: Address,
    metadata_uri: &str,
    deployer_wallet: &Wallet,
    eth_client: &EthClient,
) -> Result<(), DeployError> {
    let calldata = encode_set_metadata_uri(metadata_uri);
    send_transaction(
        on_chain_proposer_address,
        deployer_wallet,
        calldata,
        eth_client,
    ).await?;
    Ok(())
}
```

### 2.3 L2 RPC 메타데이터 엔드포인트

**파일:** `crates/l2/networking/rpc/l2/metadata.rs` (신규)

기존 `batch.rs`의 `RpcHandler` 패턴을 따름:

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize)]
struct MetadataResponse {
    metadata_uri: Option<String>,
    chain_id: u64,
    on_chain_proposer: String,
    common_bridge: String,
}

pub struct TokamakMetadataRequest;

impl RpcHandler for TokamakMetadataRequest {
    fn parse(params: &Option<Vec<Value>>) -> Result<Self, RpcErr> {
        if params.as_ref().is_some_and(|p| !p.is_empty()) {
            return Err(RpcErr::BadParams("Expected 0 params".to_owned()));
        }
        Ok(Self)
    }

    async fn handle(&self, context: RpcApiContext) -> Result<Value, RpcErr> {
        // context에서 설정 정보 읽기
        let response = MetadataResponse {
            metadata_uri: context.metadata_uri.clone(),
            chain_id: context.chain_id,
            on_chain_proposer: format!("{:?}", context.on_chain_proposer_address),
            common_bridge: format!("{:?}", context.common_bridge_address),
        };
        serde_json::to_value(response)
            .map_err(|e| RpcErr::Internal(e.to_string()))
    }
}
```

**파일:** `crates/l2/networking/rpc/rpc.rs`

`map_l2_requests`에 등록:
```rust
"ethrex_metadata" => TokamakMetadataRequest::call(req, context).await,
// 참고: RPC 메서드명은 코드에서는 ethrex_ 접두사 유지,
// 향후 tokamak_ 접두사로 전환 시 별도 마이그레이션
```

### 2.4 Platform 인덱서

**파일:** `platform/server/lib/l1-indexer.js` (신규)

```javascript
const { ethers } = require('ethers');

const CHAINS = [
  { name: 'sepolia', rpcUrl: process.env.SEPOLIA_RPC_URL, chainId: 11155111 },
  { name: 'holesky', rpcUrl: process.env.HOLESKY_RPC_URL, chainId: 17000 },
  // mainnet 추후 추가
];

// MetadataURIUpdated 이벤트 ABI
const METADATA_EVENT_ABI = [
  "event MetadataURIUpdated(string newURI)"
];

async function startIndexer() {
  for (const chain of CHAINS) {
    if (!chain.rpcUrl) continue;
    const provider = new ethers.JsonRpcProvider(chain.rpcUrl);

    // 알려진 OnChainProposer 주소 목록 (Git 레포 또는 DB에서)
    const knownProposers = await getKnownProposers(chain.chainId);

    for (const addr of knownProposers) {
      const contract = new ethers.Contract(addr, METADATA_EVENT_ABI, provider);
      contract.on("MetadataURIUpdated", async (newURI) => {
        console.log(`[${chain.name}] MetadataUpdated: ${addr} → ${newURI}`);
        // IPFS에서 메타데이터 fetch → DB 캐시 업데이트
        await fetchAndCacheMetadata(addr, newURI, chain.chainId);
      });
    }
  }
}

async function fetchAndCacheMetadata(proposerAddr, uri, l1ChainId) {
  const httpUrl = uri.replace('ipfs://', 'https://gateway.pinata.cloud/ipfs/');
  const res = await fetch(httpUrl, { signal: AbortSignal.timeout(10000) });
  const metadata = await res.json();

  // DB 업데이트 (deployments 테이블에서 proposer_address로 찾기)
  updateDeploymentByProposer(proposerAddr, {
    description: metadata.description,
    screenshots: JSON.stringify(metadata.screenshots || []),
    explorer_url: metadata.explorers?.[0]?.url,
    dashboard_url: metadata.dashboard?.bridgeUI,
    social_links: JSON.stringify(metadata.socialLinks || {}),
  });
}
```

### 2.5 Phase 1 → Phase 2 마이그레이션

```
Phase 1 상태:
  Desktop → Platform API → Platform DB (source of truth)

Phase 2 전환:
  1. OnChainProposer에 metadataURI 추가 배포
  2. 기존 Platform DB의 description/screenshots → IPFS 업로드 → CID 생성
  3. setMetadataURI(ipfsCID) 트랜잭션 실행
  4. Platform 인덱서 시작 → 이후 DB는 캐시 역할만
  5. Desktop 공개 설정 흐름 변경: API 직접 → IPFS + L1 트랜잭션
```

### 2.6 Desktop에서 L1 트랜잭션 서명

**가스비 부담:** 앱체인 소유자 (deployer 지갑)

**방식:** Desktop 앱은 이미 testnet 배포 시 OS Keychain에서 개인키를 관리. 동일 키로 `setMetadataURI` 트랜잭션 서명.

**파일:** `crates/desktop-app/ui/src-tauri/src/commands.rs`

```rust
#[tauri::command]
async fn set_metadata_uri(
    l1_rpc_url: String,
    proposer_address: String,
    metadata_uri: String,
    keychain_key: String,
) -> Result<String, String> {
    // 1. Keychain에서 개인키 로드
    // 2. ethers-rs로 setMetadataURI 트랜잭션 생성 및 전송
    // 3. 트랜잭션 해시 반환
}
```

---

## Phase 3: 소셜 기능

### 3.1 Nostr Relay 셋업

**Relay 구현체:** `nostr-rs-relay` (Rust, 가장 성숙)

```bash
# Docker로 배포
docker run -d \
  -p 7700:8080 \
  -v nostr-data:/usr/src/app/db \
  -e RELAY_NAME="Tokamak Appchain Relay" \
  -e RELAY_DESCRIPTION="Social relay for Tokamak Appchain Showroom" \
  scsibug/nostr-rs-relay:latest
```

**필터 정책:** `tokamak-appchain` 네임스페이스 태그가 있는 이벤트만 수락:
```json
{ "kinds": [0, 7, 30100, 30101], "#L": ["tokamak-appchain"] }
```

### 3.2 프론트엔드 Nostr 연동

**의존성:** `nostr-tools` (npm)

**파일:** `platform/client/lib/nostr.ts` (신규)

```typescript
import { SimplePool, finalizeEvent, generateSecretKey, getPublicKey } from 'nostr-tools';

const RELAY_URL = process.env.NEXT_PUBLIC_NOSTR_RELAY || 'wss://relay.tokamak.network';
const pool = new SimplePool();

// 리뷰 목록 조회
export async function getAppchainReviews(chainId: string) {
  const events = await pool.querySync(
    [RELAY_URL],
    { kinds: [30100], '#d': [chainId], '#L': ['tokamak-appchain'] }
  );
  return events.map(e => ({
    id: e.id,
    author: e.pubkey,
    rating: parseInt(e.tags.find(t => t[0] === 'rating')?.[1] || '0'),
    content: e.content,
    createdAt: e.created_at,
  }));
}

// 댓글 목록 조회
export async function getAppchainComments(chainId: string) {
  const events = await pool.querySync(
    [RELAY_URL],
    { kinds: [30101], '#d': [chainId], '#L': ['tokamak-appchain'] }
  );
  return events;
}

// 좋아요 수 조회
export async function getReactionCount(eventId: string) {
  const events = await pool.querySync(
    [RELAY_URL],
    { kinds: [7], '#e': [eventId] }
  );
  return events.filter(e => e.content === '+').length;
}

// 리뷰 작성
export async function publishReview(
  secretKey: Uint8Array,
  chainId: string,
  rating: number,
  content: string
) {
  const event = finalizeEvent({
    kind: 30100,
    created_at: Math.floor(Date.now() / 1000),
    tags: [
      ['d', chainId],
      ['rating', rating.toString()],
      ['L', 'tokamak-appchain'],
    ],
    content,
  }, secretKey);

  await pool.publish([RELAY_URL], event);
  return event;
}
```

### 3.3 인증: EVM 지갑 서명 → Nostr 키 파생

**원칙:** Nostr 키를 EVM 지갑 주소에서 deterministic하게 파생한다.
같은 지갑 = 항상 같은 Nostr 키. 별도의 키 관리가 필요 없다.

**플로우:**
```
1. 사용자가 "Sign in with Wallet" 클릭
2. MetaMask 등 EVM 지갑에서 고정 메시지 서명 요청
   → "Sign in to Tokamak Appchain Showroom\nThis signature links your wallet to your social identity."
3. 서명 결과(65 bytes)에서 앞 32 bytes를 Nostr secret key로 사용
4. Nostr pubkey = getPublicKey(sk)
5. sk를 sessionStorage에 캐시 (탭 닫으면 삭제, 다시 서명하면 복구)
6. 지갑 주소 + Nostr pubkey 매핑은 리뷰/댓글에 태그로 포함
```

**구현:**

```typescript
const SIGN_MESSAGE =
  "Sign in to Tokamak Appchain Showroom\n\nDomain: platform.tokamak.network\nPurpose: Nostr key derivation\n\nThis signature links your wallet to your social identity.";

async function connectWallet(): Promise<{ sk: Uint8Array; pk: string; address: string }> {
  const ethereum = (window as any).ethereum;
  if (!ethereum) throw new Error("No wallet found");

  // 1. 지갑 연결 & 주소 획득
  const accounts = await ethereum.request({ method: "eth_requestAccounts" });
  if (!accounts?.length) throw new Error("No accounts returned");
  const address = accounts[0];

  // 2. 고정 메시지 서명 → deterministic 결과
  const signature = await ethereum.request({
    method: "personal_sign",
    params: [SIGN_MESSAGE, address],
  });

  // 3. SHA-256 해시로 Nostr secret key 파생 (domain-separated, KDF)
  const sigBytes = hexToBytes(signature.slice(2)); // 0x 제거
  const hashBuffer = await crypto.subtle.digest("SHA-256", sigBytes);
  const sk = new Uint8Array(hashBuffer);

  return { sk, pk: getPublicKey(sk), address };
}
```

**장점:**
- 지갑 주소로 리뷰 작성자 검증 가능
- 여러 디바이스에서 같은 키 (같은 지갑이면 같은 서명)
- localStorage 삭제해도 다시 서명하면 복구
- 별도의 Nostr 키 관리/백업 불필요
- 리뷰/댓글에 `["wallet", "0x..."]` 태그 포함 → UI-level self-asserted metadata (향후 EVM 서명 증명 추가 가능)

**이벤트 태그 확장:**
```typescript
// 리뷰 이벤트에 지갑 주소 태그 추가
tags: [
  ["d", chainId],
  ["rating", rating.toString()],
  ["L", "tokamak-appchain"],
  ["wallet", walletAddress],  // EVM 지갑 주소
]
```

### 3.4 스팸 방지

- **Rate limiting:** Relay에서 IP당 이벤트 발행 속도 제한
- **Platform 인증 필수:** 리뷰/댓글 작성 시 Platform 로그인 상태 확인 후 서명
- **신고 기능:** 부적절한 콘텐츠 신고 → Platform admin이 Relay에서 삭제

---

## 파일 변경 요약

### Phase 1 변경 파일

| 파일 | 변경 유형 | 내용 |
|------|----------|------|
| `platform/server/db/schema.sql` | 수정 | deployments 컬럼 추가 |
| `platform/server/db/deployments.js` | 수정 | allowedFields 추가, getActiveDeploymentById 추가 |
| `platform/server/routes/store.js` | 수정 | GET /appchains/:id, POST /appchains/:id/rpc-proxy |
| `platform/client/lib/api.ts` | 수정 | storeApi에 appchain, appchainRpc 추가 |
| `platform/client/lib/types.ts` | 수정 | AppchainDetail 인터페이스 추가 |
| `platform/client/app/showroom/page.tsx` | 수정 | 카드에 Link 추가, description 미리보기 |
| `platform/client/app/showroom/[id]/page.tsx` | **신규** | 앱체인 상세 페이지 |
| `crates/desktop-app/ui/src/api/platform.ts` | 수정 | updateDeployment 메서드 추가 |
| `crates/desktop-app/ui/src/components/L2DetailPublishTab.tsx` | 수정 | description/screenshots 실제 저장 |
| `crates/desktop-app/ui/src-tauri/src/deployment_db.rs` | 수정 | platform_deployment_id 컬럼 |

### Phase 2 변경 파일

| 파일 | 변경 유형 | 내용 |
|------|----------|------|
| `crates/l2/contracts/src/l1/OnChainProposer.sol` | 수정 | metadataURI + setMetadataURI |
| `crates/l2/contracts/src/l1/interfaces/IOnChainProposer.sol` | 수정 | 인터페이스 |
| `cmd/ethrex/l2/deployer.rs` | 수정 | set_metadata_uri 함수 |
| `crates/l2/networking/rpc/l2/metadata.rs` | **신규** | tokamak_metadata RPC |
| `crates/l2/networking/rpc/rpc.rs` | 수정 | RPC 라우팅 |
| `platform/server/lib/l1-indexer.js` | **신규** | L1 이벤트 인덱서 |
| `crates/desktop-app/ui/src-tauri/src/commands.rs` | 수정 | set_metadata_uri 커맨드 |

### Phase 3 변경 파일

| 파일 | 변경 유형 | 내용 |
|------|----------|------|
| `platform/client/lib/nostr.ts` | **신규** | Nostr 클라이언트 (EVM 지갑 서명 기반) |
| `platform/client/app/showroom/[id]/page.tsx` | 수정 | 소셜 섹션 + 지갑 연결 UI |

---

## 미구현 항목 (TODO)

> 아래 항목은 Phase 1~3 설계 중 아직 코드로 반영되지 않은 항목들이다.
> 각 항목은 다음 작업 사이클에서 별도 브랜치로 진행한다.

### Phase 1 미구현

| 항목 | 설명 | 우선순위 |
|------|------|---------|
| Desktop 공개 설정 연동 | `L2DetailPublishTab`에서 Platform API로 description/screenshots 실제 저장, `platform_deployment_id` SQLite 컬럼 추가 | 높음 |
| IPFS 스크린샷 업로드 | Pinata API 연동, Desktop Settings에 JWT 입력 UI, `ipfsToHttp` 게이트웨이 변환 | 중간 |

### Phase 2 미구현

| 항목 | 설명 | 우선순위 |
|------|------|---------|
| Deployer `set_metadata_uri` | Rust `cmd/ethrex/l2/deployer.rs`에 배포 후 metadataURI 설정 함수 추가 | 높음 |
| `ethrex_metadata` RPC 엔드포인트 | `crates/l2/networking/rpc/l2/metadata.rs` — L2 노드에서 chain_id, proposer, bridge, metadataURI 반환 | 중간 |
| L1 인덱서 | `platform/server/lib/l1-indexer.js` — MetadataURIUpdated 이벤트 감시 → IPFS fetch → DB 캐시 | 중간 |
| Desktop L1 트랜잭션 서명 | `set_metadata_uri` Tauri 커맨드 — Keychain 개인키로 L1 트랜잭션 서명/전송 | 낮음 |

### Phase 3 미구현

| 항목 | 설명 | 우선순위 |
|------|------|---------|
| Nostr Relay 배포 | `nostr-rs-relay` Docker 배포, `wss://relay.tokamak.network` 설정 | 높음 |
| 스팸 방지 | Relay IP rate limiting, 부적절한 콘텐츠 신고/삭제 기능 | 낮음 |
