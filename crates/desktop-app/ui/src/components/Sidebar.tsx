import type { ViewType } from '../App'
import { useLang } from '../App'
import { t } from '../i18n'

interface SidebarProps {
  activeView: ViewType
  onNavigate: (view: ViewType) => void
}

const navItems: { view: ViewType; icon: string; labelKey: string }[] = [
  { view: 'chat', icon: '💬', labelKey: 'nav.chat' },
  { view: 'nodes', icon: '⚡', labelKey: 'nav.nodes' },
  { view: 'dashboard', icon: '📊', labelKey: 'nav.dashboard' },
  { view: 'openl2', icon: '🌐', labelKey: 'nav.openl2' },
  { view: 'wallet', icon: '💰', labelKey: 'nav.wallet' },
  { view: 'settings', icon: '⚙️', labelKey: 'nav.settings' },
]

export default function Sidebar({ activeView, onNavigate }: SidebarProps) {
  const { lang } = useLang()

  return (
    <div className="w-[72px] bg-[var(--color-bg-sidebar)] flex flex-col items-center py-4 border-r border-[var(--color-border)]">
      <div className="mb-8 text-2xl font-bold text-[var(--color-accent)]">T</div>

      <nav className="flex flex-col gap-2 flex-1">
        {navItems.map(({ view, icon, labelKey }) => (
          <button
            key={view}
            onClick={() => onNavigate(view)}
            className={`w-12 h-12 rounded-xl flex items-center justify-center text-xl transition-all cursor-pointer
              ${activeView === view
                ? 'bg-[var(--color-accent)] shadow-lg'
                : 'hover:bg-[var(--color-border)]'
              }`}
            title={t(labelKey, lang)}
          >
            {icon}
          </button>
        ))}
      </nav>

      <div className="mt-auto">
        <div className="w-10 h-10 rounded-full bg-[var(--color-border)] flex items-center justify-center text-sm">
          👤
        </div>
      </div>
    </div>
  )
}
