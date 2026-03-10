import { useState, useEffect, useCallback } from 'react';
import { registerPasskey, clearCredential, type StoredCredential, signChallenge, getStoredCredential, setStoredCredential } from '../lib/passkey';
import * as api from '../lib/api';
import { getBalance } from '../lib/chain';
import RegistrationProgress from './RegistrationProgress';

interface Props {
  credential: StoredCredential | null;
  onCredentialChange: (c: StoredCredential | null) => void;
}

export default function AccountPanel({ credential, onCredentialChange }: Props) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [balance, setBalance] = useState<string | null>(null);
  const [tokenBalance, setTokenBalance] = useState<string | null>(null);
  // Holds the passkey credential during SSE registration (before address is known)
  const [pendingCredential, setPendingCredential] = useState<StoredCredential | null>(null);

  const fetchBalance = useCallback(async () => {
    if (!credential?.address) return;
    try {
      const bal = await getBalance(credential.address);
      setBalance(bal);
    } catch {
      setBalance(null);
    }
    try {
      const { formatted } = await api.getTokenBalance(credential.address);
      setTokenBalance(formatted);
    } catch {
      setTokenBalance(null);
    }
  }, [credential?.address]);

  useEffect(() => {
    fetchBalance();
    const interval = setInterval(fetchBalance, 5000);
    return () => clearInterval(interval);
  }, [fetchBalance]);

  const handleCreate = async () => {
    setLoading(true);
    setError('');
    try {
      const name = `user-${Date.now().toString(36)}`;
      const cred = await registerPasskey(name);
      // Passkey created — show progress panel for on-chain steps
      setPendingCredential(cred);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to create passkey');
      setLoading(false);
    }
  };

  const handleRegistrationComplete = (address: string) => {
    if (!pendingCredential) return;
    pendingCredential.address = address;
    setStoredCredential(pendingCredential);
    onCredentialChange(pendingCredential);
    setPendingCredential(null);
    setLoading(false);
  };

  const handleRegistrationError = (message: string) => {
    setError(message);
    setPendingCredential(null);
    setLoading(false);
  };

  const handleSignIn = async () => {
    setLoading(true);
    setError('');
    try {
      const stored = getStoredCredential();
      if (!stored) {
        setError('No saved credential found. Create an account first.');
        return;
      }
      // Trigger passkey selection to verify the user owns this credential
      await signChallenge(stored.id, '0x0000000000000000000000000000000000000000000000000000000000000000');
      onCredentialChange(stored);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Sign-in failed');
    } finally {
      setLoading(false);
    }
  };

  const handleDisconnect = () => {
    onCredentialChange(null);
    setBalance(null);
    setTokenBalance(null);
  };

  const handleDeleteAccount = () => {
    clearCredential();
    onCredentialChange(null);
    setBalance(null);
    setTokenBalance(null);
  };

  if (credential) {
    return (
      <div className="flex items-center gap-4">
        <div className="flex items-center gap-2">
          <span className="inline-block h-2 w-2 rounded-full bg-emerald-400" />
          <code className="text-sm text-zinc-300 font-mono">
            {credential.address.slice(0, 6)}...{credential.address.slice(-4)}
          </code>
        </div>
        {balance !== null && (
          <span className="text-sm text-zinc-500">
            {parseFloat(balance).toFixed(4)} ETH
          </span>
        )}
        {tokenBalance !== null && (
          <span className="text-sm text-zinc-500">
            {tokenBalance} DEMO
          </span>
        )}
        <button
          onClick={handleDisconnect}
          className="text-xs text-zinc-500 hover:text-zinc-300 transition-colors cursor-pointer"
        >
          Lock
        </button>
        <button
          onClick={handleDeleteAccount}
          className="text-xs text-red-500/60 hover:text-red-400 transition-colors cursor-pointer"
        >
          Delete
        </button>
      </div>
    );
  }

  // Show progress panel during registration
  if (pendingCredential) {
    return (
      <div className="flex flex-col items-center gap-6">
        <RegistrationProgress
          credential={pendingCredential}
          onComplete={handleRegistrationComplete}
          onError={handleRegistrationError}
        />
        {error && (
          <p className="text-sm text-red-400 mt-2">{error}</p>
        )}
      </div>
    );
  }

  return (
    <div className="flex flex-col items-center gap-6">
      <div className="text-center">
        <h2 className="text-xl font-semibold text-zinc-100 mb-2">
          Connect with Passkey
        </h2>
        <p className="text-sm text-zinc-500 max-w-md">
          Create a new passkey-authenticated account or sign in with an existing one.
          Your private key never leaves your device.
        </p>
      </div>

      <div className="flex gap-3">
        <button
          onClick={handleCreate}
          disabled={loading}
          className="px-5 py-2.5 rounded-lg bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 disabled:cursor-not-allowed text-sm font-medium text-white transition-colors cursor-pointer"
        >
          {loading ? 'Creating...' : 'Create Account'}
        </button>
        <button
          onClick={handleSignIn}
          disabled={loading}
          className="px-5 py-2.5 rounded-lg border border-zinc-700 hover:border-zinc-500 disabled:opacity-50 disabled:cursor-not-allowed text-sm font-medium text-zinc-300 transition-colors cursor-pointer"
        >
          {loading ? 'Signing in...' : 'Sign In'}
        </button>
      </div>

      {error && (
        <p className="text-sm text-red-400 mt-2">{error}</p>
      )}
    </div>
  );
}
