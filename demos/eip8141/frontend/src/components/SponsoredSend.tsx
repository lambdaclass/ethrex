import { useState, useEffect } from 'react';
import type { StoredCredential } from '../lib/passkey';
import { signChallenge } from '../lib/passkey';
import * as api from '../lib/api';
import type { TxResult as TxResultType, AuthMethod } from '../lib/api';
import TxResult from './TxResult';
import KeyRotationInfo from './KeyRotationInfo';
import AuthMethodToggle from './AuthMethodToggle';
import FramePipeline from './FramePipeline';
import type { FrameConfig, ExecutionState } from './FramePipeline';

const PASSKEY_FRAMES: FrameConfig[] = [
  {
    mode: 'VERIFY',
    label: 'scope = 1',
    target: 'account',
    tooltip:
      'Calls verify() on the account. Uses TXPARAMLOAD(0x08) for sig_hash, verifies signature, calls APPROVE(scope=1) — authorizes as sender only.',
  },
  {
    mode: 'VERIFY',
    label: 'scope = 2',
    target: 'sponsor',
    tooltip:
      'Calls payForTransaction() on the sponsor. Calls APPROVE(scope=2) — authorizes gas payment from sponsor\'s balance.',
  },
  {
    mode: 'SENDER',
    label: 'execute()',
    target: 'account',
    tooltip:
      'Calls execute(target, value, data) on the account. Routes the ERC20 transfer through the account\'s execution logic.',
  },
];

const EPHEMERAL_FRAMES: FrameConfig[] = [
  {
    mode: 'VERIFY',
    label: 'scope = 1',
    target: 'account',
    tooltip:
      'Calls verifyEcdsa() on the account. Verifies the ECDSA signature against the current ephemeral signer, calls APPROVE(scope=1) — authorizes as sender only.',
  },
  {
    mode: 'VERIFY',
    label: 'scope = 2',
    target: 'sponsor',
    tooltip:
      'Calls payForTransaction() on the sponsor. Calls APPROVE(scope=2) — authorizes gas payment from sponsor\'s balance.',
  },
  {
    mode: 'SENDER',
    label: 'rotate()',
    target: 'account',
    tooltip:
      'Rotates the ephemeral signer to the next derived key.',
  },
  {
    mode: 'SENDER',
    label: 'execute()',
    target: 'account',
    tooltip:
      'Calls execute(target, value, data) on the account. Routes the ERC20 transfer through the account\'s execution logic.',
  },
];

interface Props {
  credential: StoredCredential;
}

