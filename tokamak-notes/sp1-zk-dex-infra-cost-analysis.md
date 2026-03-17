# SP1 ZK-DEX 인프라 비용 분석: L2 시퀀서 호스팅

**Date**: 2026-02-24
**기준 문서**: `sp1-zk-dex-vs-baseline.md`
**벤치마크 환경**: Apple M4 Max, SP1 v5.0.8, Rosetta 2

---

## 개요

SP1 ZK-DEX 프로젝트의 인프라를 **L1(외부 RPC) + L2 시퀀서(클라우드) + Prover(로컬)**로
분리 운영할 때의 비용 분석과 최적 호스팅 전략.

### 아키텍처

```
┌──────────┐     배치 제출      ┌──────────────┐     proof 생성      ┌────────────────┐
│  L1 노드  │ ◄──────────────── │ L2 시퀀서     │ ──────────────────► │ SP1 Prover     │
│ (외부 RPC) │     Groth16 proof │ (배치 생성)    │    배치 데이터       │ (로컬 M4 Max)   │
└──────────┘                   └──────────────┘                     └────────────────┘
                                     │
                                     │ tx 수신
                                     ▼
                                  사용자들
```

---

## 1. 역할별 부하 분석

### L1 노드 — 외부 RPC 사용

| 제공업체 | 플랜 | 월 비용 |
|---------|------|--------|
| Infura | Free (10만 req/day) | $0 |
| Alchemy | Growth | $49~199/월 |
| QuickNode | Build | $49~299/월 |

> L1 풀노드 직접 운영 시 최소 2TB SSD + 16GB RAM → 클라우드 기준 $300~500/월.
> 외부 RPC로 $200~500/월 절약 가능.

### L2 시퀀서 — 부하 특성

SP1 ZK-DEX는 EVM 인터프리터가 없는 app-specific circuit이므로 시퀀서 자체 부하가 매우 가벼움.

| 역할 | 설명 | 연산 부하 |
|------|------|----------|
| TX 수신/정렬 | DEX transfer 트랜잭션 수집 | **낮음** |
| 배치 구성 | N개 transfer를 AppProgramInput으로 묶음 | **낮음** |
| Merkle proof 준비 | 현재 state에서 필요한 proof 추출 | **중간** |
| 배치 → Prover 전달 | 직렬화된 input (3,309B/tx) 전송 | **낮음** |
| L1 제출 | Groth16 proof + public inputs → L1 컨트랙트 | **낮음** |

### Prover — 로컬 M4 Max (비용 $0)

| 항목 | 값 | 근거 |
|------|---|------|
| 1 tx proving | 3분 26초 | 실측 (패치 후, Rosetta 2) |
| 100 tx proving | ~5~8분 (추정) | 고정 오버헤드 ~3분 지배 |
| 1,000 tx proving | ~30~60분 (추정) | 사이클 비례 구간 진입 |

> 로컬 장비를 사용하므로 추가 비용 없음.
> 동일 사양을 클라우드에서 운영할 경우 $400~800/월 (GPU 포함 시 $1,000+).

---

## 2. 시퀀서 요구사양

SP1 ZK-DEX 시퀀서는 EVM 인터프리터가 없어 부하가 매우 가볍다.
무거운 작업(SP1 proving)은 전부 Prover(로컬 M4 Max)가 담당하므로 **저사양으로 충분**.

| 항목 | 최소 | 권장 | 근거 |
|------|------|------|------|
| CPU | 2 vCPU | 2 vCPU | TX 정렬 + Merkle proof 추출만 |
| RAM | 2 GB | 4~8 GB | state trie 유지 (incremental MPT) |
| Storage | 200 GB SSD | 200 GB SSD | L2 state + 배치 히스토리 |
| Network | 안정적 연결 | 고정 IP | Prover↔시퀀서, 시퀀서↔L1 |
| 업타임 | 99.9%+ | 99.99% | 시퀀서 다운 = TX 수신 중단 |

> 고사양 인스턴스(4+ vCPU, 16GB+)는 이 프로젝트에서는 과잉.
> state trie가 커져서 메모리가 부족해지면 그때 스케일업하면 된다.

