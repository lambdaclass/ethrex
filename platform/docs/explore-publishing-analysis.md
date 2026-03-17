# Explore Page Publishing Analysis

## Current State

### Architecture Overview

```
[Desktop Manager]                    [Platform Server]              [Explore Page]
Local SQLite DB ──platformAPI──────> Platform SQLite DB ──────────> GET /api/store/appchains
(user's machine)   registerDeployment()  (server)                    (public, no auth)
                   activateDeployment()
```

### Current Flow (Problems)

1. User deploys L2 via Desktop Manager → stored in **local** SQLite (`crates/desktop-app/local-server/db/`)
2. Manager calls `platformAPI.registerDeployment()` → creates record in **platform** SQLite
3. Manager calls `platformAPI.activateDeployment()` → sets `status = 'active'`
4. `status = 'active'` deployments are **automatically** shown on Explore page

### What's Wrong

| Problem | Description |
|---------|-------------|
| Auto-exposure | Any `active` deployment appears on Explore with no explicit publish request |
| Closed ecosystem | Only deployments made through this platform's Desktop Manager can be listed |
| No open registration | External projects (Rollup Hub, other Tokamak ecosystem chains) have no way to register |
| No owner consent | Deploying ≠ wanting to be publicly listed |
| Isolated metadata | Each platform has its own metadata format — no shared standard |

---

## Existing Solution: `tokamak-rollup-metadata-repository`

Tokamak Network already has a standardized rollup metadata registry:
**https://github.com/tokamak-network/tokamak-rollup-metadata-repository**

### How It Works

```
data/
├── mainnet/                              # Production rollups
│   └── {systemConfigAddress}.json
└── sepolia/                              # Testnet rollups
    └── {systemConfigAddress}.json        # 34 rollups registered (as of 2026-03)
```

1. Rollup operator creates a metadata JSON file (named by SystemConfig contract address)
2. Runs local validation (`npm run validate`)
3. Submits a GitHub Pull Request
4. CI validates schema + sequencer signature
5. Merged → metadata is publicly available

### Metadata Schema (L2RollupMetadata)

Well-defined TypeScript schema covering:

| Section | Fields |
|---------|--------|
| **Identity** | `name`, `description`, `logo`, `website`, `l1ChainId`, `l2ChainId` |
| **Stack** | `rollupType` (optimistic/zk/sovereign), `stack.name`, `stack.version` |
| **Network** | `rpcUrl`, `wsUrl`, `nativeToken` (type, symbol, decimals, l1Address) |
| **Status** | `status` (active/inactive/maintenance/deprecated/shutdown), `createdAt`, `lastUpdated` |
| **L1 Contracts** | `SystemConfig`, `L1StandardBridge`, `OptimismPortal`, `L2OutputOracle`, ... |
| **L2 Contracts** | `NativeToken`, `WETH`, `L2StandardBridge`, ... |
| **Bridges** | Array of `{ name, type, url, status, supportedTokens }` |
| **Explorers** | Array of `{ name, url, type, status }` |
| **Sequencer** | `address`, `batcherAddress`, `proposerAddress` |
| **Staking** | `isCandidate`, `candidateStatus`, `candidateAddress` |
| **Network Config** | `blockTime`, `gasLimit`, `baseFeePerGas` |
| **Withdrawal** | `challengePeriod`, `expectedWithdrawalDelay`, `batchSubmissionFrequency` |
| **Support** | `statusPageUrl`, `supportContactUrl`, `documentationUrl`, `communityUrl` |
| **Auth** | `metadata.signature` (sequencer signs with timestamp, 24h expiry) |

### Sequencer-Based Authorization

```
Message format:
"Tokamak Rollup Registry\nL1 Chain ID: {l1ChainId}\nL2 Chain ID: {l2ChainId}\n
Operation: {register|update}\nSystemConfig: {address}\nTimestamp: {unixTimestamp}"

Validation:
1. Signature must be from on-chain sequencer (SystemConfig.unsafeBlockSigner())
2. 24-hour expiry from timestamp
3. Immutable fields: l1ChainId, SystemConfig, rollupType, stack.name, createdAt
```

---

## Target Architecture: Integrate with Metadata Repository

### Core Principle

> Use `tokamak-rollup-metadata-repository` as the **shared source of truth** for all Tokamak ecosystem rollups.
> The Explore page reads from this repository + adds platform-specific social features on top.

### Why This Approach

