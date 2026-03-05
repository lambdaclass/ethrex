import { useState } from 'react'
import { useLang } from '../App'
import { t } from '../i18n'

interface WalletInfo {
  address: string
  l1Balance: string
  l2Balances: { name: string; chainId: number; balance: string }[]
}

export default function WalletView() {
  const { lang } = useLang()
  const [wallets, setWallets] = useState<WalletInfo[]>([
    {
      address: '0x1234...abcd',
      l1Balance: '125.0',
      l2Balances: [
        { name: 'DEX Chain', chainId: 17001, balance: '50.0' },
        { name: 'NFT Chain', chainId: 17002, balance: '20.0' },
        { name: 'Test Chain', chainId: 17003, balance: '5.0' },
      ],
    },
  ])
  const [newAddress, setNewAddress] = useState('')
  const [showAddForm, setShowAddForm] = useState(false)

  const addWallet = () => {
    if (!newAddress.trim()) return
    setWallets(prev => [...prev, {
      address: newAddress,
      l1Balance: '0.0',
      l2Balances: [],
    }])
    setNewAddress('')
    setShowAddForm(false)
  }

  const removeWallet = (address: string) => {
    setWallets(prev => prev.filter(w => w.address !== address))
  }

  const totalBalance = (wallet: WalletInfo) => {
    const l2Total = wallet.l2Balances.reduce((sum, l2) => sum + parseFloat(l2.balance), 0)
    return (parseFloat(wallet.l1Balance) + l2Total).toFixed(1)
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-chat)]">
      <div className="px-6 py-4 border-b border-[var(--color-border)] flex items-center justify-between">
        <div>
          <h1 className="text-lg font-semibold">{t('wallet.title', lang)}</h1>
          <p className="text-xs text-[var(--color-text-secondary)] mt-0.5">{t('wallet.subtitle', lang)}</p>
        </div>
        <button
          onClick={() => setShowAddForm(!showAddForm)}
          className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] text-sm px-4 py-2 rounded-lg transition-colors cursor-pointer"
        >
          + {t('wallet.addWallet', lang)}
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-6 space-y-6">
        {/* Add Wallet Form */}
        {showAddForm && (
          <div className="bg-[var(--color-bubble-ai)] rounded-xl p-4 space-y-3">
            <h3 className="text-sm font-medium">{t('wallet.addWallet', lang)}</h3>
            <div className="flex gap-2">
              <input
                type="text"
                value={newAddress}
                onChange={e => setNewAddress(e.target.value)}
                placeholder={t('wallet.addressPlaceholder', lang)}
                className="flex-1 bg-[var(--color-border)] rounded-lg px-3 py-2 text-sm outline-none font-mono placeholder-[var(--color-text-secondary)]"
              />
              <button
                onClick={addWallet}
                disabled={!newAddress.trim()}
                className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] disabled:opacity-40 px-4 py-2 rounded-lg text-sm transition-colors cursor-pointer"
              >
                {t('wallet.add', lang)}
              </button>
            </div>
          </div>
        )}

        {wallets.length === 0 ? (
          <div className="flex items-center justify-center h-64 text-[var(--color-text-secondary)] text-sm">
            {t('wallet.noWallet', lang)}
          </div>
        ) : (
          wallets.map(wallet => (
            <div key={wallet.address} className="space-y-3">
              {/* Wallet Header */}
              <div className="bg-[var(--color-bubble-ai)] rounded-xl p-5">
                <div className="flex items-center justify-between mb-3">
                  <div className="flex items-center gap-2">
                    <span className="text-lg">👛</span>
                    <span className="font-mono text-sm">{wallet.address}</span>
                  </div>
                  <button
                    onClick={() => removeWallet(wallet.address)}
                    className="text-xs text-[var(--color-text-secondary)] hover:text-[var(--color-error)] cursor-pointer"
                  >
                    ✕
                  </button>
                </div>
                <div className="flex items-baseline gap-2">
                  <span className="text-2xl font-bold">{totalBalance(wallet)} TON</span>
                  <span className="text-xs text-[var(--color-text-secondary)]">{t('wallet.totalBalance', lang)}</span>
                </div>
              </div>

              {/* L1 Balance */}
              <div className="bg-[var(--color-bubble-ai)] rounded-xl p-4 flex items-center justify-between">
                <div className="flex items-center gap-3">
                  <div className="w-2 h-8 bg-blue-500 rounded-full" />
                  <div>
                    <div className="text-sm font-medium">L1 (Ethereum)</div>
                    <div className="text-xs text-[var(--color-text-secondary)]">{t('wallet.l1Balance', lang)}</div>
                  </div>
                </div>
                <span className="font-bold">{wallet.l1Balance} TON</span>
              </div>

              {/* L2 Balances */}
              {wallet.l2Balances.map(l2 => (
                <div key={l2.chainId} className="bg-[var(--color-bubble-ai)] rounded-xl p-4 flex items-center justify-between ml-4">
                  <div className="flex items-center gap-3">
                    <div className="w-2 h-8 bg-[var(--color-accent)] rounded-full" />
                    <div>
                      <div className="text-sm font-medium">{l2.name}</div>
                      <div className="text-xs text-[var(--color-text-secondary)]">Chain #{l2.chainId}</div>
                    </div>
                  </div>
                  <span className="font-bold">{l2.balance} TON</span>
                </div>
              ))}
            </div>
          ))
        )}

        {/* AI Wallet */}
        <div className="bg-[var(--color-bubble-ai)] rounded-xl p-5 border border-dashed border-[var(--color-border)]">
          <div className="flex items-center justify-between">
            <div>
              <div className="flex items-center gap-2">
                <span className="text-lg">🤖</span>
                <span className="font-medium text-sm">{t('wallet.aiWallet', lang)}</span>
              </div>
              <div className="text-xs text-[var(--color-text-secondary)] mt-1 font-mono">0xAI...not created</div>
            </div>
            <button className="bg-[var(--color-accent)] text-sm px-4 py-2 rounded-lg hover:bg-[var(--color-accent-hover)] transition-colors cursor-pointer">
              {t('wallet.fundAi', lang)}
            </button>
          </div>
        </div>

        {/* Actions */}
        <div className="grid grid-cols-2 gap-4">
          <button className="bg-[var(--color-bubble-ai)] rounded-xl p-5 text-center hover:opacity-80 transition-opacity cursor-pointer">
            <div className="text-2xl mb-2">⬇️</div>
            <div className="font-medium text-sm">{t('wallet.deposit', lang)}</div>
            <div className="text-xs text-[var(--color-text-secondary)] mt-1">{t('wallet.depositDesc', lang)}</div>
          </button>
          <button className="bg-[var(--color-bubble-ai)] rounded-xl p-5 text-center hover:opacity-80 transition-opacity cursor-pointer">
            <div className="text-2xl mb-2">⬆️</div>
            <div className="font-medium text-sm">{t('wallet.withdraw', lang)}</div>
            <div className="text-xs text-[var(--color-text-secondary)] mt-1">{t('wallet.withdrawDesc', lang)}</div>
          </button>
        </div>
      </div>
    </div>
  )
}