---

## 3. AWS vs Google Cloud vs Hetzner vs Vultr 상세 비교

### 3.1 AWS t3 라인업 참고

| 인스턴스 | vCPU | RAM | On-Demand | 1년 RI | 적합성 |
|---------|------|-----|-----------|--------|--------|
| t3.micro | 2 | 1GB | $7.6 | ~$4.5 | RAM 부족 |
| t3.small | 2 | 2GB | $15 | ~$9 | 최소 운영 가능 |
| **t3.medium** | **2** | **4GB** | **$30** | **~$19** | **적정 (추천)** |
| t3.large | 2 | 8GB | $60 | ~$38 | 여유 있음 |
| t3.xlarge | 4 | 16GB | $121 | ~$75 | 과잉 |

### 3.2 Hetzner Cloud 라인업 참고

Hetzner는 3가지 Shared vCPU 라인이 있음:

| 라인 | 특징 | 시퀀서 적합성 |
|------|------|-------------|
| **CX (Cost-Optimized)** | 최저가, CPU 성능 약간 낮음 | **적합 — 시퀀서 부하가 가벼워 충분** |
| CPX (Regular) | 가격 대비 성능 균형 | 적합 |
| CAX (ARM/Ampere) | ARM64, 최저가 수준 | 호환성 확인 필요 |

**CX (Cost-Optimized) 라인:**

| 인스턴스 | vCPU | RAM | SSD | 가격 | 적합성 |
|---------|------|-----|-----|------|--------|
| **CX23** | **2** | **4GB** | **40GB** | **€3.49 (~$4)** | **적정 (추천)** |
| CX33 | 4 | 8GB | 80GB | €5.49 (~$6) | 여유 있음 |
| CX43 | 8 | 16GB | 160GB | €9.49 (~$10) | 과잉 |

> 스토리지 40GB가 부족해지면 Block Storage Volume 추가: €0.044/GB/월 (100GB = €4.4/월).
> 초기에는 40GB로 충분하고, 필요 시 확장.

### 3.3 Vultr Cloud Compute 라인업 참고

Vultr은 2 vCPU + 4GB 조합이 없어 시퀀서 사양에 정확히 맞는 플랜이 부재.

**Regular Performance (Shared vCPU):**

| 플랜 | vCPU | RAM | SSD | 전송 | 가격 | 적합성 |
|------|------|-----|-----|------|------|--------|
| Regular | 1 | 1GB | 25GB | 1TB | $5/월 | RAM 부족 |
| Regular | 1 | 2GB | 50GB | 2TB | $10/월 | RAM 부족 |
| Regular | 2 | 2GB | 60GB | 3TB | $15/월 | RAM 부족 |
| Regular | 2 | 4GB | 100GB | 3TB | $20/월 | 사용 가능 |
| Regular | 4 | 8GB | 160GB | 4TB | $40/월 | 과잉 |

> Vultr은 데이터 전송이 포함되고 스토리지도 포함이라 추가 비용은 없으나,
> 동일 사양 대비 Hetzner보다 5배 비쌈.

### 3.4 4사 비교 — 적정 사양 (2 vCPU, 4GB RAM)

| 항목 | AWS | Google Cloud | Hetzner | Vultr |
|------|-----|-------------|---------|-------|
| 인스턴스 | t3.medium | e2-medium | **CX23** | Regular 4GB |
| vCPU / RAM | 2 / 4GB | 2 / 4GB | 2 / 4GB | 2 / 4GB |
| **On-Demand** | **$30/월** | **$25/월** | **~$4/월** | **$20/월** |
| **1년 예약/CUD** | **$19/월** | **$16/월** | — | — |
| 스토리지 | +$16 (200GB gp3) | +$34 (200GB SSD PD) | 40GB 포함 (+€7 for 200GB) | 100GB 포함 |
| 데이터 전송 100GB | +$9 | +$12 | 포함 (20TB) | 포함 (3TB) |
| **총합** | **$55** | **$71** | **~$4** | **$20** |

### 3.5 4사 비교 — 여유 사양 (4 vCPU, 8GB RAM)

