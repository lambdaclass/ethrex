import { useState, useRef, useCallback } from 'react';
import * as api from '../lib/api';
import type { EphemeralSendResult } from '../lib/api';
import TxResult from './TxResult';
import FramePipeline from './FramePipeline';
import type { FrameConfig, ExecutionState } from './FramePipeline';

const FRAMES: FrameConfig[] = [
  {
    mode: 'VERIFY',
    label: 'scope = 3',
    target: 'account',
    tooltip:
      'Calls verifyAndPay() on the EphemeralKeyAccount. Reads sig_hash via TXPARAMLOAD, resolves the current signer from the SignerRegistry, verifies the ECDSA signature via ecrecover, and calls APPROVE(scope=3) to authorize as sender and payer.',
  },
  {
    mode: 'SENDER',
    label: 'rotate()',
    target: 'registry',
    tooltip:
      'Calls execute(registry, 0, rotate(nextSigner)) on the account. Rotates the authorized signer in the SignerRegistry to the next ephemeral key. Runs before the operation so the rotation persists.',
  },
  {
    mode: 'SENDER',
    label: 'transfer()',
    target: 'account',
    tooltip:
      'Calls transfer(to, amount) on the account. Sends ETH to the recipient. Executes with msg.sender set to the account\'s own address.',
  },
];

interface EphemeralAccount {
  address: string;
  currentSigner: string;
  keyIndex: number;
}

interface HistoryEntry extends EphemeralSendResult {
  timestamp: number;
}