| Benefit | Description |
|---------|-------------|
| Open ecosystem | Any Tokamak rollup (Rollup Hub, tokamak-appchain, thanos, future stacks) can register |
| Standard format | One metadata schema across all Tokamak tools and services |
| Decentralized auth | Sequencer signature — no platform account needed |
| Already adopted | 34+ rollups registered on Sepolia, schema is mature |
| GitHub-native | PR-based review, CI validation, version history |
| Separation of concerns | Metadata repo = identity, Platform DB = social features |

### Directory Structure in Metadata Repository

```
tokamak-rollup-metadata-repository/
├── data/                                 # Existing: Thanos rollups (OP Stack)
│   ├── mainnet/
│   │   └── {systemConfigAddress}.json
│   └── sepolia/
│       └── {systemConfigAddress}.json
│
└── tokamak-appchain-data/                # NEW: All Tokamak ecosystem appchains
    ├── 1/                                #   L1 Chain ID: Ethereum Mainnet
    │   ├── tokamak-appchain/             #     Stack type
    │   │   └── {OnChainProposer}.json
    │   └── thanos/
    │       └── {SystemConfig}.json
    ├── 11155111/                          #   L1 Chain ID: Sepolia
    │   ├── tokamak-appchain/
    │   │   └── {OnChainProposer}.json
    │   └── thanos/
    │       └── {SystemConfig}.json
    ├── 17000/                            #   L1 Chain ID: Holesky
    │   └── tokamak-appchain/
    │       └── {OnChainProposer}.json
    └── {any_chain_id}/                   #   Any L1 (non-Ethereum, L3, etc.)
        └── {stack_type}/
            └── {identityContract}.json
```

**Path convention:**
```
tokamak-appchain-data / {l1ChainId} / {stackType} / {identityContract}.json
                            ↑              ↑               ↑
                       L1 chain       Stack type      Core contract address
```

**Stack types and their identity contracts:**

| Stack Type | Identity Contract | Role |
|-----------|------------------|------|
| `tokamak-appchain` | `OnChainProposer` | Batch commit + proof verification + metadata |
| `tokamak-private-app-channel` | TBD | Tokamak Private App Channel |
| `thanos` | `SystemConfig` | Rollup configuration + sequencer registration |
| `py-ethclient` | TBD | Python-based Ethereum client |
| `{future-stack}` | `{stack-specific}` | Each stack defines its own identity contract |

**Why separate `tokamak-appchain-data/` from `data/`:**
- `data/` is already used by Thanos/OP Stack rollups (34+ entries on Sepolia)
- Different metadata schema requirements per stack
- Avoids breaking existing tooling that reads `data/`
- Unified home for ALL Tokamak ecosystem appchains regardless of stack

**Why L1 Chain ID as folder name (not network name):**
- Supports any L1 chain (Ethereum, Arbitrum, private chains)
- Supports L3 (L2-on-L2): e.g. `42161/` for Arbitrum as L1
- No naming ambiguity — chain ID is canonical
- Future-proof: new networks don't need code changes

**Why stack type as subfolder:**
- Identity contract address alone doesn't tell you what contract type it is
- File path alone gives full context: L1 chain + stack + contract
- CI validation can enforce stack-specific schema rules
- Easy to add new stacks without modifying existing structure

### New Architecture

```
┌──────────────────────────────────────────────────────────────────────────┐
│ tokamak-rollup-metadata-repository (GitHub)                             │
│   tokamak-appchain-data/{l1ChainId}/{proposerAddress}.json              │
│   ↑ PR submitted by any rollup operator (sequencer/deployer signature)  │
└──────────────────────────┬───────────────────────────────────────────────┘
                           │ sync (polling or webhook)
                           ▼
┌──────────────────────────────────────────────────────────────────────────┐
│ Platform Server                                                         │
│                                                                         │
│ ┌─────────────────────┐    ┌──────────────────────┐                     │
│ │ explore_listings     │    │ social tables         │                    │
│ │ (synced from repo)   │←──│ reviews, comments,    │                    │
│ │                      │    │ reactions, bookmarks, │                    │
│ │ + visibility toggle  │    │ announcements         │                    │
│ │ + platform extras    │    └──────────────────────┘                     │
│ └──────────┬───────────┘                                                │
│            │                                                            │
│            ▼                                                            │
│   GET /api/store/appchains                                              │
│   (metadata from repo + social stats from DB)                           │
└──────────────────────────────────────────────────────────────────────────┘
                           │
                           ▼
┌──────────────────────────────────────────────────────────────────────────┐
│ Explore Page (Web UI)                                                    │
│   All Tokamak ecosystem rollups                                          │
│   + Reviews, Comments, Ratings, Bookmarks                                │
│   + Live status (RPC probe)                                              │
│   + Announcements (owner wallet)                                         │
└──────────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────────┐
│ Desktop 메신저(Messenger) 공개 토글                                       │
│   공개 토글 ON → metadata repo에 PR 자동 생성 → CI 검증 → 자동 머지     │
│   Publish metadata → IPFS + on-chain setMetadataURI (tokamak-appchain)  │
└──────────────────────────────────────────────────────────────────────────┘
```

