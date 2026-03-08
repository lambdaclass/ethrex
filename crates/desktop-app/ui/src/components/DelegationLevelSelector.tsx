import { DelegationLevel, DELEGATION_LEVELS } from './wallet-constants'

export interface DelegationLevelSelectorProps {
  ko: boolean
  level: DelegationLevel
  levelIdx: number
  pendingFull: boolean
  savedLevel: DelegationLevel
  onSelectLevel: (level: DelegationLevel) => void
}

export default function DelegationLevelSelector({ ko, level, levelIdx, pendingFull, savedLevel, onSelectLevel }: DelegationLevelSelectorProps) {
  const selectedOption = DELEGATION_LEVELS.find(d => d.level === level)!
  return (
    <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
      <div className="pb-1">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">
          {ko ? '위임 수준' : 'Delegation Level'}
        </span>
      </div>

      {/* Level bar */}
      <div className="flex items-center gap-1 mt-2 mb-3">
        {DELEGATION_LEVELS.map((opt, i) => (
          <button
            key={opt.level}
            onClick={() => onSelectLevel(opt.level)}
            className={`flex-1 py-1.5 text-[10px] font-medium rounded-lg cursor-pointer transition-all ${
              level === opt.level
                ? pendingFull && opt.level === 'full'
                  ? 'bg-[#f59e0b] text-white shadow-sm'
                  : 'bg-[#3b82f6] text-white shadow-sm'
                : i <= levelIdx
                  ? 'bg-[#3b82f6]/20 text-[#3b82f6]'
                  : 'bg-[var(--color-bg-main)] text-[var(--color-text-secondary)] border border-[var(--color-border)]'
            }`}
          >
            {ko ? opt.labelKo : opt.labelEn}
          </button>
        ))}
      </div>

      {/* Selected level description */}
      <div className={`bg-[var(--color-bg-main)] rounded-lg p-2.5 border ${pendingFull ? 'border-[#f59e0b]/50' : 'border-[var(--color-border)]'}`}>
        <div className="flex items-center gap-2">
          <span className="text-[12px] font-medium">{ko ? selectedOption.labelKo : selectedOption.labelEn}</span>
          {pendingFull && (
            <span className="text-[9px] bg-[#f59e0b]/20 text-[#f59e0b] px-1.5 py-0.5 rounded font-medium">
              {ko ? '설정 중' : 'Pending'}
            </span>
          )}
          {savedLevel === 'full' && !pendingFull && (
            <span className="text-[9px] bg-[var(--color-success)]/20 text-[var(--color-success)] px-1.5 py-0.5 rounded font-medium">
              {ko ? '활성' : 'Active'}
            </span>
          )}
        </div>
        <div className="text-[10px] text-[var(--color-text-secondary)] mt-0.5">
          {ko ? selectedOption.descKo : selectedOption.descEn}
        </div>
        {level !== 'full' && (
          <div className="mt-1.5">
            <span className="text-[9px] text-[var(--color-text-secondary)]">
              {ko
                ? 'Tokamak AI 또는 자체 AI 모두 사용 가능합니다.'
                : 'Works with both Tokamak AI and your own AI.'}
            </span>
          </div>
        )}
      </div>
    </div>
  )
}
