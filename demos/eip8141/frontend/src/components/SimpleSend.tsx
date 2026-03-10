import { useState } from 'react';
import type { StoredCredential } from '../lib/passkey';
import { signChallenge } from '../lib/passkey';
import * as api from '../lib/api';
import type { TxResult as TxResultType } from '../lib/api';
import TxResult from './TxResult';
import FramePipeline from './FramePipeline';
import type { FrameConfig, ExecutionState } from './FramePipeline';

const FRAMES: FrameConfig[] = [
  {
    mode: 'VERIFY',
    label: 'scope = 2',
    target: 'account',
    tooltip:
      'Calls verifyAndPay() on the account contract. Uses TXPARAMLOAD(0x08) to read the sig_hash, verifies the WebAuthn P256 signature, then calls APPROVE(scope=2) to authorize as both sender and gas payer.',
  },
  {
    mode: 'SENDER',
    label: 'transfer()',
    target: 'account',
    tooltip:
      'Calls transfer(to, amount) on the account contract. Executes with msg.sender set to the account\'s own address.',
  },
];

interface Props {
  credential: StoredCredential;
}

export default function SimpleSend({ credential }: Props) {
  const [to, setTo] = useState('');
  const [amount, setAmount] = useState('');
  const [loading, setLoading] = useState(false);
  const [status, setStatus] = useState('');
  const [error, setError] = useState('');
  const [history, setHistory] = useState<TxResultType[]>([]);

  const handleSend = async () => {
    if (!to || !amount) {
      setError('Recipient and amount are required');
      return;
    }
    setLoading(true);
    setError('');
    try {
      setStatus('Building transaction...');
      const { sigHash } = await api.getSigHash('simple-send', {
        from: credential.address,
        to,
        amount,
      });

      setStatus('Sign with your passkey...');
      const signed = await signChallenge(credential.id, sigHash);

      setStatus('Submitting transaction...');
      const txResult = await api.simpleSend({
        address: credential.address,
        to,
        amount,
        signature: signed.signature,
        webauthn: signed.webauthn,
      });

      setHistory(prev => [txResult, ...prev]);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Transaction failed');
    } finally {
      setLoading(false);
      setStatus('');
    }
  };

  const executionState: ExecutionState = (() => {
    if (error && !loading) return { phase: 'error' as const, errorFrameIndex: 1 };
    if (!loading && history.length > 0 && history[0].success) return { phase: 'done' as const };
    if (status === 'Building transaction...') return { phase: 'executing' as const, activeFrameIndex: 0 };
    if (status === 'Sign with your passkey...') return { phase: 'executing' as const, activeFrameIndex: 0 };
    if (status === 'Submitting transaction...') return { phase: 'executing' as const, activeFrameIndex: 1 };
    return { phase: 'idle' as const };
  })();

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-8">
      <div>
        <h3 className="text-lg font-semibold text-zinc-100 mb-1">Simple Send</h3>
        <p className="text-sm text-zinc-500 mb-5">
          Send ETH from your passkey account to any address using a frame transaction.
        </p>

        <div className="space-y-4">
          <div>
            <label className="block text-xs text-zinc-400 mb-1.5">Recipient Address</label>
            <input
              type="text"
              placeholder="0x..."
              value={to}
              onChange={e => setTo(e.target.value)}
              className="w-full rounded-lg border border-zinc-700 bg-zinc-800/50 px-3 py-2 text-sm text-zinc-200 font-mono placeholder-zinc-600 focus:border-indigo-500 focus:outline-none"
            />
          </div>

          <div>
            <label className="block text-xs text-zinc-400 mb-1.5">Amount (ETH)</label>
            <input
              type="text"
              placeholder="0.1"
              value={amount}
              onChange={e => setAmount(e.target.value)}
              className="w-full rounded-lg border border-zinc-700 bg-zinc-800/50 px-3 py-2 text-sm text-zinc-200 font-mono placeholder-zinc-600 focus:border-indigo-500 focus:outline-none"
            />
          </div>

          <button
            onClick={handleSend}
            disabled={loading}
            className="w-full rounded-lg bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 disabled:cursor-not-allowed px-4 py-2.5 text-sm font-medium text-white transition-colors cursor-pointer"
          >
            {loading ? 'Processing...' : 'Send'}
          </button>
        </div>

        {status && (
          <div className="mt-3 flex items-center gap-2 rounded-lg border border-indigo-500/30 bg-indigo-950/30 px-3 py-2">
            <span className="inline-block h-2 w-2 rounded-full bg-indigo-400 animate-pulse" />
            <span className="text-sm text-indigo-300">{status}</span>
          </div>
        )}
        {error && <p className="text-sm text-red-400 mt-3">{error}</p>}
        {history.length > 0 && (
          <div className="mt-6 space-y-2">
            <h4 className="text-xs font-medium text-zinc-500 uppercase tracking-wider">Transaction History</h4>
            {history.map((r, i) => <TxResult key={i} result={r} />)}
          </div>
        )}
      </div>

      <div className="lg:pt-10">
        <FramePipeline frames={FRAMES} executionState={executionState} />
      </div>
    </div>
  );
}