---

## Detailed Design

### 1. Metadata Repository Sync

Platform server periodically syncs from the GitHub repository.

#### Option A: GitHub API Polling (Simple)

```js
// platform/server/lib/metadata-sync.js
const REPO = "tokamak-network/tokamak-rollup-metadata-repository";
const BASE_PATH = "tokamak-appchain-data";
const POLL_INTERVAL = 5 * 60 * 1000; // 5 minutes

async function syncMetadata() {
  // 1. List L1 chain ID folders: tokamak-appchain-data/{l1ChainId}/
  const chainDirs = await ghApi(`/repos/${REPO}/contents/${BASE_PATH}`);

  for (const chainDir of chainDirs) {
    if (chainDir.type !== "dir") continue;
    const l1ChainId = parseInt(chainDir.name);

    // 2. List stack type folders: tokamak-appchain-data/{l1ChainId}/{stackType}/
    const stackDirs = await ghApi(`/repos/${REPO}/contents/${BASE_PATH}/${chainDir.name}`);

    for (const stackDir of stackDirs) {
      if (stackDir.type !== "dir") continue;
      const stackType = stackDir.name; // "tokamak-appchain", "thanos", etc.

      // 3. List appchain files in each stack folder
      const files = await ghApi(
        `/repos/${REPO}/contents/${BASE_PATH}/${chainDir.name}/${stackDir.name}`
      );

      for (const file of files) {
        if (!file.name.endsWith(".json")) continue;
        const metadata = await fetchJson(file.download_url);
        const identityContract = file.name.replace(".json", "");

        // 4. Upsert into explore_listings
        upsertListing({
          identity_contract: identityContract,
          l1_chain_id: l1ChainId,
          stack_type: stackType,
          ...mapMetadataToListing(metadata),
        });
      }
    }
  }
}
```

#### Option B: GitHub Webhook (Real-time)

```
POST /api/webhooks/metadata-repo
  → triggered on push to main
  → re-sync changed files only
```

#### Option C: Git Clone + Watch (Most Reliable)

```
- Clone repo to server filesystem
- git pull on interval
- Parse changed files
- No API rate limits
```

**Recommendation**: Start with **Option A** (simple polling), migrate to **Option C** if rate limits become an issue.

### 2. New DB Table: `explore_listings`

Maps metadata repo entries to platform listings with social features.

```sql
CREATE TABLE IF NOT EXISTS explore_listings (
  id TEXT PRIMARY KEY,                            -- UUID
  -- From metadata repo
  identity_contract TEXT NOT NULL,                  -- Identity contract address (filename in repo)
  l1_chain_id INTEGER NOT NULL,                    -- L1 chain ID (folder name in repo)
  stack_type TEXT NOT NULL,                        -- 'tokamak-appchain' | 'thanos' | etc. (subfolder name)
  name TEXT NOT NULL,
  description TEXT,
  logo TEXT,
  website TEXT,
  l1_chain_id INTEGER,
  l2_chain_id INTEGER,
  rollup_type TEXT,                               -- 'optimistic' | 'zk' | 'sovereign'
  stack_name TEXT,                                -- 'tokamak-appchain' | 'thanos' | etc.
  stack_version TEXT,
  rpc_url TEXT,
  ws_url TEXT,
  -- Native token
  native_token_symbol TEXT,
  native_token_name TEXT,
  native_token_decimals INTEGER,
  native_token_l1_address TEXT,
  -- Contracts (JSON)
  l1_contracts TEXT,                              -- JSON object
  l2_contracts TEXT,                              -- JSON object
  -- Services
  bridges TEXT,                                   -- JSON array
  explorers TEXT,                                 -- JSON array
  support_resources TEXT,                         -- JSON object
  -- Sequencer
  sequencer_address TEXT,
  proposer_address TEXT,
  batcher_address TEXT,
  -- Staking
  is_candidate INTEGER DEFAULT 0,
  candidate_status TEXT,
  -- Network config
  block_time INTEGER,
  gas_limit TEXT,
  -- Status from repo
  repo_status TEXT DEFAULT 'active',              -- from metadata JSON
  -- Platform-specific
  visibility TEXT DEFAULT 'public',               -- owner can toggle: 'public' | 'unlisted'
  owner_wallet TEXT,                              -- derived from sequencer address
  -- Source tracking
  source TEXT DEFAULT 'metadata-repo',            -- 'metadata-repo' | 'platform' | 'manual'
  deployment_id TEXT REFERENCES deployments(id),  -- link to platform deployment (if applicable)
  repo_last_updated TEXT,                         -- lastUpdated from metadata JSON
  -- Timestamps
  synced_at INTEGER,                              -- last sync time
  created_at INTEGER NOT NULL,

  UNIQUE(identity_contract, l1_chain_id, stack_type)
);
```

