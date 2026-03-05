export default function WalletView() {
  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-chat)]">
      <div className="px-6 py-4 border-b border-[var(--color-border)]">
        <h1 className="text-lg font-semibold">TON Wallet</h1>
        <p className="text-xs text-[var(--color-text-secondary)] mt-1">Manage your TON balance across L1 and L2</p>
      </div>

      <div className="flex-1 overflow-y-auto p-6 space-y-6">
        {/* Balance cards */}
        <div className="grid grid-cols-2 gap-4">
          <div className="bg-[var(--color-bubble-ai)] rounded-xl p-5">
            <div className="text-xs text-[var(--color-text-secondary)]">L1 Balance</div>
            <div className="text-2xl font-bold mt-2">-- TON</div>
          </div>
          <div className="bg-[var(--color-bubble-ai)] rounded-xl p-5">
            <div className="text-xs text-[var(--color-text-secondary)]">L2 Balance</div>
            <div className="text-2xl font-bold mt-2">-- TON</div>
          </div>
        </div>

        {/* AI Wallet */}
        <div className="bg-[var(--color-bubble-ai)] rounded-xl p-5">
          <div className="flex items-center justify-between">
            <div>
              <div className="text-xs text-[var(--color-text-secondary)]">AI Agent Wallet (L2)</div>
              <div className="text-xl font-bold mt-2">-- TON</div>
            </div>
            <button className="bg-[var(--color-accent)] text-sm px-4 py-2 rounded-lg opacity-50 cursor-not-allowed">
              Fund AI Wallet
            </button>
          </div>
        </div>

        {/* Actions */}
        <div className="grid grid-cols-2 gap-4">
          <button className="bg-[var(--color-bubble-ai)] rounded-xl p-5 text-center opacity-50 cursor-not-allowed hover:opacity-60 transition-opacity">
            <div className="text-2xl mb-2">⬇️</div>
            <div className="font-medium">Deposit to L2</div>
            <div className="text-xs text-[var(--color-text-secondary)] mt-1">L1 → L2 Bridge</div>
          </button>
          <button className="bg-[var(--color-bubble-ai)] rounded-xl p-5 text-center opacity-50 cursor-not-allowed hover:opacity-60 transition-opacity">
            <div className="text-2xl mb-2">⬆️</div>
            <div className="font-medium">Withdraw to L1</div>
            <div className="text-xs text-[var(--color-text-secondary)] mt-1">L2 → L1 Bridge</div>
          </button>
        </div>

        <p className="text-xs text-[var(--color-text-secondary)] text-center">
          Wallet connection and TON management coming in Phase 5.
        </p>
      </div>
    </div>
  )
}