State trie가 커져서 메모리가 부족해질 경우 스케일업.

| 항목 | AWS | Google Cloud | Hetzner | Vultr |
|------|-----|-------------|---------|-------|
| 인스턴스 | t3.large | e2-standard-2 | CX33 | Regular 8GB |
| vCPU / RAM | 2 / 8GB | 2 / 8GB | 4 / 8GB | 4 / 8GB |
| **On-Demand** | **$60/월** | **$49/월** | **~$6/월** | **$40/월** |
| 스토리지 200GB | +$16 (gp3) | +$34 (SSD PD) | 80GB 포함 (+€5) | 160GB 포함 |
| 데이터 전송 100GB | +$9 | +$12 | 포함 (20TB) | 포함 (4TB) |
| **총합** | **$85** | **$95** | **~$11** | **$40** |

### 3.6 비용 시각화 (적정 사양, On-Demand)

```
Hetzner CX23:  █                                       ~$4
Vultr Regular: █████                                    $20
AWS t3.medium: ██████████████                           $55
GCP e2-medium: ██████████████████                       $71
               └──────────────────────────────────┘
               $0                                  $80
```

---

## 4. 6가지 기준 비교

### 4.1 업타임 / SLA

| | AWS | Google Cloud | Hetzner |
|--|-----|-------------|---------|
| SLA | 99.99% | 99.99% | 99.9% |
| 실제 업타임 | 99.95%+ | 99.95%+ | 99.9%+ |
| 장애 보상 | 서비스 크레딧 | 서비스 크레딧 | 제한적 |
| Multi-AZ 지원 | 있음 | 있음 | 없음 |

### 4.2 L1 노드 근접성

| | AWS | Google Cloud | Hetzner |
|--|-----|-------------|---------|
| 최적 리전 | us-east-1 (버지니아) | us-east1 (사우스캐롤라이나) | fsn1 (핀란드) |
| 이더리움 노드 밀집도 | **최고** | 높음 | 중간 |
| L1 RPC 지연 | ~1~5ms | ~5~10ms | ~20~50ms |

> SP1 ZK-DEX 시퀀서의 L1 제출은 배치당 1회이므로 수십 ms 차이는 실질적으로 무의미.

### 4.3 Prover(로컬 M4 Max) 연결성

| | AWS | Google Cloud | Hetzner |
|--|-----|-------------|---------|
| VPN 설정 | VPC + WireGuard | VPC + WireGuard | WireGuard |
| 로컬→클라우드 지연 | ~30~80ms | ~30~80ms | ~50~150ms |
| 배치 전송 (100 tx ≈ 330KB) | <1초 | <1초 | <1초 |

> Proving이 3~60분이므로 배치 전송 지연은 어떤 제공업체든 무관.

### 4.4 DDoS 방어

| | AWS | Google Cloud | Hetzner |
|--|-----|-------------|---------|
| 기본 보호 | Shield Standard (무료) | Cloud Armor 기본 (무료) | 기본 DDoS 보호 |
| 고급 보호 | Shield Advanced ($3,000/월) | Cloud Armor ($5/정책) | 없음 |

> 초기 단계에서는 기본 보호로 충분.

### 4.5 운영 편의성

| | AWS | Google Cloud | Hetzner |
|--|-----|-------------|---------|
| 모니터링 | CloudWatch (무료 기본) | Cloud Monitoring (무료 기본) | 외부 도구 필요 |
| 자동 복구 | Auto Recovery | Auto Restart | 수동 |
| IaC (Terraform) | 최고 | 우수 | 지원 |
| 한국어 지원 | 있음 | 있음 | 없음 |
| 결제 | 원화 가능 | 원화 가능 | EUR/USD |

### 4.6 무료 티어 상세 비교

| | AWS Free Tier | GCP Free Tier | Hetzner |
|--|--------------|---------------|---------|
| 인스턴스 | t2.micro | e2-micro | 없음 |
| vCPU | 1 | 0.25 (공유, 버스트 2) | — |
| RAM | **1GB** | **1GB** | — |
| 스토리지 | 30GB EBS | 30GB Standard PD | — |
| 기간 | **12개월 한정** | **영구 (Always Free)** | — |
| 시간 | 750시간/월 | 730시간/월 | — |
| 리전 제한 | 없음 | us-west1, us-central1, us-east1만 | — |