### 3. Field Mapping: Metadata Repo → Explore Listing

```
Metadata Repo JSON              →  explore_listings column       →  Explore Page UI
─────────────────────────────────────────────────────────────────────────────────────
name                            →  name                          →  Appchain title
description                     →  description                   →  Description card
logo                            →  logo                          →  Avatar/icon
website                         →  website                       →  Website link
l1ChainId                       →  l1_chain_id                   →  L1 network badge
l2ChainId                       →  l2_chain_id                   →  Chain ID display
rollupType                      →  rollup_type                   →  Type badge (Optimistic/ZK)
stack.name                      →  stack_name                    →  Stack badge (Tokamak Appchain/Thanos)
rpcUrl                          →  rpc_url                       →  Live status probe
status                          →  repo_status                   →  Status indicator
nativeToken.symbol              →  native_token_symbol           →  Token badge
bridges[0].url                  →  (from bridges JSON)           →  Bridge link
explorers[0].url                →  (from explorers JSON)         →  Explorer link
sequencer.address               →  sequencer_address             →  Owner identity
sequencer.proposerAddress       →  proposer_address              →  On-chain reference
supportResources.*              →  support_resources             →  Links section
staking.isCandidate             →  is_candidate                  →  Staking badge
─────────────────────────────────────────────────────────────────────────────────────
(platform social DB)            →  reviews/comments/reactions    →  Community section
(platform social DB)            →  announcements                 →  Pinned announcements
(platform social DB)            →  bookmarks                     →  Bookmark toggle
(RPC probe at runtime)          →  -                             →  Online/Offline indicator
```

### 4. Tokamak-Appchain-Specific Extensions

Tokamak appchains have additional data not in the standard metadata repo schema:

| Feature | Where It Comes From | How to Handle |
|---------------|---------------------|---------------|
| IPFS metadata | `OnChainProposer.setMetadataURI()` | L1 indexer caches in DB (screenshots, extra descriptions) |
| Screenshots | IPFS metadata JSON | Store in `explore_listings.screenshots` (platform-only field) |
| Dashboard URL | IPFS metadata / deployment config | Map to explorer/bridge URLs |
| Program type | `programs` table (evm-l2, zk-dex, tokamon) | Map to `stack_name` or custom field |

For tokamak-appchain deployments registered through the platform:
1. Create entry in metadata repo (PR or direct)
2. Sync populates `explore_listings`
3. Link `deployment_id` for platform-specific features
4. L1 indexer enriches with IPFS metadata (screenshots, etc.)

### 5. Registration Flows

#### Flow A: Via Metadata Repository (Primary — All Tokamak Rollups)

```
Any rollup operator:
1. Fork tokamak-rollup-metadata-repository
2. Create tokamak-appchain-data/{l1ChainId}/{stackType}/{identityContract}.json
3. Sign with deployer/sequencer key
4. Submit PR → CI validates → merge
5. Platform syncs within 5 minutes → appears on Explore
```

#### Flow B: Via Desktop Messenger 공개 토글

```
Desktop 메신저(Messenger) → 배포 상세 → "공개(Public)" 토글 ON:
1. 메신저가 배포 설정에서 메타데이터 JSON 빌드
2. Sequencer/deployer 키로 서명
3. GitHub API로 metadata-repo에 PR 자동 생성
4. CI 자동 검증 → 자동 머지
5. Platform 서버가 5분 내 동기화 → Explore 페이지에 노출
```

별도 웹 등록 폼은 불필요 — metadata-repo에 직접 PR을 올리거나, Desktop 메신저의 공개 토글을 사용.

