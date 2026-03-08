import type { DelegationLevel } from './wallet-constants'
import { DELEGATION_LEVELS, PERMISSIONS, LEVEL_ORDER } from './wallet-constants'

export interface PermissionsListProps {
  ko: boolean
  savedLevel: DelegationLevel
  pendingFull: boolean
}

export default function PermissionsList({ ko, savedLevel, pendingFull }: PermissionsListProps) {
  const isActive = (minLevel: DelegationLevel) =>
    LEVEL_ORDER.indexOf(savedLevel) >= LEVEL_ORDER.indexOf(minLevel)

  const isPending = (minLevel: DelegationLevel) =>
    pendingFull && !isActive(minLevel) && LEVEL_ORDER.indexOf('full') >= LEVEL_ORDER.indexOf(minLevel)

  return (
    <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
      <div className="pb-1">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">
          {ko ? 'AI 권한' : 'AI Permissions'}
        </span>
      </div>
      <div className="mt-1 space-y-2">
        {PERMISSIONS.map(cat => {
          const active = isActive(cat.minLevel)
          const pending = isPending(cat.minLevel)
          return (
            <div key={cat.id}>
              <div className="flex items-center gap-1.5 mb-1">
                <span className={`text-[11px] font-medium ${active ? '' : pending ? 'text-[#f59e0b]' : 'text-[var(--color-text-secondary)] opacity-50'}`}>
                  {ko ? cat.labelKo : cat.labelEn}
                </span>
                {pending && (
                  <span className="text-[9px] text-[#f59e0b] bg-[#f59e0b]/10 px-1.5 py-0.5 rounded font-medium">
                    {ko ? '저장 시 활성화' : 'Active on save'}
                  </span>
                )}
                {!active && !pending && (
                  <span className="text-[9px] text-[var(--color-text-secondary)] bg-[var(--color-bg-main)] px-1.5 py-0.5 rounded border border-[var(--color-border)]">
                    {ko ? `${DELEGATION_LEVELS.find(d => d.level === cat.minLevel)?.labelKo} 이상` : `Requires ${DELEGATION_LEVELS.find(d => d.level === cat.minLevel)?.labelEn}`}
                  </span>
                )}
              </div>
              <div className="space-y-0.5">
                {cat.items.map(item => (
                  <div key={item.id} className={`flex items-center gap-2 px-2 py-1.5 rounded-lg ${active ? 'bg-[var(--color-bg-main)]' : pending ? 'bg-[#f59e0b]/5' : 'opacity-40'}`}>
                    <div className={`w-3.5 h-3.5 rounded border flex items-center justify-center flex-shrink-0 ${
                      active ? 'border-[#3b82f6] bg-[#3b82f6]' : pending ? 'border-[#f59e0b] bg-[#f59e0b]/20' : 'border-[var(--color-border)]'
                    }`}>
                      {active && (
                        <svg width="8" height="8" viewBox="0 0 24 24" fill="none" stroke="white" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
                          <polyline points="20 6 9 17 4 12"/>
                        </svg>
                      )}
                      {pending && (
                        <svg width="8" height="8" viewBox="0 0 24 24" fill="none" stroke="#f59e0b" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
                          <circle cx="12" cy="12" r="3"/>
                        </svg>
                      )}
                    </div>
                    <span className={`text-[11px] ${pending ? 'text-[var(--color-text-secondary)]' : ''}`}>{ko ? item.labelKo : item.labelEn}</span>
                  </div>
                ))}
              </div>
            </div>
          )
        })}
      </div>
    </div>
  )
}
