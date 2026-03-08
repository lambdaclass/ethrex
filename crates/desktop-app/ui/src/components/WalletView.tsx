import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useLang } from '../App'
import { DelegationLevel, LEVEL_ORDER } from './wallet-constants'
import DelegationLevelSelector from './DelegationLevelSelector'
import FullDelegationSetup from './FullDelegationSetup'
import PermissionsList from './PermissionsList'

// Mock: recent AI actions
const MOCK_AI_ACTIONS = [
  { time: '2분 전', action: 'L2 Node 자동 재시작', type: 'operate' as const },
  { time: '15분 전', action: '블록 생산 지연 알림 발송', type: 'monitor' as const },
  { time: '1시간 전', action: '가스비 조정 제안 → 승인 대기', type: 'suggest' as const },
  { time: '3시간 전', action: 'Prover 헬스체크 이상 감지', type: 'monitor' as const },
]

const MOCK_AI_ACTIONS_EN = [
  { time: '2m ago', action: 'Auto-restarted L2 Node', type: 'operate' as const },
  { time: '15m ago', action: 'Sent block delay alert', type: 'monitor' as const },
  { time: '1h ago', action: 'Suggested gas adjustment → awaiting approval', type: 'suggest' as const },
  { time: '3h ago', action: 'Detected Prover health issue', type: 'monitor' as const },
]

export default function WalletView() {
  const { lang } = useLang()
  const ko = lang === 'ko'
  const [savedLevel, setSavedLevel] = useState<DelegationLevel>('monitor')
  const [pendingFull, setPendingFull] = useState(false)
  const [walletAddress, setWalletAddress] = useState('')
  const [walletInput, setWalletInput] = useState('')
  const [spendingLimit, setSpendingLimit] = useState('1.0')
  const [aiMode, setAiMode] = useState<string>('tokamak')
  const [hasOwnKey, setHasOwnKey] = useState(false)
  const [aiProvider, setAiProvider] = useState('claude')
  const [aiApiKey, setAiApiKey] = useState('')
  const [aiModel, setAiModel] = useState('claude-sonnet-4-6')
  const [aiSaving, setAiSaving] = useState(false)
  const [aiResult, setAiResult] = useState<{ ok: boolean; msg: string } | null>(null)

  const level = pendingFull ? 'full' as DelegationLevel : savedLevel
  const levelIdx = LEVEL_ORDER.indexOf(level)
  const canFullDelegate = aiMode !== 'tokamak' && hasOwnKey
  const actions = ko ? MOCK_AI_ACTIONS : MOCK_AI_ACTIONS_EN

  const loadAiStatus = () => {
    invoke<{ mode: string }>('get_ai_mode').then(m => setAiMode(m.mode ?? 'tokamak')).catch(() => {})
    invoke<boolean>('has_ai_key').then(ok => setHasOwnKey(ok)).catch(() => {})
  }

  useEffect(() => { loadAiStatus() }, [])

  const handleAiSave = async () => {
    if (!aiApiKey.trim()) return
    setAiSaving(true); setAiResult(null)
    try {
      await invoke('save_ai_config', { provider: aiProvider, apiKey: aiApiKey.trim(), model: aiModel })
      await invoke<string>('test_ai_connection')
      setAiResult({ ok: true, msg: ko ? 'AI 연결 성공!' : 'AI connected!' })
      setAiApiKey('')
      loadAiStatus()
    } catch (e) {
      setAiResult({ ok: false, msg: `${e}` })
    } finally { setAiSaving(false) }
  }

  const handleSelectLevel = (lvl: DelegationLevel) => {
    if (lvl === 'full') {
      setPendingFull(true)
    } else {
      setSavedLevel(lvl)
      setPendingFull(false)
    }
  }

  const handleWalletRegister = () => {
    if (walletInput.trim()) {
      setWalletAddress(walletInput.trim())
      setWalletInput('')
    }
  }

  const actionTypeColor = (type: string) => {
    if (type === 'monitor') return 'var(--color-success)'
    if (type === 'suggest') return '#3b82f6'
    if (type === 'operate') return 'var(--color-warning)'
    return 'var(--color-error)'
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      {/* Header */}
      <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)]">
        <h1 className="text-base font-semibold">{ko ? 'AI 위임 관리' : 'AI Delegation'}</h1>
        <p className="text-[11px] text-[var(--color-text-secondary)] mt-0.5">
          {ko ? 'AI에게 앱체인 운영을 어디까지 맡길지 설정합니다' : 'Configure how much control to delegate to AI'}
        </p>
      </div>

      <div className="flex-1 overflow-y-auto p-3 space-y-3">

        <DelegationLevelSelector
          ko={ko}
          level={level}
          levelIdx={levelIdx}
          pendingFull={pendingFull}
          savedLevel={savedLevel}
          onSelectLevel={handleSelectLevel}
        />

        {(pendingFull || savedLevel === 'full') && (
          <FullDelegationSetup
            ko={ko}
            pendingFull={pendingFull}
            canFullDelegate={canFullDelegate}
            aiMode={aiMode}
            walletAddress={walletAddress}
            walletInput={walletInput}
            spendingLimit={spendingLimit}
            aiProvider={aiProvider}
            aiApiKey={aiApiKey}
            aiModel={aiModel}
            aiSaving={aiSaving}
            aiResult={aiResult}
            onAiProviderChange={setAiProvider}
            onAiApiKeyChange={setAiApiKey}
            onAiModelChange={setAiModel}
            onAiResultClear={() => setAiResult(null)}
            onAiSave={handleAiSave}
            onWalletInputChange={setWalletInput}
            onWalletRegister={handleWalletRegister}
            onWalletRemove={() => { setWalletAddress(''); setWalletInput('') }}
            onSpendingLimitChange={setSpendingLimit}
            onCancel={() => setPendingFull(false)}
            onSave={() => { setSavedLevel('full'); setPendingFull(false) }}
          />
        )}

        <PermissionsList
          ko={ko}
          savedLevel={savedLevel}
          pendingFull={pendingFull}
        />

        {/* Recent AI Actions */}
        <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
          <div className="pb-1">
            <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">
              {ko ? 'AI 활동 기록' : 'AI Activity Log'}
            </span>
          </div>
          <div className="mt-1 space-y-1">
            {actions.map((a, i) => (
              <div key={i} className="flex items-start gap-2 py-1.5 px-2 rounded-lg bg-[var(--color-bg-main)]">
                <span className="w-1.5 h-1.5 rounded-full mt-1.5 flex-shrink-0" style={{ backgroundColor: actionTypeColor(a.type) }} />
                <div className="flex-1 min-w-0">
                  <div className="text-[11px]">{a.action}</div>
                  <div className="text-[9px] text-[var(--color-text-secondary)]">{a.time}</div>
                </div>
              </div>
            ))}
          </div>
        </div>

      </div>
    </div>
  )
}