### 6. Explore Page Updates

#### New Features Needed

```
Explore Page Header:
  [Search] [Filters: Stack▼ Network▼ Status▼]

Appchain Card (updated):
  ┌─────────────────────────────────────────────────┐
  │ [Logo] My Appchain                    [Online]  │
  │ Thanos · Sepolia · TON                          │
  │ "Description text..."                           │
  │                                                 │
  │ ★ 4.2 (12 reviews)  💬 5 comments              │
  │ Staking Candidate ✓                             │
  └─────────────────────────────────────────────────┘

New filters:
  - By stack: Tokamak Appchain / Tokamak Private App Channel / Thanos / py-ethclient / All
  - By network: Mainnet / Sepolia / All
  - By rollup type: Optimistic / ZK / All
  - By native token: ETH / TON / Custom
  - By staking: Candidates only
```

#### Detail Page Updates

```
Appchain Detail:
  ┌ Header Card ──────────────────────────────────┐
  │ [Logo] Name          [Stack Badge] [Network]  │
  │ Description                                   │
  │ Rollup Type: Optimistic                       │
  │ Native Token: TON (18 decimals)               │
  │ Block Time: 2s                                │
  └───────────────────────────────────────────────┘

  ┌ Network Card ─────────────────────────────────┐
  │ Services:                                     │
  │   Bridge: https://bridge.example.com [Active] │
  │   Explorer: https://explorer... [Active]      │
  │                                               │
  │ L1 Contracts:                                 │
  │   SystemConfig: 0x...                         │
  │   OptimismPortal: 0x...                       │
  │   L1StandardBridge: 0x...                     │
  │                                               │
  │ Sequencer: 0x...                              │
  │ Proposer: 0x...                               │
  │ Batcher: 0x...                                │
  │                                               │
  │ Staking: ✓ Active Candidate                   │
  └───────────────────────────────────────────────┘

  ┌ Community (unchanged) ────────────────────────┐
  │ Announcements / Reviews / Comments            │
  └───────────────────────────────────────────────┘
```

---

## Development Tasks

### Phase 1: Metadata Repo Sync (Core)

| # | Task | Files | Effort |
|---|------|-------|--------|
| 1 | Create `explore_listings` table + migration | `schema.sql`, `db.js` | S |
| 2 | Create `platform/server/db/listings.js` — CRUD | new file | M |
| 3 | Create `platform/server/lib/metadata-sync.js` — GitHub API polling | new file | M |
| 4 | Update `routes/store.js` — read from `explore_listings` | existing | M |
| 5 | Update Explore page — new card layout, new filters (stack, network, rollup type) | `explore/page.tsx` | L |
| 6 | Update Explore detail page — display full metadata | `explore/[id]/page.tsx` | L |
| 7 | Migrate social features to reference `listing_id` | `social.js`, `announcements.js`, `bookmarks.js` | M |

### Phase 2: Desktop Messenger 공개 토글 연동

| # | Task | Files | Effort |
|---|------|-------|--------|
| 8 | "공개(Public)" 토글 UI in 메신저 배포 상세 | `L2DetailPublishTab.tsx` | M |
| 9 | metadata-repo PR 자동 생성 모듈 | `lib/metadata-repo-pr.ts` (new) | L |
| 10 | Remove auto-exposure — `activateDeployment()` no longer publishes to Explore | `routes/deployments.js` | S |
| 11 | Link `deployment_id` in listing after sync | `metadata-sync.js` | S |

### Phase 4: Enhancements

| # | Task | Files | Effort |
|---|------|-------|--------|
| 15 | Tokamak-appchain-specific metadata extension (screenshots, IPFS) | `l1-indexer.js`, `metadata-sync.js` | M |
| 16 | GitHub webhook for real-time sync (replace polling) | `routes/webhooks.js` (new) | M |
| 17 | Metadata schema extension PR to metadata repo (add tokamak-appchain fields) | external repo | S |

---

## Schema Compatibility: Tokamak Appchain vs Metadata Repo

### What Tokamak Appchain Has That the Repo Doesn't

| Feature | Metadata Repo Equivalent | Action Needed |
|---------------|-------------------------|---------------|
| `OnChainProposer` | Not in OP Stack contracts list | Add to `l1Contracts` as tokamak-appchain field |
| `CommonBridge` | `L1StandardBridge` | Map tokamak-appchain bridge to standard field |
| `GuestProgramRegistry` | No equivalent | Add as custom field |
| `SP1Verifier` | `ZkVerifier` (future) | Already planned in schema |
| `screenshots` | Not in schema | Propose extension or store platform-side |
| `hashtags` | Not in schema | Store platform-side |
| IPFS metadata | Not in schema | Store platform-side (tokamak-appchain enrichment) |

