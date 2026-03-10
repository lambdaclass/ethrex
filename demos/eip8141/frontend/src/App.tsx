import { useState, useEffect } from 'react';
import { getStoredCredential, type StoredCredential } from './lib/passkey';
import AccountPanel from './components/AccountPanel';
import TabLayout from './components/TabLayout';
import Footer from './components/Footer';

export default function App() {
  const [credential, setCredential] = useState<StoredCredential | null>(null);

  useEffect(() => {
    const stored = getStoredCredential();
    if (stored?.address) {
      setCredential(stored);
    }
  }, []);

  return (
    <div className="min-h-screen bg-gradient-to-b from-zinc-950 to-zinc-900 text-zinc-100">
      <header className="border-b border-zinc-800/50">
        <div className="max-w-6xl mx-auto px-6 py-4 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <h1 className="text-lg font-semibold tracking-tight">
              ethrex
              <span className="text-zinc-600 font-normal mx-2">·</span>
              <span className="text-zinc-500 font-normal text-sm">
                EIP-8141 Frame Transactions
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

      <main className="max-w-6xl mx-auto px-6 py-10">
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

      <Footer />
    </div>
  );
}
