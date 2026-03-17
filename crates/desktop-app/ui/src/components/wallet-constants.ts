export type DelegationLevel = 'monitor' | 'operate' | 'full'

export interface DelegationOption {
  level: DelegationLevel
  labelKo: string
  labelEn: string
  descKo: string
  descEn: string
  needsWallet: boolean
}

export const DELEGATION_LEVELS: DelegationOption[] = [
  {
    level: 'monitor', labelKo: '모니터링', labelEn: 'Monitoring',
    descKo: '감시 + 이상 감지 시 알림 발송 및 조치 제안. 운영자 승인 후 실행합니다.',
    descEn: 'Monitors, sends alerts, and suggests actions. Executes only after operator approval.',
    needsWallet: false,
  },
  {
    level: 'operate', labelKo: '자동 운영', labelEn: 'Auto-Operate',
    descKo: '서비스 시작/중지/재시작, 가스 파라미터 조정을 AI가 자동으로 수행합니다.',
    descEn: 'AI automatically starts/stops/restarts services and adjusts parameters.',
    needsWallet: false,
  },
  {
    level: 'full', labelKo: '전체 위임', labelEn: 'Full Delegation',
    descKo: '온체인 트랜잭션(배치 커밋, 브릿지 등)까지 AI가 자동 집행합니다.',
    descEn: 'AI executes on-chain transactions (batch commits, bridge ops, etc.).',
    needsWallet: true,
  },
]

export interface PermissionCategory {
  id: string
  labelKo: string
  labelEn: string
  minLevel: DelegationLevel
  items: { id: string; labelKo: string; labelEn: string }[]
}

export const PERMISSIONS: PermissionCategory[] = [
  {
    id: 'monitoring', labelKo: '감시 · 제안', labelEn: 'Monitor & Suggest', minLevel: 'monitor',
    items: [
      { id: 'health_check', labelKo: '서비스 헬스체크', labelEn: 'Service health check' },
      { id: 'chain_metrics', labelKo: '체인 지표 수집', labelEn: 'Chain metrics collection' },
      { id: 'log_analysis', labelKo: '에러 로그 분석', labelEn: 'Error log analysis' },
      { id: 'alert', labelKo: '텔레그램 알림 발송', labelEn: 'Telegram alert dispatch' },
      { id: 'suggest', labelKo: '이상 감지 시 조치 제안', labelEn: 'Suggest actions on anomalies' },
    ],
  },
  {
    id: 'infra', labelKo: '인프라 제어', labelEn: 'Infrastructure Control', minLevel: 'operate',
    items: [
      { id: 'restart_service', labelKo: '서비스 자동 재시작', labelEn: 'Auto-restart services' },
      { id: 'scale', labelKo: '리소스 스케일링', labelEn: 'Resource scaling' },
      { id: 'config_adjust', labelKo: '가스 파라미터 조정', labelEn: 'Gas parameter adjustment' },
    ],
  },
  {
    id: 'onchain', labelKo: '온체인 액션', labelEn: 'On-chain Actions', minLevel: 'full',
    items: [
      { id: 'batch_commit', labelKo: '배치 커밋 트랜잭션', labelEn: 'Batch commit transactions' },
      { id: 'proof_submit', labelKo: '증명 제출 트랜잭션', labelEn: 'Proof submission transactions' },
      { id: 'bridge_ops', labelKo: '브릿지 운영', labelEn: 'Bridge operations' },
    ],
  },
]

export const LEVEL_ORDER: DelegationLevel[] = ['monitor', 'operate', 'full']

export const AI_PROVIDERS = [
  { value: 'claude', label: 'Claude (Anthropic)' },
  { value: 'openai', label: 'OpenAI (GPT)' },
]

export const AI_MODELS: Record<string, { value: string; label: string }[]> = {
  claude: [
    { value: 'claude-sonnet-4-6', label: 'Claude Sonnet 4.6' },
    { value: 'claude-opus-4-6', label: 'Claude Opus 4.6' },
    { value: 'claude-haiku-4-5-20251001', label: 'Claude Haiku 4.5' },
  ],
  openai: [
    { value: 'gpt-4o', label: 'GPT-4o' },
    { value: 'gpt-4o-mini', label: 'GPT-4o Mini' },
  ],
  custom: [
    { value: 'default', label: 'Default' },
  ],
}

export function maskAddress(addr: string): string {
  if (addr.length <= 10) return addr
  return `${addr.slice(0, 6)}...${addr.slice(-4)}`
}