### What the Repo Has That the Explore Page Doesn't Show

| Repo Field | Missing in Explore | Action |
|-----------|-------------------|--------|
| `nativeToken` (full detail) | Only name shown | Add token section to detail page |
| `bridges` (multiple) | Single bridge URL | Show all bridges |
| `staking` info | Not shown | Add staking badge |
| `withdrawalConfig` | Not shown | Add withdrawal info section |
| `networkConfig` (blockTime, gas) | Not shown | Add to detail page |
| `rollupType` | Not shown | Add badge |
| `stack` (name, version) | Partially shown as `program_name` | Add stack badge |
| `supportResources` | Partially in `social_links` | Map to Links section |

---

## Migration Plan

### Step 1: Create `explore_listings` table

### Step 2: Initial sync from metadata repo
- Fetch all JSON files from `tokamak-appchain-data/{l1ChainId}/{stackType}/`
- Insert into `explore_listings`
- Status = repo's status field
- Visibility = 'public' (default)

### Step 3: Link existing platform deployments
- For each existing `active` deployment:
  - Match by `proposer_address` + `l1_chain_id`
  - Set `deployment_id` in listing
  - Preserve social data (reviews, comments) by mapping `deployment_id` → `listing_id`

### Step 4: Update API
- `GET /api/store/appchains` reads from `explore_listings` + social stats
- Keep old `deployments` table for provisioning lifecycle (unchanged)

### Step 5: Update frontend
- Explore page uses new listing data shape
- Detail page shows full metadata repo fields

---

## Summary

| Aspect | Current | Target |
|--------|---------|--------|
| Data source | Platform SQLite only | `tokamak-rollup-metadata-repository` + Platform SQLite |
| Who can list | Platform Desktop 매니저 users only | Any Tokamak rollup operator (via metadata repo PR or 메신저 공개 토글) |
| How to list | Auto-exposed on deployment activation | Explicit: submit PR to metadata repo (or use web form) |
| Metadata format | Custom per-platform | Standardized `L2RollupMetadata` schema |
| Auth for registration | Platform account (OAuth) | Sequencer signature (on-chain identity) |
| Supported stacks | tokamak-appchain only | Tokamak Appchain, Tokamak Private App Channel, Thanos, py-ethclient, future stacks |
| External chains | Not supported | Fully supported — 34+ already registered |
| Social features | Tied to `deployments` | Layered on top of `explore_listings` |
| Visibility control | None (all active = public) | Owner-controlled (public/unlisted) |

---

## How This Platform Uses the Metadata Repository

Once the `tokamak-appchain-data/` directory structure and schema are established in the metadata repository, this platform (Tokamak Appchain Platform) consumes the data as follows:

### End-to-End Flow

