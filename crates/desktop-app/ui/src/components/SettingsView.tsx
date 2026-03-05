export default function SettingsView() {
  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-chat)]">
      <div className="px-6 py-4 border-b border-[var(--color-border)]">
        <h1 className="text-lg font-semibold">Settings</h1>
      </div>

      <div className="flex-1 overflow-y-auto p-6 space-y-6">
        {/* AI Provider */}
        <section className="bg-[var(--color-bubble-ai)] rounded-xl p-5 space-y-4">
          <h2 className="font-medium">AI Provider</h2>
          <div className="space-y-3">
            <div>
              <label className="text-xs text-[var(--color-text-secondary)] block mb-1">Provider</label>
              <select className="w-full bg-[var(--color-border)] rounded-lg px-3 py-2 text-sm outline-none">
                <option>Claude (Anthropic)</option>
                <option>GPT (OpenAI)</option>
                <option>Gemini (Google)</option>
              </select>
            </div>
            <div>
              <label className="text-xs text-[var(--color-text-secondary)] block mb-1">API Key</label>
              <input
                type="password"
                placeholder="Enter your API key..."
                className="w-full bg-[var(--color-border)] rounded-lg px-3 py-2 text-sm outline-none placeholder-[var(--color-text-secondary)]"
              />
            </div>
            <div>
              <label className="text-xs text-[var(--color-text-secondary)] block mb-1">Model</label>
              <select className="w-full bg-[var(--color-border)] rounded-lg px-3 py-2 text-sm outline-none">
                <option>claude-opus-4-6</option>
                <option>claude-sonnet-4-6</option>
              </select>
            </div>
          </div>
        </section>

        {/* Node Configuration */}
        <section className="bg-[var(--color-bubble-ai)] rounded-xl p-5 space-y-4">
          <h2 className="font-medium">Node Configuration</h2>
          <div className="space-y-3">
            <div>
              <label className="text-xs text-[var(--color-text-secondary)] block mb-1">ethrex Binary Path</label>
              <input
                type="text"
                placeholder="/usr/local/bin/ethrex"
                className="w-full bg-[var(--color-border)] rounded-lg px-3 py-2 text-sm outline-none placeholder-[var(--color-text-secondary)]"
              />
            </div>
            <div>
              <label className="text-xs text-[var(--color-text-secondary)] block mb-1">L2 RPC Port</label>
              <input
                type="number"
                placeholder="8545"
                className="w-full bg-[var(--color-border)] rounded-lg px-3 py-2 text-sm outline-none placeholder-[var(--color-text-secondary)]"
              />
            </div>
          </div>
        </section>

        {/* Dashboard URLs */}
        <section className="bg-[var(--color-bubble-ai)] rounded-xl p-5 space-y-4">
          <h2 className="font-medium">Dashboard URLs</h2>
          <div className="space-y-3">
            {[
              { label: 'L1 Dashboard', defaultUrl: 'http://localhost:3010' },
              { label: 'L2 Dashboard', defaultUrl: 'http://localhost:3011' },
              { label: 'Explorer', defaultUrl: 'http://localhost:3012' },
              { label: 'Prover', defaultUrl: 'http://localhost:3013' },
              { label: 'Metrics (Grafana)', defaultUrl: 'http://localhost:3000' },
            ].map(({ label, defaultUrl }) => (
              <div key={label}>
                <label className="text-xs text-[var(--color-text-secondary)] block mb-1">{label}</label>
                <input
                  type="text"
                  placeholder={defaultUrl}
                  className="w-full bg-[var(--color-border)] rounded-lg px-3 py-2 text-sm outline-none placeholder-[var(--color-text-secondary)]"
                />
              </div>
            ))}
          </div>
        </section>

        <button className="w-full bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] rounded-xl py-3 text-sm font-medium transition-colors cursor-pointer opacity-50">
          Save Settings (Coming Soon)
        </button>
      </div>
    </div>
  )
}
