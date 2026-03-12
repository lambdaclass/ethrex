# Open Appchain Showroom & Social Features 설계

> 작성일: 2026-03-12
> 브랜치: feat/showroom-social-features
> 상태: 설계 단계

## 1. 배경 및 문제

### 현재 상태
- Desktop 앱에서 **공개 토글** ON → Platform API에 등록 → Showroom에 카드 표시
- 소개글/스크린샷 UI만 있고 실제 저장 미구현
- L2 대시보드 정보 (Explorer, Bridge, 컨트랙트 등) Showroom에 미노출
- Showroom 앱체인 상세 페이지 없음 (카드만 존재)

### 핵심 질문: 데이터를 어디에 저장할 것인가?

블록체인 서비스인데 중앙 서버 DB에 앱체인 메타데이터와 소셜 데이터를 저장하는 것은 철학적으로 맞지 않다. 탈중앙화된 저장소를 활용해야 한다.

---

## 2. 데이터 분류

### 2.1 정적 메타데이터 (변경 빈도: 낮음)
앱체인의 기본 정보. 등록 시 한 번 작성하고 가끔 업데이트.

```
- 앱체인 이름, 설명, 로고/아이콘
- Chain ID, RPC URL, WS URL
- L1/L2 컨트랙트 주소들
- Bridge 정보, Explorer URL
- 네이티브 토큰 정보
- 스크린샷, 소셜 링크
- 네트워크 설정 (blockTime, gasLimit 등)
```

### 2.2 동적 상태 데이터 (변경 빈도: 실시간)
L2 노드에서 직접 조회 가능한 라이브 데이터.

```
- 최신 블록 번호, 배치 번호
- 서비스 상태 (running/stopped)
- 검증된 배치 수
- TPS, 가스 사용률
```

### 2.3 소셜 데이터 (변경 빈도: 중간)
사용자 상호작용 데이터.

```
- 좋아요/별점
- 댓글/리뷰
- 북마크/즐겨찾기
- 사용자 프로필 (크리에이터 페이지)
```

---

## 3. 저장소 옵션 분석

### 3.1 Git-based 메타데이터 (tokamak-rollup-metadata-repository 방식)