#### 시퀀서로 사용 가능한가?

| | AWS t2.micro | GCP e2-micro | Hetzner CX23 ($4) |
|--|-------------|-------------|-------------------|
| RAM | 1GB | 1GB | **4GB** |
| 스토리지 | 30GB | 30GB | 40GB |
| State trie 유지 | **빡빡함** | **빡빡함** | **충분** |
| 시퀀서 운영 | 테스트용만 | 백업용만 | **실 운영 가능** |
| 기간 | 12개월 후 유료 전환 | 영구 무료 | 영구 ~$4/월 |

> AWS/GCP 프리 티어는 둘 다 RAM 1GB라 실제 시퀀서 운영에는 부족.
> **테스트/백업 용도**로만 적합하고, 실 운영은 Hetzner CX23 (~$4/월)이 가장 현실적.
> GCP e2-micro는 영구 무료이므로 **비상 백업 시퀀서**로 활용 가치가 있음.

---

## 5. 총 운영 비용 시나리오

적정 사양(2 vCPU, 4GB) 기준. On-Demand로도 부담 없는 가격대이므로
RI/CUD 없이 On-Demand를 기본으로 비교.

### 시나리오 A: 최저가 — Hetzner (월 ~$4)

| 구성 | 월비용 |
|------|-------|
| L1 RPC: Infura Free | $0 |
| 시퀀서: Hetzner CX23 (2 vCPU, 4GB, 40GB SSD) | ~$4 |
| Prover: 로컬 M4 Max | $0 |
| 모니터링: UptimeRobot Free | $0 |
| **총합** | **~$4/월 ($48/년)** |

### 시나리오 B: AWS (월 ~$55)

| 구성 | 월비용 |
|------|-------|
| L1 RPC: Alchemy Free | $0 |
| 시퀀서: AWS t3.medium ($30) + 200GB gp3 ($16) + 전송 ($9) | ~$55 |
| Prover: 로컬 M4 Max | $0 |
| 모니터링: CloudWatch 기본 | $0 |
| **총합** | **~$55/월 ($660/년)** |

### 시나리오 C: Google Cloud (월 ~$71)

| 구성 | 월비용 |
|------|-------|
| L1 RPC: Alchemy Free | $0 |
| 시퀀서: GCP e2-medium ($25) + 200GB SSD PD ($34) + 전송 ($12) | ~$71 |
| Prover: 로컬 M4 Max | $0 |
| 모니터링: Cloud Monitoring 기본 | $0 |
| **총합** | **~$71/월 ($852/년)** |

> GCP는 인스턴스는 저렴하나 SSD PD가 AWS gp3 대비 2배 이상 비싸서 총합이 높아짐.

### 시나리오 D: Vultr (월 ~$20)

| 구성 | 월비용 |
|------|-------|
| L1 RPC: Alchemy Free | $0 |
| 시퀀서: Vultr Regular (2 vCPU, 4GB, 100GB SSD, 3TB 전송 포함) | $20 |
| Prover: 로컬 M4 Max | $0 |
| **총합** | **~$20/월 ($240/년)** |

> Vultr은 스토리지/전송 포함이라 추가 비용 없음. AWS/GCP 대비 저렴하나 Hetzner 대비 5배.

### 시나리오 E: 하이브리드 — 추천 (월 ~$4)

| 구성 | 월비용 |
|------|-------|
| L1 RPC: Infura Free | $0 |
| 시퀀서 Primary: Hetzner CX23 | ~$4 |
| 시퀀서 Standby: GCP e2-micro (Always Free) | $0 |
| Prover: 로컬 M4 Max | $0 |
| **총합** | **~$4/월 ($48/년)** |

### 연간 비용 비교