export default function EphemeralKeys() {
  const [account, setAccount] = useState<EphemeralAccount | null>(null);
  const [registering, setRegistering] = useState(false);
  const [registerStatus, setRegisterStatus] = useState('');
  const [to, setTo] = useState('');
  const [amount, setAmount] = useState('');
  const [loading, setLoading] = useState(false);
  const [status, setStatus] = useState('');
  const [error, setError] = useState('');
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const readerRef = useRef<ReadableStreamDefaultReader<Uint8Array> | null>(null);

  const handleRegister = useCallback(async () => {
    setRegistering(true);
    setRegisterStatus('Initializing...');
    setError('');

    try {
      const response = await api.ephemeralRegisterStream();
      const reader = response.body!.getReader();
      readerRef.current = reader;
      const decoder = new TextDecoder();
      let buffer = '';

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() ?? '';

        for (const line of lines) {
          if (line.startsWith('event: ')) {
            const event = line.slice(7).trim();
            if (event === 'error') {
              // Next data line has the error
            }
          } else if (line.startsWith('data: ')) {
            const data = JSON.parse(line.slice(6));

            if (data.message) {
              // Error event
              throw new Error(data.message);
            }

            if (data.step) {
              const stepLabels: Record<string, string> = {
                deploy: 'Deploying account contract...',
                fund: 'Funding with ETH...',
                'register-signer': 'Registering initial signer...',
                mint: 'Minting demo tokens...',
              };
              if (data.status === 'pending') {
                setRegisterStatus(stepLabels[data.step] ?? data.step);
              }
            }

            // Complete event
            if (data.address && data.currentSigner !== undefined) {
              setAccount({
                address: data.address,
                currentSigner: data.currentSigner,
                keyIndex: data.keyIndex,
              });
            }
          }
        }
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Registration failed');
    } finally {
      setRegistering(false);
      setRegisterStatus('');
      readerRef.current = null;
    }
  }, []);

  const handleSend = async () => {
    if (!account) return;
    if (!to || !amount) {
      setError('Recipient and amount are required');
      return;
    }
    setLoading(true);
    setError('');

    try {
      setStatus('Building & signing transaction...');
      const result = await api.ephemeralSend({
        address: account.address,
        to,
        amount,
      });

      setStatus('');

      // Update signer info
      if (result.newSigner && result.keyIndex !== undefined) {
        setAccount(prev => prev ? {
          ...prev,
          currentSigner: result.newSigner!,
          keyIndex: result.keyIndex!,
        } : prev);
      }

      setHistory(prev => [{ ...result, timestamp: Date.now() }, ...prev]);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Transaction failed');
    } finally {
      setLoading(false);
      setStatus('');
    }
  };

  const executionState: ExecutionState = (() => {
    if (error && !loading) return { phase: 'error' as const, errorFrameIndex: 2 };
    if (!loading && history.length > 0 && history[0].success) return { phase: 'done' as const };
    if (status) return { phase: 'executing' as const, activeFrameIndex: 1 };
    return { phase: 'idle' as const };
  })();

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-8">
      <div>
        <h3 className="text-lg font-semibold text-zinc-100 mb-1">Ephemeral Key Rotation</h3>
        <p className="text-sm text-zinc-500 mb-5">
          Send ETH using ephemeral ECDSA keys that rotate with every transaction.
          No biometrics needed — keys are derived from a server-side seed.
        </p>

        {!account ? (
          <div className="space-y-4">
            <div className="rounded-lg border border-zinc-800 bg-zinc-900/50 p-5">
              <p className="text-sm text-zinc-400 mb-4">
                Create an ephemeral key account to get started. This deploys a new
                account contract with ECDSA verification and registers the first
                signing key in the SignerRegistry.
              </p>
              <button
                onClick={handleRegister}
                disabled={registering}
                className="w-full rounded-lg bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 disabled:cursor-not-allowed px-4 py-2.5 text-sm font-medium text-white transition-colors cursor-pointer"
              >
                {registering ? 'Creating...' : 'Create Ephemeral Account'}
              </button>
              {registerStatus && (
                <div className="mt-3 flex items-center gap-2 rounded-lg border border-indigo-500/30 bg-indigo-950/30 px-3 py-2">
                  <span className="inline-block h-2 w-2 rounded-full bg-indigo-400 animate-pulse" />
                  <span className="text-sm text-indigo-300">{registerStatus}</span>
                </div>
              )}
              {error && <p className="text-sm text-red-400 mt-3">{error}</p>}
            </div>
          </div>
        ) : (
          <div className="space-y-4">
            {/* Account info */}
            <div className="rounded-lg border border-zinc-800 bg-zinc-900/50 p-4 space-y-2">
              <div>
                <span className="text-xs text-zinc-500">Account</span>
                <p className="text-xs text-zinc-300 font-mono truncate">{account.address}</p>
              </div>
              <div>
                <span className="text-xs text-zinc-500">Current Signer (key #{account.keyIndex})</span>
                <p className="text-xs text-emerald-400 font-mono truncate">{account.currentSigner}</p>
              </div>
            </div>

            {/* Send form */}
            <div>
              <label className="block text-xs text-zinc-400 mb-1.5">Recipient Address</label>
              <div className="flex gap-2">
                <input
                  type="text"
                  placeholder="0x..."
                  value={to}
                  onChange={e => setTo(e.target.value)}
                  className="flex-1 rounded-lg border border-zinc-700 bg-zinc-800/50 px-3 py-2 text-sm text-zinc-200 font-mono placeholder-zinc-600 focus:border-indigo-500 focus:outline-none"
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
              {loading ? 'Processing...' : 'Send + Rotate Key'}
            </button>

            {status && (
              <div className="flex items-center gap-2 rounded-lg border border-indigo-500/30 bg-indigo-950/30 px-3 py-2">
                <span className="inline-block h-2 w-2 rounded-full bg-indigo-400 animate-pulse" />
                <span className="text-sm text-indigo-300">{status}</span>
              </div>
            )}
            {error && <p className="text-sm text-red-400 mt-3">{error}</p>}

            {/* Transaction history with key rotation info */}
            {history.length > 0 && (
              <div className="mt-6 space-y-2">
                <h4 className="text-xs font-medium text-zinc-500 uppercase tracking-wider">Transaction History</h4>
                {history.map((entry, i) => (
                  <div key={i}>
                    {entry.oldSigner && entry.newSigner && (
                      <div className="rounded-t-lg border border-b-0 border-zinc-700 bg-zinc-900/50 px-3 py-2">
                        <span className="text-xs text-zinc-500">Key rotation: </span>
                        <span className="text-xs text-zinc-400 font-mono">{entry.oldSigner.slice(0, 10)}...</span>
                        <span className="text-xs text-zinc-500 mx-1">{'->'}</span>
                        <span className="text-xs text-emerald-400 font-mono">{entry.newSigner.slice(0, 10)}...</span>
                      </div>
                    )}
                    <TxResult result={entry} />
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </div>

      <div className="lg:pt-10">
        <FramePipeline frames={FRAMES} executionState={executionState} />
      </div>
    </div>
  );
}