**기존 구현:** [tokamak-rollup-metadata-repository](https://github.com/tokamak-network/tokamak-rollup-metadata-repository)

이미 Tokamak 네트워크에는 롤업 메타데이터를 Git 레포에 JSON으로 저장하는 체계가 있다:
- 파일명: `<systemConfig_address>.json`
- 시퀀서 서명으로 인증 (24시간 유효)
- PR 기반 등록/수정 → GitHub Actions 자동 검증
- Thanos 스택 기반 (Optimistic Rollup) 전용
- 네트워크별 폴더: `data/sepolia/`, `data/mainnet/` 등

**현재 스키마 주요 필드:**
```json
{
  "l1ChainId": 11155111,
  "l2ChainId": 123456789,
  "name": "Example L2",
  "description": "...",
  "logo": "https://...",
  "rollupType": "optimistic",
  "stack": { "name": "thanos", "version": "1.0.0" },
  "rpcUrl": "https://...",
  "nativeToken": { "type": "erc20", "symbol": "TON", ... },
  "l1Contracts": { "SystemConfig": "0x...", ... },
  "bridges": [{ "name": "...", "url": "...", ... }],
  "explorers": [{ "name": "...", "url": "...", ... }],
  "sequencer": { "address": "0x...", ... },
  "supportResources": { "communityUrl": "...", ... }
}
```

**장점:**
- 이미 존재하는 인프라
- 투명한 변경 이력 (Git history)
- 시퀀서 서명 검증 체계
- 오프체인이지만 검증 가능

**한계:**
- PR 기반이라 실시간 업데이트 불가
- 소셜 데이터(좋아요, 댓글) 저장 부적합
- Thanos 스택 전용 (Tokamak Appchain ZK 스택과 스키마 상이)
- 읽기 성능: GitHub API rate limit
- GitHub에 의존 — 서비스 중단/검열 시 무력화

---

### 3.2 L2 노드 자체 제공 (Self-Describing L2)

각 L2 노드가 자신의 메타데이터를 직접 서빙하는 방식.

**이미 가능한 것 (Tokamak Appchain L2 RPC):**
```
tokamak_batchNumber          → 최신 배치 번호
tokamak_getBatchByNumber     → 배치 상세 (state_root, blocks, proofs)
eth_chainId                  → 체인 ID
eth_blockNumber              → 최신 블록
eth_gasPrice                 → 가스 가격
```

**추가 구현 필요:**
```
tokamak_metadata             → 앱체인 이름, 설명, 로고 URL, 링크 등
tokamak_contracts            → L1 배포 컨트랙트 주소 목록
tokamak_health               → 서비스 상태 요약
```

**장점:**
- 완전 탈중앙: 각 노드가 자체 정보 제공
- 실시간: 항상 최신 상태
- 신뢰 불필요: 직접 노드에서 조회
- 중앙 서버 DB 불필요

**한계:**
- 노드가 오프라인이면 정보 조회 불가
- 소셜 데이터 저장 불가 (좋아요/댓글은 노드 데이터가 아님)
- 디스커버리 문제: "어떤 L2가 있는지" 목록을 어디서 얻나?

---

### 3.3 온체인 메타데이터 (기존 컨트랙트 확장)

Tokamak Appchain은 L1에 이미 핵심 컨트랙트를 배포한다:
- `OnChainProposer` — 배치 커밋/검증
- `CommonBridge` — L1↔L2 브릿지
- `GuestProgramRegistry` — 프로그램 등록
- `SharedBridgeRouter` — chainId → bridge 매핑

**별도 Registry 컨트랙트 없이, 기존 컨트랙트에 metadataURI 필드를 추가:**

```solidity
// OnChainProposer.sol 에 추가
string public metadataURI;  // "ipfs://Qm..." 또는 Git raw URL

function setMetadataURI(string calldata _uri) external onlyOwner {
    metadataURI = _uri;
    emit MetadataUpdated(_uri);
}

// CommonBridge.sol 에도 동일하게 추가 가능
```

**장점:**
- 새 컨트랙트 배포 불필요
- 이미 앱체인 소유자만 호출 가능한 권한 체계 존재
- 앱체인의 존재 자체가 이 컨트랙트들로 이미 증명됨
- L1이 여러 개여도 (Sepolia, Holesky, Mainnet 등) 각 체인에서 자연스럽게 동작
- `metadataURI`만 읽으면 IPFS/Git의 상세 정보에 접근 가능

**한계:**
- 메타데이터 업데이트마다 가스 비용 (URI 변경 시)
- 큰 데이터 저장 불가 → IPFS 연계 필수
- 소셜 데이터 온체인 저장은 비용 비현실적
- 디스커버리: "어떤 OnChainProposer가 있는지" 목록은 별도 필요

---

### 3.4 탈중앙 소셜 프로토콜 비교

소셜 데이터(좋아요, 댓글)를 위한 탈중앙화 솔루션:

| 기준 | OrbitDB | Nostr | Lens Protocol | Ceramic/ComposeDB | GunDB |
|------|---------|-------|---------------|-------------------|-------|
| **아키텍처** | IPFS + CRDT P2P DB | Relay + WebSocket | Polygon L2 온체인 | Event stream + SQL index | P2P graph DB |
| **데이터 모델** | Key-Value, Log, Feed | JSON Events (NIPs) | NFT (Profile, Post) | GraphQL Composites | Graph (nodes/edges) |
| **인증** | IPFS keys | secp256k1 keypair | EVM wallet | DID (Sign-in with Ethereum) | SEA (Security, Encryption, Authorization) |
| **소셜 기능** | 기본 없음, 직접 구현 | NIP 확장으로 지원 | 네이티브 (follow, comment, mirror) | GraphQL 스키마로 정의 | 직접 구현 |
| **성숙도** | v3 출시 (2025) | 활발 ($10M Dorsey 지원) | 메인넷 출시 | 3Box Labs + Textile 합병 중 | 안정적이나 업데이트 느림 |
| **EVM 호환** | 없음 | 키 페어 호환 (secp256k1) | 네이티브 (Polygon) | SIWE 지원 | 없음 |
| **경량성** | Node.js 필요 + IPFS | Relay 운영 필요 | 체인 의존 | Ceramic 노드 필요 | ~9KB (매우 경량) |
| **오프라인** | ✓ (CRDT) | ✗ | ✗ | ✓ (부분) | ✓ (local-first) |
| **리스크** | 커뮤니티 주도, 펀딩 불안정 | 분산화 잘 됨 | 합병 후 변화 가능 | Textile 합병으로 전환기 | 개발 정체 |

#### 추천 순위

**1순위: Nostr 프로토콜**
- 가장 단순하고 검증된 아키텍처 (smart client / dumb relay)
- WebSocket 기반 실시간 통신
- secp256k1 키 = EVM 주소와 호환 가능한 암호화 체계
- 자체 Relay 운영 가능 → 데이터 주권
- 좋아요(Reaction), 댓글(Reply), 프로필(Metadata) NIP 표준 존재
- 커뮤니티 활발, Jack Dorsey의 $10M 투자

**2순위: OrbitDB**
- IPFS 기반으로 Tokamak 생태계의 탈중앙 철학과 일치
- CRDT 기반 conflict-free 동기화
- v3에서 모듈식 암호화 지원
- 커스텀 데이터 모델 자유롭게 정의 가능
- 단점: IPFS 의존성, 네트워크 부트스트랩 필요

**3순위: 자체 L2 소셜 컨트랙트**
- Tokamak Appchain L2 체인 위에 소셜 스마트 컨트랙트 배포
- 가장 Tokamak-native한 접근
- 단점: L2별로 분산, 크로스체인 집계 필요

---

## 4. 확정 아키텍처

논의를 통해 확정된 하이브리드 설계:

```
┌──────────────────────────────────────────────────────────────┐
│                      Showroom Frontend                        │
│               (Platform Web + Desktop App)                    │
└──────┬─────────────────┬──────────────────┬──────────────────┘
       │                 │                  │
  ┌────▼─────┐    ┌──────▼──────┐    ┌──────▼──────┐
  │ 디스커버리 │    │  메타데이터   │    │    소셜     │
  │ + 존재증명 │    │  (상세정보)  │    │ (상호작용)  │
  └────┬─────┘    └──────┬──────┘    └──────┬──────┘
       │                 │                  │
  ┌────▼──────────┐ ┌────▼────────┐  ┌──────▼───────┐
  │ 기존 컨트랙트  │ │ IPFS + L2   │  │ Nostr Relay  │
  │ + metadataURI │ │ RPC 직접조회 │  │ (또는 OrbitDB)│
  └───────────────┘ └─────────────┘  └──────────────┘
       │
  ┌────▼──────────┐
  │ Git 메타데이터  │
  │ 레포 (백업용)  │
  └───────────────┘

  * Platform DB = 위 데이터의 캐시/인덱서 (source of truth 아님)
```

### 계층 1: 디스커버리 + 존재 증명 (온체인 — 각 L1별)

**방식:** 별도 Registry 컨트랙트 없이, 기존 `OnChainProposer`/`CommonBridge`에 `metadataURI` 필드 추가.

앱체인이 L1에 배포되는 순간 이미 컨트랙트가 존재하므로, 별도 등록 절차 불필요. 공개 설정 시 `metadataURI`만 세팅하면 됨.

```
Sepolia L1
  ├── AppchainA의 OnChainProposer (metadataURI = "ipfs://QmA...")
  └── AppchainB의 OnChainProposer (metadataURI = "ipfs://QmB...")

Holesky L1
  └── AppchainC의 OnChainProposer (metadataURI = "ipfs://QmC...")

Ethereum Mainnet
  └── AppchainD의 OnChainProposer (metadataURI = "ipfs://QmD...")
```

**디스커버리 흐름:**
- Platform 인덱서가 각 L1의 `MetadataUpdated` 이벤트를 리스닝
- 또는 Git 메타데이터 레포에 등록된 컨트랙트 주소 목록을 순회
- 캐시된 목록을 Showroom API로 제공

**Git 메타데이터 레포의 역할:**
- 크로스체인 통합 목록 (여러 L1에 흩어진 앱체인을 한 곳에 집계)
- GitHub 의존성 리스크가 있지만, 온체인 데이터가 1차 source of truth
- 기존 Thanos 스택의 레포는 그대로 유지
- Tokamak Appchain(ZK)용 스키마는 별도 확장 또는 같은 레포에 추가

### 계층 2: 메타데이터 (IPFS + L2 RPC)

**정적 메타데이터 → IPFS**

`metadataURI`가 가리키는 IPFS JSON:

```json
{
  "name": "My ZK Appchain",
  "description": "ZK-proven appchain for DeFi applications",
  "logo": "ipfs://QmLogo...",
  "screenshots": [
    "ipfs://QmScreenshot1...",
    "ipfs://QmScreenshot2..."
  ],
  "website": "https://my-appchain.com",

  "rollupType": "zk",
  "stack": {
    "name": "tokamak-appchain",
    "version": "0.1.0",
    "zkProofSystem": "sp1"
  },

  "l1ChainId": 11155111,
  "l2ChainId": 49152,
  "rpcUrl": "https://rpc.my-appchain.com",
  "wsUrl": "wss://ws.my-appchain.com",

  "nativeToken": {
    "type": "erc20",
    "symbol": "TON",
    "name": "Tokamak Network Token",
    "decimals": 18,
    "l1Address": "0xa30fe40285b8f5c0457dbc3b7c8a280373c40044"
  },

  "l1Contracts": {
    "OnChainProposer": "0x...",
    "CommonBridge": "0x...",
    "SP1Verifier": "0x...",
    "GuestProgramRegistry": "0x...",
    "Timelock": "0x..."
  },

  "explorers": [
    { "name": "L2 Explorer", "url": "https://explorer.my-appchain.com", "type": "blockscout" }
  ],

  "dashboard": {
    "bridgeUI": "https://bridge.my-appchain.com"
  },

  "socialLinks": {
    "twitter": "https://x.com/my_appchain",
    "discord": "https://discord.gg/...",
    "telegram": "https://t.me/..."
  },

  "supportResources": {
    "documentationUrl": "https://docs.my-appchain.com",
    "communityUrl": "https://t.me/my_appchain_community"
  }
}
```

**동적 상태 → L2 RPC 직접 조회**

Showroom 프론트엔드가 `rpcUrl`을 통해 직접 L2 노드에 호출:

```
eth_blockNumber              → 최신 블록
eth_chainId                  → 체인 ID
tokamak_batchNumber          → 최신 배치 번호
tokamak_metadata             → 노드가 서빙하는 기본 정보 (추가 구현 필요)
tokamak_health               → 서비스 상태 요약 (추가 구현 필요)
```

### 계층 3: 소셜 (Nostr Relay)

**Nostr 활용 설계:**

```
Event Kind 정의 (Custom NIP):

Kind 30100: Appchain Review
{
  "kind": 30100,
  "tags": [
    ["d", "<chainId>"],             // 앱체인 식별
    ["rating", "5"],                // 1-5 별점
    ["L", "tokamak-appchain"]       // 네임스페이스
  ],
  "content": "Great DeFi appchain with low fees!"
}

Kind 30101: Appchain Comment
{
  "kind": 30101,
  "tags": [
    ["d", "<chainId>"],
    ["e", "<parent_event_id>"],     // 답글 체인
    ["L", "tokamak-appchain"]
  ],
  "content": "How do I connect my wallet?"
}

Kind 7: Reaction (좋아요)
{
  "kind": 7,
  "tags": [
    ["e", "<review_event_id>"],
    ["L", "tokamak-appchain"]
  ],
  "content": "+"                     // +는 좋아요, -는 싫어요
}

Kind 0: User Profile
{
  "kind": 0,
  "content": "{\"name\":\"Alice\",\"about\":\"Appchain builder\",\"picture\":\"ipfs://...\"}"
}
```

**Relay 운영:**
- Tokamak 자체 Nostr Relay 운영 (1차 수집)
- 공개 Relay에도 발행 가능 (탈중앙 보장)
- WebSocket 기반 실시간 업데이트

**인증:**
- secp256k1 키페어 = EVM 지갑과 동일 곡선
- "Sign-in with Ethereum" → Nostr 키 파생 가능
- 또는 NIP-07 브라우저 확장 지원

---

## 5. 데이터 흐름

### 공개 설정 시 (Desktop → Showroom)

```
사용자가 Desktop 앱에서 "공개" 토글 ON
  ↓
Desktop App
  ├── 1. 스크린샷/로고 → IPFS 업로드 → CID 획득
  ├── 2. 메타데이터 JSON 생성 → IPFS 업로드 → metadataURI 획득
  ├── 3. OnChainProposer.setMetadataURI(metadataURI) 트랜잭션 (해당 L1에)
  ├── 4. (선택) Git 메타데이터 레포에 등록 (크로스체인 집계용)
  └── 5. 로컬 DB is_public = true
  ↓
Platform Server (인덱서)
  ├── L1 이벤트(MetadataUpdated) 리스닝 → 캐시 업데이트
  └── IPFS에서 메타데이터 JSON fetch → DB에 캐싱
```

### Showroom 조회 시

```
Showroom 페이지 로드
  ↓
  1. Platform API → 앱체인 목록 (캐시)
  2. 각 앱체인 카드 클릭 → 상세 페이지
     ├── 정적 정보: Platform 캐시 (원천: IPFS metadataURI)
     ├── 라이브 상태: L2 RPC 직접 호출 (eth_blockNumber, tokamak_batchNumber)
     └── 소셜 데이터: Nostr Relay 구독 (리뷰, 댓글, 좋아요)
```

### 검증 흐름 (trust-minimized)

```
Showroom 사용자가 데이터를 신뢰하고 싶을 때:
  ↓
  1. Platform 캐시의 metadataURI를 L1 컨트랙트에서 직접 확인
  2. IPFS에서 메타데이터 JSON을 직접 fetch
  3. L2 RPC로 라이브 상태 직접 확인
  → Platform이 거짓 데이터를 제공해도 검증 가능
```

---

## 6. 기술 스택 정리

| 계층 | 저장소 | 데이터 | 기술 |
|------|--------|--------|------|
| 디스커버리 + 존재증명 | 기존 L1 컨트랙트 (각 체인별) | metadataURI, isPublic | Solidity (OnChainProposer 확장) |
| 메타데이터 (정적) | IPFS | 설명, 스크린샷, 링크, 컨트랙트 주소 | IPFS/Pinata, JSON |
| 메타데이터 (동적) | L2 RPC 직접 조회 | 블록, 배치, 가스, TPS | Tokamak Appchain RPC |
| 소셜 | Nostr Relay | 리뷰, 댓글, 좋아요, 프로필 | nostr-tools, WebSocket |
| 크로스체인 집계 | Git 메타데이터 레포 (백업) | 전체 앱체인 목록 통합 | GitHub, JSON |
| 캐시/인덱스 | Platform DB | 위 데이터의 통합 캐시 | SQLite (기존 유지) |

---

## 7. 개발 단계

### Phase 1: Showroom 기반 완성 (우선순위: 높음)

Platform DB를 임시 저장소로 활용하되, 향후 탈중앙 마이그레이션을 고려한 구조.

**1.1 Showroom 앱체인 상세 페이지**
- [ ] `/showroom/[id]` 페이지 생성
- [ ] 앱체인 정보 카드 (이름, 설명, Chain ID, 컨트랙트 주소 등)
- [ ] L2 RPC 직접 조회로 라이브 상태 표시 (블록 번호, 배치 번호)
- [ ] Explorer, Bridge Dashboard 링크
- [ ] 스크린샷 갤러리

**1.2 Desktop 공개 설정 완성**
- [ ] 소개글 실제 저장 (Platform API로 전송)
- [ ] 스크린샷 IPFS 업로드 → CID 저장
- [ ] Dashboard URL (Explorer, Bridge UI) 자동 포함
- [ ] `deployments` 테이블 필드 추가: `description`, `screenshots`, `dashboard_url`, `explorer_url`, `social_links`

**1.3 Tokamak Appchain 메타데이터 RPC**
- [ ] `tokamak_metadata` 커스텀 RPC 메서드 설계
- [ ] L2 노드가 자신의 기본 정보를 직접 서빙

### Phase 2: 온체인 메타데이터 (우선순위: 중간)

**2.1 기존 컨트랙트 확장**
- [ ] `OnChainProposer`에 `metadataURI` + `setMetadataURI()` 추가
- [ ] `CommonBridge`에도 동일 패턴 적용 (선택)
- [ ] `MetadataUpdated` 이벤트 정의

**2.2 IPFS 메타데이터 체계**
- [ ] Tokamak Appchain 메타데이터 JSON 스키마 정의
- [ ] IPFS Pinning 서비스 연동 (Pinata 또는 자체 IPFS 노드)
- [ ] Desktop 앱에서 공개 설정 시 IPFS 업로드 + 온체인 URI 등록 자동화

**2.3 Platform 인덱서**
- [ ] L1 이벤트 리스닝 → DB 캐시 자동 업데이트
- [ ] Platform DB를 source of truth에서 캐시로 전환

**2.4 Git 메타데이터 레포 연동**

기존 레포는 Thanos 스택 전용 (`data/<network>/` 폴더). Tokamak Appchain(ZK)은 별도 폴더 체계를 추가:

```
tokamak-rollup-metadata-repository/
├── data/                              ← 기존 Thanos 스택 (그대로 유지)
│   └── sepolia/
│       └── <systemConfig_address>.json
│
├── tokamak-appchain-data/             ← Tokamak Appchain(ZK) 전용 (신규)
│   ├── sepolia/
│   │   └── <onChainProposer_address>.json
│   ├── holesky/
│   │   └── <onChainProposer_address>.json
│   └── mainnet/
│       └── <onChainProposer_address>.json
│
├── schemas/
│   ├── rollup-metadata.ts             ← 기존 Thanos 스키마
│   ├── tokamak-appchain-metadata.ts   ← Tokamak Appchain 스키마 (신규)
│   └── example-tokamak-appchain.json  ← 예시 (신규)
│
└── validators/
    ├── ...                            ← 기존 검증기
    └── tokamak-appchain-validator.ts  ← Tokamak Appchain 검증기 (신규)
```

**키 차이점:**
- Thanos: `systemConfig` 주소가 파일명 (Optimistic Rollup의 핵심 컨트랙트)
- Tokamak Appchain: `onChainProposer` 주소가 파일명 (ZK Rollup의 핵심 컨트랙트)
- 서명 검증: Thanos는 시퀀서 서명, Tokamak Appchain은 OnChainProposer owner 서명

**Tokamak Appchain 메타데이터 예시** (`tokamak-appchain-data/sepolia/0xABCD...1234.json`):
```json
{
  "l1ChainId": 11155111,
  "l2ChainId": 49152,
  "name": "My ZK Appchain",
  "description": "ZK-proven appchain for DeFi",
  "logo": "ipfs://QmLogo...",
  "screenshots": ["ipfs://QmScreenshot1..."],
  "rollupType": "zk",
  "stack": { "name": "tokamak-appchain", "version": "0.1.0", "zkProofSystem": "sp1" },
  "rpcUrl": "https://rpc.my-appchain.com",
  "nativeToken": { "type": "erc20", "symbol": "TON", "decimals": 18 },
  "l1Contracts": {
    "OnChainProposer": "0xABCD...1234",
    "CommonBridge": "0x...",
    "SP1Verifier": "0x...",
    "GuestProgramRegistry": "0x..."
  },
  "explorers": [{ "name": "L2 Explorer", "url": "https://...", "type": "blockscout" }],
  "dashboard": { "bridgeUI": "https://bridge.my-appchain.com" },
  "socialLinks": { "twitter": "...", "discord": "...", "telegram": "..." },
  "status": "active",
  "createdAt": "2026-03-12T00:00:00Z",
  "lastUpdated": "2026-03-12T00:00:00Z",
  "metadata": {
    "version": "1.0.0",
    "signature": "0x[OWNER_SIGNATURE]",
    "signedBy": "0x[OWNER_ADDRESS]"
  }
}
```

- [ ] `tokamak-appchain-data/` 폴더 구조 및 스키마 정의
- [ ] Tokamak Appchain 전용 검증기 구현 (OnChainProposer owner 서명 검증)
- [ ] Desktop 앱에서 공개 설정 시 자동 PR 생성 또는 API 연동

### Phase 3: 소셜 기능 (우선순위: 중간)

**3.1 Nostr Relay 셋업**
- [ ] Tokamak Nostr Relay 서버 운영
- [ ] Custom Event Kind 정의 (리뷰 Kind 30100, 댓글 Kind 30101)
- [ ] 앱체인별 이벤트 필터링 체계

**3.2 Showroom 소셜 UI**
- [ ] 별점/리뷰 작성 (Nostr 이벤트 발행)
- [ ] 댓글 스레드 (Nostr 답글 체인)
- [ ] 좋아요 (Nostr Reaction)
- [ ] 크리에이터 프로필 페이지

**3.3 인증 연동**
- [ ] Platform 계정 ↔ Nostr 키 연결
- [ ] 또는 EVM 지갑으로 직접 서명 (Sign-in with Ethereum)

---

## 8. 미결정 사항

1. **소셜 프로토콜 최종 선택:** Nostr를 1순위로 권장하지만, OrbitDB가 IPFS 생태계와 더 자연스러운 연동 가능. 프로토타입 비교 필요.
2. **IPFS Pinning 서비스:** Pinata vs nft.storage vs 자체 IPFS 노드
3. **Nostr 키 ↔ EVM 주소 매핑:** NIP-07 확장 vs 커스텀 서명 기반 파생
4. **Git 메타데이터 레포 통합:** 기존 tokamak-rollup-metadata-repository에 `appchains/` 폴더 구조를 추가하는 방향으로 결정 (7.2.4 참조). 세부 스키마 필드 확정 필요.
5. **metadataURI 업데이트 권한:** OnChainProposer의 owner만? 별도 metadata admin 역할?
6. **Showroom에서 오프라인 노드 처리:** L2 RPC 호출 실패 시 UI 표시 방법

---

## 9. 참고 자료

- [tokamak-rollup-metadata-repository](https://github.com/tokamak-network/tokamak-rollup-metadata-repository) — 기존 Thanos 롤업 메타데이터
- [OrbitDB](https://orbitdb.org/) — P2P IPFS 데이터베이스
- [Nostr Protocol](https://github.com/nostr-protocol/nostr) — 탈중앙 소셜 프로토콜
- [Lens Protocol](https://www.lens.xyz/) — 온체인 소셜 그래프
- [Ceramic/ComposeDB](https://ceramic.network/composedb) — 탈중앙 GraphQL DB
- [Farcaster vs Lens 비교](https://blockeden.xyz/blog/2026/01/13/farcaster-vs-lens-socialfi-web3-social-graph/)
- [GunDB](https://github.com/amark/gun) — P2P 그래프 데이터베이스