```
Hetzner 단독 (A):     █                               $48
하이브리드 (E, 추천):   █                               $48
Vultr (D):            ███                              $240
AWS On-Demand (B):    ████████████                     $660
GCP On-Demand (C):    ████████████████                 $852
                      └──────────────────────────────┘
                      $0                           $900
```

---

## 6. 종합 평가

| 기준 | 1위 | 2위 | 3위 | 4위 |
|------|-----|-----|-----|-----|
| **비용** | Hetzner (~$4) | Vultr ($20) | AWS ($55) | GCP ($71) |
| **안정성** | AWS = GCP | | Vultr | Hetzner |
| **L1 근접성** | AWS | GCP | Vultr | Hetzner |
| **운영 편의** | GCP ≈ AWS | | Vultr | Hetzner |
| **무료 티어** | **GCP** (영구 무료) | AWS (12개월) | Vultr (없음) | Hetzner (없음) |
| **스토리지 가성비** | Hetzner | Vultr (포함) | **AWS** (gp3) | GCP (2배 비쌈) |
| **전송 포함** | Hetzner (20TB) | Vultr (3TB) | AWS (유료) | GCP (유료) |

---

## 7. 단계별 추천

| 단계 | 추천 구성 | 월비용 | 이유 |
|------|----------|-------|------|
| **개발/테스트넷** | Hetzner CX23 + GCP Free 백업 | ~$4 | 비용 최소화, 충분한 성능 |
| **메인넷 초기** | AWS t3.medium (On-Demand) | ~$55 | 안정성 + L1 근접성 |
| **메인넷 확장** | AWS t3.medium + Hetzner (Standby) | ~$59 | 이중화 + 비용 균형 |

> state trie가 커져서 메모리 부족 시 t3.large ($60 On-Demand)로 스케일업.
> 고사양(t3.xlarge 이상)을 처음부터 잡을 필요 없음.

### 핵심 판단 근거

1. **시퀀서에 고사양이 필요한가?** — 아니오. EVM 없는 app-specific circuit이라 매우 가벼움.
   t3.medium (2 vCPU, 4GB)이면 충분하고, 필요 시 스케일업.
2. **RI/CUD가 필요한가?** — 아니오. 적정 사양이면 On-Demand로도 $4~71/월 수준이라 약정 불필요.
3. **GCP vs AWS?** — GCP 인스턴스가 약간 저렴하나 SSD PD가 gp3 대비 2배 비싸서 총합은 AWS가 유리.
   GCP의 장점은 e2-micro Always Free (백업 노드 활용).
4. **Prover를 로컬에 두는 건?** — 최적의 선택. M4 Max의 SP1 성능이 우수하고 클라우드 대비 월 $380~780 절약.
5. **시퀀서↔Prover 지연?** — proving이 3~60분이므로 통신 지연 100ms는 무의미.

---

## 8. SP1 ZK-DEX 고유 이점

SP1 ZK-DEX의 app-specific circuit 최적화(사이클 182배 감소)는 인프라 비용에도 직접적 영향:

| | EVM L2 (baseline) | ZK-DEX (패치 후) |
|--|-------------------|-----------------|
| 시퀀서 부하 | 높음 (EVM 실행) | **매우 낮음** |
| 시퀀서 요구사양 | 8+ vCPU, 32GB+ | **2 vCPU, 4GB** |
| Prover 비용 | 클라우드 $400~1,000+/월 | **로컬 $0 (자체 장비)** |
| **전체 인프라 최소 비용** | **$300~600/월** | **~$4/월** |

> app-specific circuit의 182배 사이클 감소는 성능뿐 아니라 **인프라 비용도 극적으로 절감**시킴.
> Prover를 로컬 장비로, 시퀀서를 Hetzner CX23으로 운영하면 **월 $4로 전체 L2를 운영**할 수 있는 구조 — ZK-DEX의 경제적 실용성을 입증.

---

## Related Documents

- [SP1 ZK-DEX vs Baseline 벤치마크](sp1-zk-dex-vs-baseline.md)
- [SP1 프로파일링 베이스라인](sp1-profiling-baseline.md)
- [프루버 경제모델 및 수수료](analysis-doc-kr/11-프루버-경제모델-및-수수료.md)
