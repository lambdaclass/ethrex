import { AI_PROVIDERS, AI_MODELS, maskAddress } from './wallet-constants'

export interface FullDelegationSetupProps {
  ko: boolean
  pendingFull: boolean
  canFullDelegate: boolean
  aiMode: string
  walletAddress: string
  walletInput: string
  spendingLimit: string
  aiProvider: string
  aiApiKey: string
  aiModel: string
  aiSaving: boolean
  aiResult: { ok: boolean; msg: string } | null
  onAiProviderChange: (v: string) => void
  onAiApiKeyChange: (v: string) => void
  onAiModelChange: (v: string) => void
  onAiResultClear: () => void
  onAiSave: () => void
  onWalletInputChange: (v: string) => void
  onWalletRegister: () => void
  onWalletRemove: () => void
  onSpendingLimitChange: (v: string) => void
  onCancel: () => void
  onSave: () => void
}

export default function FullDelegationSetup({
  ko, pendingFull, canFullDelegate, aiMode,
  walletAddress, walletInput, spendingLimit,
  aiProvider, aiApiKey, aiModel, aiSaving, aiResult,
  onAiProviderChange, onAiApiKeyChange, onAiModelChange, onAiResultClear, onAiSave,
  onWalletInputChange, onWalletRegister, onWalletRemove,
  onSpendingLimitChange, onCancel, onSave,
}: FullDelegationSetupProps) {
  const providers = [
    ...AI_PROVIDERS,
    { value: 'custom', label: ko ? '커스텀 URL' : 'Custom URL' },
  ]

  return (
    <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
      <div className="pb-1">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">
          {ko ? '전체 위임 필수 설정' : 'Full Delegation Requirements'}
        </span>
      </div>

      {/* Step 1: AI */}
      <div className="mt-1">
        <div className="flex items-center gap-2 mb-1.5">
          <div className={`w-5 h-5 rounded-full flex items-center justify-center text-[10px] font-bold flex-shrink-0 ${
            canFullDelegate ? 'bg-[var(--color-success)] text-white' : 'bg-[#3b82f6] text-white'
          }`}>
            {canFullDelegate ? '✓' : '1'}
          </div>
          <span className="text-[11px] font-medium">{ko ? '자체 AI 연결' : 'Connect Own AI'}</span>
          {canFullDelegate && (
            <span className="text-[9px] text-[var(--color-success)] ml-auto">{aiMode}</span>
          )}
        </div>
        {!canFullDelegate ? (
          <div className="ml-7 space-y-1.5">
            <div className="text-[9px] text-[var(--color-text-secondary)] mb-1.5">
              {ko ? '자산을 다루므로 공유 AI(Tokamak AI)는 사용할 수 없습니다.' : 'Shared AI cannot be used for asset operations.'}
            </div>
            <select value={aiProvider} onChange={e => { onAiProviderChange(e.target.value); onAiResultClear() }}
              className="w-full bg-[var(--color-bg-main)] rounded-lg px-2.5 py-1.5 text-[11px] outline-none border border-[var(--color-border)] cursor-pointer">
              {providers.map(p => <option key={p.value} value={p.value}>{p.label}</option>)}
            </select>
            <input type="password" value={aiApiKey} onChange={e => { onAiApiKeyChange(e.target.value); onAiResultClear() }}
              placeholder={ko ? 'API 키 입력' : 'Enter API key'}
              className="w-full bg-[var(--color-bg-main)] rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none border border-[var(--color-border)]" />
            <select value={aiModel} onChange={e => onAiModelChange(e.target.value)}
              className="w-full bg-[var(--color-bg-main)] rounded-lg px-2.5 py-1.5 text-[11px] outline-none border border-[var(--color-border)] cursor-pointer">
              {(AI_MODELS[aiProvider] || []).map(m => <option key={m.value} value={m.value}>{m.label}</option>)}
            </select>
            <button onClick={onAiSave} disabled={aiSaving || !aiApiKey.trim()}
              className="w-full bg-[#3b82f6] text-white text-[11px] font-medium py-1.5 rounded-lg hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-40">
              {aiSaving ? (ko ? '연결 확인 중...' : 'Testing...') : (ko ? 'AI 연결' : 'Connect AI')}
            </button>
            {aiResult && (
              <p className={`text-[10px] ${aiResult.ok ? 'text-[var(--color-success)]' : 'text-[var(--color-error)]'}`}>{aiResult.msg}</p>
            )}
          </div>
        ) : (
          <div className="ml-7 text-[9px] text-[var(--color-text-secondary)]">
            {ko ? '자체 AI가 연결되어 있습니다.' : 'Own AI is connected.'}
          </div>
        )}
      </div>

      {/* Divider */}
      <div className="border-t border-[var(--color-border)] my-2.5" />

      {/* Step 2: Wallet */}
      <div>
        <div className="flex items-center gap-2 mb-1.5">
          <div className={`w-5 h-5 rounded-full flex items-center justify-center text-[10px] font-bold flex-shrink-0 ${
            walletAddress ? 'bg-[var(--color-success)] text-white' : canFullDelegate ? 'bg-[#3b82f6] text-white' : 'bg-[var(--color-border)] text-[var(--color-text-secondary)]'
          }`}>
            {walletAddress ? '✓' : '2'}
          </div>
          <span className="text-[11px] font-medium">{ko ? '운영 지갑 등록' : 'Register Wallet'}</span>
          {walletAddress && (
            <code className="text-[9px] font-mono text-[var(--color-success)] ml-auto">{maskAddress(walletAddress)}</code>
          )}
        </div>

        {/* Security Notice */}
        <div className="ml-7 p-2 rounded-lg bg-[var(--color-success)]/10 border border-[var(--color-success)]/30 mb-1.5">
          <div className="flex items-center gap-1.5">
            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="var(--color-success)" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
              <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>
            </svg>
            <span className="text-[9px] text-[var(--color-success)] font-medium">
              {ko ? '하드웨어 보안 저장소(Keychain/TPM)에 암호화 저장' : 'Encrypted in hardware secure storage (Keychain/TPM)'}
            </span>
          </div>
        </div>

        {!walletAddress ? (
          <div className="ml-7 space-y-1.5">
            <input type="password" value={walletInput} onChange={e => onWalletInputChange(e.target.value)}
              placeholder={ko ? '0x... 개인키를 입력하세요' : '0x... Enter private key'}
              disabled={!canFullDelegate}
              className="w-full bg-[var(--color-bg-main)] rounded-lg px-2.5 py-2 text-[11px] font-mono outline-none border border-[var(--color-border)] disabled:opacity-40" />
            <button
              onClick={onWalletRegister}
              disabled={!walletInput.trim() || !canFullDelegate}
              className="w-full bg-[#3b82f6] text-white text-[11px] font-medium py-1.5 rounded-lg hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-40">
              {ko ? '보안 저장소에 등록' : 'Save to Secure Storage'}
            </button>
            {!canFullDelegate && (
              <p className="text-[9px] text-[var(--color-text-secondary)]">{ko ? '먼저 AI를 연결하세요.' : 'Connect AI first.'}</p>
            )}
          </div>
        ) : (
          <div className="ml-7">
            <div className="flex items-center justify-between bg-[var(--color-bg-main)] rounded-lg px-2.5 py-2 border border-[var(--color-border)]">
              <div className="flex items-center gap-2">
                <span className="w-2 h-2 rounded-full bg-[var(--color-success)]" />
                <code className="text-[11px] font-mono">{maskAddress(walletAddress)}</code>
              </div>
              <button onClick={onWalletRemove}
                className="text-[10px] text-[var(--color-error)] hover:underline cursor-pointer">
                {ko ? '삭제' : 'Remove'}
              </button>
            </div>
          </div>
        )}
      </div>

      {/* Balance & Limits (shown when both are set) */}
      {canFullDelegate && walletAddress && (<>
        <div className="border-t border-[var(--color-border)] my-2.5" />
        <div className="grid grid-cols-3 gap-2">
          <div className="bg-[var(--color-bg-main)] rounded-lg p-2 border border-[var(--color-border)]">
            <div className="text-[9px] text-[var(--color-text-secondary)]">{ko ? '잔액' : 'Balance'}</div>
            <div className="text-[13px] font-semibold font-mono mt-0.5">2.45</div>
            <div className="text-[9px] text-[var(--color-text-secondary)]">ETH</div>
          </div>
          <div className="bg-[var(--color-bg-main)] rounded-lg p-2 border border-[var(--color-border)]">
            <div className="text-[9px] text-[var(--color-text-secondary)]">{ko ? '오늘 사용' : 'Today'}</div>
            <div className="text-[13px] font-semibold font-mono mt-0.5">0.08</div>
            <div className="text-[9px] text-[var(--color-text-secondary)]">ETH</div>
          </div>
          <div className="bg-[var(--color-bg-main)] rounded-lg p-2 border border-[var(--color-border)]">
            <div className="text-[9px] text-[var(--color-text-secondary)]">{ko ? '이번 달' : 'Month'}</div>
            <div className="text-[13px] font-semibold font-mono mt-0.5">0.32</div>
            <div className="text-[9px] text-[var(--color-text-secondary)]">ETH</div>
          </div>
        </div>
        <div className="mt-2 space-y-2">
          <div className="flex items-center justify-between">
            <div>
              <div className="text-[11px] font-medium">{ko ? '일일 한도' : 'Daily Limit'}</div>
            </div>
            <div className="flex items-center gap-1">
              <input type="text" value={spendingLimit} onChange={e => onSpendingLimitChange(e.target.value)}
                className="w-14 bg-[var(--color-bg-main)] rounded-lg px-2 py-1 text-[11px] font-mono outline-none border border-[var(--color-border)] text-right" />
              <span className="text-[10px] text-[var(--color-text-secondary)]">ETH</span>
            </div>
          </div>
          <div className="flex items-center justify-between">
            <div>
              <div className="text-[11px] font-medium">{ko ? '건당 한도' : 'Per-Tx Limit'}</div>
            </div>
            <div className="flex items-center gap-1">
              <input type="text" defaultValue="0.1"
                className="w-14 bg-[var(--color-bg-main)] rounded-lg px-2 py-1 text-[11px] font-mono outline-none border border-[var(--color-border)] text-right" />
              <span className="text-[10px] text-[var(--color-text-secondary)]">ETH</span>
            </div>
          </div>
        </div>
      </>)}

      {/* Save / Cancel buttons */}
      <div className="mt-3 flex gap-2">
        {pendingFull && (
          <button
            onClick={onCancel}
            className="flex-1 bg-[var(--color-bg-main)] text-[var(--color-text-secondary)] text-[11px] font-medium py-2 rounded-lg hover:opacity-80 transition-opacity cursor-pointer border border-[var(--color-border)]"
          >
            {ko ? '취소' : 'Cancel'}
          </button>
        )}
        <button
          onClick={onSave}
          disabled={!canFullDelegate || !walletAddress}
          className={`${pendingFull ? 'flex-1' : 'w-full'} bg-[var(--color-success)] text-white text-[11px] font-medium py-2 rounded-lg hover:opacity-80 transition-opacity cursor-pointer disabled:opacity-30`}
        >
          {!canFullDelegate
            ? (ko ? 'AI 설정을 먼저 완료하세요' : 'Complete AI setup first')
            : !walletAddress
              ? (ko ? '지갑을 먼저 등록하세요' : 'Register wallet first')
              : (ko ? '전체 위임 저장' : 'Save Full Delegation')}
        </button>
      </div>
    </div>
  )
}
