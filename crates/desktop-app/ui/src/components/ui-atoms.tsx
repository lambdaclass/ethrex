export const SectionHeader = ({ title }: { title: string }) => (
  <div className="pb-1">
    <span className="text-[10px] font-semibold uppercase tracking-wider text-[var(--color-text-secondary)]">{title}</span>
  </div>
)

export const StatCard = ({ label, value, sub }: { label: string; value: string | number; sub?: string }) => (
  <div className="bg-[var(--color-bg-main)] rounded-lg p-2.5 border border-[var(--color-border)]">
    <div className="text-[10px] text-[var(--color-text-secondary)]">{label}</div>
    <div className="text-[14px] font-semibold mt-0.5 font-mono">{value}</div>
    {sub && <div className="text-[9px] text-[var(--color-text-secondary)] mt-0.5">{sub}</div>}
  </div>
)

export const KV = ({ label, value, mono }: { label: string; value: string; mono?: boolean }) => (
  <div className="flex items-center justify-between text-[11px]">
    <span className="text-[var(--color-text-secondary)]">{label}</span>
    <span className={`truncate ml-2 max-w-[200px] ${mono ? 'font-mono text-[10px]' : ''}`}>{value}</span>
  </div>
)
