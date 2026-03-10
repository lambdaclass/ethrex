import { useState, useMemo } from 'react';
import type { StoredCredential } from '../lib/passkey';
import { signChallenge } from '../lib/passkey';
import * as api from '../lib/api';
import type { TxResult as TxResultType } from '../lib/api';
import TxResult from './TxResult';
import FramePipeline from './FramePipeline';
import type { FrameConfig, ExecutionState } from './FramePipeline';

interface Operation {
  to: string;
  value: string;
  data: string;
}

interface Props {
  credential: StoredCredential;
}

const emptyOp = (): Operation => ({ to: '', value: '', data: '' });

export default function BatchOps({ credential }: Props) {
  const [ops, setOps] = useState<Operation[]>([emptyOp()]);
  const [loading, setLoading] = useState(false);
  const [status, setStatus] = useState('');
  const [error, setError] = useState('');
  const [history, setHistory] = useState<TxResultType[]>([]);

  const updateOp = (index: number, field: keyof Operation, value: string) => {
    setOps(prev => prev.map((op, i) => (i === index ? { ...op, [field]: value } : op)));
  };

  const addOp = () => setOps(prev => [...prev, emptyOp()]);

  const removeOp = (index: number) => {
    if (ops.length <= 1) return;
    setOps(prev => prev.filter((_, i) => i !== index));
  };

  const handleExecute = async () => {
    const valid = ops.every(op => op.to);
    if (!valid) {
      setError('Each operation needs a target address');
      return;
    }
    setLoading(true);
    setError('');
    try {
      setStatus('Building transaction...');
      const { sigHash } = await api.getSigHash('batch-ops', {
        from: credential.address,
        operations: ops,
      });

      setStatus('Sign with your passkey...');
      const signed = await signChallenge(credential.id, sigHash);

      setStatus('Submitting transaction...');
      const txResult = await api.batchOps({
        address: credential.address,
        operations: ops,
        signature: signed.signature,
        webauthn: signed.webauthn,
      });

      setHistory(prev => [txResult, ...prev]);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Batch execution failed');
    } finally {
      setLoading(false);
      setStatus('');
    }
  };

  const frames: FrameConfig[] = useMemo(() => {
    const verifyFrame: FrameConfig = {
      mode: 'VERIFY',
      label: 'scope = 2',
      target: 'account',
      tooltip:
        'Calls verifyAndPay() on the account contract. Uses TXPARAMLOAD(0x08) to read the sig_hash, verifies the WebAuthn P256 signature, then calls APPROVE(scope=2) to authorize as both sender and gas payer.',
    };
    const senderFrames: FrameConfig[] = ops.map((_, i) => ({
      mode: 'SENDER' as const,
      label: `op #${i + 1}`,
      target: 'account',
      tooltip:
        'Calls execute(target, value, data) on the account. Routes the inner call through the account\'s execution logic.',
    }));
    return [verifyFrame, ...senderFrames];
  }, [ops.length]);

  const executionState: ExecutionState = (() => {
    if (error && !loading) return { phase: 'error' as const, errorFrameIndex: frames.length - 1 };
    if (!loading && history.length > 0 && history[0].success) return { phase: 'done' as const };
    if (status === 'Building transaction...') return { phase: 'executing' as const, activeFrameIndex: 0 };
    if (status === 'Sign with your passkey...') return { phase: 'executing' as const, activeFrameIndex: 0 };
    if (status === 'Submitting transaction...') return { phase: 'executing' as const, activeFrameIndex: Math.floor(frames.length / 2) };
    return { phase: 'idle' as const };
  })();

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-8">
      <div>
        <h3 className="text-lg font-semibold text-zinc-100 mb-1">Batch Operations</h3>
        <p className="text-sm text-zinc-500 mb-5">
          Execute multiple operations in a single frame transaction.
        </p>

        <div className="space-y-3 mb-4">
          {ops.map((op, i) => (
            <div
              key={i}
              className="rounded-lg border border-zinc-700/50 bg-zinc-800/30 p-3 space-y-2"
            >
              <div className="flex items-center justify-between mb-1">
                <span className="text-xs text-zinc-500 font-medium">
                  Operation {i + 1}
                </span>
                {ops.length > 1 && (
                  <button
                    onClick={() => removeOp(i)}
                    className="text-xs text-zinc-600 hover:text-red-400 transition-colors cursor-pointer"
                  >
                    Remove
                  </button>
                )}
              </div>
              <input
                type="text"
                placeholder="To address (0x...)"
                value={op.to}
                onChange={e => updateOp(i, 'to', e.target.value)}
                className="w-full rounded border border-zinc-700 bg-zinc-900/50 px-2.5 py-1.5 text-sm text-zinc-200 font-mono placeholder-zinc-600 focus:border-indigo-500 focus:outline-none"
              />
              <div className="flex gap-2">
                <input
                  type="text"
                  placeholder="Value (ETH)"
                  value={op.value}
                  onChange={e => updateOp(i, 'value', e.target.value)}
                  className="flex-1 rounded border border-zinc-700 bg-zinc-900/50 px-2.5 py-1.5 text-sm text-zinc-200 font-mono placeholder-zinc-600 focus:border-indigo-500 focus:outline-none"
                />
                <input
                  type="text"
                  placeholder="Calldata (0x...)"
                  value={op.data}
                  onChange={e => updateOp(i, 'data', e.target.value)}
                  className="flex-1 rounded border border-zinc-700 bg-zinc-900/50 px-2.5 py-1.5 text-sm text-zinc-200 font-mono placeholder-zinc-600 focus:border-indigo-500 focus:outline-none"
                />
              </div>
            </div>
          ))}
        </div>

        <div className="flex gap-3">
          <button
            onClick={addOp}
            className="rounded-lg border border-zinc-700 hover:border-zinc-500 px-3 py-2 text-sm text-zinc-400 hover:text-zinc-200 transition-colors cursor-pointer"
          >
            + Add Operation
          </button>
          <button
            onClick={handleExecute}
            disabled={loading}
            className="flex-1 rounded-lg bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 disabled:cursor-not-allowed px-4 py-2.5 text-sm font-medium text-white transition-colors cursor-pointer"
          >
            {loading ? 'Processing...' : `Execute Batch (${ops.length})`}
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
        <FramePipeline frames={frames} executionState={executionState} />
      </div>
    </div>
  );
}
