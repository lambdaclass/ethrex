import { useLang, useTheme } from '../App'
import { t, langNames } from '../i18n'
import type { Lang } from '../i18n'
import type { Theme } from '../App'

export default function SettingsView() {
  const { lang, setLang } = useLang()
  const { theme, setTheme } = useTheme()

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-sidebar)]">
      <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-main)]">
        <h1 className="text-base font-semibold">{t('settings.title', lang)}</h1>
      </div>

      <div className="flex-1 overflow-y-auto p-4 space-y-3">
        {/* Theme */}
        <section className="bg-[var(--color-bg-main)] rounded-xl p-4 space-y-3 border border-[var(--color-border)]">
          <h2 className="text-[13px] font-medium">{t('settings.theme', lang)}</h2>
          <div className="flex gap-2">
            {([['light', t('settings.themeLight', lang)], ['dark', t('settings.themeDark', lang)]] as [Theme, string][]).map(([code, name]) => (
              <button
                key={code}
                onClick={() => setTheme(code)}
                className={`flex-1 py-2 rounded-lg text-[13px] transition-colors cursor-pointer border ${
                  theme === code
                    ? 'bg-[var(--color-accent)] text-[var(--color-accent-text)] border-[var(--color-accent)]'
                    : 'bg-[var(--color-bg-sidebar)] border-[var(--color-border)] hover:bg-[var(--color-border)]'
                }`}
              >
                {code === 'light' ? '☀️' : '🌙'} {name}
              </button>
            ))}
          </div>
        </section>

        {/* Language */}
        <section className="bg-[var(--color-bg-main)] rounded-xl p-4 space-y-3 border border-[var(--color-border)]">
          <h2 className="text-[13px] font-medium">{t('settings.language', lang)}</h2>
          <div className="flex gap-2">
            {(Object.entries(langNames) as [Lang, string][]).map(([code, name]) => (
              <button
                key={code}
                onClick={() => setLang(code)}
                className={`flex-1 py-2 rounded-lg text-[13px] transition-colors cursor-pointer border ${
                  lang === code
                    ? 'bg-[var(--color-accent)] text-[var(--color-accent-text)] border-[var(--color-accent)]'
                    : 'bg-[var(--color-bg-sidebar)] border-[var(--color-border)] hover:bg-[var(--color-border)]'
                }`}
              >
                {name}
              </button>
            ))}
          </div>
        </section>

        {/* AI Provider */}
        <section className="bg-[var(--color-bg-main)] rounded-xl p-4 space-y-3 border border-[var(--color-border)]">
          <h2 className="text-[13px] font-medium">{t('settings.aiProvider', lang)}</h2>
          <div>
            <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">{t('settings.provider', lang)}</label>
            <select className="w-full bg-[var(--color-bg-sidebar)] rounded-lg px-3 py-2 text-[13px] outline-none border border-[var(--color-border)]">
              <option>Claude (Anthropic)</option>
              <option>GPT (OpenAI)</option>
              <option>Gemini (Google)</option>
            </select>
          </div>
          <div>
            <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">{t('settings.apiKey', lang)}</label>
            <input
              type="password"
              placeholder={t('settings.apiKeyPlaceholder', lang)}
              className="w-full bg-[var(--color-bg-sidebar)] rounded-lg px-3 py-2 text-[13px] outline-none border border-[var(--color-border)] placeholder-[var(--color-text-secondary)]"
            />
          </div>
          <div>
            <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">{t('settings.model', lang)}</label>
            <select className="w-full bg-[var(--color-bg-sidebar)] rounded-lg px-3 py-2 text-[13px] outline-none border border-[var(--color-border)]">
              <option>claude-opus-4-6</option>
              <option>claude-sonnet-4-6</option>
            </select>
          </div>
        </section>

        {/* Node Config */}
        <section className="bg-[var(--color-bg-main)] rounded-xl p-4 space-y-3 border border-[var(--color-border)]">
          <h2 className="text-[13px] font-medium">{t('settings.nodeConfig', lang)}</h2>
          <div>
            <label className="text-[11px] text-[var(--color-text-secondary)] block mb-1">{t('settings.binaryPath', lang)}</label>
            <input type="text" placeholder="/usr/local/bin/ethrex"
              className="w-full bg-[var(--color-bg-sidebar)] rounded-lg px-3 py-2 text-[13px] outline-none border border-[var(--color-border)] placeholder-[var(--color-text-secondary)]" />
          </div>
        </section>
      </div>
    </div>
  )
}