```
┌─ Appchain Operator ─────────────────────────────────────────────────────────┐
│                                                                             │
│  Option 1: Direct PR to metadata repo                                       │
│    $ fork → create JSON → sign → submit PR → CI validates → merge           │
│                                                                             │
│  Option 2: Via Desktop 메신저(Messenger) 공개 토글                            │
│    Deploy L2 → 공개 토글 ON → 메신저가 JSON 빌드 + 서명 → PR 자동 생성      │
│                                                                             │
└──────────────────────────────────────────┬──────────────────────────────────┘
                                           │ merged to main branch
                                           ▼
┌─ tokamak-rollup-metadata-repository ────────────────────────────────────────┐
│  tokamak-appchain-data/                                                     │
│    11155111/tokamak-appchain/0xABC...json          ← Sepolia tokamak-appchain       │
│    11155111/tokamak-private-app-channel/0xGHI...json ← Sepolia private channel    │
│    11155111/thanos/0xDEF...json                    ← Sepolia Thanos rollup         │
│    11155111/py-ethclient/0x789...json               ← Sepolia py-ethclient         │
│    1/tokamak-appchain/0x123...json                 ← Mainnet tokamak-appchain      │
│    42161/tokamak-appchain/0x456...json              ← Arbitrum L3                  │
└──────────────────────────────────────────┬──────────────────────────────────┘
                                           │ sync (every 5 min)
                                           ▼
┌─ Platform Server ───────────────────────────────────────────────────────────┐
│                                                                             │
│  1. metadata-sync.js                                                        │
│     - Polls GitHub API for tokamak-appchain-data/ directory                 │
│     - Parses each {l1ChainId}/{stackType}/{contract}.json                   │
│     - Upserts into explore_listings table                                   │
│     - Detects new/updated/removed entries                                   │
│                                                                             │
│  2. explore_listings table (synced from repo)                               │
│     - Identity: identity_contract + l1_chain_id + stack_type (unique)       │
│     - All metadata fields from JSON                                         │
│     - Platform-only fields: visibility, deployment_id                       │
│                                                                             │
│  3. Social tables (platform-only, NOT in repo)                              │
│     - reviews, comments, reactions → linked to listing_id                   │
│     - announcements → owner_wallet writes, public reads                     │
│     - bookmarks → per user account                                          │
│                                                                             │
│  4. L1 Indexer (enrichment for tokamak-appchain type)                       │
│     - Watches MetadataURIUpdated events on known proposer contracts          │
│     - Fetches IPFS metadata (screenshots, extra descriptions)               │
│     - Updates explore_listings with enriched data                           │
│                                                                             │
│  5. API endpoints                                                           │
│     GET /api/store/appchains                                                │
│       → explore_listings (repo data) + social stats (platform data)         │
│     GET /api/store/appchains/:id                                            │
│       → full listing + reviews + comments + announcements                   │
│     POST /api/store/appchains/:id/rpc-proxy                                 │
│       → live status probe (eth_blockNumber, eth_chainId, etc.)              │
│                                                                             │
└──────────────────────────────────────────┬──────────────────────────────────┘
                                           │
                                           ▼
┌─ Explore Page (Web UI) ─────────────────────────────────────────────────────┐
│                                                                             │
│  List View:                                                                 │
│    - All appchains from all stacks (tokamak-appchain + thanos + ...)        │
│    - Filters: stack type, L1 network, rollup type, native token, staking    │
│    - Sort: newest, top-rated, most-reviewed                                 │
│    - Live online/offline status via RPC probe                               │
│    - Social stats (rating, review count, comment count)                     │
│                                                                             │
│  Detail View:                                                               │
│    - Full metadata from repo (contracts, bridges, explorers, config)        │
│    - Live status (block number, batch number, gas price)                    │
│    - Community: announcements, reviews, comments, reactions                 │
│    - Bookmarking                                                            │
│                                                                             │
│  등록 방법:                                                                  │
│    - 직접 metadata-repo에 PR 제출 (Git 사용)                                 │
│    - Desktop 메신저(Messenger)의 공개 토글로 자동 PR 생성                      │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Data Ownership Separation

```
Metadata Repository (GitHub)          Platform Server (SQLite)
═══════════════════════════           ═══════════════════════════
Appchain identity                     Social features
  - name, description, logo             - reviews (wallet-based)
  - contracts (L1/L2)                   - comments (wallet-based)
  - RPC URL, chain ID                   - reactions (wallet-based)
  - bridges, explorers                  - bookmarks (account-based)
  - sequencer info                      - announcements (owner wallet)
  - native token config
  - network config                    Platform-specific
  - staking info                        - visibility toggle
  - support resources                   - deployment_id link
                                        - IPFS screenshots cache
Who controls: appchain operator         - live status cache
How: PR to GitHub repo
Auth: sequencer/deployer signature    Who controls: platform
                                      How: API calls
                                      Auth: wallet signature / OAuth
```

### What Changes vs What Stays the Same

**Changes (this work):**
1. Explore page reads from `explore_listings` (synced from repo) instead of `deployments`
2. Any Tokamak rollup operator can submit to Explore (not just platform users)
3. Desktop 메신저(Messenger) 공개 토글이 metadata repo에 PR을 자동 생성
4. Social features (reviews, comments) reference `listing_id` instead of `deployment_id`
5. New filters on Explore page: stack type, L1 network, rollup type

---

## Registration Validation & Auto-Publishing

### How Does a Registered Appchain Appear on the Explore Page?

```
Operator submits PR  →  CI Bot validates  →  Auto-merge  →  Platform syncs  →  Visible on Explore
     (1 min)              (automated)         (if pass)       (5 min poll)       (automatic)
