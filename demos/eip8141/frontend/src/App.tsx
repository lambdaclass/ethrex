import { useState, useEffect } from 'react';
import { getStoredCredential, type StoredCredential } from './lib/passkey';
import AccountPanel from './components/AccountPanel';
import TabLayout from './components/TabLayout';

export default function App() {
  const [credential, setCredential] = useState<StoredCredential | null>(null);

  useEffect(() => {
    const stored = getStoredCredential();
    if (stored?.address) {
      setCredential(stored);
    }
  }, []);

  return (
    <div className="min-h-screen bg-zinc-950 text-zinc-100">
      <header className="border-b border-zinc-800/50">
        <div className="max-w-3xl mx-auto px-6 py-4 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <h1 className="text-lg font-semibold tracking-tight">
              EIP-8141
              <span className="text-zinc-500 font-normal ml-2 text-sm">
                Frame Transactions
              </span>
            </h1>
          </div>
          {credential && (
            <AccountPanel
              credential={credential}
              onCredentialChange={setCredential}
            />
          )}
        </div>
      </header>

      <main className="max-w-3xl mx-auto px-6 py-10">
        {!credential ? (
          <div className="flex items-center justify-center min-h-[60vh]">
            <AccountPanel
              credential={credential}
              onCredentialChange={setCredential}
            />
          </div>
        ) : (
          <TabLayout credential={credential} />
        )}
      </main>
    </div>
  );
}
