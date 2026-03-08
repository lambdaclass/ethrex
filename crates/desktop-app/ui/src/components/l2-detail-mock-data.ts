import type { L2Config } from './MyL2View'
import type { Comment } from '../types/comments'
import type { ChainMetrics, EconomyMetrics, Product } from './L2DetailView'

export function getMockChainMetrics(l2: L2Config): ChainMetrics {
  const on = l2.status === 'running'
  return {
    l1BlockNumber: on ? 1247 : 0, l2BlockNumber: on ? 3891 : 0,
    l1ChainId: 3151908, l2ChainId: l2.chainId || 65536999,
    l2Tps: on ? 12.4 : 0, l2BlockTime: 2,
    totalTxCount: on ? 48210 : 0, activeAccounts: on ? 156 : 0,
    lastCommittedBatch: on ? 142 : 0, lastVerifiedBatch: on ? 139 : 0, latestBatch: on ? 145 : 0,
  }
}

export function getMockEconomyMetrics(l2: L2Config): EconomyMetrics {
  const on = l2.status === 'running'
  return {
    tvl: on ? '125.4 ETH' : '0 ETH', tvlUsd: on ? '$312,500' : '$0',
    nativeToken: 'TON', l1TokenAddress: '0x2be5e8c109e2197D077D13A82dAead6a9b3433C5',
    l1GasPrice: on ? '1.2' : '-', l2GasPrice: on ? '0.001' : '-',
    gasRevenue: on ? '2.18 TON' : '0 TON',
    bridgeDeposits: on ? 342 : 0, bridgeWithdrawals: on ? 89 : 0,
  }
}

export function getMockProducts(l2: L2Config): Product[] {
  const base: Product[] = [
    { name: 'Bridge', type: 'infra', status: 'active', description: 'L1↔L2 asset bridge' },
    { name: 'Block Explorer', type: 'tool', status: 'active', description: 'Blockscout-based explorer' },
  ]
  if (l2.programSlug === 'zk-dex') {
    base.unshift({ name: 'ZK-DEX', type: 'dapp', status: 'active', description: 'ZK proof-based decentralized exchange' })
  } else if (l2.programSlug === 'tokamon') {
    base.unshift({ name: 'Tokamon', type: 'dapp', status: 'active', description: 'On-chain gaming state machine' })
  } else {
    base.unshift({ name: 'EVM Runtime', type: 'core', status: 'active', description: 'Full EVM-compatible execution' })
  }
  return base
}

export const L2_DETAIL_MOCK_COMMENTS: Comment[] = [
  {
    id: '1', author: 'kim_dev', avatar: 'K', text: 'ZK-DEX 성능이 정말 좋네요! TPS가 어느정도까지 나오나요?',
    time: '2시간 전', likes: 5, liked: false,
    replies: [
      { id: '1-1', author: 'operator_01', avatar: 'O', text: '현재 테스트넷에서 약 12 TPS 정도 나오고 있습니다. 최적화 진행 중이에요.',
        time: '1시간 전', likes: 3, liked: false, replies: [] },
      { id: '1-2', author: 'lee_blockchain', avatar: 'L', text: '저도 비슷한 결과 확인했습니다. 프루버 성능이 핵심인 것 같아요.',
        time: '45분 전', likes: 1, liked: false, replies: [] },
    ],
  },
  {
    id: '2', author: 'eth_researcher', avatar: 'E', text: '브릿지 수수료가 다른 L2 대비 어느정도인가요? 비교 자료가 있으면 좋겠습니다.',
    time: '5시간 전', likes: 8, liked: true,
    replies: [
      { id: '2-1', author: 'operator_01', avatar: 'O', text: 'L1 가스비 기준으로 약 0.001 gwei 수준입니다. 상세 비교는 문서에 추가할 예정입니다.',
        time: '4시간 전', likes: 2, liked: false, replies: [] },
    ],
  },
  {
    id: '3', author: 'web3_builder', avatar: 'W', text: '이 앱체인에 dApp 배포하려면 어떤 절차가 필요한가요?',
    time: '1일 전', likes: 12, liked: false, replies: [],
  },
]
