import { useLang } from '../App'
import { t } from '../i18n'

export default function WalletView() {
  const { lang } = useLang()

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-chat)]">
      <div className="px-6 py-4 border-b border-[var(--color-border)]">
        <h1 className="text-lg font-semibold">{t('wallet.title', lang)}</h1>
        <p className="text-xs text-[var(--color-text-secondary)] mt-1">{t('wallet.subtitle', lang)}</p>
      </div>

      <div className="flex-1 overflow-y-auto p-6 space-y-6">
        <div className="grid grid-cols-2 gap-4">
          <div className="bg-[var(--color-bubble-ai)] rounded-xl p-5">
            <div className="text-xs text-[var(--color-text-secondary)]">{t('wallet.l1Balance', lang)}</div>
            <div className="text-2xl font-bold mt-2">-- TON</div>
          </div>
          <div className="bg-[var(--color-bubble-ai)] rounded-xl p-5">
            <div className="text-xs text-[var(--color-text-secondary)]">{t('wallet.l2Balance', lang)}</div>
            <div className="text-2xl font-bold mt-2">-- TON</div>
          </div>
        </div>

        <div className="bg-[var(--color-bubble-ai)] rounded-xl p-5">
          <div className="flex items-center justify-between">
            <div>
              <div className="text-xs text-[var(--color-text-secondary)]">{t('wallet.aiWallet', lang)}</div>
              <div className="text-xl font-bold mt-2">-- TON</div>
            </div>
            <button className="bg-[var(--color-accent)] text-sm px-4 py-2 rounded-lg opacity-50 cursor-not-allowed">
              {t('wallet.fundAi', lang)}
            </button>
          </div>
        </div>

        <div className="grid grid-cols-2 gap-4">
          <button className="bg-[var(--color-bubble-ai)] rounded-xl p-5 text-center opacity-50 cursor-not-allowed hover:opacity-60 transition-opacity">
            <div className="text-2xl mb-2">⬇️</div>
            <div className="font-medium">{t('wallet.deposit', lang)}</div>
            <div className="text-xs text-[var(--color-text-secondary)] mt-1">{t('wallet.depositDesc', lang)}</div>
          </button>
          <button className="bg-[var(--color-bubble-ai)] rounded-xl p-5 text-center opacity-50 cursor-not-allowed hover:opacity-60 transition-opacity">
            <div className="text-2xl mb-2">⬆️</div>
            <div className="font-medium">{t('wallet.withdraw', lang)}</div>
            <div className="text-xs text-[var(--color-text-secondary)] mt-1">{t('wallet.withdrawDesc', lang)}</div>
          </button>
        </div>

        <p className="text-xs text-[var(--color-text-secondary)] text-center">
          {t('wallet.hint', lang)}
        </p>
      </div>
    </div>
  )
}