```

**Fully automated — no manual approval needed.** If CI validation passes, the PR is auto-merged and the appchain appears on Explore within ~5 minutes.

### What Gets Validated (CI Bot)

The metadata repository's CI pipeline validates every PR automatically:

| Check | What It Verifies | How |
|-------|-----------------|-----|
| **Schema validation** | JSON matches the required schema (all required fields present, correct types) | JSON Schema validator |
| **Signature verification** | Signed by the actual on-chain sequencer/deployer of the appchain | Recover signer from signature → compare to on-chain `sequencer()` or contract deployer |
| **Contract existence** | Identity contract (OnChainProposer / SystemConfig) exists on the specified L1 chain | `eth_getCode(address)` on L1 RPC |
| **Chain ID match** | `l2ChainId` in metadata matches what the L2 RPC actually reports | `eth_chainId` call to the provided `rpcUrl` |
| **L1 chain match** | File is in the correct `{l1ChainId}/` folder | Compare folder name to `l1ChainId` in JSON |
| **Stack type match** | File is in the correct `{stackType}/` folder | Compare folder name to `stack.name` in JSON |
| **Immutable fields** | On update: `l1ChainId`, identity contract, `rollupType`, `stack.name`, `createdAt` haven't changed | Compare to existing file in main branch |
| **Timestamp** | `metadata.signature` timestamp within 24 hours, `lastUpdated` after previous value | Clock check |
| **File naming** | Filename matches identity contract address (lowercase) | String comparison |
| **No duplicate** | No existing entry with same identity contract + L1 chain ID + stack type | File existence check |

### Validation Flow

```
PR Submitted
    │
    ▼
┌─ CI Bot (GitHub Actions) ──────────────────────────────────┐
│                                                             │
│  Step 1: Parse JSON → schema validation                     │
│    ✗ fail → bot comments "Missing required field: rpcUrl"   │
│    ✓ pass → continue                                        │
│                                                             │
│  Step 2: Verify on-chain                                    │
│    - eth_getCode(identityContract) on L1                    │
│    - eth_chainId on L2 rpcUrl                               │
│    ✗ fail → "Contract not found on chain 11155111"          │
│    ✓ pass → continue                                        │
│                                                             │
│  Step 3: Verify signature                                   │
│    - Recover signer address from metadata.signature          │
│    - For tokamak-appchain:                                   │
│        Call OnChainProposer.owner() → Timelock.admin()      │
│    - For thanos:                                             │
│        Call SystemConfig.unsafeBlockSigner()                 │
│    ✗ fail → "Signer 0xABC does not match on-chain 0xDEF"   │
│    ✓ pass → continue                                        │
│                                                             │
│  Step 4: All checks pass                                    │
│    → Bot approves PR                                        │
│    → Auto-merge enabled                                     │
│                                                             │
└─────────────────────────────────────────────────────────────┘
    │
    ▼ (merged to main)
    │
    ▼
┌─ Platform Server (metadata-sync.js) ───────────────────────┐
│  Polls every 5 minutes                                      │
│    → Detects new/updated JSON file                          │
│    → Upserts into explore_listings                          │
│    → Appchain visible on Explore page                       │
└─────────────────────────────────────────────────────────────┘
```

### Why No Manual Approval?

| Concern | How It's Handled Without Manual Review |
|---------|---------------------------------------|
| Spam/fake chains | Signature must match on-chain sequencer — you can't fake owning a contract |
| Invalid data | Schema validation rejects malformed JSON |
| Non-existent chains | On-chain contract check + RPC reachability check |
| Impersonation | Cryptographic signature verification against on-chain identity |
| Inappropriate content | Description/name fields can be moderated post-publish (flag + review) |

The on-chain signature is the key — **only the actual operator of the appchain can register it.** This is stronger than any manual review because it's cryptographically proven.

### Edge Case: Offensive Content Moderation

For the rare case of inappropriate descriptions or names:
- Platform admin can set `visibility = 'hidden'` on a listing (platform-side only, doesn't affect repo)
- Community flagging system (future enhancement)
- The metadata repo itself can have a `blocklist.json` for extreme cases

### Update & Removal

```
Update metadata:  Edit JSON → sign with new timestamp → PR → CI validates → auto-merge → synced
Remove listing:   Delete JSON file → PR → auto-merge → platform marks as removed on next sync
```

**Stays the same:**
- Social features infrastructure (reviews, comments, reactions, announcements, bookmarks)
- Wallet-based authentication for social actions
- RPC proxy for live status checks
- Desktop 매니저(Manager) deployment lifecycle (local provisioning)
- L1 Indexer for tokamak-appchain IPFS metadata enrichment
- Platform user accounts (OAuth: Google, Naver, Kakao)
