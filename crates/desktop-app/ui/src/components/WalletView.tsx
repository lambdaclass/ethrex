import { useState } from 'react'
import { useLang } from '../App'
import { t } from '../i18n'

interface TokenBalance {
  symbol: string
  name: string
  balance: string
  icon: string
  iconBg: string
}

interface AppchainBalance {
  name: string
  chainId: number
  icon: string
  tokens: { symbol: string; balance: string }[]
}

// Bridge-allowed tokens (would come from bridge contract in production)
const bridgeTokens: TokenBalance[] = [
  { symbol: 'TON', name: 'Tokamak Network', balance: '1,250.0', icon: 'T', iconBg: 'bg-blue-100' },
  { symbol: 'WTON', name: 'Wrapped TON', balance: '500.0', icon: 'W', iconBg: 'bg-indigo-100' },
  { symbol: 'TOS', name: 'TONStarter', balance: '3,000.0', icon: 'S', iconBg: 'bg-purple-100' },
  { symbol: 'DOC', name: 'Door Open Close', balance: '10,000', icon: 'D', iconBg: 'bg-green-100' },
]

export default function WalletView() {
  const { lang } = useLang()
  const [address, setAddress] = useState('0x1234...abcd')
  const [isEditing, setIsEditing] = useState(false)
  const [editAddress, setEditAddress] = useState('')

  // L1 balances
  const l1Tokens: TokenBalance[] = [
    { symbol: 'ETH', name: 'Ethereum', balance: '2.45', icon: '\u039E', iconBg: 'bg-slate-100' },
    { symbol: 'TON', name: 'Tokamak Network', balance: '1,250.0', icon: 'T', iconBg: 'bg-blue-100' },
    { symbol: 'WTON', name: 'Wrapped TON', balance: '500.0', icon: 'W', iconBg: 'bg-indigo-100' },
  ]

  // Per-appchain balances
  const appchainBalances: AppchainBalance[] = [
    { name: 'DEX Chain', chainId: 17001, icon: '\uD83D\uDD04', tokens: [{ symbol: 'TON', balance: '50.0' }, { symbol: 'WTON', balance: '100.0' }] },
    { name: 'NFT Chain', chainId: 17002, icon: '\uD83C\uDFA8', tokens: [{ symbol: 'TON', balance: '20.0' }] },
    { name: 'Test Chain', chainId: 17003, icon: '\uD83E\uDDEA', tokens: [{ symbol: 'TON', balance: '5.0' }] },
  ]

  const saveAddress = () => {
    if (editAddress.trim()) setAddress(editAddress.trim())
    setIsEditing(false)
    setEditAddress('')
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-sidebar)]">
      {/* Header */}
      <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-main)]">
        <h1 className="text-base font-semibold">{t('wallet.title', lang)}</h1>
      </div>

      <div className="flex-1 overflow-y-auto">
        {/* Address Card */}
        <div className="m-4 rounded-2xl p-4 bg-[var(--color-accent)] text-[var(--color-accent-text)]">
          {isEditing ? (
            <div className="flex gap-2 mb-2">
              <input
                type="text"
                value={editAddress}
                onChange={e => setEditAddress(e.target.value)}
                placeholder={t('wallet.addressPlaceholder', lang)}
                autoFocus
                onKeyDown={e => e.key === 'Enter' && saveAddress()}
                className="flex-1 bg-black/10 rounded-lg px-2.5 py-1.5 text-[11px] font-mono outline-none placeholder-black/40"
              />
              <button onClick={saveAddress} className="bg-black/10 px-3 py-1.5 rounded-lg text-[11px] font-medium cursor-pointer hover:bg-black/20">
                {t('wallet.add', lang)}
              </button>
            </div>
          ) : (
            <button
              onClick={() => { setIsEditing(true); setEditAddress('') }}
              className="text-[11px] font-mono opacity-70 hover:opacity-100 cursor-pointer mb-2 block"
            >
              {address} &#x270F;&#xFE0E;
            </button>
          )}
          <div className="text-xl font-bold">$12,450.00</div>
          <div className="text-[11px] opacity-60 mt-0.5">{t('wallet.totalBalance', lang)}</div>
        </div>

        {/* Quick Actions */}
        <div className="flex gap-2 mx-4 mb-4">
          <button className="flex-1 bg-[var(--color-bg-main)] rounded-xl py-2.5 flex flex-col items-center gap-1 hover:bg-[var(--color-border)] transition-colors cursor-pointer border border-[var(--color-border)]">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-[var(--color-text-secondary)]">
              <line x1="12" y1="5" x2="12" y2="19"/><polyline points="19 12 12 19 5 12"/>
            </svg>
            <span className="text-[11px]">{t('wallet.deposit', lang)}</span>
          </button>
          <button className="flex-1 bg-[var(--color-bg-main)] rounded-xl py-2.5 flex flex-col items-center gap-1 hover:bg-[var(--color-border)] transition-colors cursor-pointer border border-[var(--color-border)]">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-[var(--color-text-secondary)]">
              <line x1="12" y1="19" x2="12" y2="5"/><polyline points="5 12 12 5 19 12"/>
            </svg>
            <span className="text-[11px]">{t('wallet.withdraw', lang)}</span>
          </button>
          <button className="flex-1 bg-[var(--color-bg-main)] rounded-xl py-2.5 flex flex-col items-center gap-1 hover:bg-[var(--color-border)] transition-colors cursor-pointer border border-[var(--color-border)]">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-[var(--color-text-secondary)]">
              <polyline points="7 17 2 12 7 7"/><polyline points="17 7 22 12 17 17"/><line x1="2" y1="12" x2="22" y2="12"/>
            </svg>
            <span className="text-[11px]">{t('wallet.bridge', lang)}</span>
          </button>
          <button className="flex-1 bg-[var(--color-bg-main)] rounded-xl py-2.5 flex flex-col items-center gap-1 hover:bg-[var(--color-border)] transition-colors cursor-pointer border border-[var(--color-border)]">
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="text-[var(--color-text-secondary)]">
              <rect x="3" y="3" width="18" height="18" rx="2"/><line x1="3" y1="9" x2="21" y2="9"/><line x1="9" y1="21" x2="9" y2="9"/>
            </svg>
            <span className="text-[11px]">{t('wallet.txHistory', lang)}</span>
          </button>
        </div>

        {/* L1 Balances */}
        <div className="mx-4 mb-2 text-[11px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider">
          {t('wallet.balanceBreakdown', lang)}
        </div>
        <div className="mx-4 mb-4 bg-[var(--color-bg-main)] rounded-xl overflow-hidden border border-[var(--color-border)]">
          {l1Tokens.map((token, i) => (
            <div key={token.symbol} className={`flex items-center justify-between px-3.5 py-2.5 ${i < l1Tokens.length - 1 ? 'border-b border-[var(--color-border)]' : ''}`}>
              <div className="flex items-center gap-2.5">
                <div className={`w-8 h-8 rounded-lg ${token.iconBg} flex items-center justify-center text-sm font-bold text-gray-700`}>
                  {token.icon}
                </div>
                <div>
                  <div className="text-[13px] font-medium">{token.symbol}</div>
                  <div className="text-[10px] text-[var(--color-text-secondary)]">{token.name}</div>
                </div>
              </div>
              <div className="text-[13px] font-semibold text-right">
                <div>{token.balance}</div>
              </div>
            </div>
          ))}
        </div>

        {/* Bridge Allowed Tokens */}
        <div className="mx-4 mb-2 flex items-center gap-2">
          <span className="text-[11px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider">
            {t('wallet.bridgeTokens', lang)}
          </span>
          <span className="text-[9px] bg-[var(--color-tag-bg)] text-[var(--color-tag-text)] px-1.5 py-0.5 rounded font-medium">
            {bridgeTokens.length}
          </span>
        </div>
        <div className="mx-4 mb-4 bg-[var(--color-bg-main)] rounded-xl overflow-hidden border border-[var(--color-border)]">
          {bridgeTokens.map((token, i) => (
            <div key={token.symbol} className={`flex items-center justify-between px-3.5 py-2.5 ${i < bridgeTokens.length - 1 ? 'border-b border-[var(--color-border)]' : ''}`}>
              <div className="flex items-center gap-2.5">
                <div className={`w-8 h-8 rounded-lg ${token.iconBg} flex items-center justify-center text-sm font-bold text-gray-700`}>
                  {token.icon}
                </div>
                <div>
                  <div className="text-[13px] font-medium">{token.symbol}</div>
                  <div className="text-[10px] text-[var(--color-text-secondary)]">{token.name}</div>
                </div>
              </div>
              <div className="text-[13px] font-semibold">{token.balance}</div>
            </div>
          ))}
        </div>

        {/* Appchain Balances */}
        <div className="mx-4 mb-2 text-[11px] font-medium text-[var(--color-text-secondary)] uppercase tracking-wider">
          {t('wallet.appchainBalances', lang)}
        </div>
        <div className="mx-4 mb-6 space-y-2">
          {appchainBalances.map(chain => (
            <div key={chain.chainId} className="bg-[var(--color-bg-main)] rounded-xl overflow-hidden border border-[var(--color-border)]">
              <div className="flex items-center gap-2.5 px-3.5 py-2.5 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)]">
                <span className="text-base">{chain.icon}</span>
                <div>
                  <div className="text-[12px] font-medium">{chain.name}</div>
                  <div className="text-[10px] text-[var(--color-text-secondary)]">#{chain.chainId}</div>
                </div>
              </div>
              {chain.tokens.map((token, i) => (
                <div key={token.symbol} className={`flex items-center justify-between px-3.5 py-2 ${i < chain.tokens.length - 1 ? 'border-b border-[var(--color-border)]' : ''}`}>
                  <span className="text-[12px] text-[var(--color-text-secondary)]">{token.symbol}</span>
                  <span className="text-[13px] font-semibold">{token.balance}</span>
                </div>
              ))}
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}