export default function SponsoredSend({ credential }: Props) {
  const [to, setTo] = useState('');
  const [amount, setAmount] = useState('');
  const [authMethod, setAuthMethod] = useState<AuthMethod>('passkey');
  const [loading, setLoading] = useState(false);
  const [status, setStatus] = useState('');
  const [error, setError] = useState('');
  const [history, setHistory] = useState<TxResultType[]>([]);
  const [tokenBalance, setTokenBalance] = useState<string | null>(null);

  const frames = authMethod === 'ephemeral' ? EPHEMERAL_FRAMES : PASSKEY_FRAMES;

  const fetchBalance = async () => {
    try {
      const { formatted } = await api.getTokenBalance(credential.address);
      setTokenBalance(formatted);
    } catch {
      setTokenBalance(null);
    }
  };

  useEffect(() => {
    fetchBalance();
  }, [credential.address]);

  const handleSend = async () => {
    if (!to || !amount) {
      setError('Recipient and amount are required');
      return;
    }
    setLoading(true);
    setError('');
    try {
      setStatus('Building transaction...');
      const { sigHash } = await api.getSigHash('sponsored-send', {
        from: credential.address,
        to,
        amount,
      }, authMethod);

      if (authMethod === 'ephemeral') {
        setStatus('Signing with ephemeral key...');
        const txResult = await api.sponsoredSend({
          address: credential.address,
          to,
          amount,
          authMethod: 'ephemeral',
        });
        setHistory(prev => [txResult, ...prev]);
        fetchBalance();
      } else {
        setStatus('Sign with your passkey...');
        const signed = await signChallenge(credential.id, sigHash);

        setStatus('Submitting transaction...');
        const txResult = await api.sponsoredSend({
          address: credential.address,
          to,
          amount,
          authMethod: 'passkey',
          signature: signed.signature,
          webauthn: signed.webauthn,
        });
        setHistory(prev => [txResult, ...prev]);
        fetchBalance();
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Transaction failed');
    } finally {
      setLoading(false);
      setStatus('');
    }
  };

  const executionState: ExecutionState = (() => {
    if (error && !loading) return { phase: 'error' as const, errorFrameIndex: frames.length - 1 };
    if (!loading && history.length > 0 && history[0].success) return { phase: 'done' as const };
    if (status === 'Building transaction...') return { phase: 'executing' as const, activeFrameIndex: 0 };
    if (status === 'Sign with your passkey...') return { phase: 'executing' as const, activeFrameIndex: 1 };
    if (status === 'Signing with ephemeral key...') return { phase: 'executing' as const, activeFrameIndex: 1 };
    if (status === 'Submitting transaction...') return { phase: 'executing' as const, activeFrameIndex: frames.length - 1 };
    return { phase: 'idle' as const };
  })();

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-8">
      <div>
        <div className="flex items-center justify-between mb-1">
          <h3 className="text-lg font-semibold text-zinc-100">Sponsored ERC20 Send</h3>
          <AuthMethodToggle value={authMethod} onChange={setAuthMethod} />
        </div>
        <p className="text-sm text-zinc-500 mb-5">
          Send ERC20 tokens without paying gas. A sponsor account covers the transaction fee.
          {authMethod === 'ephemeral' && ' No biometric prompt — signed with an ephemeral key.'}
        </p>

        <div className="mb-4 rounded-lg border border-emerald-500/20 bg-emerald-950/20 px-3 py-2">
          <span className="text-xs text-emerald-400">
            Gas is paid by the backend sponsor account
          </span>
        </div>

        {tokenBalance !== null && (
          <div className="mb-4 text-xs text-zinc-400">
            Token balance: <span className="font-mono text-zinc-200">{tokenBalance}</span> DEMO
          </div>
        )}

        <div className="space-y-4">
          <div>
            <label className="block text-xs text-zinc-400 mb-1.5">Recipient Address</label>
            <div className="flex gap-2">
              <input
                type="text"
                placeholder="0x..."
                value={to}
                onChange={e => setTo(e.target.value)}
                className="flex-1 rounded-lg border border-zinc-700 bg-zinc-800/50 px-3 py-2 text-sm text-zinc-200 font-mono placeholder-zinc-600 focus:border-emerald-500 focus:outline-none"
              />
              <button
                type="button"
                onClick={() => setTo('0x' + Array.from(crypto.getRandomValues(new Uint8Array(20)), b => b.toString(16).padStart(2, '0')).join(''))}
                className="rounded-lg border border-zinc-700 bg-zinc-800/50 px-2.5 text-xs text-zinc-400 hover:text-zinc-200 hover:border-zinc-500 transition-colors cursor-pointer"
                title="Random address"
              >
                Random
              </button>
            </div>
          </div>

          <div>
            <label className="block text-xs text-zinc-400 mb-1.5">Amount (DEMO tokens)</label>
            <input
              type="text"
              placeholder="100"
              value={amount}
              onChange={e => setAmount(e.target.value)}
              className="w-full rounded-lg border border-zinc-700 bg-zinc-800/50 px-3 py-2 text-sm text-zinc-200 font-mono placeholder-zinc-600 focus:border-emerald-500 focus:outline-none"
            />
          </div>

          <button
            onClick={handleSend}
            disabled={loading}
            className="w-full rounded-lg bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 disabled:cursor-not-allowed px-4 py-2.5 text-sm font-medium text-white transition-colors cursor-pointer"
          >
            {loading ? 'Processing...' : 'Send (Sponsored)'}
          </button>
        </div>

        {status && (
          <div className="mt-3 flex items-center gap-2 rounded-lg border border-emerald-500/30 bg-emerald-950/30 px-3 py-2">
            <span className="inline-block h-2 w-2 rounded-full bg-emerald-400 animate-pulse" />
            <span className="text-sm text-emerald-300">{status}</span>
          </div>
        )}
        {error && <p className="text-sm text-red-400 mt-3">{error}</p>}
        {history.length > 0 && (
          <div className="mt-6 space-y-2">
            <h4 className="text-xs font-medium text-zinc-500 uppercase tracking-wider">Transaction History</h4>
            {history.map((r, i) => (
              <div key={i}>
                <KeyRotationInfo result={r} />
                <TxResult result={r} />
              </div>
            ))}
          </div>
        )}
      </div>

      <div className="lg:pt-10">
        <FramePipeline frames={frames} executionState={executionState} />
      </div>
    </div>
  );
}
